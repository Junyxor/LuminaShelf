use super::{
    download_control::{ControlledDownload, TransferControl},
    resumable_destination, AppState,
};
use lumina_core::{
    download::{DownloadConfig, DownloadProgress, DownloadState, SegmentedDownloader},
    BookFormat, LibraryItem, PersistedDownloadTask,
};
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DownloadReceipt {
    path: PathBuf,
    item: LibraryItem,
}

fn publish(app: &AppHandle, state: &AppState, task: &PersistedDownloadTask) -> Result<(), String> {
    let mut task = task.clone();
    task.updated_at_unix_ms = lumina_core::library::now_unix_ms();
    state
        .state_store
        .upsert_download_task(&task)
        .map_err(|error| error.to_string())?;
    let _ = app.emit("download-task-updated", &task);
    Ok(())
}

#[tauri::command]
pub(super) fn download_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<PersistedDownloadTask>, String> {
    state
        .state_store
        .list_download_tasks()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) fn remove_download_task(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
    let _lease = state.transfers.begin(&task_id)?;
    // Removing history never deletes the user's ebook or reusable partial data.
    state
        .state_store
        .remove_download_task(&task_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(super) async fn pause_download(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
    state
        .transfers
        .stop(&task_id, TransferControl::Paused)
        .await
}

#[tauri::command]
pub(super) async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
    if state
        .transfers
        .stop(&task_id, TransferControl::Cancelled)
        .await?
    {
        return Ok(true);
    }
    let _lease = state.transfers.begin(&task_id)?;
    if let Some(mut task) = state
        .state_store
        .list_download_tasks()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|task| task.id == task_id)
    {
        if task.state == DownloadState::Completed {
            return Ok(false);
        }
        task.state = DownloadState::Cancelled;
        task.error = None;
        publish(&app, &state, &task)?;
        return Ok(true);
    }
    Ok(false)
}

#[tauri::command]
pub(super) async fn download_book(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
    download_dir: Option<String>,
) -> Result<DownloadReceipt, String> {
    let task_id = format!("{provider_id}:{book_id}:{}", format.extension());
    // Claim before acquiring a URL or changing persisted state, including connecting.
    let mut lease = state.transfers.begin(&task_id)?;
    let existing = state
        .state_store
        .list_download_tasks()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|task| task.id == task_id);
    let mut task = match existing {
        Some(task) => task,
        None => {
            let directory = destination_dir(&app, download_dir)?;
            let mut destination = resumable_destination(
                &directory,
                &title,
                format.extension(),
                &provider_id,
                &book_id,
            );
            // Never overwrite an untracked user file.
            if destination.exists() {
                destination = directory.join(format!(
                    "{}-{}.{}",
                    super::sanitize_filename(&title),
                    uuid::Uuid::new_v4(),
                    format.extension()
                ));
            }
            PersistedDownloadTask::new(
                task_id.clone(),
                provider_id,
                book_id,
                title,
                format,
                destination,
            )
        }
    };
    let staging = PathBuf::from(format!("{}.lumina-download", task.destination.display()));
    let legacy_manifest = PathBuf::from(format!("{}.lumina-part.json", task.destination.display()));
    let recovered_final = !staging.exists()
        && !legacy_manifest.exists()
        && task.total_bytes.is_some_and(|total| {
            total > 0
                && task.downloaded_bytes == total
                && std::fs::metadata(&task.destination)
                    .is_ok_and(|metadata| metadata.len() == total)
        });
    if task.destination.is_file() && (task.state == DownloadState::Completed || recovered_final) {
        lumina_core::library::validate_ebook(
            &task.destination,
            lumina_core::LibraryFormat::from_path(&task.destination),
        )
        .map_err(|error| error.to_string())?;
        let item = inspect_download(&task)?;
        state
            .state_store
            .upsert_library_item(&item)
            .map_err(|error| error.to_string())?;
        task.state = DownloadState::Completed;
        publish(&app, &state, &task)?;
        let _ = app.emit("library-updated", ());
        return Ok(DownloadReceipt {
            path: task.destination,
            item,
        });
    }
    // Upgrade partial files created by the older direct-to-destination downloader.
    if task.destination.exists() && !staging.exists() && task.state != DownloadState::Completed {
        std::fs::rename(&task.destination, &staging).map_err(|error| error.to_string())?;
        if legacy_manifest.exists() {
            std::fs::rename(
                &legacy_manifest,
                format!("{}.lumina-part.json", staging.display()),
            )
            .map_err(|error| error.to_string())?;
        }
    }
    task.state = DownloadState::Connecting;
    task.error = None;
    publish(&app, &state, &task)?;
    let (tx, mut rx) = tokio::sync::watch::channel(DownloadProgress {
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
    });
    let transfer = async {
        let url = state
            .providers
            .acquisition_url(&task.provider_id, &task.book_id, task.format)
            .await?;
        let headers = state.providers.download_headers(&task.provider_id).await?;
        let downloader =
            SegmentedDownloader::with_resolver(state.resolver.clone(), DownloadConfig::default())?;
        downloader
            .download_to_with_context(url, staging.clone(), headers, Some(task_id.clone()), tx)
            .await
    };
    let mut last_persisted = std::time::Instant::now();
    let outcome = {
        let controlled = lease.run(transfer);
        tokio::pin!(controlled);
        loop {
            tokio::select! {
                result = &mut controlled => break result,
                changed = rx.changed(), if rx.has_changed().is_ok() => {
                    if changed.is_ok() {
                        let progress = *rx.borrow_and_update();
                        if last_persisted.elapsed() >= std::time::Duration::from_millis(200) {
                            let mut snapshot = task.clone();
                            snapshot.state = DownloadState::Downloading;
                            snapshot.downloaded_bytes = progress.downloaded_bytes;
                            snapshot.total_bytes = progress.total_bytes;
                            // Keep cleanup reachable even if persistence temporarily fails.
                            let _ = publish(&app, &state, &snapshot);
                            last_persisted = std::time::Instant::now();
                        }
                    }
                }
            }
        }
    };
    // Dropping a segmented transfer aborts its children. Drain their senders before
    // releasing the lease so no old attempt can write progress after a new one.
    while rx.changed().await.is_ok() {}
    let final_progress = *rx.borrow();
    task.downloaded_bytes = final_progress.downloaded_bytes;
    task.total_bytes = final_progress.total_bytes;
    match outcome {
        ControlledDownload::Paused | ControlledDownload::Cancelled => {
            let paused = matches!(outcome, ControlledDownload::Paused);
            task.state = if paused {
                DownloadState::Paused
            } else {
                DownloadState::Cancelled
            };
            publish(&app, &state, &task)?;
            return Err(if paused {
                "__LUMINA_PAUSED__"
            } else {
                "__LUMINA_CANCELLED__"
            }
            .into());
        }
        ControlledDownload::Finished(Err(error)) => {
            task.state = DownloadState::Failed;
            // Network errors may include temporary URLs. Do not persist their query strings.
            task.error = Some("下载失败，请检查网络和账户后重试。".into());
            publish(&app, &state, &task)?;
            let _ = error;
            return Err(task.error.unwrap());
        }
        ControlledDownload::Finished(Ok(())) => {}
    }
    // Finalization is deliberately outside the cancellable transfer.
    task.state = DownloadState::Verifying;
    publish(&app, &state, &task)?;
    let finalized = async {
        let publish_staging = staging.clone();
        let publish_destination = task.destination.clone();
        tauri::async_runtime::spawn_blocking(move || {
            lumina_core::library::publish_download(&publish_staging, &publish_destination)
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|error| error.to_string())??;
        let item = inspect_download(&task)?;
        state
            .state_store
            .upsert_library_item(&item)
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(item)
    }
    .await;
    match finalized {
        Ok(item) => {
            task.state = DownloadState::Completed;
            task.downloaded_bytes = item.size_bytes;
            task.total_bytes = Some(item.size_bytes);
            publish(&app, &state, &task)?;
            let _ = app.emit("library-updated", ());
            Ok(DownloadReceipt {
                path: task.destination,
                item,
            })
        }
        Err(error) => {
            task.state = DownloadState::Failed;
            task.error = Some(error.clone());
            publish(&app, &state, &task)?;
            Err(error)
        }
    }
}

fn inspect_download(task: &PersistedDownloadTask) -> Result<LibraryItem, String> {
    LibraryItem::inspect(
        &task.destination,
        Some(&task.title),
        Some(task.provider_id.clone()),
        Some(task.book_id.clone()),
    )
    .map_err(|error| error.to_string())
}

fn destination_dir(app: &AppHandle, requested: Option<String>) -> Result<PathBuf, String> {
    #[cfg(target_os = "android")]
    {
        let _ = requested;
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("library"))
    }
    #[cfg(not(target_os = "android"))]
    {
        if let Some(path) = requested.filter(|path| !path.trim().is_empty()) {
            return Ok(PathBuf::from(path.trim()));
        }
        Ok(app
            .path()
            .download_dir()
            .map_err(|error| error.to_string())?
            .join("LuminaShelf"))
    }
}

use super::*;
use lumina_core::download::{DownloadSettings, DownloadState, DownloadTask};

#[cfg(target_os = "android")]
struct AndroidDownloads(tauri::plugin::PluginHandle<tauri::Wry>);
#[cfg(target_os = "android")]
static ANDROID_MANAGER: std::sync::OnceLock<DownloadManager> = std::sync::OnceLock::new();

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("lumina-downloads")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                app.manage(AndroidDownloads(api.register_android_plugin(
                    "app.luminashelf.client",
                    "DownloadPlugin",
                )?));
            }
            #[cfg(not(target_os = "android"))]
            let _ = (app, api);
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_app_luminashelf_client_DownloadNative_pauseAll(
    _env: *mut jni::sys::JNIEnv,
    _class: jni::sys::jclass,
) {
    if let Some(manager) = ANDROID_MANAGER.get() {
        let manager = manager.clone();
        tauri::async_runtime::spawn(async move {
            let _ = manager.pause_all().await;
        });
    }
}

async fn background(app: &AppHandle, active: bool) {
    #[cfg(target_os = "android")]
    {
        let plugin = app.state::<AndroidDownloads>().0.clone();
        if plugin
            .run_mobile_plugin_async::<()>("setActive", serde_json::json!({"active": active}))
            .await
            .is_err()
            && active
        {
            let _ = app.emit(
                "download-background-warning",
                "后台保活未能启动，请暂时保持应用打开；下载进度仍会保存",
            );
        }
    }
    #[cfg(not(target_os = "android"))]
    let _ = (app, active);
}

pub fn observe(app: AppHandle, manager: DownloadManager) {
    #[cfg(target_os = "android")]
    let _ = ANDROID_MANAGER.set(manager.clone());
    let mut events = manager.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut background_active = false;
        loop {
            let task = match events.recv().await {
                Ok(task) => task,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            let _ = app.emit("download-task", &task);
            let active = manager.active_count().await > 0;
            if active != background_active {
                background(&app, active).await;
                background_active = active;
            }
        }
    });
}

#[tauri::command]
pub async fn download_tasks(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    state.downloads.list().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn download_settings(state: State<'_, AppState>) -> Result<DownloadSettings, String> {
    Ok(state.downloads.settings().await)
}
#[tauri::command]
pub async fn save_download_settings(
    state: State<'_, AppState>,
    settings: DownloadSettings,
) -> Result<DownloadSettings, String> {
    state
        .downloads
        .set_settings(settings)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn enqueue_download_book(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
    download_dir: Option<String>,
) -> Result<DownloadTask, String> {
    let directory = match download_dir.filter(|s| !s.trim().is_empty()) {
        Some(path) => PathBuf::from(path),
        None => app
            .path()
            .download_dir()
            .map_err(|e| e.to_string())?
            .join("LuminaShelf"),
    };
    // A visible user action starts dataSync before link lookup, without runtime permission dialogs.
    background(&app, true).await;
    let result = state
        .downloads
        .enqueue(provider_id, book_id, title, format, directory)
        .await
        .map_err(|e| e.to_string());
    if state.downloads.active_count().await == 0 {
        background(&app, false).await;
    }
    result
}
#[tauri::command]
pub async fn pause_download(
    state: State<'_, AppState>,
    id: String,
) -> Result<DownloadTask, String> {
    state.downloads.pause(&id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn resume_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<DownloadTask, String> {
    background(&app, true).await;
    let result = state.downloads.resume(&id).await.map_err(|e| e.to_string());
    if state.downloads.active_count().await == 0 {
        background(&app, false).await;
    }
    result
}
#[tauri::command]
pub async fn restart_download(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<DownloadTask, String> {
    background(&app, true).await;
    let result = state
        .downloads
        .restart(&id)
        .await
        .map_err(|e| e.to_string());
    if state.downloads.active_count().await == 0 {
        background(&app, false).await;
    }
    result
}
#[tauri::command]
pub async fn cancel_download(
    state: State<'_, AppState>,
    id: String,
) -> Result<DownloadTask, String> {
    state.downloads.cancel(&id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn pause_all_downloads(state: State<'_, AppState>) -> Result<(), String> {
    state.downloads.pause_all().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn remove_download(
    state: State<'_, AppState>,
    id: String,
    delete_file: bool,
) -> Result<(), String> {
    state
        .downloads
        .remove(&id, delete_file)
        .await
        .map_err(|e| e.to_string())
}

pub(super) async fn legacy_receipt(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
    download_dir: Option<String>,
) -> Result<DownloadReceipt, String> {
    let task = enqueue_download_book(
        app,
        state.clone(),
        provider_id,
        book_id,
        title,
        format,
        download_dir,
    )
    .await?;
    let completed = state
        .downloads
        .wait(&task.id)
        .await
        .map_err(|e| e.to_string())?;
    if completed.state != DownloadState::Completed {
        return Err(completed
            .error_details
            .or(completed.error)
            .unwrap_or_else(|| "下载已暂停，进度已保存".into()));
    }
    let item = state
        .state_store
        .list_library_items()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|item| Some(&item.id) == completed.library_id.as_ref())
        .ok_or("下载文件未入库")?;
    Ok(DownloadReceipt {
        path: item.path.clone(),
        item,
    })
}

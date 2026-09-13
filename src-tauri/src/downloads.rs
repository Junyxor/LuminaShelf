use super::{unique_destination, AppState};
use lumina_core::{
    download::{DownloadConfig, DownloadProgress, DownloadState, DownloadTask, SegmentedDownloader},
    library::now_unix_ms,
    AppResolver, BookFormat, LibraryItem, ProviderRegistry, StateStore,
};
use reqwest::header::HeaderMap;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::{sync::Mutex, task::AbortHandle};
use url::Url;

static DOWNLOAD_MANAGER: OnceLock<DownloadManager> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTaskSnapshot {
    pub id: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub book_id: Option<String>,
    pub destination: String,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub bytes_per_second: f64,
    pub error: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

impl From<&DownloadTask> for DownloadTaskSnapshot {
    fn from(task: &DownloadTask) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            provider_id: task.provider_id.clone(),
            book_id: task.book_id.clone(),
            destination: task.destination.to_string_lossy().into_owned(),
            state: task.state,
            total_bytes: task.total_bytes,
            downloaded_bytes: task.downloaded_bytes,
            bytes_per_second: task.bytes_per_second,
            error: task.error.clone(),
            created_at_unix_ms: task.created_at_unix_ms,
            updated_at_unix_ms: task.updated_at_unix_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadCompletedEvent {
    task: DownloadTaskSnapshot,
    item: LibraryItem,
}

#[derive(Clone)]
pub struct DownloadManager {
    store: StateStore,
    providers: ProviderRegistry,
    resolver: AppResolver,
    active: Arc<Mutex<HashMap<String, AbortHandle>>>,
}

impl DownloadManager {
    pub fn new(store: StateStore, providers: ProviderRegistry, resolver: AppResolver) -> Self {
        Self {
            store,
            providers,
            resolver,
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn recover_interrupted(&self) -> Result<usize, String> {
        self.store
            .pause_interrupted_downloads()
            .map_err(|error| error.to_string())
    }

    pub fn list(&self) -> Result<Vec<DownloadTaskSnapshot>, String> {
        self.store
            .list_download_tasks()
            .map(|tasks| tasks.iter().map(DownloadTaskSnapshot::from).collect())
            .map_err(|error| error.to_string())
    }

    pub async fn enqueue(
        &self,
        app: AppHandle,
        task: DownloadTask,
    ) -> Result<DownloadTaskSnapshot, String> {
        self.store
            .upsert_download_task(&task)
            .map_err(|error| error.to_string())?;
        let snapshot = DownloadTaskSnapshot::from(&task);
        self.spawn(app, task).await;
        Ok(snapshot)
    }

    pub async fn pause(&self, app: &AppHandle, id: &str) -> Result<DownloadTaskSnapshot, String> {
        if let Some(handle) = self.active.lock().await.remove(id) {
            handle.abort();
        }
        let mut task = self.require_task(id)?;
        if task.state == DownloadState::Completed || task.state == DownloadState::Cancelled {
            return Ok(DownloadTaskSnapshot::from(&task));
        }
        task.state = DownloadState::Paused;
        task.bytes_per_second = 0.0;
        task.error = None;
        task.updated_at_unix_ms = now_unix_ms();
        self.persist_and_emit(app, &task)?;
        Ok(DownloadTaskSnapshot::from(&task))
    }

    pub async fn resume(&self, app: AppHandle, id: &str) -> Result<DownloadTaskSnapshot, String> {
        if self.active.lock().await.contains_key(id) {
            return self.snapshot(id);
        }
        let mut task = self.require_task(id)?;
        if task.state == DownloadState::Completed || task.state == DownloadState::Cancelled {
            return Ok(DownloadTaskSnapshot::from(&task));
        }
        task.state = DownloadState::Queued;
        task.bytes_per_second = 0.0;
        task.error = None;
        task.updated_at_unix_ms = now_unix_ms();
        self.store
            .upsert_download_task(&task)
            .map_err(|error| error.to_string())?;
        let snapshot = DownloadTaskSnapshot::from(&task);
        self.spawn(app, task).await;
        Ok(snapshot)
    }

    pub async fn cancel(&self, app: &AppHandle, id: &str) -> Result<DownloadTaskSnapshot, String> {
        if let Some(handle) = self.active.lock().await.remove(id) {
            handle.abort();
        }
        let mut task = self.require_task(id)?;
        task.state = DownloadState::Cancelled;
        task.bytes_per_second = 0.0;
        task.error = None;
        task.updated_at_unix_ms = now_unix_ms();
        self.persist_and_emit(app, &task)?;
        let _ = tokio::fs::remove_file(&task.destination).await;
        let _ = tokio::fs::remove_file(manifest_path(&task.destination)).await;
        Ok(DownloadTaskSnapshot::from(&task))
    }

    pub async fn retry(&self, app: AppHandle, id: &str) -> Result<DownloadTaskSnapshot, String> {
        if self.active.lock().await.contains_key(id) {
            return self.snapshot(id);
        }
        let mut task = self.require_task(id)?;
        if task.state == DownloadState::Completed {
            return Ok(DownloadTaskSnapshot::from(&task));
        }
        if task.state == DownloadState::Cancelled {
            task.downloaded_bytes = 0;
            task.total_bytes = None;
        }
        task.state = DownloadState::Queued;
        task.bytes_per_second = 0.0;
        task.error = None;
        task.updated_at_unix_ms = now_unix_ms();
        self.store
            .upsert_download_task(&task)
            .map_err(|error| error.to_string())?;
        let snapshot = DownloadTaskSnapshot::from(&task);
        self.spawn(app, task).await;
        Ok(snapshot)
    }

    pub async fn remove(&self, id: &str) -> Result<bool, String> {
        if let Some(handle) = self.active.lock().await.remove(id) {
            handle.abort();
        }
        self.store
            .remove_download_task(id)
            .map_err(|error| error.to_string())
    }

    fn snapshot(&self, id: &str) -> Result<DownloadTaskSnapshot, String> {
        self.require_task(id)
            .map(|task| DownloadTaskSnapshot::from(&task))
    }

    fn require_task(&self, id: &str) -> Result<DownloadTask, String> {
        self.store
            .download_task(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("download task not found: {id}"))
    }

    async fn spawn(&self, app: AppHandle, mut task: DownloadTask) {
        let manager = self.clone();
        let task_id = task.id.clone();
        let handle = tokio::spawn(async move {
            let result = manager.run_task(&app, &mut task).await;
            if let Err(error) = result {
                task.state = DownloadState::Failed;
                task.bytes_per_second = 0.0;
                task.error = Some(error);
                task.updated_at_unix_ms = now_unix_ms();
                let _ = manager.persist_and_emit(&app, &task);
            }
        });
        let abort = handle.abort_handle();
        self.active.lock().await.insert(task_id.clone(), abort);
        let active = self.active.clone();
        tokio::spawn(async move {
            let _ = handle.await;
            active.lock().await.remove(&task_id);
        });
    }

    async fn run_task(&self, app: &AppHandle, task: &mut DownloadTask) -> Result<(), String> {
        task.state = DownloadState::Connecting;
        task.error = None;
        task.updated_at_unix_ms = now_unix_ms();
        self.persist_and_emit(app, task)?;

        let (url, headers) = self.fresh_request(task).await?;
        task.url = url.clone();

        let downloader = SegmentedDownloader::with_resolver(
            self.resolver.clone(),
            DownloadConfig::default(),
        )
        .map_err(|error| error.to_string())?;
        let (tx, mut rx) = tokio::sync::watch::channel(DownloadProgress::default());
        let future = downloader.download_to_with_context(
            url,
            task.destination.clone(),
            headers,
            task.provider_id
                .as_ref()
                .zip(task.book_id.as_ref())
                .map(|(provider, book)| format!("{provider}:{book}")),
            tx,
        );
        tokio::pin!(future);

        let mut last_progress_at = Instant::now();
        let mut last_progress_bytes = task.downloaded_bytes;
        loop {
            tokio::select! {
                result = &mut future => {
                    result.map_err(|error| error.to_string())?;
                    break;
                }
                changed = rx.changed() => {
                    if changed.is_err() {
                        continue;
                    }
                    let progress = *rx.borrow_and_update();
                    let elapsed = last_progress_at.elapsed().as_secs_f64();
                    if elapsed >= 0.25 {
                        let delta = progress.downloaded_bytes.saturating_sub(last_progress_bytes);
                        task.bytes_per_second = delta as f64 / elapsed.max(0.001);
                        last_progress_at = Instant::now();
                        last_progress_bytes = progress.downloaded_bytes;
                    }
                    task.state = DownloadState::Downloading;
                    task.downloaded_bytes = progress.downloaded_bytes;
                    task.total_bytes = progress.total_bytes;
                    task.updated_at_unix_ms = now_unix_ms();
                    self.persist_and_emit(app, task)?;
                }
            }
        }

        task.state = DownloadState::Verifying;
        task.bytes_per_second = 0.0;
        task.updated_at_unix_ms = now_unix_ms();
        self.persist_and_emit(app, task)?;

        let item = LibraryItem::inspect(
            &task.destination,
            Some(&task.title),
            task.provider_id.clone(),
            task.book_id.clone(),
        )
        .map_err(|error| error.to_string())?;

        task.state = DownloadState::Completed;
        if let Ok(metadata) = tokio::fs::metadata(&task.destination).await {
            task.downloaded_bytes = metadata.len();
            task.total_bytes = Some(metadata.len());
        }
        task.updated_at_unix_ms = now_unix_ms();
        self.persist_and_emit(app, task)?;
        let _ = app.emit(
            "download-completed",
            DownloadCompletedEvent {
                task: DownloadTaskSnapshot::from(&*task),
                item,
            },
        );
        Ok(())
    }

    async fn fresh_request(&self, task: &DownloadTask) -> Result<(Url, HeaderMap), String> {
        match (task.provider_id.as_deref(), task.book_id.as_deref()) {
            (Some(provider_id), Some(book_id)) => {
                let format = task
                    .destination
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(BookFormat::parse)
                    .unwrap_or(BookFormat::Other);
                let url = self
                    .providers
                    .acquisition_url(provider_id, book_id, format)
                    .await
                    .map_err(|error| error.to_string())?;
                let headers = self
                    .providers
                    .download_headers(provider_id)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok((url, headers))
            }
            _ => Ok((task.url.clone(), HeaderMap::new())),
        }
    }

    fn persist_and_emit(&self, app: &AppHandle, task: &DownloadTask) -> Result<(), String> {
        self.store
            .upsert_download_task(task)
            .map_err(|error| error.to_string())?;
        let _ = app.emit("download-task-updated", DownloadTaskSnapshot::from(task));
        Ok(())
    }
}

fn manager(app: &AppHandle, state: &State<'_, AppState>) -> Result<&'static DownloadManager, String> {
    if let Some(manager) = DOWNLOAD_MANAGER.get() {
        return Ok(manager);
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
    let store = StateStore::open(config_dir.join("state.sqlite3"))
        .map_err(|error| error.to_string())?;
    let manager = DownloadManager::new(store, state.providers.clone(), state.resolver.clone());
    manager.recover_interrupted()?;
    let _ = DOWNLOAD_MANAGER.set(manager);
    DOWNLOAD_MANAGER
        .get()
        .ok_or_else(|| "failed to initialize download manager".to_string())
}

#[tauri::command]
pub fn list_downloads(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<DownloadTaskSnapshot>, String> {
    manager(&app, &state)?.list()
}

#[tauri::command]
pub async fn enqueue_download(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
    download_dir: Option<String>,
) -> Result<DownloadTaskSnapshot, String> {
    let system_download_dir = app
        .path()
        .download_dir()
        .map_err(|error| error.to_string())?;
    let destination_dir = download_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| system_download_dir.join("LuminaShelf"));
    let destination = unique_destination(&destination_dir, &title, format.extension());
    let placeholder = Url::parse("about:blank").expect("valid placeholder URL");
    let task = DownloadTask::new(
        title,
        placeholder,
        destination,
        Some(provider_id),
        Some(book_id),
    );
    manager(&app, &state)?.enqueue(app, task).await
}

#[tauri::command]
pub async fn pause_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<DownloadTaskSnapshot, String> {
    manager(&app, &state)?.pause(&app, &task_id).await
}

#[tauri::command]
pub async fn resume_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<DownloadTaskSnapshot, String> {
    manager(&app, &state)?.resume(app, &task_id).await
}

#[tauri::command]
pub async fn cancel_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<DownloadTaskSnapshot, String> {
    manager(&app, &state)?.cancel(&app, &task_id).await
}

#[tauri::command]
pub async fn retry_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<DownloadTaskSnapshot, String> {
    manager(&app, &state)?.retry(app, &task_id).await
}

#[tauri::command]
pub async fn remove_download(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<bool, String> {
    manager(&app, &state)?.remove(&task_id).await
}

fn manifest_path(destination: &Path) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}.lumina-part.json", destination.display()))
}

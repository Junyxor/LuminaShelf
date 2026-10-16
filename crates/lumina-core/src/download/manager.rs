use super::*;
use crate::{
    library::{validate_readable_file, LibraryItem},
    ProviderRegistry, StateStore,
};
use anyhow::{anyhow, ensure, Context};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io::Read,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DownloadSettings {
    pub connections: usize,
    pub simultaneous_books: usize,
}
impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            connections: 4,
            simultaneous_books: 2,
        }
    }
}
struct Running {
    token: Uuid,
    control: DownloadControl,
    done: watch::Receiver<bool>,
}
struct Queue {
    tasks: BTreeMap<String, DownloadTask>,
    running: HashMap<String, Running>,
    settings: DownloadSettings,
    busy: HashSet<String>,
    needs_recovery: bool,
}
struct Inner {
    store: StateStore,
    providers: ProviderRegistry,
    resolver: AppResolver,
    queue: Mutex<Queue>,
    events: broadcast::Sender<DownloadTask>,
}
#[derive(Clone)]
pub struct DownloadManager(Arc<Inner>);

fn clean_name(value: &str) -> String {
    let value = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>();
    let value = value.trim_matches([' ', '.']);
    if value.is_empty() {
        "book".into()
    } else {
        value.chars().take(120).collect()
    }
}
pub fn destination_for(
    directory: &Path,
    title: &str,
    extension: &str,
    provider: &str,
    book: &str,
) -> PathBuf {
    let title = clean_name(title).chars().take(80).collect::<String>();
    let identity = clean_name(&format!("{provider}-{book}"))
        .chars()
        .take(36)
        .collect::<String>();
    directory.join(format!("{title} [{identity}].{extension}"))
}
impl DownloadManager {
    pub fn new(
        store: StateStore,
        providers: ProviderRegistry,
        resolver: AppResolver,
    ) -> Result<Self> {
        let settings = store
            .setting("download-settings")?
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let mut tasks = BTreeMap::new();
        for mut task in store.download_tasks()? {
            if task.state.is_active() {
                task.state = DownloadState::Paused;
                task.note = Some("上次下载已中断，已保留进度，点击继续下载".into());
                task.active_connections = 0;
                task.bytes_per_second = 0.0;
                task.revision += 1;
                store.save_download_task(&task)?;
            }
            tasks.insert(task.id.clone(), task);
        }
        Ok(Self(Arc::new(Inner {
            store,
            providers,
            resolver,
            queue: Mutex::new(Queue {
                tasks,
                running: HashMap::new(),
                settings,
                busy: HashSet::new(),
                needs_recovery: true,
            }),
            events: broadcast::channel(128).0,
        })))
    }
    pub fn subscribe(&self) -> broadcast::Receiver<DownloadTask> {
        self.0.events.subscribe()
    }
    fn save(&self, task: &mut DownloadTask) -> Result<()> {
        task.updated_at_unix_ms = now_unix_ms();
        task.revision += 1;
        self.0.store.save_download_task(task)?;
        let _ = self.0.events.send(task.clone());
        Ok(())
    }
    pub async fn list(&self) -> Result<Vec<DownloadTask>> {
        let mut queue = self.0.queue.lock().await;
        if queue.needs_recovery {
            for task in queue
                .tasks
                .values_mut()
                .filter(|task| !task.state.is_active() && task.state != DownloadState::Completed)
            {
                let bytes = partial_progress(&task.destination).await;
                if bytes != task.downloaded_bytes {
                    task.downloaded_bytes = bytes;
                    self.save(task)?;
                }
            }
            queue.needs_recovery = false;
        }
        let mut tasks = queue.tasks.values().cloned().collect::<Vec<_>>();
        tasks.sort_by(|a, b| {
            b.created_at_unix_ms
                .cmp(&a.created_at_unix_ms)
                .then(a.id.cmp(&b.id))
        });
        Ok(tasks)
    }
    pub async fn task(&self, id: &str) -> Result<DownloadTask> {
        self.0
            .queue
            .lock()
            .await
            .tasks
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("下载任务不存在"))
    }
    pub async fn active_count(&self) -> usize {
        self.0
            .queue
            .lock()
            .await
            .tasks
            .values()
            .filter(|task| task.state.is_active())
            .count()
    }
    pub async fn settings(&self) -> DownloadSettings {
        self.0.queue.lock().await.settings.clone()
    }
    pub async fn set_settings(&self, settings: DownloadSettings) -> Result<DownloadSettings> {
        ensure!(
            (1..=8).contains(&settings.connections),
            "单文件连接数应为 1–8"
        );
        ensure!(
            (1..=3).contains(&settings.simultaneous_books),
            "同时下载书籍应为 1–3"
        );
        self.0
            .store
            .set_setting("download-settings", &serde_json::to_string(&settings)?)?;
        self.0.queue.lock().await.settings = settings.clone();
        self.schedule();
        Ok(settings)
    }
    pub async fn enqueue(
        &self,
        provider_id: String,
        book_id: String,
        title: String,
        format: BookFormat,
        directory: PathBuf,
    ) -> Result<DownloadTask> {
        ensure!(
            self.0
                .providers
                .descriptors()
                .iter()
                .any(|p| p.id == provider_id),
            "书源不存在"
        );
        let mut queue = self.0.queue.lock().await;
        let mut task = DownloadTask::new(
            title.clone(),
            provider_id.clone(),
            book_id.clone(),
            format,
            destination_for(
                &directory,
                &title,
                format.extension(),
                &provider_id,
                &book_id,
            ),
            queue.settings.connections,
        );
        ensure!(!queue.busy.contains(&task.id), "正在处理此任务，请稍后再试");
        if let Some(existing) = queue.tasks.get(&task.id) {
            if existing.state.is_active() {
                return Ok(existing.clone());
            }
            task = existing.clone();
            if task.state == DownloadState::Completed && self.reattach(&mut task).is_ok() {
                self.save(&mut task)?;
                queue.tasks.insert(task.id.clone(), task.clone());
                return Ok(task);
            }
            task.connections = queue.settings.connections;
            task.state = DownloadState::Queued;
            task.error = None;
            task.error_details = None;
        } else if let Some(item) = self.0.store.list_library_items()?.into_iter().find(|item| {
            item.source_provider.as_deref() == Some(&provider_id)
                && item.source_book_id.as_deref() == Some(&book_id)
                && item.format.as_str() == format.extension()
                && validate_readable_file(&item.path).is_ok()
        }) {
            task.state = DownloadState::Completed;
            task.downloaded_bytes = item.size_bytes;
            task.total_bytes = Some(item.size_bytes);
            task.destination = item.path;
            task.library_id = Some(item.id);
        }
        task.downloaded_bytes = partial_progress(&task.destination).await;
        self.save(&mut task)?;
        queue.tasks.insert(task.id.clone(), task.clone());
        drop(queue);
        self.schedule();
        Ok(task)
    }
    fn schedule(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut queue = this.0.queue.lock().await;
            while queue.running.len() < queue.settings.simultaneous_books {
                let next = queue
                    .tasks
                    .values()
                    .filter(|task| {
                        task.state == DownloadState::Queued && !queue.busy.contains(&task.id)
                    })
                    .min_by_key(|task| task.created_at_unix_ms)
                    .map(|task| task.id.clone());
                let Some(id) = next else { break };
                let task = queue.tasks.get_mut(&id).unwrap();
                task.state = DownloadState::Connecting;
                if let Err(error) = this.save(task) {
                    task.state = DownloadState::Failed;
                    task.error = Some(error.to_string());
                    break;
                }
                let task = task.clone();
                let token = Uuid::new_v4();
                let control = DownloadControl::default();
                let (done, receiver) = watch::channel(false);
                queue.running.insert(
                    id,
                    Running {
                        token,
                        control: control.clone(),
                        done: receiver,
                    },
                );
                let worker = this.clone();
                tokio::spawn(async move {
                    worker.run(task, token, control, done).await;
                    worker.schedule();
                });
            }
        });
    }
    pub async fn resume(&self, id: &str) -> Result<DownloadTask> {
        let mut queue = self.0.queue.lock().await;
        let connections = queue.settings.connections;
        ensure!(!queue.busy.contains(id), "正在处理此任务，请稍后再试");
        let task = queue
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("下载任务不存在"))?;
        if task.state.is_active() {
            return Ok(task.clone());
        }
        if task.state == DownloadState::Completed && self.reattach(task).is_ok() {
            self.save(task)?;
            return Ok(task.clone());
        }
        task.state = DownloadState::Queued;
        task.connections = connections;
        task.error = None;
        task.error_details = None;
        task.downloaded_bytes = partial_progress(&task.destination).await;
        self.save(task)?;
        let result = task.clone();
        drop(queue);
        self.schedule();
        Ok(result)
    }
    fn reattach(&self, task: &mut DownloadTask) -> Result<()> {
        validate_readable_file(&task.destination)?;
        let item = LibraryItem::inspect(
            &task.destination,
            Some(&task.title),
            Some(task.provider_id.clone()),
            Some(task.book_id.clone()),
        )?;
        self.0.store.upsert_library_item(&item)?;
        task.library_id = Some(item.id);
        task.downloaded_bytes = item.size_bytes;
        task.total_bytes = Some(item.size_bytes);
        Ok(())
    }
    pub async fn pause(&self, id: &str) -> Result<DownloadTask> {
        {
            let mut queue = self.0.queue.lock().await;
            ensure!(
                queue.busy.insert(id.to_string()),
                "正在处理此任务，请稍后再试"
            );
        }
        let result = self.pause_worker(id).await;
        self.0.queue.lock().await.busy.remove(id);
        result
    }
    async fn pause_worker(&self, id: &str) -> Result<DownloadTask> {
        let mut queue = self.0.queue.lock().await;
        let waiting = queue.running.get(id).map(|run| {
            run.control.pause();
            run.done.clone()
        });
        let task = queue
            .tasks
            .get_mut(id)
            .ok_or_else(|| anyhow!("下载任务不存在"))?;
        if task.state == DownloadState::Completed {
            return Ok(task.clone());
        }
        task.state = if waiting.is_some() {
            DownloadState::Pausing
        } else {
            DownloadState::Paused
        };
        task.bytes_per_second = 0.0;
        self.save(task)?;
        drop(queue);
        if let Some(mut done) = waiting {
            while !*done.borrow() {
                if done.changed().await.is_err() {
                    break;
                }
            }
        }
        self.task(id).await
    }
    pub async fn pause_all(&self) -> Result<()> {
        // Capture and mark under one lock: a worker can finish immediately after
        // releasing it, and its Completed state must never be overwritten here.
        let ids = {
            let mut queue = self.0.queue.lock().await;
            let ids = queue
                .tasks
                .values()
                .filter(|task| task.state.is_active())
                .map(|task| task.id.clone())
                .collect::<Vec<_>>();
            for id in &ids {
                if let Some(run) = queue.running.get(id) {
                    run.control.pause();
                }
                if let Some(task) = queue.tasks.get_mut(id) {
                    task.state = DownloadState::Pausing;
                    self.save(task)?;
                }
            }
            ids
        };
        for id in ids {
            self.pause_worker(&id).await?;
        }
        Ok(())
    }
    pub async fn cancel(&self, id: &str) -> Result<DownloadTask> {
        {
            let mut queue = self.0.queue.lock().await;
            ensure!(
                queue.busy.insert(id.to_string()),
                "正在处理此任务，请稍后再试"
            );
        }
        let result = async {
            self.pause_worker(id).await?;
            let mut queue = self.0.queue.lock().await;
            let task = queue
                .tasks
                .get_mut(id)
                .ok_or_else(|| anyhow!("下载任务不存在"))?;
            if task.state != DownloadState::Completed {
                task.state = DownloadState::Cancelled;
                task.note = Some("已取消，部分文件保留，可继续或删除".into());
                self.save(task)?;
            }
            Ok(task.clone())
        }
        .await;
        self.0.queue.lock().await.busy.remove(id);
        result
    }
    pub async fn remove(&self, id: &str, delete_file: bool) -> Result<()> {
        {
            let mut queue = self.0.queue.lock().await;
            ensure!(
                queue.busy.insert(id.to_string()),
                "正在处理此任务，请稍后再试"
            );
        }
        let result = async {
            let task = self.pause_worker(id).await?;
            if delete_file {
                ensure!(
                    task.library_id.is_none()
                        && !self
                            .0
                            .store
                            .list_library_items()?
                            .iter()
                            .any(|item| item.path == task.destination),
                    "已完成的书籍请在文件管理中删除"
                );
                discard_partial(&task.destination).await?;
            }
            self.0.store.remove_download_task(id)?;
            self.0.queue.lock().await.tasks.remove(id);
            Ok(())
        }
        .await;
        self.0.queue.lock().await.busy.remove(id);
        result
    }
    pub async fn restart(&self, id: &str) -> Result<DownloadTask> {
        {
            let mut queue = self.0.queue.lock().await;
            ensure!(
                queue.busy.insert(id.to_string()),
                "正在处理此任务，请稍后再试"
            );
        }
        let result = async {
            let task = self.pause_worker(id).await?;
            // Existing completed books remain recoverable while their replacement is downloaded.
            if task.destination.is_file() && task.library_id.is_some() {
                let backup = task.destination.with_file_name(format!(
                    ".previous-{}.{}",
                    Uuid::new_v4(),
                    task.destination
                        .extension()
                        .and_then(|s| s.to_str())
                        .unwrap_or("bin")
                ));
                tokio::fs::copy(&task.destination, &backup).await?;
                if let Some(mut item) = self
                    .0
                    .store
                    .list_library_items()?
                    .into_iter()
                    .find(|item| Some(&item.id) == task.library_id.as_ref())
                {
                    item.path = backup;
                    self.0.store.upsert_library_item(&item)?;
                }
            }
            discard_partial(&task.destination).await?;
            let mut queue = self.0.queue.lock().await;
            let row = queue.tasks.get_mut(id).context("下载任务不存在")?;
            row.state = DownloadState::Paused;
            row.downloaded_bytes = 0;
            row.total_bytes = None;
            row.library_id = None;
            self.save(row)?;
            Ok::<(), anyhow::Error>(())
        }
        .await;
        self.0.queue.lock().await.busy.remove(id);
        result?;
        self.resume(id).await
    }
    pub async fn wait(&self, id: &str) -> Result<DownloadTask> {
        loop {
            let task = self.task(id).await?;
            if !task.state.is_active() {
                return Ok(task);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    async fn run(
        &self,
        task: DownloadTask,
        token: Uuid,
        control: DownloadControl,
        done: watch::Sender<bool>,
    ) {
        let result = self.acquire(&task, token, &control).await;
        let mut queue = self.0.queue.lock().await;
        if queue
            .running
            .get(&task.id)
            .is_some_and(|r| r.token == token)
        {
            let row = queue.tasks.get_mut(&task.id).unwrap();
            row.active_connections = 0;
            row.bytes_per_second = 0.0;
            match result {
                Ok((item, note)) => {
                    row.state = DownloadState::Completed;
                    row.downloaded_bytes = item.size_bytes;
                    row.total_bytes = Some(item.size_bytes);
                    row.library_id = Some(item.id);
                    row.destination = item.path;
                    row.note = note;
                    row.error = None;
                    row.error_details = None;
                }
                Err(error) => {
                    row.downloaded_bytes = partial_progress(&row.destination).await;
                    if control.is_paused()
                        || matches!(
                            error.downcast_ref::<TransferError>(),
                            Some(TransferError::Paused)
                        )
                    {
                        row.state = DownloadState::Paused;
                        row.note = Some("进度已保存，点击继续下载".into());
                        row.error = None;
                        row.error_details = None;
                    } else {
                        row.state = DownloadState::Failed;
                        row.error = Some(download_error(&error));
                        row.error_details = Some(format!("{error:#}"));
                    }
                }
            }
            let _ = self.save(row);
            queue.running.remove(&task.id);
        }
        done.send_replace(true);
    }
    async fn acquire(
        &self,
        task: &DownloadTask,
        token: Uuid,
        control: &DownloadControl,
    ) -> Result<(LibraryItem, Option<String>)> {
        // Refresh a provider's expiring signed URL on each explicit resume and once on an HTTP expiry.
        for renewal in 0..2 {
            let url = tokio::select! { biased; _ = control.paused() => return Err(TransferError::Paused.into()), result = self.0.providers.acquisition_url(&task.provider_id, &task.book_id, task.format) => result? };
            let headers = self
                .0
                .providers
                .download_headers_for(&task.provider_id, &url)
                .await?;
            preserve_error_page(&task.destination)?;
            let downloader = SegmentedDownloader::with_resolver(
                self.0.resolver.clone(),
                DownloadConfig {
                    concurrency: task.connections,
                    ..Default::default()
                },
            )?;
            let (tx, mut rx) = watch::channel(DownloadProgress::default());
            let this = self.clone();
            let id = task.id.clone();
            let monitor = tokio::spawn(async move {
                let mut last_saved = Instant::now() - Duration::from_secs(1);
                let mut last_bytes = this
                    .task(&id)
                    .await
                    .map(|task| task.downloaded_bytes)
                    .unwrap_or(0);
                let mut last_time = Instant::now();
                while rx.changed().await.is_ok() {
                    let progress = *rx.borrow_and_update();
                    let mut queue = this.0.queue.lock().await;
                    if !queue.running.get(&id).is_some_and(|run| run.token == token) {
                        break;
                    }
                    let row = queue.tasks.get_mut(&id).unwrap();
                    if row.state != DownloadState::Pausing {
                        row.state = DownloadState::Downloading;
                    }
                    let elapsed = last_time.elapsed().as_secs_f64();
                    if elapsed >= 0.4 {
                        row.bytes_per_second =
                            progress.downloaded_bytes.saturating_sub(last_bytes) as f64 / elapsed;
                        last_bytes = progress.downloaded_bytes;
                        last_time = Instant::now();
                    }
                    row.downloaded_bytes = progress.downloaded_bytes;
                    row.total_bytes = progress.total_bytes;
                    row.active_connections = progress.active_connections;
                    row.peak_connections = row.peak_connections.max(progress.peak_connections);
                    row.supports_resume = Some(progress.supports_resume);
                    row.retry_count = progress.retry_count;
                    if !progress.supports_resume {
                        row.note = Some("服务器未支持续传或分段，重新开始时会从头下载".into());
                    } else if progress.restarted {
                        row.note = Some("文件版本已变化，已安全重新开始下载".into());
                    } else {
                        row.note = None;
                    }
                    if last_saved.elapsed() >= Duration::from_millis(350)
                        || progress.active_connections == 0
                    {
                        let _ = this.save(row);
                        last_saved = Instant::now();
                    }
                }
            });
            let result = downloader
                .download_controlled(
                    url,
                    task.destination.clone(),
                    headers,
                    Some(format!("{}:{}", task.provider_id, task.book_id)),
                    tx,
                    control.clone(),
                )
                .await;
            let _ = monitor.await;
            if renewal == 0
                && result.as_ref().err().is_some_and(|error| {
                    matches!(
                        error.downcast_ref::<TransferError>(),
                        Some(TransferError::Http(401 | 403 | 404 | 410))
                    )
                })
            {
                continue;
            }
            result?;
            {
                let mut queue = self.0.queue.lock().await;
                let row = queue.tasks.get_mut(&task.id).unwrap();
                row.state = DownloadState::Verifying;
                self.save(row)?;
            }
            let file_task = task.clone();
            let (item, note) =
                tokio::task::spawn_blocking(move || inspect_completed(&file_task)).await??;
            self.0.store.upsert_library_item(&item)?;
            return Ok((item, note));
        }
        Err(anyhow!("无法取得有效下载链接，请检查账户或书源"))
    }
}
fn inspect_completed(task: &DownloadTask) -> Result<(LibraryItem, Option<String>)> {
    let mut path = task.destination.clone();
    let mut note = None;
    if validate_readable_file(&path).is_err() && task.format == BookFormat::Epub {
        // Some source entries explicitly distribute a ZIP of multiple EPUB books as .epub.
        // Keep the completed bytes accessible as an archive instead of retrying a valid bundle forever.
        if let Ok(mut archive) = zip::ZipArchive::new(std::fs::File::open(&path)?) {
            let contains_books = (0..archive.len()).any(|index| {
                archive.by_index(index).is_ok_and(|entry| {
                    let name = entry.name().to_ascii_lowercase();
                    [".epub", ".pdf", ".txt"]
                        .iter()
                        .any(|extension| name.ends_with(extension))
                })
            });
            drop(archive);
            if contains_books {
                let destination = path.with_extension("zip");
                ensure!(
                    !destination.exists(),
                    "书包导出位置已存在，请先在文件管理中处理"
                );
                std::fs::rename(&path, &destination)?;
                path = destination;
                note = Some("这是包含多本书的压缩包，可导出或用解压应用打开".into());
            }
        }
    }
    validate_readable_file(&path).context("文件已传完，但书籍格式校验未通过")?;
    let item = LibraryItem::inspect(
        &path,
        Some(&task.title),
        Some(task.provider_id.clone()),
        Some(task.book_id.clone()),
    )?;
    Ok((item, note))
}
fn preserve_error_page(path: &Path) -> Result<()> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Ok(());
    };
    let mut header = [0; 512];
    let count = file.read(&mut header)?;
    let text = String::from_utf8_lossy(&header[..count])
        .trim_start()
        .to_ascii_lowercase();
    if text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("{\"success\"")
        || text.starts_with("{\"error\"")
    {
        let backup = path.with_file_name(format!(".invalid-{}", Uuid::new_v4()));
        drop(file);
        std::fs::rename(path, backup)?;
    }
    Ok(())
}
fn download_error(error: &anyhow::Error) -> String {
    match error.downcast_ref::<TransferError>() {
        Some(TransferError::Http(401 | 403)) => {
            "下载链接失效或账户权限不足，进度已保留；继续时将重新取得链接".into()
        }
        Some(TransferError::Http(429)) => "书源请求过多，稍后继续下载，或减少连接数".into(),
        Some(TransferError::InvalidRange) => {
            "服务器返回的分段或文件版本不一致，已停止合并；进度保留".into()
        }
        _ => "下载中断，已保留可续传的进度。检查网络后点击继续下载".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[tokio::test]
    async fn completing_a_book_during_pause_all_keeps_it_completed() {
        let manager = DownloadManager::new(
            StateStore::open_memory().unwrap(),
            ProviderRegistry::default(),
            AppResolver::system().unwrap(),
        )
        .unwrap();
        let mut task = DownloadTask::new(
            "Completed book".into(),
            "fixture".into(),
            "one".into(),
            BookFormat::Txt,
            PathBuf::from("kept.txt"),
            1,
        );
        task.state = DownloadState::Downloading;
        let id = task.id.clone();
        let mut guard = manager.0.queue.lock().await;
        guard.tasks.insert(id.clone(), task);
        let pausing = manager.clone();
        let pause = tokio::spawn(async move {
            pausing.pause_all().await.unwrap();
        });
        tokio::task::yield_now().await;
        let completing = manager.clone();
        let completed_id = id.clone();
        let complete = tokio::spawn(async move {
            let mut queue = completing.0.queue.lock().await;
            let row = queue.tasks.get_mut(&completed_id).unwrap();
            row.state = DownloadState::Completed;
            completing.save(row).unwrap();
        });
        tokio::task::yield_now().await;
        drop(guard);
        pause.await.unwrap();
        complete.await.unwrap();
        assert_eq!(
            manager.task(&id).await.unwrap().state,
            DownloadState::Completed,
            "Pause-all changed an already completed file into a resumable task"
        );
    }
    #[test]
    fn an_epub_named_book_bundle_is_preserved_as_an_exportable_zip() {
        let directory = std::env::temp_dir().join(format!("lumina-bundle-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("selected-volumes.epub");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        archive
            .start_file("volumes/01.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(b"Selected volume one").unwrap();
        archive.finish().unwrap();
        let task = DownloadTask::new(
            "Selected volumes".into(),
            "fixture".into(),
            "bundle".into(),
            BookFormat::Epub,
            path.clone(),
            4,
        );
        let (item, note) = inspect_completed(&task).unwrap();
        assert_eq!(item.format, crate::library::LibraryFormat::Zip);
        assert!(item.path.is_file());
        assert!(!path.exists());
        assert!(note.unwrap().contains("压缩包"));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&item.path).unwrap()).unwrap();
        let mut bytes = String::new();
        archive
            .by_index(0)
            .unwrap()
            .read_to_string(&mut bytes)
            .unwrap();
        assert_eq!(bytes, "Selected volume one");
        drop(archive);
        assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

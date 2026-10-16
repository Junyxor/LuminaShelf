use super::transfer_tests::{Behavior, Fixture};
use super::*;
use crate::{
    provider::{BookProvider, ProviderCapabilities, SearchQuery, SearchResult},
    BookDetails, ProviderRegistry, StateStore,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Provider {
    url: Url,
    acquisitions: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl BookProvider for Provider {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn name(&self) -> &'static str {
        "Fixture"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            downloadable: true,
            ..Default::default()
        }
    }
    async fn search(&self, _: SearchQuery) -> Result<SearchResult> {
        anyhow::bail!("unused")
    }
    async fn details(&self, _: &str) -> Result<BookDetails> {
        anyhow::bail!("unused")
    }
    async fn acquisition_url(&self, _: &str, _: BookFormat) -> Result<Url> {
        let number = self.acquisitions.fetch_add(1, Ordering::SeqCst) + 1;
        let mut url = self.url.clone();
        url.set_query(Some(&format!("token={number}")));
        Ok(url)
    }
}
fn registry(fixture: &Fixture) -> (ProviderRegistry, Arc<AtomicUsize>) {
    let providers = ProviderRegistry::default();
    let calls = Arc::new(AtomicUsize::new(0));
    providers.register(Provider {
        url: fixture.url.clone(),
        acquisitions: calls.clone(),
    });
    (providers, calls)
}
async fn wait_bytes(manager: &DownloadManager, id: &str) -> DownloadTask {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let task = manager.task(id).await.unwrap();
            if task.downloaded_bytes >= 8192 && task.state == DownloadState::Downloading {
                return task;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Download never made progress")
}

#[tokio::test]
async fn paused_progress_survives_process_recovery_and_refreshed_signed_url() {
    let fixture = Fixture::new(Behavior::Slow).await;
    let database = fixture.directory.join("tasks.sqlite");
    let store = StateStore::open(&database).unwrap();
    let (providers, calls) = registry(&fixture);
    let manager = DownloadManager::new(
        store.clone(),
        providers.clone(),
        AppResolver::system().unwrap(),
    )
    .unwrap();
    let task = manager
        .enqueue(
            "fixture".into(),
            "book".into(),
            "Resume book".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    wait_bytes(&manager, &task.id).await;
    let paused = manager.pause(&task.id).await.unwrap();
    assert_eq!(paused.state, DownloadState::Paused);
    assert!(paused.downloaded_bytes >= 8192);
    let saved = tokio::fs::read(&paused.destination).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert_eq!(
        tokio::fs::read(&paused.destination).await.unwrap(),
        saved,
        "Paused worker continued writing"
    );
    // An OS-killed process leaves an active SQLite row: the next process must recover it as paused.
    let mut interrupted = paused.clone();
    interrupted.state = DownloadState::Downloading;
    store.save_download_task(&interrupted).unwrap();
    drop(manager);
    drop(store);
    let store = StateStore::open(&database).unwrap();
    let recovered =
        DownloadManager::new(store.clone(), providers, AppResolver::system().unwrap()).unwrap();
    assert_eq!(
        recovered.list().await.unwrap()[0].state,
        DownloadState::Paused
    );
    assert_eq!(
        recovered.list().await.unwrap()[0].downloaded_bytes,
        paused.downloaded_bytes
    );
    recovered.resume(&task.id).await.unwrap();
    let completed =
        tokio::time::timeout(std::time::Duration::from_secs(5), recovered.wait(&task.id))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(completed.state, DownloadState::Completed, "{completed:?}");
    assert_eq!(
        tokio::fs::read(&completed.destination).await.unwrap(),
        *fixture.data
    );
    assert!(fixture
        .ranges
        .lock()
        .await
        .iter()
        .any(|(start, _)| *start == paused.downloaded_bytes));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "Resume must refresh the signed link once"
    );
    assert_eq!(store.list_library_items().unwrap().len(), 1);
    assert!(!serde_json::to_string(&store.download_tasks().unwrap())
        .unwrap()
        .contains("token="));
    recovered.remove(&task.id, false).await.unwrap();
    assert!(store.download_tasks().unwrap().is_empty());
    assert!(completed.destination.is_file());
    assert_eq!(
        store.list_library_items().unwrap().len(),
        1,
        "Removing a task removed a library book"
    );
}

#[tokio::test]
async fn expired_link_is_renewed_once_without_losing_the_task() {
    let fixture = Fixture::new(Behavior::Expired).await;
    let store = StateStore::open_memory().unwrap();
    let (providers, calls) = registry(&fixture);
    let manager =
        DownloadManager::new(store.clone(), providers, AppResolver::system().unwrap()).unwrap();
    let task = manager
        .enqueue(
            "fixture".into(),
            "expired".into(),
            "Expired link".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), manager.wait(&task.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.state, DownloadState::Completed, "{done:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(store.download_tasks().unwrap().len(), 1);
    store
        .remove_library_item(done.library_id.as_ref().unwrap())
        .unwrap();
    let reattached = manager
        .enqueue(
            "fixture".into(),
            "expired".into(),
            "Expired link".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    assert_eq!(reattached.state, DownloadState::Completed);
    assert_eq!(
        store.list_library_items().unwrap().len(),
        1,
        "An existing completed task did not reattach its removed shelf entry"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "Reattaching a saved file should not use the network"
    );
}

#[tokio::test]
async fn cancelling_keeps_partial_bytes_until_explicit_delete() {
    let fixture = Fixture::new(Behavior::Slow).await;
    let store = StateStore::open_memory().unwrap();
    let (providers, _) = registry(&fixture);
    let manager =
        DownloadManager::new(store.clone(), providers, AppResolver::system().unwrap()).unwrap();
    let task = manager
        .enqueue(
            "fixture".into(),
            "cancel".into(),
            "Cancel".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    wait_bytes(&manager, &task.id).await;
    let cancelled = manager.cancel(&task.id).await.unwrap();
    assert_eq!(cancelled.state, DownloadState::Cancelled);
    assert!(cancelled.destination.is_file());
    manager.remove(&task.id, true).await.unwrap();
    assert!(!cancelled.destination.exists());
    assert!(store.download_tasks().unwrap().is_empty());
}

#[tokio::test]
async fn queue_limit_deduplication_and_pause_all_do_not_start_waiting_books() {
    let fixture = Fixture::new(Behavior::Slow).await;
    let store = StateStore::open_memory().unwrap();
    let (providers, calls) = registry(&fixture);
    let manager =
        DownloadManager::new(store.clone(), providers, AppResolver::system().unwrap()).unwrap();
    manager
        .set_settings(DownloadSettings {
            connections: 2,
            simultaneous_books: 1,
        })
        .await
        .unwrap();
    let first = manager
        .enqueue(
            "fixture".into(),
            "one".into(),
            "One".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    wait_bytes(&manager, &first.id).await;
    let duplicate = manager
        .enqueue(
            "fixture".into(),
            "one".into(),
            "One".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.id, first.id);
    let second = manager
        .enqueue(
            "fixture".into(),
            "two".into(),
            "Two".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    assert_eq!(second.state, DownloadState::Queued);
    manager.pause_all().await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "Queued book started while pause-all was running"
    );
    assert!(manager
        .list()
        .await
        .unwrap()
        .iter()
        .all(|task| task.state == DownloadState::Paused));
    manager.resume(&first.id).await.unwrap();
    assert_eq!(
        manager.wait(&first.id).await.unwrap().state,
        DownloadState::Completed
    );
    manager.remove(&first.id, false).await.unwrap();
    let reused = manager
        .enqueue(
            "fixture".into(),
            "one".into(),
            "One".into(),
            BookFormat::Txt,
            fixture.directory.clone(),
        )
        .await
        .unwrap();
    assert_eq!(reused.state, DownloadState::Completed);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "Existing completed library file was unnecessarily redownloaded"
    );
    assert_ne!(reused.generation, first.generation);
}

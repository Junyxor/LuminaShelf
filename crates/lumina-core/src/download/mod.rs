use crate::{
    library::now_unix_ms,
    network::{AppResolver, NetworkClient},
    BookFormat,
};
use anyhow::Result;
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::watch;
use url::Url;
use uuid::Uuid;

mod manager;
mod transfer;
pub use manager::{destination_for, DownloadManager, DownloadSettings};
pub use transfer::{discard_partial, partial_progress};
#[cfg(test)]
mod manager_tests;
#[cfg(test)]
mod transfer_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Connecting,
    Downloading,
    Pausing,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}
impl DownloadState {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Connecting | Self::Downloading | Self::Pausing | Self::Verifying
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    #[serde(default)]
    pub generation: String,
    pub title: String,
    pub provider_id: String,
    pub book_id: String,
    pub format: BookFormat,
    pub destination: PathBuf,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub bytes_per_second: f64,
    pub connections: usize,
    pub active_connections: usize,
    pub peak_connections: usize,
    pub supports_resume: Option<bool>,
    pub retry_count: usize,
    pub error: Option<String>,
    pub error_details: Option<String>,
    pub note: Option<String>,
    pub library_id: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    #[serde(default)]
    pub revision: u64,
}
impl DownloadTask {
    pub fn new(
        title: String,
        provider_id: String,
        book_id: String,
        format: BookFormat,
        destination: PathBuf,
        connections: usize,
    ) -> Self {
        let now = now_unix_ms();
        let id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("{provider_id}:{book_id}:{}", format.extension()).as_bytes(),
        )
        .to_string();
        Self {
            id,
            generation: Uuid::new_v4().to_string(),
            title,
            provider_id,
            book_id,
            format,
            destination,
            state: DownloadState::Queued,
            total_bytes: None,
            downloaded_bytes: 0,
            bytes_per_second: 0.0,
            connections,
            active_connections: 0,
            peak_connections: 0,
            supports_resume: None,
            retry_count: 0,
            error: None,
            error_details: None,
            note: None,
            library_id: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub concurrency: usize,
    pub segment_size: u64,
    pub retries: usize,
}
impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            concurrency: 4,
            segment_size: 1024 * 1024,
            retries: 4,
        }
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub active_connections: usize,
    pub peak_connections: usize,
    pub supports_resume: bool,
    pub retry_count: usize,
    pub restarted: bool,
}
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("下载已暂停")]
    Paused,
    #[error("下载服务器返回 HTTP {0}")]
    Http(u16),
    #[error("服务器未按请求返回分段数据")]
    RangeUnsupported,
    #[error("服务器返回的文件分段位置或版本不一致")]
    InvalidRange,
    #[error("书源返回的不是书籍文件：{0}")]
    NotABook(String),
}
impl TransferError {
    /// Statuses that describe a momentary server condition rather than a broken link.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Http(408 | 425 | 429 | 500 | 502 | 503 | 504))
    }
}

#[derive(Clone)]
pub struct DownloadControl(watch::Sender<bool>);
impl Default for DownloadControl {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}
impl DownloadControl {
    pub fn pause(&self) {
        self.0.send_replace(true);
    }
    pub fn is_paused(&self) -> bool {
        *self.0.borrow()
    }
    pub(crate) async fn paused(&self) {
        let mut receiver = self.0.subscribe();
        if *receiver.borrow() {
            return;
        }
        while receiver.changed().await.is_ok() {
            if *receiver.borrow() {
                return;
            }
        }
    }
}

#[derive(Clone)]
pub struct SegmentedDownloader {
    client: DownloadClient,
    config: DownloadConfig,
}
#[derive(Clone)]
enum DownloadClient {
    Fixed(Client),
    Configured(NetworkClient),
}
impl DownloadClient {
    fn client(&self) -> Result<Client> {
        match self {
            Self::Fixed(client) => Ok(client.clone()),
            Self::Configured(client) => client.client(),
        }
    }
}
impl SegmentedDownloader {
    pub fn new(client: Client, config: DownloadConfig) -> Self {
        Self {
            client: DownloadClient::Fixed(client),
            config,
        }
    }
    pub fn with_resolver(resolver: AppResolver, config: DownloadConfig) -> Result<Self> {
        Ok(Self {
            client: DownloadClient::Configured(NetworkClient::for_download(resolver)?),
            config,
        })
    }
    pub async fn download(
        &self,
        url: Url,
        destination: impl AsRef<Path>,
    ) -> Result<watch::Receiver<DownloadProgress>> {
        let (tx, rx) = watch::channel(DownloadProgress::default());
        let this = self.clone();
        let destination = destination.as_ref().to_path_buf();
        tokio::spawn(async move {
            let _ = this.download_to(url, destination, tx).await;
        });
        Ok(rx)
    }
    pub async fn download_to(
        &self,
        url: Url,
        destination: PathBuf,
        tx: watch::Sender<DownloadProgress>,
    ) -> Result<()> {
        self.download_to_with_headers(url, destination, header::HeaderMap::new(), tx)
            .await
    }
    pub async fn download_to_with_headers(
        &self,
        url: Url,
        destination: PathBuf,
        headers: header::HeaderMap,
        tx: watch::Sender<DownloadProgress>,
    ) -> Result<()> {
        self.download_to_with_context(url, destination, headers, None, tx)
            .await
    }
    pub async fn download_to_with_context(
        &self,
        url: Url,
        destination: PathBuf,
        headers: header::HeaderMap,
        resume_key: Option<String>,
        tx: watch::Sender<DownloadProgress>,
    ) -> Result<()> {
        self.download_controlled(
            url,
            destination,
            headers,
            resume_key,
            tx,
            DownloadControl::default(),
        )
        .await
    }
    pub async fn download_controlled(
        &self,
        url: Url,
        destination: PathBuf,
        headers: header::HeaderMap,
        resume_key: Option<String>,
        tx: watch::Sender<DownloadProgress>,
        control: DownloadControl,
    ) -> Result<()> {
        transfer::run(self, url, destination, headers, resume_key, tx, control).await
    }
}

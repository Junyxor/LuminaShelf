use crate::{
    library::now_unix_ms,
    network::{AppResolver, ReqwestResolver},
};
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::{watch, Mutex, Semaphore},
    task::JoinSet,
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadState {
    Queued,
    Connecting,
    Downloading,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

impl DownloadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Downloading => "downloading",
            Self::Paused => "paused",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "queued" => Self::Queued,
            "connecting" => Self::Connecting,
            "downloading" => Self::Downloading,
            "paused" => Self::Paused,
            "verifying" => Self::Verifying,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Failed,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Connecting | Self::Downloading | Self::Verifying
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub book_id: Option<String>,
    pub url: Url,
    pub destination: PathBuf,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub bytes_per_second: f64,
    pub error: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

impl DownloadTask {
    pub fn new(
        title: impl Into<String>,
        url: Url,
        destination: PathBuf,
        provider_id: Option<String>,
        book_id: Option<String>,
    ) -> Self {
        let now = now_unix_ms();
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            provider_id,
            book_id,
            url,
            destination,
            state: DownloadState::Queued,
            total_bytes: None,
            downloaded_bytes: 0,
            bytes_per_second: 0.0,
            error: None,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
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
            concurrency: 6,
            segment_size: 8 * 1024 * 1024,
            retries: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SegmentManifest {
    url: String,
    #[serde(default)]
    resume_key: Option<String>,
    total_bytes: u64,
    segment_size: u64,
    completed: BTreeSet<u64>,
}

#[derive(Clone)]
pub struct SegmentedDownloader {
    client: Client,
    config: DownloadConfig,
}

#[derive(Debug, Clone, Copy)]
struct RemoteShape {
    total: Option<u64>,
    accepts_ranges: bool,
}

impl SegmentedDownloader {
    pub fn new(client: Client, config: DownloadConfig) -> Self {
        Self { client, config }
    }

    pub fn with_resolver(resolver: AppResolver, config: DownloadConfig) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(8))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .user_agent("LuminaShelf/0.5 downloader")
            .dns_resolver(Arc::new(ReqwestResolver::new(resolver)))
            .build()?;
        Ok(Self::new(client, config))
    }

    pub async fn download(
        &self,
        url: Url,
        destination: impl AsRef<Path>,
    ) -> Result<watch::Receiver<DownloadProgress>> {
        let destination = destination.as_ref().to_path_buf();
        let (tx, rx) = watch::channel(DownloadProgress::default());
        let this = self.clone();
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
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let shape = self.inspect_remote(&url, &headers).await?;
        let segmented = shape.accepts_ranges && shape.total.unwrap_or(0) > self.config.segment_size;
        if !segmented {
            if let (Some(total), Ok(metadata)) =
                (shape.total, tokio::fs::metadata(&destination).await)
            {
                if metadata.is_file() && metadata.len() == total {
                    let _ = tx.send(DownloadProgress {
                        downloaded_bytes: total,
                        total_bytes: Some(total),
                    });
                    return Ok(());
                }
            }
        }
        let initial = resumable_bytes(
            &destination,
            shape.total,
            shape.accepts_ranges,
            self.config.segment_size,
            &url,
            resume_key.as_deref(),
        )
        .await;
        let _ = tx.send(DownloadProgress {
            downloaded_bytes: initial,
            total_bytes: shape.total,
        });

        if segmented {
            self.download_segmented(
                url,
                destination,
                shape.total.unwrap(),
                headers,
                resume_key,
                tx,
            )
            .await
        } else {
            self.download_sequential(url, destination, shape, headers, tx)
                .await
        }
    }

    async fn inspect_remote(&self, url: &Url, headers: &header::HeaderMap) -> Result<RemoteShape> {
        if let Ok(head) = self
            .client
            .head(url.clone())
            .headers(headers.clone())
            .send()
            .await
        {
            if head.status().is_success() {
                let total = head.content_length();
                let accepts_ranges = header_has_bytes(head.headers());
                if total.is_some() {
                    return Ok(RemoteShape {
                        total,
                        accepts_ranges,
                    });
                }
            }
        }

        let probe = self
            .client
            .get(url.clone())
            .headers(headers.clone())
            .header(header::RANGE, "bytes=0-0")
            .send()
            .await?;
        if probe.status() == StatusCode::PARTIAL_CONTENT {
            let total = content_range_total(probe.headers()).or_else(|| probe.content_length());
            return Ok(RemoteShape {
                total,
                accepts_ranges: true,
            });
        }
        let total = probe.content_length();
        if probe.status().is_success() {
            Ok(RemoteShape {
                total,
                accepts_ranges: header_has_bytes(probe.headers()),
            })
        } else {
            Err(anyhow!("download endpoint returned {}", probe.status()))
        }
    }

    async fn download_sequential(
        &self,
        url: Url,
        destination: PathBuf,
        shape: RemoteShape,
        headers: header::HeaderMap,
        tx: watch::Sender<DownloadProgress>,
    ) -> Result<()> {
        let existing = tokio::fs::metadata(&destination)
            .await
            .ok()
            .map(|meta| meta.len())
            .unwrap_or(0);
        let can_resume = shape.accepts_ranges
            && existing > 0
            && shape.total.is_some_and(|total| existing < total);

        let mut request = self.client.get(url).headers(headers);
        if can_resume {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }
        let response = request.send().await?;
        if can_resume && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("server stopped honoring byte ranges during resume"));
        }
        if !response.status().is_success() && response.status() != StatusCode::PARTIAL_CONTENT {
            return Err(anyhow!("download failed: {}", response.status()));
        }

        let mut file = if can_resume {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&destination)
                .await?
        } else {
            File::create(&destination).await?
        };
        let mut stream = response.bytes_stream();
        let mut downloaded = if can_resume { existing } else { 0 };
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            let _ = tx.send(DownloadProgress {
                downloaded_bytes: downloaded,
                total_bytes: shape.total,
            });
        }
        file.flush().await?;
        Ok(())
    }

    async fn download_segmented(
        &self,
        url: Url,
        destination: PathBuf,
        total: u64,
        headers: header::HeaderMap,
        resume_key: Option<String>,
        tx: watch::Sender<DownloadProgress>,
    ) -> Result<()> {
        let manifest_path = manifest_path(&destination);
        let mut manifest = load_manifest(&manifest_path)
            .await
            .unwrap_or_else(|| SegmentManifest {
                url: url.to_string(),
                resume_key: resume_key.clone(),
                total_bytes: total,
                segment_size: self.config.segment_size,
                completed: BTreeSet::new(),
            });

        let existing_len = tokio::fs::metadata(&destination)
            .await
            .ok()
            .map(|meta| meta.len())
            .unwrap_or(0);
        let identity_matches = match resume_key.as_deref() {
            Some(key) => manifest.resume_key.as_deref() == Some(key),
            None => manifest.url == url.as_str(),
        };
        if !identity_matches
            || manifest.total_bytes != total
            || manifest.segment_size != self.config.segment_size
            || existing_len != total
        {
            manifest = SegmentManifest {
                url: url.to_string(),
                resume_key: resume_key.clone(),
                total_bytes: total,
                segment_size: self.config.segment_size,
                completed: BTreeSet::new(),
            };
            let prealloc_path = destination.clone();
            tokio::task::spawn_blocking(move || -> Result<()> {
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&prealloc_path)?;
                file.set_len(total)?;
                Ok(())
            })
            .await??;
            store_manifest(&manifest_path, &manifest).await?;
        } else if manifest.url != url.as_str() {
            manifest.url = url.to_string();
            store_manifest(&manifest_path, &manifest).await?;
        }

        let initial = manifest
            .completed
            .iter()
            .map(|index| segment_len(*index, self.config.segment_size, total))
            .sum::<u64>();
        let downloaded = Arc::new(AtomicU64::new(initial));
        let _ = tx.send(DownloadProgress {
            downloaded_bytes: initial,
            total_bytes: Some(total),
        });

        let manifest = Arc::new(Mutex::new(manifest));
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency.max(1)));
        let mut joins = JoinSet::new();
        let segments = total.div_ceil(self.config.segment_size);

        for index in 0..segments {
            if manifest.lock().await.completed.contains(&index) {
                continue;
            }
            let start = index * self.config.segment_size;
            let end = (start + self.config.segment_size - 1).min(total - 1);
            let permit = semaphore.clone().acquire_owned().await?;
            let client = self.client.clone();
            let url = url.clone();
            let path = destination.clone();
            let downloaded = downloaded.clone();
            let tx = tx.clone();
            let retries = self.config.retries;
            let headers = headers.clone();
            let manifest = manifest.clone();
            let manifest_path = manifest_path.clone();

            joins.spawn(async move {
                let _permit = permit;
                download_range(
                    client, url, path, start, end, total, downloaded, tx, retries, headers,
                )
                .await?;
                {
                    let mut guard = manifest.lock().await;
                    guard.completed.insert(index);
                    store_manifest(&manifest_path, &guard).await?;
                }
                Result::<()>::Ok(())
            });
        }

        while let Some(result) = joins.join_next().await {
            result??;
        }
        let _ = tokio::fs::remove_file(manifest_path).await;
        Ok(())
    }
}

async fn resumable_bytes(
    destination: &Path,
    total: Option<u64>,
    accepts_ranges: bool,
    segment_size: u64,
    url: &Url,
    resume_key: Option<&str>,
) -> u64 {
    if !accepts_ranges {
        return 0;
    }
    if total.is_some_and(|total| total > segment_size) {
        if let Some(manifest) = load_manifest(&manifest_path(destination)).await {
            let identity_matches = match resume_key {
                Some(key) => manifest.resume_key.as_deref() == Some(key),
                None => manifest.url == url.as_str(),
            };
            if identity_matches
                && Some(manifest.total_bytes) == total
                && manifest.segment_size == segment_size
            {
                return manifest
                    .completed
                    .iter()
                    .map(|index| segment_len(*index, segment_size, manifest.total_bytes))
                    .sum();
            }
        }
        0
    } else {
        let existing = tokio::fs::metadata(destination)
            .await
            .ok()
            .map(|meta| meta.len())
            .unwrap_or(0);
        if total.is_some_and(|total| existing <= total) {
            existing
        } else {
            0
        }
    }
}

fn segment_len(index: u64, segment_size: u64, total: u64) -> u64 {
    let start = index * segment_size;
    if start >= total {
        return 0;
    }
    (start + segment_size).min(total) - start
}

fn header_has_bytes(headers: &header::HeaderMap) -> bool {
    headers
        .get(header::ACCEPT_RANGES)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("bytes"))
}

fn content_range_total(headers: &header::HeaderMap) -> Option<u64> {
    let value = headers.get(header::CONTENT_RANGE)?.to_str().ok()?;
    value.rsplit_once('/')?.1.parse().ok()
}

fn manifest_path(destination: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lumina-part.json", destination.display()))
}

async fn load_manifest(path: &Path) -> Option<SegmentManifest> {
    let raw = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&raw).ok()
}

async fn store_manifest(path: &Path, manifest: &SegmentManifest) -> Result<()> {
    let raw = serde_json::to_vec(manifest)?;
    tokio::fs::write(path, raw).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn download_range(
    client: Client,
    url: Url,
    path: PathBuf,
    start: u64,
    end: u64,
    total: u64,
    downloaded: Arc<AtomicU64>,
    tx: watch::Sender<DownloadProgress>,
    retries: usize,
    headers: header::HeaderMap,
) -> Result<()> {
    let mut last_error = None;
    for attempt in 0..=retries {
        match fetch_range(
            &client,
            &url,
            &path,
            start,
            end,
            total,
            &downloaded,
            &tx,
            &headers,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt < retries {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        180 * (attempt as u64 + 1),
                    ))
                    .await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("range download failed")))
}

#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: &Client,
    url: &Url,
    path: &Path,
    start: u64,
    end: u64,
    total: u64,
    downloaded: &AtomicU64,
    tx: &watch::Sender<DownloadProgress>,
    headers: &header::HeaderMap,
) -> Result<()> {
    let response = client
        .get(url.clone())
        .headers(headers.clone())
        .header(header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(anyhow!("server ignored byte range: {}", response.status()));
    }

    let body = response.bytes().await?;
    let expected = end - start + 1;
    if body.len() as u64 != expected {
        return Err(anyhow!(
            "short range response: expected {expected}, got {}",
            body.len()
        ))
        .with_context(|| format!("range {start}-{end}"));
    }

    let mut file = OpenOptions::new().write(true).open(path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    file.write_all(&body).await?;
    file.flush().await?;

    let now = downloaded.fetch_add(expected, Ordering::Relaxed) + expected;
    let _ = tx.send(DownloadProgress {
        downloaded_bytes: now.min(total),
        total_bytes: Some(total),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_segment_manifest_without_resume_key_is_still_readable() {
        let raw = br#"{
          "url":"https://example.test/file.epub",
          "totalBytes":16777216,
          "segmentSize":8388608,
          "completed":[0]
        }"#;
        let manifest: SegmentManifest = serde_json::from_slice(raw).unwrap();
        assert_eq!(manifest.resume_key, None);
        assert!(manifest.completed.contains(&0));
    }

    #[test]
    fn provider_resume_key_is_non_secret_and_serialized_separately_from_url() {
        let manifest = SegmentManifest {
            url: "https://mirror-a.example/file".to_string(),
            resume_key: Some("zlibrary:123:hash".to_string()),
            total_bytes: 10,
            segment_size: 5,
            completed: BTreeSet::from([0]),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("resumeKey"));
        assert!(!json.contains("remix_userkey"));
    }
}

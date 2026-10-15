use super::*;
use anyhow::{anyhow, ensure, Context};
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    url: String,
    #[serde(default)]
    resume_key: Option<String>,
    total_bytes: u64,
    segment_size: u64,
    #[serde(default)]
    completed: BTreeSet<u64>,
    #[serde(default)]
    partial: BTreeMap<u64, u64>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
}
impl Manifest {
    fn bytes(&self) -> u64 {
        if self.segment_size == 0 {
            return self.partial.get(&0).copied().unwrap_or(0);
        }
        self.completed
            .iter()
            .map(|i| segment_len(*i, self.segment_size, self.total_bytes))
            .sum::<u64>()
            + self
                .partial
                .iter()
                .filter(|(i, _)| !self.completed.contains(i))
                .map(|(i, n)| (*n).min(segment_len(*i, self.segment_size, self.total_bytes)))
                .sum::<u64>()
    }
    fn prefix(&self) -> u64 {
        if self.segment_size == 0 {
            return self.bytes();
        }
        let mut index = 0;
        while self.completed.contains(&index) {
            index += 1;
        }
        (index * self.segment_size + self.partial.get(&index).copied().unwrap_or(0))
            .min(self.total_bytes)
    }
}
fn manifest_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lumina-part.json", path.display()))
}
async fn load(path: &Path) -> Option<Manifest> {
    serde_json::from_slice(&tokio::fs::read(manifest_path(path)).await.ok()?).ok()
}
async fn store(path: &Path, value: &Manifest) -> Result<()> {
    let path = manifest_path(path);
    let temporary = PathBuf::from(format!("{}.tmp", path.display()));
    tokio::fs::write(&temporary, serde_json::to_vec(value)?).await?;
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}
pub async fn partial_progress(path: &Path) -> u64 {
    let length = tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if let Some(manifest) = load(path).await {
        return if manifest.segment_size == 0 {
            manifest.bytes().min(length)
        } else if length == manifest.total_bytes {
            manifest.bytes()
        } else {
            manifest.prefix().min(length)
        };
    }
    length
}
/// Called only after the owning task has stopped; the caller must check ownership.
pub async fn discard_partial(path: &Path) -> Result<()> {
    for file in [
        path.to_path_buf(),
        manifest_path(path),
        PathBuf::from(format!("{}.tmp", manifest_path(path).display())),
    ] {
        match tokio::fs::remove_file(file).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn segment_len(index: u64, size: u64, total: u64) -> u64 {
    total.saturating_sub(index.saturating_mul(size)).min(size)
}

#[derive(Clone)]
struct Shape {
    total: Option<u64>,
    ranges: bool,
    etag: Option<String>,
    modified: Option<String>,
}
fn text_header(headers: &header::HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_string)
}
fn content_range(response: &reqwest::Response) -> Option<(u64, u64, Option<u64>)> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes ")?;
    let (positions, total) = value.split_once('/')?;
    let (start, end) = positions.split_once('-')?;
    let (start, end) = (start.parse().ok()?, end.parse().ok()?);
    let total = if total == "*" {
        None
    } else {
        Some(total.parse::<u64>().ok()?)
    };
    (start <= end && total.is_none_or(|total| end < total)).then_some((start, end, total))
}
fn check_encoding(response: &reqwest::Response) -> Result<()> {
    ensure!(
        response
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_none_or(|v| v == "identity"),
        "服务器未遵守原始字节传输要求，请重试或切换下载方式"
    );
    Ok(())
}
async fn send(
    request: reqwest::RequestBuilder,
    control: &DownloadControl,
) -> Result<reqwest::Response> {
    let (client, request) = request.build_split();
    let mut request = request.map_err(reqwest::Error::without_url)?;
    for hop in 0..=5 {
        let previous = request
            .try_clone()
            .ok_or_else(|| anyhow!("下载请求无法重试"))?;
        let response = tokio::select! { biased; _ = control.paused() => return Err(TransferError::Paused.into()), result = tokio::time::timeout(Duration::from_secs(25), client.execute(request)) => result.context("等待下载服务器响应超时，进度已保存")?.map_err(reqwest::Error::without_url)? };
        if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        ensure!(hop < 5, "下载重定向次数过多");
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .context("下载重定向缺少有效地址")?;
        let next = previous
            .url()
            .join(location)
            .map_err(|_| anyhow!("下载重定向地址无效"))?;
        ensure!(
            matches!(next.scheme(), "http" | "https")
                && next.username().is_empty()
                && next.password().is_none(),
            "下载重定向协议无效"
        );
        ensure!(
            previous.url().scheme() != "https" || next.scheme() == "https",
            "拒绝降级为不加密的下载连接"
        );
        request = previous;
        if request.url().origin() != next.origin() {
            // Account plugins can use nonstandard header names. Keep only transfer
            // protocol headers when crossing origins, never forward their credentials.
            let mut safe = header::HeaderMap::new();
            for name in [
                header::RANGE,
                header::IF_RANGE,
                header::ACCEPT_ENCODING,
                header::ACCEPT,
            ] {
                if let Some(value) = request.headers().get(&name) {
                    safe.insert(name, value.clone());
                }
            }
            *request.headers_mut() = safe;
        }
        *request.url_mut() = next;
    }
    unreachable!()
}
async fn inspect(
    downloader: &SegmentedDownloader,
    url: &Url,
    headers: &header::HeaderMap,
    control: &DownloadControl,
) -> Result<Shape> {
    let mut response = send(
        downloader
            .client
            .client()?
            .get(url.clone())
            .headers(headers.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, "bytes=0-0")
            .timeout(Duration::from_secs(15)),
        control,
    )
    .await?;
    check_encoding(&response)?;
    let ranges = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(TransferError::Http(response.status().as_u16()).into());
    }
    let total = if ranges {
        let (start, end, total) = content_range(&response).ok_or(TransferError::InvalidRange)?;
        ensure!(start == 0 && end == 0, TransferError::InvalidRange);
        total
    } else {
        response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()?.parse().ok())
    };
    let shape = Shape {
        total,
        ranges,
        etag: text_header(response.headers(), header::ETAG).filter(|s| !s.starts_with("W/")),
        modified: text_header(response.headers(), header::LAST_MODIFIED),
    };
    if ranges {
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(reqwest::Error::without_url)?
        {
            ensure!(body.len() + chunk.len() <= 1, TransferError::InvalidRange);
            body.extend_from_slice(&chunk);
        }
        ensure!(body.len() == 1, TransferError::InvalidRange);
    }
    Ok(shape)
}

struct Progress {
    tx: watch::Sender<DownloadProgress>,
    downloaded: AtomicU64,
    active: AtomicUsize,
    peak: AtomicUsize,
    retries: AtomicUsize,
    total: Option<u64>,
    ranges: bool,
    restarted: bool,
}

// Legacy downloads have no trustworthy version validator. Compare every saved
// byte to the current resource before adopting them; ZIP magic alone is not an identity.
async fn verify_saved(
    downloader: &SegmentedDownloader,
    url: &Url,
    headers: &header::HeaderMap,
    path: &Path,
    shape: &Shape,
    spans: Vec<(u64, u64)>,
    control: &DownloadControl,
) -> Result<bool> {
    let mut file = File::open(path).await?;
    for (start, length) in spans.into_iter().filter(|(_, length)| *length > 0) {
        let end = start + length - 1;
        let mut request = downloader
            .client
            .client()?
            .get(url.clone())
            .headers(headers.clone())
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::RANGE, format!("bytes={start}-{end}"));
        if let Some(validator) = shape.etag.as_ref().or(shape.modified.as_ref()) {
            request = request.header(header::IF_RANGE, validator);
        }
        let mut response = send(request, control).await?;
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || content_range(&response) != Some((start, end, shape.total))
        {
            return Ok(false);
        }
        check_encoding(&response)?;
        file.seek(std::io::SeekFrom::Start(start)).await?;
        let mut read = 0;
        while let Some(chunk) = tokio::select! { biased; _ = control.paused() => return Err(TransferError::Paused.into()), result = tokio::time::timeout(Duration::from_secs(25), response.chunk()) => result.context("校验已下载字节超时")?.map_err(reqwest::Error::without_url)? }
        {
            read += chunk.len() as u64;
            if read > length {
                return Ok(false);
            }
            let mut local = vec![0; chunk.len()];
            if file.read_exact(&mut local).await.is_err() || local.as_slice() != chunk.as_ref() {
                return Ok(false);
            }
        }
        if read != length {
            return Ok(false);
        }
    }
    Ok(true)
}
impl Progress {
    fn publish(&self) {
        self.tx.send_replace(DownloadProgress {
            downloaded_bytes: self.downloaded.load(Ordering::Relaxed),
            total_bytes: self.total,
            active_connections: self.active.load(Ordering::Relaxed),
            peak_connections: self.peak.load(Ordering::Relaxed),
            supports_resume: self.ranges,
            retry_count: self.retries.load(Ordering::Relaxed),
            restarted: self.restarted,
        });
    }
    fn connected(self: &Arc<Self>) -> Connection {
        let count = self.active.fetch_add(1, Ordering::Relaxed) + 1;
        self.peak.fetch_max(count, Ordering::Relaxed);
        self.publish();
        Connection(self.clone())
    }
}
struct Connection(Arc<Progress>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
        self.0.publish();
    }
}

#[derive(Clone)]
struct Transfer {
    client: DownloadClient,
    url: Url,
    path: PathBuf,
    headers: header::HeaderMap,
    shape: Shape,
    manifest: Arc<Mutex<Manifest>>,
    progress: Arc<Progress>,
    control: DownloadControl,
    retries: usize,
}
impl Transfer {
    async fn checkpoint(
        &self,
        file: &mut File,
        index: u64,
        written: u64,
        complete: bool,
    ) -> Result<()> {
        file.flush().await?;
        let mut manifest = self.manifest.lock().await;
        if complete && manifest.segment_size != 0 {
            manifest.partial.remove(&index);
            manifest.completed.insert(index);
        } else {
            manifest.partial.insert(index, written);
        }
        store(&self.path, &manifest).await
    }
    async fn part(&self, index: u64, range: Option<(u64, u64)>) -> Result<()> {
        for attempt in 0..=self.retries {
            if self.control.is_paused() {
                return Err(TransferError::Paused.into());
            }
            match self.part_once(index, range).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if error.downcast_ref::<TransferError>().is_some() || attempt == self.retries {
                        return Err(error);
                    }
                    self.progress.retries.fetch_add(1, Ordering::Relaxed);
                    self.progress.publish();
                    tokio::select! { biased; _ = self.control.paused() => return Err(TransferError::Paused.into()),
                    _ = tokio::time::sleep(Duration::from_millis(250 * (attempt as u64 + 1))) => {} }
                }
            }
        }
        unreachable!()
    }
    async fn part_once(&self, index: u64, range: Option<(u64, u64)>) -> Result<()> {
        let mut written = self
            .manifest
            .lock()
            .await
            .partial
            .get(&index)
            .copied()
            .unwrap_or(0);
        let start = range.map(|r| r.0).unwrap_or(0);
        let end = range
            .map(|r| r.1)
            .or_else(|| self.shape.total.and_then(|n| n.checked_sub(1)));
        if end.is_some_and(|end| start + written > end) {
            return Ok(());
        }
        let request_range = range.is_some() || (written > 0 && self.shape.ranges);
        if written > 0 && !request_range {
            self.progress
                .downloaded
                .fetch_sub(written, Ordering::Relaxed);
            written = 0;
            self.manifest.lock().await.partial.insert(index, 0);
        }
        let request_start = start + written;
        let mut request = self
            .client
            .client()?
            .get(self.url.clone())
            .headers(self.headers.clone())
            .header(header::ACCEPT_ENCODING, "identity");
        if request_range {
            request = request.header(
                header::RANGE,
                end.map(|end| format!("bytes={request_start}-{end}"))
                    .unwrap_or_else(|| format!("bytes={request_start}-")),
            );
            if let Some(validator) = self.shape.etag.as_ref().or(self.shape.modified.as_ref()) {
                request = request.header(header::IF_RANGE, validator);
            }
        }
        let response = send(request, &self.control).await?;
        if request_range && response.status() == reqwest::StatusCode::OK {
            return Err(TransferError::RangeUnsupported.into());
        }
        if !response.status().is_success() {
            return Err(TransferError::Http(response.status().as_u16()).into());
        }
        check_encoding(&response)?;
        if request_range || response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let (actual_start, actual_end, total) =
                content_range(&response).ok_or(TransferError::InvalidRange)?;
            ensure!(
                actual_start == request_start
                    && end.is_none_or(|end| actual_end == end)
                    && total == self.shape.total,
                TransferError::InvalidRange
            );
        }
        if let Some(expected) = self.shape.etag.as_ref() {
            if let Some(actual) = text_header(response.headers(), header::ETAG) {
                ensure!(&actual == expected, TransferError::InvalidRange);
            }
        }
        let _connection = self.progress.connected();
        let mut file = OpenOptions::new().write(true).open(&self.path).await?;
        if range.is_none() && written == 0 {
            file.set_len(0).await?;
        }
        file.seek(std::io::SeekFrom::Start(start + written)).await?;
        let mut body = response.bytes_stream();
        let mut checkpoint_bytes = written;
        let mut checkpoint_time = Instant::now();
        loop {
            let chunk = tokio::select! { biased;
                _ = self.control.paused() => { self.checkpoint(&mut file, index, written, false).await?; return Err(TransferError::Paused.into()); },
                result = tokio::time::timeout(Duration::from_secs(25), body.next()) => result,
            };
            let chunk = match chunk {
                Ok(Some(Ok(chunk))) => chunk,
                Ok(None) => break,
                Ok(Some(Err(error))) => {
                    self.checkpoint(&mut file, index, written, false).await?;
                    return Err(error.without_url().into());
                }
                Err(_) => {
                    self.checkpoint(&mut file, index, written, false).await?;
                    return Err(anyhow!("传输等待数据超时，进度已保存"));
                }
            };
            ensure!(
                end.is_none_or(|end| start + written + chunk.len() as u64 <= end + 1),
                TransferError::InvalidRange
            );
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            self.progress
                .downloaded
                .fetch_add(chunk.len() as u64, Ordering::Relaxed);
            self.progress.publish();
            if written - checkpoint_bytes >= 256 * 1024
                || checkpoint_time.elapsed() >= Duration::from_millis(400)
            {
                self.checkpoint(&mut file, index, written, false).await?;
                checkpoint_bytes = written;
                checkpoint_time = Instant::now();
            }
        }
        if end.is_some_and(|end| written != end - start + 1) {
            self.checkpoint(&mut file, index, written, false).await?;
            return Err(anyhow!("服务器提前结束文件传输"));
        }
        self.checkpoint(&mut file, index, written, true).await?;
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    downloader: &SegmentedDownloader,
    url: Url,
    destination: PathBuf,
    headers: header::HeaderMap,
    resume_key: Option<String>,
    tx: watch::Sender<DownloadProgress>,
    control: DownloadControl,
) -> Result<()> {
    ensure!(downloader.config.segment_size > 0, "分段大小不能为 0");
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut shape = inspect(downloader, &url, &headers, &control).await?;
    // A range that is ignored later is retried as one full stream, never appended.
    for downgrade in 0..2 {
        let previous = load(&destination).await;
        let existing = tokio::fs::metadata(&destination)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        let identity = previous
            .as_ref()
            .is_some_and(|m| match resume_key.as_deref() {
                Some(key) => m.resume_key.as_deref() == Some(key),
                None => m.url == url.as_str(),
            });
        let mut same_resource = previous.as_ref().is_some_and(|m| {
            identity
                && m.total_bytes == shape.total.unwrap_or(0)
                && (m.bytes() == 0
                    || if m.segment_size > 0 {
                        existing == m.total_bytes
                    } else {
                        existing >= m.bytes()
                    })
                && m.etag
                    .as_ref()
                    .is_none_or(|etag| Some(etag) == shape.etag.as_ref())
                && (m.etag.is_some()
                    || m.last_modified
                        .as_ref()
                        .is_none_or(|modified| Some(modified) == shape.modified.as_ref()))
        });
        if same_resource && shape.ranges {
            let saved = previous.as_ref().unwrap();
            if saved.etag.is_none() && saved.last_modified.is_none() && saved.bytes() > 0 {
                let spans = if saved.segment_size == 0 {
                    vec![(0, saved.bytes())]
                } else {
                    saved
                        .completed
                        .iter()
                        .map(|i| {
                            (
                                *i * saved.segment_size,
                                segment_len(*i, saved.segment_size, saved.total_bytes),
                            )
                        })
                        .chain(
                            saved
                                .partial
                                .iter()
                                .filter(|(i, _)| !saved.completed.contains(i))
                                .map(|(i, n)| (*i * saved.segment_size, *n)),
                        )
                        .collect()
                };
                same_resource = verify_saved(
                    downloader,
                    &url,
                    &headers,
                    &destination,
                    &shape,
                    spans,
                    &control,
                )
                .await?;
            }
        }
        let segmented = shape.ranges
            && shape
                .total
                .is_some_and(|n| n > downloader.config.segment_size);
        let segment_size = if segmented {
            previous
                .as_ref()
                .filter(|_| same_resource)
                .map(|m| m.segment_size)
                .filter(|n| *n > 0)
                .unwrap_or(downloader.config.segment_size)
        } else {
            0
        };
        let legacy_prefix = if previous.is_none()
            && resume_key.is_some()
            && shape.ranges
            && shape.total.is_some_and(|n| existing < n)
        {
            if existing > 0
                && verify_saved(
                    downloader,
                    &url,
                    &headers,
                    &destination,
                    &shape,
                    vec![(0, existing)],
                    &control,
                )
                .await?
            {
                existing
            } else {
                0
            }
        } else {
            0
        };
        let keep_manifest = same_resource
            && previous
                .as_ref()
                .is_some_and(|m| m.segment_size == segment_size)
            && shape.ranges;
        let prefix = if keep_manifest {
            0
        } else if same_resource && shape.ranges {
            previous.as_ref().map(Manifest::prefix).unwrap_or(0)
        } else {
            legacy_prefix
        };
        let mut manifest = if keep_manifest {
            previous.clone().unwrap()
        } else {
            Manifest {
                url: String::new(),
                resume_key: resume_key.clone(),
                total_bytes: shape.total.unwrap_or(0),
                segment_size,
                completed: BTreeSet::new(),
                partial: BTreeMap::new(),
                etag: shape.etag.clone(),
                last_modified: shape.modified.clone(),
            }
        };
        if !keep_manifest && prefix > 0 {
            if let Some(whole) = prefix.checked_div(segment_size) {
                for index in 0..whole {
                    manifest.completed.insert(index);
                }
                if prefix % segment_size > 0 {
                    manifest.partial.insert(whole, prefix % segment_size);
                }
            } else {
                manifest.partial.insert(0, prefix);
            }
        }
        manifest.url = resume_key.clone().unwrap_or_else(|| url.to_string());
        manifest.etag = shape.etag.clone();
        manifest.last_modified = shape.modified.clone();
        let initial = manifest.bytes();
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&destination)
            .await?;
        file.set_len(if segmented {
            shape.total.unwrap()
        } else {
            initial
        })
        .await?;
        store(&destination, &manifest).await?;
        let progress = Arc::new(Progress {
            tx: tx.clone(),
            downloaded: AtomicU64::new(initial),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            retries: AtomicUsize::new(0),
            total: shape.total,
            ranges: shape.ranges,
            restarted: existing > 0 && initial == 0,
        });
        progress.publish();
        let transfer = Transfer {
            client: downloader.client.clone(),
            url: url.clone(),
            path: destination.clone(),
            headers: headers.clone(),
            shape: shape.clone(),
            manifest: Arc::new(Mutex::new(manifest)),
            progress: progress.clone(),
            control: control.clone(),
            retries: downloader.config.retries,
        };
        let result = if segmented {
            let total = shape.total.unwrap();
            let count = total.div_ceil(segment_size);
            let completed = transfer.manifest.lock().await.completed.clone();
            let mut parts = stream::iter((0..count).filter(|i| !completed.contains(i)))
                .map(|index| {
                    let transfer = transfer.clone();
                    async move {
                        transfer
                            .part(
                                index,
                                Some((
                                    index * segment_size,
                                    ((index + 1) * segment_size).min(total) - 1,
                                )),
                            )
                            .await
                    }
                })
                .buffer_unordered(downloader.config.concurrency.clamp(1, 8));
            let mut paused = false;
            let mut error = None;
            while let Some(result) = parts.next().await {
                if let Err(result) = result {
                    if matches!(
                        result.downcast_ref::<TransferError>(),
                        Some(TransferError::Paused)
                    ) {
                        paused = true;
                    } else if error.is_none() {
                        error = Some(result);
                    }
                }
            }
            if let Some(error) = error {
                Err(error)
            } else if paused {
                Err(TransferError::Paused.into())
            } else {
                Ok(())
            }
        } else {
            transfer.part(0, None).await
        };
        match result {
            Err(error)
                if matches!(
                    error.downcast_ref::<TransferError>(),
                    Some(TransferError::RangeUnsupported)
                ) && downgrade == 0 =>
            {
                discard_partial(&destination).await?;
                shape.ranges = false;
            }
            other => {
                if other.is_ok() {
                    let file = OpenOptions::new().write(true).open(&destination).await?;
                    file.sync_data().await?;
                    if let Some(total) = shape.total {
                        ensure!(
                            file.metadata().await?.len() == total,
                            "下载长度与服务器不一致"
                        );
                    }
                    tokio::fs::remove_file(manifest_path(&destination)).await?;
                }
                return other;
            }
        }
    }
    Err(anyhow!("服务器无法完成文件传输"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_completed_segments_remain_readable() {
        let old: Manifest = serde_json::from_str(r#"{"url":"https://example.org/book","resumeKey":"zlibrary:123:hash","totalBytes":16777216,"segmentSize":8388608,"completed":[0]}"#).unwrap();
        assert_eq!(old.bytes(), 8 * 1024 * 1024);
        assert_eq!(old.prefix(), 8 * 1024 * 1024);
    }
}

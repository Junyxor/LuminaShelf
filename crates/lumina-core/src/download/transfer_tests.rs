use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};

#[derive(Clone, Copy)]
pub(super) enum Behavior {
    Disconnect,
    UndeclaredRanges,
    IgnoreRanges,
    WrongRange,
    Slow,
    Expired,
    LateIgnoreRanges,
    Unsatisfiable,
    StallHeaders,
    TransientProbe,
    SlowProbe,
    EncodedProbe,
}

pub(super) struct Fixture {
    pub(super) url: Url,
    pub(super) data: Arc<Vec<u8>>,
    pub(super) ranges: Arc<Mutex<Vec<(u64, u64)>>>,
    pub(super) peak: Arc<AtomicUsize>,
    pub(super) version: Arc<AtomicUsize>,
    pub(super) directory: PathBuf,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        assert_eq!(
            self.directory.parent(),
            Some(std::env::temp_dir().as_path())
        );
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Fixture {
    pub(super) async fn new(behavior: Behavior) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let url = Url::parse(&format!("http://{}/book", listener.local_addr().unwrap())).unwrap();
        let data = Arc::new(if matches!(behavior, Behavior::Slow | Behavior::Expired) {
            b"A reliable book line for download recovery.\n".repeat(2400)
        } else {
            (0..96 * 1024)
                .map(|i| ((i * 31 + i / 7) % 251) as u8)
                .collect::<Vec<_>>()
        });
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let full_requests = Arc::new(AtomicUsize::new(0));
        let probe_requests = Arc::new(AtomicUsize::new(0));
        let server_data = data.clone();
        let server_ranges = ranges.clone();
        let server_peak = peak.clone();
        let version = Arc::new(AtomicUsize::new(0));
        let server_version = version.clone();
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let data = server_data.clone();
                let ranges = server_ranges.clone();
                let active = active.clone();
                let peak = server_peak.clone();
                let full_requests = full_requests.clone();
                let probe_requests = probe_requests.clone();
                let version = server_version.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 2048];
                    while !request.ends_with(b"\r\n\r\n") {
                        let count = socket.read(&mut buffer).await.unwrap_or(0);
                        if count == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                    let version = version.load(Ordering::SeqCst);
                    let data = if version > 0 {
                        Arc::new(data.iter().map(|b| b.wrapping_add(1)).collect::<Vec<_>>())
                    } else {
                        data
                    };
                    if matches!(behavior, Behavior::Expired)
                        && request
                            .lines()
                            .next()
                            .is_some_and(|line| line.contains("token=1"))
                    {
                        let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        return;
                    }
                    if request.starts_with("head ") {
                        let declaration = if matches!(behavior, Behavior::UndeclaredRanges) {
                            ""
                        } else {
                            "Accept-Ranges: bytes\r\n"
                        };
                        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{declaration}ETag: \"fixture-v1\"\r\nConnection: close\r\n\r\n", data.len());
                        let _ = socket.write_all(header.as_bytes()).await;
                        return;
                    }
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .and_then(|range| {
                            let (start, end) = range.split_once('-')?;
                            Some((
                                start.parse::<u64>().ok()?,
                                end.parse::<u64>().unwrap_or(data.len() as u64 - 1),
                            ))
                        });
                    if matches!(behavior, Behavior::TransientProbe)
                        && range == Some((0, 0))
                        && probe_requests.fetch_add(1, Ordering::SeqCst) == 0
                    {
                        return;
                    }
                    if matches!(behavior, Behavior::StallHeaders) && range != Some((0, 0)) {
                        tokio::time::sleep(std::time::Duration::from_secs(40)).await;
                        return;
                    }
                    if matches!(behavior, Behavior::Unsatisfiable)
                        && range.is_some_and(|(start, end)| end > start)
                    {
                        let _ = socket.write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        return;
                    }
                    if let Some((start, end)) = range.filter(|range| {
                        !matches!(behavior, Behavior::IgnoreRanges)
                            && (!matches!(behavior, Behavior::LateIgnoreRanges) || *range == (0, 0))
                    }) {
                        ranges.lock().await.push((start, end));
                        if end > start {
                            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
                        }
                        let offset =
                            u64::from(matches!(behavior, Behavior::WrongRange) && end > start);
                        let encoding = if matches!(behavior, Behavior::EncodedProbe) {
                            "Content-Encoding: gzip\r\n"
                        } else {
                            ""
                        };
                        let header = format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n{encoding}ETag: \"fixture-v{version}\"\r\nConnection: close\r\n\r\n", end - start + 1, start + offset, end + offset, data.len());
                        let _ = socket.write_all(header.as_bytes()).await;
                        if matches!(behavior, Behavior::SlowProbe) && range == Some((0, 0)) {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                            return;
                        }
                        let body = &data[start as usize..=end as usize];
                        if matches!(behavior, Behavior::Slow) && end > start {
                            for chunk in body.chunks(1024) {
                                if socket.write_all(chunk).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                            }
                        } else {
                            let _ = socket.write_all(body).await;
                        }
                        if end > start {
                            active.fetch_sub(1, Ordering::SeqCst);
                        }
                    } else {
                        let attempt = full_requests.fetch_add(1, Ordering::SeqCst);
                        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"fixture-v{version}\"\r\nConnection: close\r\n\r\n", data.len());
                        let _ = socket.write_all(header.as_bytes()).await;
                        let body = if matches!(behavior, Behavior::Disconnect) && attempt == 0 {
                            &data[..8192]
                        } else {
                            &data[..]
                        };
                        if matches!(behavior, Behavior::Slow) {
                            for chunk in body.chunks(1024) {
                                if socket.write_all(chunk).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                            }
                        } else {
                            let _ = socket.write_all(body).await;
                        }
                    }
                });
            }
        });
        let directory = std::env::temp_dir().join(format!("lumina-transfer-{}", Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        Self {
            url,
            data,
            ranges,
            peak,
            version,
            directory,
            server,
        }
    }

    fn downloader(&self, concurrency: usize, segment_size: u64) -> SegmentedDownloader {
        SegmentedDownloader::new(
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            DownloadConfig {
                concurrency,
                segment_size,
                retries: 2,
            },
        )
    }
}

#[tokio::test]
async fn interrupted_body_is_retried_from_the_saved_offset() {
    let fixture = Fixture::new(Behavior::Disconnect).await;
    let path = fixture.directory.join("book.bin");
    let (tx, _) = watch::channel(DownloadProgress::default());
    let result = fixture
        .downloader(1, 1024 * 1024)
        .download_to(fixture.url.clone(), path.clone(), tx)
        .await;
    assert!(
        result.is_ok(),
        "An interrupted body must resume automatically: {result:?}"
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), *fixture.data);
    assert!(
        fixture
            .ranges
            .lock()
            .await
            .iter()
            .any(|(start, _)| *start == 8192),
        "Retry did not request the bytes after the saved prefix"
    );
}

#[tokio::test]
async fn an_interrupted_initial_probe_is_retried_before_marking_the_task_failed() {
    let fixture = Fixture::new(Behavior::TransientProbe).await;
    let path = fixture.directory.join("book.bin");
    let (tx, _) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(2, 16 * 1024)
        .download_to(fixture.url.clone(), path.clone(), tx)
        .await
        .expect("A dropped initial probe must be retried just like a dropped file body");
    assert_eq!(tokio::fs::read(path).await.unwrap(), *fixture.data);
}

#[tokio::test]
async fn pause_interrupts_a_probe_that_has_headers_but_no_body() {
    let fixture = Fixture::new(Behavior::SlowProbe).await;
    let downloader = fixture.downloader(2, 16 * 1024);
    let (tx, _) = watch::channel(DownloadProgress::default());
    let control = DownloadControl::default();
    let worker_control = control.clone();
    let url = fixture.url.clone();
    let path = fixture.directory.join("book.bin");
    let worker = tokio::spawn(async move {
        downloader
            .download_controlled(
                url,
                path,
                header::HeaderMap::new(),
                None,
                tx,
                worker_control,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while fixture.ranges.lock().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    control.pause();
    let error = tokio::time::timeout(std::time::Duration::from_millis(300), worker)
        .await
        .expect("Pause ignored while probe waited for its first byte")
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<TransferError>(),
        Some(TransferError::Paused)
    ));
}

#[tokio::test]
async fn a_probe_encoding_violation_is_not_retried_as_a_network_failure() {
    let fixture = Fixture::new(Behavior::EncodedProbe).await;
    let downloader = SegmentedDownloader::with_resolver(
        AppResolver::system().unwrap(),
        DownloadConfig {
            concurrency: 2,
            segment_size: 16 * 1024,
            retries: 2,
        },
    )
    .unwrap();
    let (tx, _) = watch::channel(DownloadProgress::default());
    let error = downloader
        .download_to(fixture.url.clone(), fixture.directory.join("book.bin"), tx)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("原始字节"));
    assert_eq!(
        fixture.ranges.lock().await.len(),
        1,
        "A deterministic encoding violation was treated as a transient connection failure"
    );
}

#[tokio::test]
async fn range_probe_enables_real_parallelism_without_a_head_declaration() {
    let fixture = Fixture::new(Behavior::UndeclaredRanges).await;
    let path = fixture.directory.join("book.bin");
    let (tx, _) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(3, 16 * 1024)
        .download_to(fixture.url.clone(), path.clone(), tx)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), *fixture.data);
    assert!(
        fixture.peak.load(Ordering::SeqCst) >= 2,
        "The server received no concurrent range requests"
    );
}

#[tokio::test]
async fn a_server_ignoring_ranges_falls_back_to_an_intact_single_stream() {
    let fixture = Fixture::new(Behavior::IgnoreRanges).await;
    let path = fixture.directory.join("book.bin");
    let (tx, _) = watch::channel(DownloadProgress::default());
    let result = fixture
        .downloader(3, 16 * 1024)
        .download_to(fixture.url.clone(), path.clone(), tx)
        .await;
    assert!(
        result.is_ok(),
        "A server may ignore Range; the full stream should still download: {result:?}"
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), *fixture.data);
}

#[tokio::test]
async fn a_mismatched_content_range_is_rejected_before_merge() {
    let fixture = Fixture::new(Behavior::WrongRange).await;
    let path = fixture.directory.join("book.bin");
    let (tx, _) = watch::channel(DownloadProgress::default());
    let result = fixture
        .downloader(3, 16 * 1024)
        .download_to(fixture.url.clone(), path, tx)
        .await;
    assert!(
        result.is_err(),
        "Wrong byte positions must not become a completed book"
    );
}

async fn pause_segmented(fixture: &Fixture, path: &Path) -> u64 {
    let downloader = fixture.downloader(3, 16 * 1024);
    let (tx, mut rx) = watch::channel(DownloadProgress::default());
    let control = DownloadControl::default();
    let worker_control = control.clone();
    let url = fixture.url.clone();
    let destination = path.to_path_buf();
    let worker = tokio::spawn(async move {
        downloader
            .download_controlled(
                url,
                destination,
                header::HeaderMap::new(),
                Some("stable-book".into()),
                tx,
                worker_control,
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while rx.borrow_and_update().downloaded_bytes < 8192 {
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    control.pause();
    assert!(matches!(
        worker
            .await
            .unwrap()
            .unwrap_err()
            .downcast_ref::<TransferError>(),
        Some(TransferError::Paused)
    ));
    let saved = tokio::fs::read(path).await.unwrap();
    let bytes = partial_progress(path).await;
    assert!(bytes >= 8192 && bytes < fixture.data.len() as u64);
    tokio::time::sleep(std::time::Duration::from_millis(60)).await;
    assert_eq!(
        tokio::fs::read(path).await.unwrap(),
        saved,
        "A paused range was still writing"
    );
    bytes
}

#[tokio::test]
async fn segmented_resume_keeps_out_of_order_bytes_with_a_new_url_and_connection_limit() {
    let fixture = Fixture::new(Behavior::Slow).await;
    let path = fixture.directory.join("book.bin");
    let saved = pause_segmented(&fixture, &path).await;
    let mut url = fixture.url.clone();
    url.set_query(Some("signature=refreshed"));
    let (tx, rx) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(1, 64 * 1024)
        .download_to_with_context(
            url,
            path.clone(),
            header::HeaderMap::new(),
            Some("stable-book".into()),
            tx,
        )
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(path).await.unwrap(), *fixture.data);
    assert_eq!(rx.borrow().peak_connections, 1);
    assert!(fixture
        .ranges
        .lock()
        .await
        .iter()
        .any(|(start, _)| start % (16 * 1024) > 0));
    assert!(saved > 0);
}

#[tokio::test]
async fn a_changed_etag_restarts_without_combining_versions() {
    let fixture = Fixture::new(Behavior::Slow).await;
    let path = fixture.directory.join("book.bin");
    pause_segmented(&fixture, &path).await;
    fixture.version.store(1, Ordering::SeqCst);
    let (tx, rx) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(3, 16 * 1024)
        .download_to_with_context(
            fixture.url.clone(),
            path.clone(),
            header::HeaderMap::new(),
            Some("stable-book".into()),
            tx,
        )
        .await
        .unwrap();
    assert!(rx.borrow().restarted);
    assert_eq!(
        tokio::fs::read(path).await.unwrap(),
        fixture
            .data
            .iter()
            .map(|b| b.wrapping_add(1))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn range_support_disappearing_after_probe_safely_downgrades() {
    let fixture = Fixture::new(Behavior::LateIgnoreRanges).await;
    let path = fixture.directory.join("book.bin");
    let (tx, rx) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(4, 16 * 1024)
        .download_to(fixture.url.clone(), path.clone(), tx)
        .await
        .unwrap();
    assert!(!rx.borrow().supports_resume);
    assert_eq!(tokio::fs::read(path).await.unwrap(), *fixture.data);
}

#[tokio::test]
async fn an_unsatisfiable_range_never_becomes_a_completed_file() {
    let fixture = Fixture::new(Behavior::Unsatisfiable).await;
    let (tx, _) = watch::channel(DownloadProgress::default());
    let error = fixture
        .downloader(2, 16 * 1024)
        .download_to(fixture.url.clone(), fixture.directory.join("book.bin"), tx)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<TransferError>(),
        Some(TransferError::Http(416))
    ));
}

#[tokio::test]
async fn a_legacy_prefix_with_identical_magic_but_different_bytes_is_not_adopted() {
    let fixture = Fixture::new(Behavior::UndeclaredRanges).await;
    let path = fixture.directory.join("book.bin");
    let mut wrong = fixture.data[..8192].to_vec();
    wrong[4096] ^= 127;
    tokio::fs::write(&path, wrong).await.unwrap();
    let (tx, rx) = watch::channel(DownloadProgress::default());
    fixture
        .downloader(2, 16 * 1024)
        .download_to_with_context(
            fixture.url.clone(),
            path.clone(),
            header::HeaderMap::new(),
            Some("stable-book".into()),
            tx,
        )
        .await
        .unwrap();
    assert!(rx.borrow().restarted);
    assert_eq!(tokio::fs::read(path).await.unwrap(), *fixture.data);
}

#[tokio::test]
async fn stalled_response_headers_fail_within_the_idle_deadline() {
    let fixture = Fixture::new(Behavior::StallHeaders).await;
    let downloader = SegmentedDownloader::new(
        Client::builder().no_proxy().build().unwrap(),
        DownloadConfig {
            concurrency: 1,
            segment_size: 1024 * 1024,
            retries: 0,
        },
    );
    let (tx, _) = watch::channel(DownloadProgress::default());
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(28),
        downloader.download_to(fixture.url.clone(), fixture.directory.join("book.bin"), tx),
    )
    .await
    .expect("Stalled headers hung forever")
    .unwrap_err();
    assert!(format!("{error:#}").contains("响应超时"));
}

#[tokio::test]
async fn cross_origin_file_redirects_do_not_forward_custom_account_headers() {
    let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let target_url = format!("http://{}/bytes", target.local_addr().unwrap());
    let sink = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).to_ascii_lowercase();
            assert!(
                !request.contains("remix-user-key")
                    && !request.contains("authorization:")
                    && !request.contains("cookie:"),
                "Account credentials crossed download origin"
            );
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nbook body",
                )
                .await
                .unwrap();
        }
    });
    let source = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let url = Url::parse(&format!("http://{}/download", source.local_addr().unwrap())).unwrap();
    let redirect = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut stream, _) = source.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).await.unwrap();
            assert!(
                String::from_utf8_lossy(&bytes[..count]).contains("remix-user-key: private-key")
            );
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        }
    });
    let directory = std::env::temp_dir().join(format!("lumina-redirect-{}", Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("book.txt");
    let mut headers = header::HeaderMap::new();
    headers.insert("remix-user-key", "private-key".parse().unwrap());
    headers.insert(header::AUTHORIZATION, "Bearer secret".parse().unwrap());
    headers.insert(header::COOKIE, "session=secret".parse().unwrap());
    let downloader = SegmentedDownloader::with_resolver(
        AppResolver::system().unwrap(),
        DownloadConfig::default(),
    )
    .unwrap();
    let (tx, _) = watch::channel(DownloadProgress::default());
    downloader
        .download_to_with_headers(url, path.clone(), headers, tx)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"book body");
    redirect.await.unwrap();
    sink.await.unwrap();
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

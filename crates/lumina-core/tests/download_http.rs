use lumina_core::download::{DownloadConfig, DownloadProgress, SegmentedDownloader};
use reqwest::{header::HeaderMap, Client};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::watch,
};
use url::Url;

const BOOK: &[u8] = b"abcdefghijkl";

struct Server {
    url: Url,
    ranges: Arc<Mutex<Vec<String>>>,
    handle: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn serve(fail_second_once: bool, wrong_range: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = Url::parse(&format!(
        "http://{}/book?token=temporary-secret",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let requests = ranges.clone();
    let fail = Arc::new(AtomicBool::new(fail_second_once));
    let handle = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let requests = requests.clone();
            let fail = fail.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                    let mut byte = [0];
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request);
                let head = request.starts_with("HEAD ");
                let range = request.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                });
                let (start, end) = range
                    .as_deref()
                    .and_then(|range| range.split_once('-'))
                    .map(|(start, end)| {
                        (
                            start.parse::<usize>().unwrap(),
                            end.parse::<usize>().unwrap_or(BOOK.len() - 1),
                        )
                    })
                    .unwrap_or((0, BOOK.len() - 1));
                if !head {
                    requests
                        .lock()
                        .unwrap()
                        .push(range.clone().unwrap_or_default());
                    if start == 4 && fail.swap(false, Ordering::SeqCst) {
                        let _ = socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                        return;
                    }
                }
                let is_range = range.is_some() && !head;
                let status = if is_range {
                    "206 Partial Content"
                } else {
                    "200 OK"
                };
                let content_range = if is_range {
                    format!(
                        "Content-Range: bytes {}-{end}/{}\r\n",
                        if wrong_range { start + 1 } else { start },
                        BOOK.len()
                    )
                } else {
                    String::new()
                };
                let response = format!("HTTP/1.1 {status}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\n{content_range}Connection: close\r\n\r\n", if head { BOOK.len() } else { end-start+1 });
                let _ = socket.write_all(response.as_bytes()).await;
                if !head {
                    let _ = socket.write_all(&BOOK[start..=end]).await;
                }
            });
        }
    });
    Server {
        url,
        ranges,
        handle,
    }
}

fn downloader(segment_size: u64) -> SegmentedDownloader {
    SegmentedDownloader::new(
        Client::builder().no_proxy().build().unwrap(),
        DownloadConfig {
            concurrency: 1,
            segment_size,
            retries: 0,
        },
    )
}

#[tokio::test]
async fn segmented_retry_reuses_completed_ranges_and_produces_exact_bytes() {
    let server = serve(true, false).await;
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("book.part");
    let (tx, _rx) = watch::channel(DownloadProgress::default());
    assert!(downloader(4)
        .download_to_with_context(
            server.url.clone(),
            destination.clone(),
            HeaderMap::new(),
            Some("public:1:txt".into()),
            tx
        )
        .await
        .is_err());
    let manifest =
        std::fs::read_to_string(format!("{}.lumina-part.json", destination.display())).unwrap();
    assert!(!manifest.contains("temporary-secret"));
    server.ranges.lock().unwrap().clear();
    let (tx, rx) = watch::channel(DownloadProgress::default());
    downloader(4)
        .download_to_with_context(
            server.url.clone(),
            destination.clone(),
            HeaderMap::new(),
            Some("public:1:txt".into()),
            tx,
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), BOOK);
    assert!(
        !server.ranges.lock().unwrap().contains(&"0-3".to_string()),
        "completed first segment must not be downloaded again"
    );
    assert_eq!(rx.borrow().downloaded_bytes, 12);
}

#[tokio::test]
async fn sequential_retry_only_requests_missing_bytes() {
    let server = serve(false, false).await;
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("book.part");
    std::fs::write(&destination, b"abcd").unwrap();
    let (tx, _rx) = watch::channel(DownloadProgress::default());
    downloader(64)
        .download_to(server.url.clone(), destination.clone(), tx)
        .await
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), BOOK);
    assert_eq!(*server.ranges.lock().unwrap(), vec!["4-"]);
}

#[tokio::test]
async fn incorrect_content_range_is_rejected_instead_of_corrupting_a_book() {
    let server = serve(false, true).await;
    let temp = tempfile::tempdir().unwrap();
    let destination = temp.path().join("book.part");
    let (tx, _rx) = watch::channel(DownloadProgress::default());
    let error = downloader(4)
        .download_to(server.url.clone(), destination, tx)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Content-Range"));
}

use lumina_core::{
    download::{DownloadConfig, SegmentedDownloader},
    library::scan_folder,
    AppResolver, BookDetails, BookFormat, GutendexProvider, LibraryItem, ProviderDescriptor,
    ProviderRegistry, SearchQuery, SearchResult,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    resolver: AppResolver,
    providers: ProviderRegistry,
}

impl AppState {
    fn new() -> Result<Self, String> {
        let resolver = AppResolver::system().map_err(|error| error.to_string())?;
        let providers = ProviderRegistry::default();
        let gutendex = GutendexProvider::new(resolver.clone()).map_err(|error| error.to_string())?;
        providers.register(gutendex);
        Ok(Self {
            resolver,
            providers,
        })
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoreStatus {
    name: &'static str,
    version: &'static str,
    rust_core: bool,
    network_stack: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressEvent {
    provider_id: String,
    book_id: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadReceipt {
    path: PathBuf,
    item: LibraryItem,
}

#[tauri::command]
fn core_status() -> CoreStatus {
    CoreStatus {
        name: "LuminaShelf",
        version: env!("CARGO_PKG_VERSION"),
        rust_core: true,
        network_stack: "Tokio · Reqwest · Hickory",
    }
}

#[tauri::command]
fn provider_descriptors(state: State<'_, AppState>) -> Vec<ProviderDescriptor> {
    state.providers.descriptors()
}

#[tauri::command]
async fn search_books(
    state: State<'_, AppState>,
    provider_id: String,
    text: String,
    page: Option<u32>,
    page_size: Option<u16>,
    formats: Option<Vec<BookFormat>>,
) -> Result<SearchResult, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("search text cannot be empty".to_string());
    }
    state
        .providers
        .search(
            &provider_id,
            SearchQuery {
                text,
                page: page.unwrap_or(1).max(1),
                page_size: page_size.unwrap_or(24).clamp(1, 50),
                formats: formats.unwrap_or_default(),
            },
        )
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn book_details(
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
) -> Result<BookDetails, String> {
    state
        .providers
        .details(&provider_id, &book_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn scan_library(
    path: String,
    recursive: Option<bool>,
    max_items: Option<usize>,
) -> Result<Vec<LibraryItem>, String> {
    let paths = scan_folder(
        path,
        recursive.unwrap_or(true),
        max_items.unwrap_or(500).clamp(1, 5000),
    )
    .map_err(|error| error.to_string())?;
    paths
        .into_iter()
        .map(|path| LibraryItem::inspect(path, None, None, None).map_err(|error| error.to_string()))
        .collect()
}

#[tauri::command]
async fn download_book(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
) -> Result<DownloadReceipt, String> {
    let url = state
        .providers
        .acquisition_url(&provider_id, &book_id, format)
        .await
        .map_err(|error| error.to_string())?;
    let headers = state
        .providers
        .download_headers(&provider_id)
        .await
        .map_err(|error| error.to_string())?;

    let download_dir = app.path().download_dir().map_err(|error| error.to_string())?;
    let destination_dir = download_dir.join("LuminaShelf");
    let destination = unique_destination(&destination_dir, &title, format.extension());
    let downloader =
        SegmentedDownloader::with_resolver(state.resolver.clone(), DownloadConfig::default())
            .map_err(|error| error.to_string())?;
    let (tx, mut rx) = tokio::sync::watch::channel(Default::default());

    let progress_app = app.clone();
    let progress_provider = provider_id.clone();
    let progress_book = book_id.clone();
    let monitor = tauri::async_runtime::spawn(async move {
        while rx.changed().await.is_ok() {
            let progress = *rx.borrow_and_update();
            let _ = progress_app.emit(
                "download-progress",
                DownloadProgressEvent {
                    provider_id: progress_provider.clone(),
                    book_id: progress_book.clone(),
                    downloaded_bytes: progress.downloaded_bytes,
                    total_bytes: progress.total_bytes,
                },
            );
        }
    });

    let result = downloader
        .download_to_with_context(
            url,
            destination.clone(),
            headers,
            Some(format!("{provider_id}:{book_id}")),
            tx,
        )
        .await;
    let _ = monitor.await;
    result.map_err(|error| error.to_string())?;

    let item = LibraryItem::inspect(
        &destination,
        Some(&title),
        Some(provider_id),
        Some(book_id),
    )
    .map_err(|error| error.to_string())?;
    Ok(DownloadReceipt {
        path: destination,
        item,
    })
}

fn unique_destination(directory: &Path, title: &str, extension: &str) -> PathBuf {
    let base = sanitize_filename(title);
    let first = directory.join(format!("{base}.{extension}"));
    if !first.exists() {
        return first;
    }
    for suffix in 2..10_000 {
        let candidate = directory.join(format!("{base} ({suffix}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{base}-{}.{}", std::process::id(), extension))
}

fn sanitize_filename(value: &str) -> String {
    let cleaned = value
        .trim()
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches([' ', '.']);
    if cleaned.is_empty() {
        "book".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = AppState::new().expect("failed to initialize LuminaShelf core bridge");
    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            core_status,
            provider_descriptors,
            search_books,
            book_details,
            scan_library,
            download_book
        ])
        .run(tauri::generate_context!())
        .expect("failed to run LuminaShelf");
}

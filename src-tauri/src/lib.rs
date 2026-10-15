mod announcement_commands;
mod imports;
mod library_commands;
mod network_commands;
mod pdf;
mod reader_bridge;
mod source_commands;

use lumina_core::network::NetworkClient;
use lumina_core::{
    download::{DownloadConfig, DownloadProgress, SegmentedDownloader},
    library::{now_unix_ms, scan_folder},
    AppResolver, BookDetails, BookFormat, GutendexProvider, LibraryItem, ProviderDescriptor,
    ProviderRegistry, ReadingProgress, ResolveResult, ResolverPolicy, SearchQuery, SearchResult,
    StateStore, ZLibraryHistoryPage, ZLibraryProfile, ZLibraryProvider,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use url::Url;

const ZLIBRARY_CANDIDATES: &[&str] = &[
    "https://zlib.bz",
    "https://z-library.sk",
    "https://1lib.sk",
    "https://z-lib.gd",
    "https://z-lib.gl",
    "https://z-library.ec",
    "https://article.sk",
    "https://articles.sk",
];

struct AppState {
    resolver: AppResolver,
    providers: ProviderRegistry,
    resolver_policy_path: PathBuf,
    state_store: StateStore,
    zlibrary: Arc<ZLibraryProvider>,
    zlibrary_probe: NetworkClient,
    zlibrary_config: RwLock<ZLibraryConfig>,
    zlibrary_config_path: PathBuf,
    book_dir: PathBuf,
}

impl AppState {
    fn new(config_dir: PathBuf) -> Result<Self, String> {
        let resolver_policy_path = config_dir.join("resolver-policy.json");
        let persisted = load_resolver_policy(&resolver_policy_path).ok();
        let resolver = match persisted.and_then(|policy| AppResolver::new(policy).ok()) {
            Some(resolver) => resolver,
            None => {
                let _ = fs::remove_file(&resolver_policy_path);
                AppResolver::system().map_err(|error| error.to_string())?
            }
        };
        let state_store = StateStore::open(config_dir.join("state.sqlite3"))
            .map_err(|error| error.to_string())?;

        let zlibrary_config_path = config_dir.join("zlibrary.json");
        let mut zlibrary_config = load_zlibrary_config(&zlibrary_config_path).unwrap_or_default();
        let initial_origin = zlibrary_config
            .origin
            .as_deref()
            .and_then(|value| normalize_origin(value).ok())
            .unwrap_or_else(|| {
                Url::parse(ZLIBRARY_CANDIDATES[0]).expect("valid Z-Library fallback")
            });
        if zlibrary_config.origin.is_none() {
            zlibrary_config.origin = Some(initial_origin.to_string());
        }

        let providers = ProviderRegistry::default();
        let gutendex =
            GutendexProvider::new(resolver.clone()).map_err(|error| error.to_string())?;
        providers.register(gutendex);

        let zlibrary = Arc::new(
            ZLibraryProvider::new(resolver.clone(), initial_origin)
                .map_err(|error| error.to_string())?,
        );
        providers.register_shared(zlibrary.clone());

        let zlibrary_probe = NetworkClient::new(
            resolver.clone(),
            "LuminaShelf/0.6 connection",
            Some(Duration::from_secs(12)),
        )
        .map_err(|error| error.to_string())?;

        Ok(Self {
            book_dir: config_dir.join("books"),
            resolver,
            providers,
            resolver_policy_path,
            state_store,
            zlibrary,
            zlibrary_probe,
            zlibrary_config: RwLock::new(zlibrary_config),
            zlibrary_config_path,
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

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ZLibraryOriginMode {
    #[default]
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ZLibraryConfig {
    origin: Option<String>,
    origin_mode: ZLibraryOriginMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ZLibraryAccountStatus {
    signed_in: bool,
    origin: String,
    origin_mode: ZLibraryOriginMode,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ZLibraryLoginResult {
    status: ZLibraryAccountStatus,
    profile: Option<ZLibraryProfile>,
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
async fn resolver_policy(state: State<'_, AppState>) -> Result<ResolverPolicy, String> {
    Ok(state.resolver.policy().await)
}

#[tauri::command]
async fn set_resolver_policy(
    state: State<'_, AppState>,
    policy: ResolverPolicy,
) -> Result<ResolverPolicy, String> {
    let previous = state.resolver.policy().await;
    state
        .resolver
        .set_policy(policy)
        .await
        .map_err(|error| error.to_string())?;
    let applied = state.resolver.policy().await;
    if let Err(error) = save_resolver_policy(&state.resolver_policy_path, &applied) {
        let _ = state.resolver.set_policy(previous).await;
        return Err(error);
    }
    Ok(applied)
}

#[tauri::command]
async fn resolve_host(state: State<'_, AppState>, host: String) -> Result<ResolveResult, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("host cannot be empty".to_string());
    }
    state
        .resolver
        .resolve(host)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn zlibrary_status(state: State<'_, AppState>) -> Result<ZLibraryAccountStatus, String> {
    Ok(zlibrary_account_status(&state).await)
}

#[tauri::command]
async fn zlibrary_login(
    state: State<'_, AppState>,
    email: String,
    password: String,
    automatic: bool,
    origin: Option<String>,
) -> Result<ZLibraryLoginResult, String> {
    let email = email.trim();
    if email.is_empty() || password.is_empty() {
        return Err("email and password are required".to_string());
    }

    let (selected_origin, origin_mode) = if automatic {
        (
            select_automatic_zlibrary_origin(&state).await?,
            ZLibraryOriginMode::Automatic,
        )
    } else {
        let requested = origin
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "manual EAPI origin is required".to_string())?;
        let origin = normalize_origin(requested)?;
        probe_zlibrary_origin(&state, &origin).await?;
        (origin, ZLibraryOriginMode::Manual)
    };

    let config = ZLibraryConfig {
        origin: Some(selected_origin.to_string()),
        origin_mode,
    };
    save_zlibrary_config(&state.zlibrary_config_path, &config)?;
    *state.zlibrary_config.write().await = config;

    state.zlibrary.clear_session().await;
    state
        .zlibrary
        .set_origin(selected_origin)
        .await
        .map_err(|error| error.to_string())?;
    state
        .zlibrary
        .login_direct(email, &password)
        .await
        .map_err(|error| error.to_string())?;

    let profile = state.zlibrary.profile().await.ok();
    Ok(ZLibraryLoginResult {
        status: zlibrary_account_status(&state).await,
        profile,
    })
}

#[tauri::command]
async fn zlibrary_logout(state: State<'_, AppState>) -> Result<ZLibraryAccountStatus, String> {
    state.zlibrary.clear_session().await;
    Ok(zlibrary_account_status(&state).await)
}

#[tauri::command]
async fn zlibrary_profile(state: State<'_, AppState>) -> Result<ZLibraryProfile, String> {
    state
        .zlibrary
        .profile()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn zlibrary_history(
    state: State<'_, AppState>,
    page: Option<u32>,
) -> Result<ZLibraryHistoryPage, String> {
    state
        .zlibrary
        .download_history(page.unwrap_or(1).max(1))
        .await
        .map_err(|error| error.to_string())
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
        .map_err(|error| format!("{error:#}"))
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
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
fn library_items(state: State<'_, AppState>) -> Result<Vec<LibraryItem>, String> {
    state
        .state_store
        .list_library_items()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn scan_library(
    state: State<'_, AppState>,
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
    let items = paths
        .into_iter()
        .map(|path| LibraryItem::inspect(path, None, None, None).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    state
        .state_store
        .upsert_library_items(&items)
        .map_err(|error| error.to_string())?;
    Ok(items)
}

#[tauri::command]
fn reading_progress(
    state: State<'_, AppState>,
    library_id: String,
) -> Result<Option<ReadingProgress>, String> {
    state
        .state_store
        .reading_progress(&library_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_reading_progress(
    state: State<'_, AppState>,
    library_id: String,
    locator: Option<String>,
    fraction: f64,
) -> Result<ReadingProgress, String> {
    let progress = ReadingProgress {
        library_id,
        locator,
        fraction,
        updated_at_unix_ms: now_unix_ms(),
    }
    .normalized();
    state
        .state_store
        .set_reading_progress(progress.clone())
        .map_err(|error| error.to_string())?;
    Ok(progress)
}

#[tauri::command]
async fn download_book(
    app: AppHandle,
    state: State<'_, AppState>,
    provider_id: String,
    book_id: String,
    title: String,
    format: BookFormat,
    download_dir: Option<String>,
) -> Result<DownloadReceipt, String> {
    let url = state
        .providers
        .acquisition_url(&provider_id, &book_id, format)
        .await
        .map_err(|error| error.to_string())?;
    let headers = state
        .providers
        .download_headers_for(&provider_id, &url)
        .await
        .map_err(|error| error.to_string())?;

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
    let destination = resumable_destination(
        &destination_dir,
        &title,
        format.extension(),
        &provider_id,
        &book_id,
    );
    if destination.is_file() && lumina_core::library::validate_readable_file(&destination).is_err()
    {
        let preserved = destination.with_file_name(format!(".invalid-{}", uuid::Uuid::new_v4()));
        fs::rename(&destination, preserved)
            .map_err(|error| format!("无法保留旧错误文件以重新下载：{error}"))?;
    }
    let downloader =
        SegmentedDownloader::with_resolver(state.resolver.clone(), DownloadConfig::default())
            .map_err(|error| error.to_string())?;
    let (tx, mut rx) = tokio::sync::watch::channel(DownloadProgress::default());

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
    result.map_err(|error| format!("{error:#}"))?;
    lumina_core::library::validate_readable_file(&destination)
        .map_err(|error| format!("下载文件校验失败，未加入已下载书架：{error:#}"))?;

    let item = LibraryItem::inspect(&destination, Some(&title), Some(provider_id), Some(book_id))
        .map_err(|error| error.to_string())?;
    state
        .state_store
        .upsert_library_item(&item)
        .map_err(|error| error.to_string())?;
    Ok(DownloadReceipt {
        path: destination,
        item,
    })
}

async fn zlibrary_account_status(state: &AppState) -> ZLibraryAccountStatus {
    let config = state.zlibrary_config.read().await.clone();
    ZLibraryAccountStatus {
        signed_in: state.zlibrary.has_session().await,
        origin: state.zlibrary.origin().await.to_string(),
        origin_mode: config.origin_mode,
    }
}

async fn select_automatic_zlibrary_origin(state: &AppState) -> Result<Url, String> {
    let configured = state.zlibrary_config.read().await.origin.clone();
    let mut candidates = Vec::with_capacity(ZLIBRARY_CANDIDATES.len() + 1);
    if let Some(origin) = configured {
        candidates.push(origin);
    }
    candidates.extend(ZLIBRARY_CANDIDATES.iter().map(|value| (*value).to_string()));

    let mut seen = HashSet::new();
    let mut failures = Vec::new();
    let mut origins = Vec::new();
    for candidate in candidates {
        let Ok(origin) = normalize_origin(&candidate) else {
            continue;
        };
        let key = origin.to_string();
        if !seen.insert(key.clone()) {
            continue;
        }
        origins.push(origin);
    }
    use futures::{stream, StreamExt};
    let probes = stream::iter(origins)
        .map(|origin| async move {
            let result = probe_zlibrary_origin(state, &origin).await;
            (origin, result)
        })
        .buffer_unordered(2);
    tokio::pin!(probes);
    while let Some((origin, result)) = probes.next().await {
        match result {
            Ok(()) => return Ok(origin),
            Err(error) => failures.push(format!("{}: {error}", origin.host_str().unwrap_or("?"))),
        }
    }

    Err(format!(
        "no usable Z-Library EAPI origin found ({})",
        failures.join("; ")
    ))
}

async fn probe_zlibrary_origin(state: &AppState, origin: &Url) -> Result<(), String> {
    let endpoint = origin
        .join("/eapi/info/domains")
        .map_err(|error| format!("build EAPI probe URL: {error}"))?;
    let response = state
        .zlibrary_probe
        .client()
        .map_err(|error| format!("{error:#}"))?
        .get(endpoint)
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|error| format!("EAPI probe failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("read EAPI probe: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| format!("expected EAPI JSON, received non-JSON HTTP {status}"))?;
    source_commands::parse_directory(&value)?;
    Ok(())
}

fn normalize_origin(value: &str) -> Result<Url, String> {
    let mut url = Url::parse(value).map_err(|error| format!("invalid EAPI origin: {error}"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("EAPI origin must be an HTTPS host".to_string());
    }
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn load_resolver_policy(path: &Path) -> Result<ResolverPolicy, String> {
    if !path.exists() {
        return Ok(ResolverPolicy::default());
    }
    let bytes = fs::read(path).map_err(|error| format!("read resolver policy: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("decode resolver policy: {error}"))
}

fn save_resolver_policy(path: &Path, policy: &ResolverPolicy) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(policy)
        .map_err(|error| format!("encode resolver policy: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("write resolver policy: {error}"))
}

fn load_zlibrary_config(path: &Path) -> Result<ZLibraryConfig, String> {
    if !path.exists() {
        return Ok(ZLibraryConfig::default());
    }
    let bytes = fs::read(path).map_err(|error| format!("read Z-Library config: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("decode Z-Library config: {error}"))
}

fn save_zlibrary_config(path: &Path, config: &ZLibraryConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config directory: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("encode Z-Library config: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("write Z-Library config: {error}"))
}

fn resumable_destination(
    directory: &Path,
    title: &str,
    extension: &str,
    provider_id: &str,
    book_id: &str,
) -> PathBuf {
    let title = sanitize_filename(title);
    let title = title.chars().take(80).collect::<String>();
    let identity = sanitize_filename(&format!("{provider_id}-{book_id}"));
    let identity = identity.chars().take(36).collect::<String>();
    directory.join(format!("{title} [{identity}].{extension}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumable_destination_is_stable_per_book() {
        let directory = Path::new("downloads");
        let first = resumable_destination(directory, "A / Book", "epub", "zlibrary", "12345");
        let second = resumable_destination(directory, "A / Book", "epub", "zlibrary", "12345");
        assert_eq!(first, second);
        assert!(first.to_string_lossy().contains("zlibrary-12345"));
    }

    #[test]
    fn resumable_destination_separates_books_with_same_title() {
        let directory = Path::new("downloads");
        let first = resumable_destination(directory, "Same Title", "epub", "zlibrary", "1");
        let second = resumable_destination(directory, "Same Title", "epub", "zlibrary", "2");
        assert_ne!(first, second);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            #[cfg(debug_assertions)]
            let config_dir = std::env::var_os("LUMINA_QA_DIR")
                .map(PathBuf::from)
                .unwrap_or(config_dir);
            let state = AppState::new(config_dir).map_err(std::io::Error::other)?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_status,
            announcement_commands::announcement_feeds,
            announcement_commands::check_app_update,
            announcement_commands::open_announcement_link,
            provider_descriptors,
            resolver_policy,
            set_resolver_policy,
            network_commands::test_network,
            network_commands::dns_profiles,
            network_commands::save_dns_profile,
            network_commands::remove_dns_profile,
            network_commands::apply_dns_profile,
            network_commands::optimize_network,
            network_commands::book_cover_bytes,
            library_commands::favorite_books,
            library_commands::delete_library_download,
            library_commands::set_book_favorite,
            library_commands::discover_books,
            library_commands::book_download_options,
            library_commands::validate_library_file,
            library_commands::convert_library_book,
            library_commands::export_library_book,
            library_commands::open_external_file,
            library_commands::open_book_website,
            source_commands::mirror_configuration,
            source_commands::save_mirror_subscription,
            source_commands::add_book_mirror,
            source_commands::remove_book_mirror,
            source_commands::refresh_book_mirrors,
            source_commands::select_book_mirror,
            imports::import_local_books,
            imports::import_paths,
            imports::remove_library_item,
            resolve_host,
            zlibrary_status,
            zlibrary_login,
            zlibrary_logout,
            zlibrary_profile,
            zlibrary_history,
            search_books,
            book_details,
            library_items,
            scan_library,
            reader_bridge::open_local_book,
            reader_bridge::open_local_book_metadata,
            reader_bridge::open_local_book_chapter,
            reading_progress,
            save_reading_progress,
            download_book,
            pdf::read_pdf_bytes
        ])
        .run(tauri::generate_context!())
        .expect("failed to run LuminaShelf");
}

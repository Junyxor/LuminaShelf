mod announcement_commands;
mod download_commands;
mod imports;
mod library_commands;
mod network_commands;
mod pdf;
mod reader_bridge;
mod source_commands;

use lumina_core::network::NetworkClient;
use lumina_core::{
    download::DownloadManager,
    library::{now_unix_ms, scan_folder},
    AppResolver, BookDetails, BookFormat, GutendexProvider, LibraryItem, ProviderDescriptor,
    ProviderRegistry, ReadingProgress, ResolveResult, ResolverPolicy, SearchQuery, SearchResult,
    StateStore, ZLibraryHistoryPage, ZLibraryProfile, ZLibraryProvider, ZLibrarySession,
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
    zlibrary_session_path: PathBuf,
    book_dir: PathBuf,
    downloads: DownloadManager,
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

        // The account survives restarts so queued downloads can resume without a new login.
        let zlibrary_session_path = config_dir.join("zlibrary-session.json");
        let zlibrary = Arc::new(
            ZLibraryProvider::with_session(
                resolver.clone(),
                initial_origin,
                load_zlibrary_session(&zlibrary_session_path),
            )
            .map_err(|error| error.to_string())?,
        );
        providers.register_shared(zlibrary.clone());
        sync_zlibrary_failover(&zlibrary, &state_store, zlibrary_config.origin_mode);

        let zlibrary_probe = NetworkClient::new(
            resolver.clone(),
            &format!("LuminaShelf/{} connection", env!("CARGO_PKG_VERSION")),
            Some(Duration::from_secs(12)),
        )
        .map_err(|error| error.to_string())?;

        let downloads =
            DownloadManager::new(state_store.clone(), providers.clone(), resolver.clone())
                .map_err(|error| error.to_string())?;
        Ok(Self {
            downloads,
            book_dir: config_dir.join("books"),
            resolver,
            providers,
            resolver_policy_path,
            state_store,
            zlibrary,
            zlibrary_probe,
            zlibrary_config: RwLock::new(zlibrary_config),
            zlibrary_config_path,
            zlibrary_session_path,
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
    sync_zlibrary_failover(&state.zlibrary, &state.state_store, origin_mode);

    state.zlibrary.clear_session().await;
    state
        .zlibrary
        .set_origin(selected_origin)
        .await
        .map_err(|error| error.to_string())?;
    let session = state
        .zlibrary
        .login_direct(email, &password)
        .await
        .map_err(|error| format!("{error:#}"))?;
    save_zlibrary_session(&state.zlibrary_session_path, &session)?;

    let profile = state.zlibrary.profile().await.ok();
    Ok(ZLibraryLoginResult {
        status: zlibrary_account_status(&state).await,
        profile,
    })
}

#[tauri::command]
async fn zlibrary_logout(state: State<'_, AppState>) -> Result<ZLibraryAccountStatus, String> {
    state.zlibrary.clear_session().await;
    match fs::remove_file(&state.zlibrary_session_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("退出登录时无法删除已保存的账户：{error}")),
    }
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
    download_commands::legacy_receipt(
        app,
        state,
        provider_id,
        book_id,
        title,
        format,
        download_dir,
    )
    .await
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

/// Automatic mode may move to any known mirror when the current one stops answering.
fn sync_zlibrary_failover(
    zlibrary: &ZLibraryProvider,
    store: &StateStore,
    mode: ZLibraryOriginMode,
) {
    zlibrary.set_fallback_origins(match mode {
        ZLibraryOriginMode::Automatic => source_commands::failover_candidates(store),
        ZLibraryOriginMode::Manual => Vec::new(),
    });
}

fn load_zlibrary_session(path: &Path) -> Option<ZLibrarySession> {
    let session: ZLibrarySession = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    session.is_complete().then_some(session)
}

fn save_zlibrary_session(path: &Path, session: &ZLibrarySession) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("create config directory: {error}"))?;
    }
    let bytes = serde_json::to_vec(session).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // The key is an account credential: keep it readable by this user only.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    std::io::Write::write_all(
        &mut options
            .open(&temporary)
            .map_err(|error| format!("保存登录状态失败：{error}"))?,
        &bytes,
    )
    .map_err(|error| format!("保存登录状态失败：{error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("保存登录状态失败：{error}"))
}

/// Persists the site that failover moved to, so the next launch starts from a working mirror.
fn persist_zlibrary_failover(app: AppHandle, mut changes: tokio::sync::watch::Receiver<Url>) {
    tauri::async_runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            let origin = changes.borrow_and_update().to_string();
            let state = app.state::<AppState>();
            let mut config = state.zlibrary_config.write().await;
            if config.origin.as_deref() == Some(origin.as_str()) {
                continue;
            }
            config.origin = Some(origin);
            let _ = save_zlibrary_config(&state.zlibrary_config_path, &config);
        }
    });
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

#[cfg(test)]
fn resumable_destination(
    directory: &Path,
    title: &str,
    extension: &str,
    provider_id: &str,
    book_id: &str,
) -> PathBuf {
    lumina_core::download::destination_for(directory, title, extension, provider_id, book_id)
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
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(download_commands::plugin())
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
            download_commands::observe(app.handle().clone(), state.downloads.clone());
            let origin_changes = state.zlibrary.origin_changes();
            app.manage(state);
            persist_zlibrary_failover(app.handle().clone(), origin_changes);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            core_status,
            download_commands::download_tasks,
            download_commands::enqueue_download_book,
            download_commands::pause_download,
            download_commands::resume_download,
            download_commands::cancel_download,
            download_commands::restart_download,
            download_commands::remove_download,
            download_commands::pause_all_downloads,
            download_commands::download_settings,
            download_commands::save_download_settings,
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
            library_commands::library_cover,
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
    fn saved_account_survives_a_restart_and_stays_private() {
        let directory =
            std::env::temp_dir().join(format!("lumina-session-{}", uuid::Uuid::new_v4()));
        let path = directory.join("zlibrary-session.json");
        assert!(load_zlibrary_session(&path).is_none());
        let session = ZLibrarySession {
            user_id: "42".into(),
            user_key: "secret".into(),
        };
        save_zlibrary_session(&path, &session).unwrap();
        let restored = load_zlibrary_session(&path).unwrap();
        assert_eq!(
            (restored.user_id.as_str(), restored.user_key.as_str()),
            ("42", "secret")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
        fs::write(&path, r#"{"userId":"","userKey":""}"#).unwrap();
        assert!(
            load_zlibrary_session(&path).is_none(),
            "an empty session is not a login"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn resumable_destination_separates_books_with_same_title() {
        let directory = Path::new("downloads");
        let first = resumable_destination(directory, "Same Title", "epub", "zlibrary", "1");
        let second = resumable_destination(directory, "Same Title", "epub", "zlibrary", "2");
        assert_ne!(first, second);
    }
}

use super::*;
use futures::{stream, StreamExt};

const KEY: &str = "book-mirror-config-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    id: String,
    name: String,
    url: String,
    enabled: bool,
    #[serde(default)]
    last_error: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMirror {
    id: String,
    origin: String,
    sources: Vec<String>,
    #[serde(default)]
    checked_at: Option<i64>,
    #[serde(default)]
    latency_ms: Option<f64>,
    #[serde(default)]
    available: Option<bool>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default = "yes")]
    content_available: bool,
    #[serde(default)]
    redirector: bool,
}
fn yes() -> bool {
    true
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorConfiguration {
    subscriptions: Vec<Subscription>,
    mirrors: Vec<BookMirror>,
    last_refresh: Option<i64>,
    selected_origin: String,
}

fn defaults() -> MirrorConfiguration {
    MirrorConfiguration {
        subscriptions: vec![
            Subscription {
                id: "site-directory".into(),
                name: "书源发布的域名目录".into(),
                url: "https://zlib.bz/eapi/info/domains".into(),
                enabled: true,
                last_error: None,
            },
            Subscription {
                id: "community-github".into(),
                name: "Awesome-Zlibrary 镜像表（社区来源）".into(),
                url: "https://github.com/dongyubin/Awesome-Zlibrary/blob/main/README.md".into(),
                enabled: true,
                last_error: None,
            },
        ],
        mirrors: ZLIBRARY_CANDIDATES
            .iter()
            .map(|origin| BookMirror {
                id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, origin.as_bytes()).to_string(),
                origin: normalize_origin(origin).unwrap().to_string(),
                sources: vec!["应用内置候选".into()],
                checked_at: None,
                latency_ms: None,
                available: None,
                error: None,
                content_available: true,
                redirector: false,
            })
            .collect(),
        last_refresh: None,
        selected_origin: String::new(),
    }
}
fn load(store: &StateStore) -> Result<MirrorConfiguration, String> {
    match store.setting(KEY).map_err(|e| e.to_string())? {
        Some(raw) => {
            let mut configuration: MirrorConfiguration =
                serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            let count = configuration.mirrors.len();
            let mut seen = std::collections::HashSet::new();
            configuration
                .mirrors
                .retain(|item| seen.insert(item.origin.clone()));
            if configuration.mirrors.len() != count {
                persist(store, &configuration)?;
            }
            Ok(configuration)
        }
        None => {
            let configuration = defaults();
            persist(store, &configuration)?;
            Ok(configuration)
        }
    }
}
/// Known mirrors in the order failover should try them: last checked good first, known bad last.
pub(super) fn failover_candidates(store: &StateStore) -> Vec<Url> {
    let mut mirrors = load(store).map(|config| config.mirrors).unwrap_or_default();
    mirrors.retain(|item| item.content_available && !item.redirector);
    mirrors.sort_by(|a, b| {
        let rank = |item: &BookMirror| match item.available {
            Some(true) => 0,
            None => 1,
            Some(false) => 2,
        };
        rank(a).cmp(&rank(b)).then(
            a.latency_ms
                .unwrap_or(f64::MAX)
                .total_cmp(&b.latency_ms.unwrap_or(f64::MAX)),
        )
    });
    let mut seen = std::collections::HashSet::new();
    mirrors
        .iter()
        .map(|item| item.origin.as_str())
        .chain(ZLIBRARY_CANDIDATES.iter().copied())
        .filter_map(|origin| normalize_origin(origin).ok())
        .filter(|origin| seen.insert(origin.to_string()))
        .collect()
}

fn persist(store: &StateStore, config: &MirrorConfiguration) -> Result<(), String> {
    store
        .set_setting(
            KEY,
            &serde_json::to_string(config).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn mirror_configuration(
    state: State<'_, AppState>,
) -> Result<MirrorConfiguration, String> {
    let mut config = load(&state.state_store)?;
    config.selected_origin = state.zlibrary.origin().await.to_string();
    Ok(config)
}

fn source_url(raw: &str) -> Result<Url, String> {
    let mut url = Url::parse(raw.trim()).map_err(|_| "发布源地址格式不正确")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("发布源需要 HTTPS 地址".into());
    }
    if url.host_str() == Some("github.com") || url.host_str() == Some("raw.githubusercontent.com") {
        let parts: Vec<_> = url
            .path_segments()
            .map(|parts| parts.collect())
            .unwrap_or_default();
        let (owner, repo, reference, path) =
            if parts.len() >= 4 && url.host_str() == Some("raw.githubusercontent.com") {
                (parts[0], parts[1], parts[2], parts[3..].join("/"))
            } else if parts.len() >= 5 && parts[2] == "blob" {
                (parts[0], parts[1], parts[3], parts[4..].join("/"))
            } else if parts.len() == 2 {
                (
                    parts[0],
                    parts[1].trim_end_matches(".git"),
                    "HEAD",
                    "README.md".into(),
                )
            } else {
                return Err("请填写 GitHub 仓库地址或具体清单文件地址".into());
            };
        let reference = reference.to_string();
        url = Url::parse(&format!(
            "https://api.github.com/repos/{owner}/{repo}/contents/{path}"
        ))
        .map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("ref", &reference);
    }
    Ok(url)
}

#[tauri::command]
pub async fn save_mirror_subscription(
    state: State<'_, AppState>,
    id: Option<String>,
    name: String,
    url: String,
    enabled: bool,
) -> Result<MirrorConfiguration, String> {
    source_url(&url)?;
    if name.trim().is_empty() || name.chars().count() > 100 {
        return Err("请填写 1–100 字的发布源名称".into());
    }
    let mut config = load(&state.state_store)?;
    let subscription = Subscription {
        id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: name.trim().into(),
        url: url.trim().into(),
        enabled,
        last_error: None,
    };
    if let Some(old) = config
        .subscriptions
        .iter_mut()
        .find(|item| item.id == subscription.id)
    {
        *old = subscription;
    } else {
        if config.subscriptions.len() >= 16 {
            return Err("最多保存 16 个发布源".into());
        }
        config.subscriptions.push(subscription);
    }
    persist(&state.state_store, &config)?;
    config.selected_origin = state.zlibrary.origin().await.to_string();
    Ok(config)
}

#[tauri::command]
pub async fn add_book_mirror(
    state: State<'_, AppState>,
    origin: String,
) -> Result<MirrorConfiguration, String> {
    let origin = normalize_origin(&origin)?.to_string();
    let mut config = load(&state.state_store)?;
    merge(&mut config, origin, "手动添加".into(), true, false);
    persist(&state.state_store, &config)?;
    config.selected_origin = state.zlibrary.origin().await.to_string();
    Ok(config)
}

#[tauri::command]
pub async fn remove_book_mirror(
    state: State<'_, AppState>,
    id: String,
) -> Result<MirrorConfiguration, String> {
    let mut config = load(&state.state_store)?;
    config.mirrors.retain(|item| item.id != id);
    persist(&state.state_store, &config)?;
    config.selected_origin = state.zlibrary.origin().await.to_string();
    Ok(config)
}

fn merge(
    config: &mut MirrorConfiguration,
    origin: String,
    source: String,
    content: bool,
    redirector: bool,
) {
    if let Some(item) = config.mirrors.iter_mut().find(|item| item.origin == origin) {
        if !item.sources.contains(&source) {
            item.sources.push(source);
        }
        item.content_available = content;
        item.redirector = redirector;
    } else if config.mirrors.len() < 32 {
        config.mirrors.push(BookMirror {
            id: uuid::Uuid::new_v4().to_string(),
            origin,
            sources: vec![source],
            checked_at: None,
            latency_ms: None,
            available: None,
            error: None,
            content_available: content,
            redirector,
        });
    }
}

pub(super) fn parse_directory(
    value: &serde_json::Value,
) -> Result<Vec<(String, bool, bool)>, String> {
    if value.get("success").and_then(serde_json::Value::as_i64) != Some(1) {
        return Err("域名目录接口未返回成功结果".into());
    }
    let entries = value
        .get("domains")
        .and_then(serde_json::Value::as_array)
        .ok_or("响应不是有效的书源域名目录")?;
    let mut result = Vec::new();
    for item in entries.iter().take(32) {
        if let Some(domain) = item.get("domain").and_then(serde_json::Value::as_str) {
            if let Ok(origin) =
                normalize_origin(&format!("https://{}", domain.trim().trim_end_matches('/')))
            {
                result.push((
                    origin.to_string(),
                    item.get("contentAvailable")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    item.get("isRedirector")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                ));
            }
        }
    }
    Ok(result)
}

async fn fetch_source(
    state: &AppState,
    subscription: &Subscription,
) -> Result<Vec<(String, bool, bool)>, String> {
    let url = source_url(&subscription.url)?;
    let mut response = state
        .zlibrary_probe
        .client()
        .map_err(|e| e.to_string())?
        .get(url)
        .header("accept", "application/vnd.github.raw+json")
        .send()
        .await
        .map_err(|e| format!("{:#}", e.without_url()))?
        .error_for_status()
        .map_err(|e| format!("{:#}", e.without_url()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("发布源超过 1 MB，已停止读取".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let text = String::from_utf8(bytes).map_err(|_| "发布源不是 UTF-8 文本")?;
    if subscription
        .url
        .to_ascii_lowercase()
        .contains("dongyubin/awesome-zlibrary")
    {
        return parse_active_publisher_table(&text);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        if value.get("domains").is_some() || value.get("success").is_some() {
            return parse_directory(&value);
        }
    }
    if text.trim_start().starts_with('[') {
        return lumina_core::mirror::parse_endpoint_list(
            &text,
            lumina_core::MirrorSourceKind::Subscription,
            false,
        )
        .map(|items| {
            items
                .into_iter()
                .filter_map(|item| {
                    normalize_origin(item.origin.as_str())
                        .ok()
                        .map(|url| (url.to_string(), true, false))
                })
                .collect()
        })
        .map_err(|e| e.to_string());
    }
    let mut result = Vec::new();
    for field in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '`' | '{' | '}' | ','
            )
    }) {
        if field.starts_with("https://") {
            if let Ok(origin) = normalize_origin(field.trim_end_matches(['.', ':', ';'])) {
                let host = origin.host_str().unwrap_or("");
                if subscription.id == "community-github"
                    && !(host.contains("z-lib")
                        || host.starts_with("zlib")
                        || host.starts_with("1lib"))
                {
                    continue;
                }
                let value = (origin.to_string(), true, false);
                if !result.contains(&value) {
                    result.push(value);
                }
            }
        }
    }
    if result.is_empty() {
        return Err("发布源未找到 HTTPS 镜像地址；保留原有清单".into());
    }
    Ok(result)
}

fn parse_active_publisher_table(text: &str) -> Result<Vec<(String, bool, bool)>, String> {
    let mut active = false;
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('|') && line.contains("Z-Library 最新官方/镜像网址") {
            active = true;
            continue;
        }
        if !active {
            continue;
        }
        if !line.is_empty() && !line.starts_with('|') {
            break;
        }
        if !line.contains('✅') || line.contains('🚫') {
            continue;
        }
        for field in line.split(|c: char| {
            c.is_whitespace() || matches!(c, '[' | ']' | '(' | ')' | '<' | '>' | '"' | '\'' | '|')
        }) {
            if field.starts_with("https://") {
                if let Ok(origin) = normalize_origin(field) {
                    let entry = (origin.to_string(), true, false);
                    if !result.contains(&entry) {
                        result.push(entry);
                    }
                }
            }
        }
    }
    if result.is_empty() {
        return Err("社区发布页的有效镜像表没有可接受的 HTTPS 地址，已保留原清单".into());
    }
    Ok(result)
}

#[tauri::command]
pub async fn refresh_book_mirrors(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MirrorConfiguration, String> {
    let _ = app.emit(
        "mirror-progress",
        serde_json::json!({"phase":"拉取发布源","done":0,"total":0}),
    );
    let mut config = load(&state.state_store)?;
    let sources = config
        .subscriptions
        .iter()
        .filter(|source| source.enabled)
        .cloned()
        .collect::<Vec<_>>();
    let state_ref = &*state;
    let results = stream::iter(sources)
        .map(|source| async move {
            let result = fetch_source(state_ref, &source).await;
            (source, result)
        })
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    for (source, result) in results {
        match result {
            Ok(items) => {
                for (origin, content, redirector) in items {
                    merge(
                        &mut config,
                        origin,
                        format!("{} · {}", source.name, source.url),
                        content,
                        redirector,
                    );
                }
                if let Some(item) = config
                    .subscriptions
                    .iter_mut()
                    .find(|item| item.id == source.id)
                {
                    item.last_error = None;
                }
            }
            Err(error) => {
                if let Some(item) = config
                    .subscriptions
                    .iter_mut()
                    .find(|item| item.id == source.id)
                {
                    item.last_error = Some(error);
                }
            }
        }
    }
    let total = config.mirrors.len();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let _ = app.emit(
        "mirror-progress",
        serde_json::json!({"phase":"检测镜像","done":0,"total":total}),
    );
    let probes = stream::iter(config.mirrors.clone())
        .map(|mut item| {
            let app = app.clone();
            let done = done.clone();
            async move {
                let start = std::time::Instant::now();
                let result = tokio::time::timeout(
                    Duration::from_secs(7),
                    probe_zlibrary_origin(state_ref, &Url::parse(&item.origin).unwrap()),
                )
                .await
                .map_err(|_| "连接超时".to_string())
                .and_then(|result| result);
                item.checked_at = Some(now_unix_ms());
                item.latency_ms = Some(start.elapsed().as_secs_f64() * 1000.);
                item.available = Some(result.is_ok() && item.content_available);
                item.error = result.err();
                let count = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let _ = app.emit(
                    "mirror-progress",
                    serde_json::json!({"phase":"检测镜像","done":count,"total":total}),
                );
                item
            }
        })
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    config.mirrors = probes;
    config.last_refresh = Some(now_unix_ms());
    config.selected_origin = state.zlibrary.origin().await.to_string();
    persist(&state.state_store, &config)?;
    let mode = state.zlibrary_config.read().await.origin_mode;
    sync_zlibrary_failover(&state.zlibrary, &state.state_store, mode);
    Ok(config)
}

#[tauri::command]
pub async fn select_book_mirror(
    state: State<'_, AppState>,
    id: String,
) -> Result<ZLibraryAccountStatus, String> {
    let config = load(&state.state_store)?;
    let item = config
        .mirrors
        .iter()
        .find(|item| item.id == id)
        .ok_or("镜像已不存在")?;
    let origin = normalize_origin(&item.origin)?;
    let next = ZLibraryConfig {
        origin: Some(origin.to_string()),
        origin_mode: ZLibraryOriginMode::Manual,
    };
    save_zlibrary_config(&state.zlibrary_config_path, &next)?;
    *state.zlibrary_config.write().await = next;
    // The account key is valid on every mirror of the same service; switching keeps it.
    sync_zlibrary_failover(
        &state.zlibrary,
        &state.state_store,
        ZLibraryOriginMode::Manual,
    );
    state
        .zlibrary
        .set_origin(origin)
        .await
        .map_err(|e| e.to_string())?;
    Ok(zlibrary_account_status(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_candidates_have_unique_origins_and_stable_ids() {
        let config = defaults();
        let repeated = defaults();
        let origins: std::collections::HashSet<_> = config
            .mirrors
            .iter()
            .map(|item| item.origin.as_str())
            .collect();
        let ids: std::collections::HashSet<_> =
            config.mirrors.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(origins.len(), config.mirrors.len());
        assert_eq!(ids.len(), config.mirrors.len());
        assert_eq!(config.mirrors[0].id, repeated.mirrors[0].id);
    }
    #[test]
    fn publisher_parser_excludes_dead_phishing_and_http_rows() {
        let input="## 地址\n| Z-Library 最新官方/镜像网址 | 状态 |\n|---|---|\n| [https://works.example/](https://works.example/) | ✅ |\n| [http://plain.example/](http://plain.example/) | ✅ |\n| [https://dead.example/](https://dead.example/) | 🚫 |\n### 官方网址最新（已失效）\n| https://phishing.example/ | ✅ |";
        let parsed = parse_active_publisher_table(input).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "https://works.example/");
        assert!(
            source_url("https://github.com/owner/repo/blob/main/list.json")
                .unwrap()
                .as_str()
                .starts_with("https://api.github.com/repos/owner/repo/contents/list.json?ref=main")
        );
    }
    #[test]
    fn validates_structured_directory_and_keeps_availability_flags() {
        let value = serde_json::json!({"success":1,"domains":[{"domain":"books.example.org","contentAvailable":true,"isRedirector":false},{"domain":"go.example.org","contentAvailable":false,"isRedirector":true}]});
        let parsed = parse_directory(&value).unwrap();
        assert_eq!(
            parsed[0],
            ("https://books.example.org/".into(), true, false)
        );
        assert!(!parsed[1].1);
        assert!(parse_directory(&serde_json::json!({"success":0,"error":"Please login"})).is_err());
        assert!(parse_directory(&serde_json::json!({"ok":true})).is_err());
    }
}

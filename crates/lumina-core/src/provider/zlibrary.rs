use super::{
    BookDetails, BookFormat, BookProvider, BookSummary, ProviderCapabilities, ProviderError,
    SearchQuery, SearchResult,
};
use crate::network::{AppResolver, NetworkClient};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use futures::{stream, StreamExt};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE, COOKIE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::{watch, Mutex, RwLock};
use url::Url;

/// Whether an EAPI route needs, accepts or ignores the account session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Auth {
    Anonymous,
    Optional,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZLibrarySession {
    pub user_id: String,
    pub user_key: String,
}

impl ZLibrarySession {
    pub fn is_complete(&self) -> bool {
        !self.user_id.trim().is_empty() && !self.user_key.trim().is_empty()
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mut user_id =
            HeaderValue::from_str(&self.user_id).context("invalid session user id")?;
        let mut user_key =
            HeaderValue::from_str(&self.user_key).context("invalid session user key")?;
        let mut cookie = HeaderValue::from_str(&format!(
            "remix_userid={}; remix_userkey={}",
            self.user_id, self.user_key
        ))
        .context("invalid session cookie")?;
        user_id.set_sensitive(true);
        user_key.set_sensitive(true);
        cookie.set_sensitive(true);
        headers.insert(HeaderName::from_static("remix-userid"), user_id);
        headers.insert(HeaderName::from_static("remix-userkey"), user_key);
        headers.insert(COOKIE, cookie);
        Ok(headers)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZLibraryProfile {
    pub downloads_today: i64,
    pub downloads_limit: i64,
    pub downloads_remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZLibraryHistoryItem {
    pub book: BookSummary,
    pub downloaded_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZLibraryHistoryPage {
    pub items: Vec<ZLibraryHistoryItem>,
    pub page: u32,
    pub has_next: bool,
}

#[derive(Clone)]
pub struct ZLibraryProvider {
    client: NetworkClient,
    origin: Arc<RwLock<Url>>,
    session: Arc<RwLock<Option<ZLibrarySession>>>,
    cache: Arc<DashMap<String, BookDetails>>,
    acquisition_hints: Arc<DashMap<String, String>>,
    // Empty in manual mode: a site the user picked is never replaced behind their back.
    fallback_origins: Arc<std::sync::RwLock<Vec<Url>>>,
    failover: Arc<Mutex<()>>,
    origin_changes: Arc<watch::Sender<Url>>,
}

impl ZLibraryProvider {
    pub fn new(resolver: AppResolver, origin: Url) -> Result<Self> {
        Self::with_session(resolver, origin, None)
    }

    /// Restores a saved account so downloads queued before a restart can continue.
    pub fn with_session(
        resolver: AppResolver,
        origin: Url,
        session: Option<ZLibrarySession>,
    ) -> Result<Self> {
        if !secure_origin(&origin) {
            return Err(anyhow!("Z-Library provider requires HTTPS"));
        }
        let client = NetworkClient::with_redirects(
            resolver,
            &format!("LuminaShelf/{} ZLibrary", env!("CARGO_PKG_VERSION")),
            Some(Duration::from_secs(25)),
            false,
        )?;
        let session = session.filter(ZLibrarySession::is_complete);
        Ok(Self {
            client,
            origin_changes: Arc::new(watch::channel(origin.clone()).0),
            origin: Arc::new(RwLock::new(origin)),
            session: Arc::new(RwLock::new(session)),
            cache: Arc::new(DashMap::new()),
            acquisition_hints: Arc::new(DashMap::new()),
            fallback_origins: Arc::new(std::sync::RwLock::new(Vec::new())),
            failover: Arc::new(Mutex::new(())),
        })
    }

    pub async fn set_origin(&self, origin: Url) -> Result<()> {
        if !secure_origin(&origin) {
            return Err(anyhow!("Z-Library provider requires HTTPS"));
        }
        *self.origin.write().await = origin;
        self.cache.clear();
        self.acquisition_hints.clear();
        Ok(())
    }

    pub async fn origin(&self) -> Url {
        self.origin.read().await.clone()
    }

    /// Sites tried in order when the current one stops answering. Empty disables failover.
    pub fn set_fallback_origins(&self, origins: Vec<Url>) {
        let origins = origins.into_iter().filter(secure_origin).collect();
        *self
            .fallback_origins
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = origins;
    }

    /// Yields the new origin whenever failover moves to another site.
    pub fn origin_changes(&self) -> watch::Receiver<Url> {
        self.origin_changes.subscribe()
    }

    pub async fn set_session(&self, session: ZLibrarySession) -> Result<()> {
        if !session.is_complete() {
            return Err(anyhow!("session user id/key cannot be empty"));
        }
        *self.session.write().await = Some(session);
        Ok(())
    }

    pub async fn clear_session(&self) {
        *self.session.write().await = None;
        self.cache.clear();
        self.acquisition_hints.clear();
    }

    pub async fn has_session(&self) -> bool {
        self.session.read().await.is_some()
    }

    pub async fn current_session(&self) -> Option<ZLibrarySession> {
        self.session.read().await.clone()
    }

    pub async fn alternate_formats(&self, id: &str) -> Result<Vec<BookSummary>> {
        let (book, hash) = id
            .split_once(':')
            .context("书籍缺少下载标识，请重新打开书籍详情")?;
        let path = format!(
            "/eapi/book/{}/{}/formats",
            path_component(book),
            path_component(hash)
        );
        let response: SearchEnvelope = self.request_json(&path, None, Auth::Optional).await?;
        if response.success != 1 {
            return Err(rejected(response.error, "无法读取其他格式"));
        }
        Ok(response
            .books
            .into_iter()
            .filter_map(|book| map_eapi_book(book).map(|value| value.0))
            .collect())
    }

    pub async fn book_features(&self, id: &str) -> Result<Value> {
        let (book, hash) = id.split_once(':').context("书籍缺少标识")?;
        let path = format!(
            "/eapi/book/{}/{}",
            path_component(book),
            path_component(hash)
        );
        let value: Value = self.request_json(&path, None, Auth::Optional).await?;
        let book = value.get("book").context("书籍详情不完整")?;
        Ok(
            serde_json::json!({"email":book.get("sendToEmailAvailable").and_then(Value::as_bool).unwrap_or(false),"kindle":book.get("kindleAvailable").and_then(Value::as_bool).unwrap_or(false),"onlineReading":book.get("readOnlineAvailable").and_then(Value::as_bool).unwrap_or(false),"nativeSendSupported":false}),
        )
    }

    async fn fetch_details(&self, id: &str) -> Result<BookDetails> {
        let (book, hash) = id.split_once(':').context("书籍缺少详情标识")?;
        let path = format!(
            "/eapi/book/{}/{}",
            path_component(book),
            path_component(hash)
        );
        let value: Value = self.request_json(&path, None, Auth::Optional).await?;
        if value.get("success").and_then(Value::as_i64) != Some(1) {
            return Err(rejected(
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                "书源未返回书籍详情",
            ));
        }
        let book: EapiBook =
            serde_json::from_value(value.get("book").cloned().context("详情响应没有书籍")?)?;
        let (_, mut details, hint) = map_eapi_book(book).context("书籍详情不完整")?;
        details.summary.id = id.to_string();
        if let Some(hint) = hint {
            self.acquisition_hints.insert(id.to_string(), hint);
        }
        self.cache.insert(id.to_string(), details.clone());
        Ok(details)
    }

    pub async fn profile(&self) -> Result<ZLibraryProfile> {
        let response: ProfileEnvelope = self
            .request_json("/eapi/user/profile", None, Auth::Required)
            .await?;
        if response.success != 1 {
            return Err(rejected(response.error, "书源拒绝读取账户信息"));
        }
        let user = response
            .user
            .ok_or_else(|| anyhow!("profile response has no user"))?;
        let downloads_today = first_non_zero(&user.downloads_today, &user.daily_downloads_count);
        let downloads_limit = first_non_zero(&user.downloads_limit, &user.daily_download_limit);
        Ok(ZLibraryProfile {
            downloads_today,
            downloads_limit,
            downloads_remaining: (downloads_limit - downloads_today).max(0),
        })
    }

    pub async fn download_history(&self, page: u32) -> Result<ZLibraryHistoryPage> {
        let page = page.max(1);
        let path = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("page", &page.to_string());
            query.append_pair("limit", "50");
            format!("/eapi/user/book/downloaded?{}", query.finish())
        };
        let response: SearchEnvelope = self.request_json(&path, None, Auth::Required).await?;
        if response.success != 1 {
            return Err(rejected(response.error, "书源拒绝读取下载记录"));
        }
        let current = if response.pagination.current == 0 {
            page
        } else {
            response.pagination.current
        };
        let total_pages = response.pagination.total_pages;
        let mut items = Vec::with_capacity(response.books.len());
        for book in response.books {
            if let Some((summary, details, acquisition_hint)) = map_eapi_book(book.clone()) {
                if let Some(hint) = acquisition_hint {
                    self.acquisition_hints.insert(summary.id.clone(), hint);
                }
                self.cache.insert(summary.id.clone(), details);
                items.push(ZLibraryHistoryItem {
                    downloaded_at: non_empty(if book.date.trim().is_empty() {
                        book.downloaded_at
                    } else {
                        book.date
                    }),
                    book: summary,
                });
            }
        }
        Ok(ZLibraryHistoryPage {
            items,
            page: current,
            has_next: total_pages > current,
        })
    }

    pub async fn login_direct(&self, email: &str, password: &str) -> Result<ZLibrarySession> {
        let body = form(&[("email", email), ("password", password)]);
        let value: LoginEnvelope = self
            .request_json("/eapi/user/login", Some(body), Auth::Anonymous)
            .await?;
        if value.success != 1 {
            return Err(anyhow!(value
                .error
                .unwrap_or_else(|| "书源拒绝了登录，请检查邮箱和密码".to_string())));
        }
        let user = value
            .user
            .ok_or_else(|| anyhow!("login response has no user"))?;
        let user_id = json_string(&user.id);
        let user_key = user
            .remix_userkey
            .or(user.remix_user_key)
            .unwrap_or_default();
        let session = ZLibrarySession { user_id, user_key };
        self.set_session(session.clone()).await?;
        Ok(session)
    }

    async fn request_json<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: Option<String>,
        auth: Auth,
    ) -> Result<T> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let session = self.session.read().await.clone();
        match (auth, session) {
            (Auth::Required, None) => return Err(ProviderError::LoginRequired.into()),
            (Auth::Required | Auth::Optional, Some(session)) => headers.extend(session.headers()?),
            _ => {}
        }
        let (_, response) = self.send(path, body.as_deref(), headers).await?;
        decode_response(response).await
    }

    /// Sends one EAPI request, moving to a working mirror once if this site is unreachable.
    async fn send(
        &self,
        path: &str,
        body: Option<&str>,
        headers: HeaderMap,
    ) -> Result<(Url, reqwest::Response)> {
        let mut origin = self.origin().await;
        let mut failed_over = false;
        loop {
            let endpoint = origin.join(path).context("build EAPI URL")?;
            let client = self.client.client()?;
            let request = match body {
                Some(body) => client
                    .post(endpoint.clone())
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(body.to_owned()),
                None => client.get(endpoint.clone()),
            };
            let failure = match request.headers(headers.clone()).send().await {
                Ok(response) if !site_unusable(&response) => return Ok((endpoint, response)),
                Ok(response) => anyhow!(
                    "{} 返回 HTTP {}",
                    origin.host_str().unwrap_or("书源"),
                    response.status().as_u16()
                ),
                Err(error) if error.is_connect() || error.is_timeout() => {
                    anyhow::Error::new(error.without_url())
                }
                Err(error) => {
                    return Err(anyhow::Error::new(error.without_url()).context("书源请求失败"))
                }
            };
            if !failed_over {
                failed_over = true;
                if let Some(next) = self.fail_over(&origin).await {
                    origin = next;
                    continue;
                }
            }
            return Err(failure.context(ProviderError::OriginUnavailable));
        }
    }

    async fn fail_over(&self, failed: &Url) -> Option<Url> {
        // Parallel requests share one probe round instead of each racing to switch.
        let _guard = self.failover.lock().await;
        let current = self.origin().await;
        if &current != failed {
            return Some(current);
        }
        let candidates: Vec<Url> = self
            .fallback_origins
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|origin| *origin != failed)
            .cloned()
            .collect();
        let client = self.client.client().ok()?;
        let mut probes = stream::iter(candidates)
            .map(|origin| {
                let client = client.clone();
                async move { probe_origin(&client, &origin).await.then_some(origin) }
            })
            .buffer_unordered(3);
        while let Some(result) = probes.next().await {
            if let Some(origin) = result {
                *self.origin.write().await = origin.clone();
                self.origin_changes.send_replace(origin.clone());
                return Some(origin);
            }
        }
        None
    }
}

fn secure_origin(origin: &Url) -> bool {
    // Unit tests drive the request path against plain local servers.
    origin.scheme() == "https" || (cfg!(test) && origin.scheme() == "http")
}

async fn probe_origin(client: &reqwest::Client, origin: &Url) -> bool {
    let Ok(endpoint) = origin.join("/eapi/info/domains") else {
        return false;
    };
    let Ok(response) = client
        .get(endpoint)
        .header(ACCEPT, "application/json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
    else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    response
        .json::<Value>()
        .await
        .is_ok_and(|value| value.get("success").and_then(Value::as_i64) == Some(1))
}

/// Gateway errors and challenge pages mean this mirror cannot serve the API right now.
fn site_unusable(response: &reqwest::Response) -> bool {
    let status = response.status().as_u16();
    let html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
    matches!(status, 500..=599) || (status == 403 && html)
}

/// Turns an EAPI `success: 0` reply into an error, recognising an expired or missing login.
fn rejected(error: Option<String>, fallback: &str) -> anyhow::Error {
    match error {
        Some(message) if needs_login(&message) => ProviderError::LoginRequired.into(),
        Some(message) if !message.trim().is_empty() => anyhow!(message),
        _ => anyhow!(fallback.to_string()),
    }
}

fn needs_login(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "please login",
        "please log in",
        "not logged",
        "unauthorized",
        "auth required",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[async_trait]
impl BookProvider for ZLibraryProvider {
    async fn discover(&self) -> Result<super::DiscoveryPage> {
        let (path, title, auth) = if self.has_session().await {
            ("/eapi/user/book/recommended", "为你推荐", Auth::Required)
        } else {
            ("/eapi/book/most-popular", "站点热门", Auth::Anonymous)
        };
        let response: SearchEnvelope = self.request_json(path, None, auth).await?;
        if response.success != 1 {
            return Err(rejected(response.error, "无法读取书源首页"));
        }
        let items = response
            .books
            .into_iter()
            .filter_map(|book| map_eapi_book(book).map(|value| value.0))
            .collect();
        Ok(super::DiscoveryPage {
            title: title.into(),
            source_url: self.origin().await.join(path)?.to_string(),
            items,
            fetched_at_unix_ms: crate::library::now_unix_ms(),
            stale: false,
            error: None,
        })
    }
    fn id(&self) -> &'static str {
        "zlibrary"
    }
    fn name(&self) -> &'static str {
        "Z-Library · EAPI"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            authenticated: true,
            downloadable: true,
            searchable: true,
            paginated: true,
        }
    }

    async fn search(&self, query: SearchQuery) -> Result<SearchResult> {
        let page = query.page.max(1);
        let limit = query.page_size.clamp(1, 50).to_string();
        let page_raw = page.to_string();
        let pairs = vec![
            ("message", query.text.as_str()),
            ("page", page_raw.as_str()),
            ("limit", limit.as_str()),
        ];
        let mut owned_formats = Vec::new();
        for format in &query.formats {
            owned_formats.push(format.extension().to_string());
        }
        let mut encoded = form(&pairs);
        for format in &owned_formats {
            if !encoded.is_empty() {
                encoded.push('&');
            }
            encoded.push_str(
                &url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("extensions[]", format)
                    .finish(),
            );
        }
        // EAPI search answers anonymously; the session only personalizes results.
        let response: SearchEnvelope = self
            .request_json("/eapi/book/search", Some(encoded), Auth::Optional)
            .await?;
        if response.success != 1 {
            return Err(rejected(response.error, "书源拒绝了搜索请求"));
        }
        let mut items = Vec::with_capacity(response.books.len());
        for book in response.books {
            if let Some((summary, details, acquisition_hint)) = map_eapi_book(book) {
                if let Some(hint) = acquisition_hint {
                    self.acquisition_hints.insert(summary.id.clone(), hint);
                }
                self.cache.insert(summary.id.clone(), details);
                items.push(summary);
            }
        }
        let current = if response.pagination.current == 0 {
            page
        } else {
            response.pagination.current
        };
        Ok(SearchResult {
            items,
            page: current,
            has_next: response.pagination.total_pages > current,
        })
    }

    async fn details(&self, id: &str) -> Result<BookDetails> {
        if let Some(value) = self.cache.get(id) {
            if value
                .summary
                .format
                .is_some_and(|format| format != BookFormat::Other)
            {
                return Ok(value.clone());
            }
        }
        self.fetch_details(id).await
    }

    async fn acquisition_url(&self, id: &str, format: BookFormat) -> Result<Url> {
        if let Some((book_id, hash)) = id.split_once(':') {
            if book_id.is_empty() || hash.is_empty() {
                return Err(anyhow!("invalid download reference"));
            }
            let path = format!(
                "/eapi/book/{}/{}/file",
                path_component(book_id),
                path_component(hash)
            );
            let (endpoint, response) = self
                .send(&path, None, self.download_headers().await?)
                .await
                .context("无法读取下载文件信息")?;
            if response.status().is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .context("下载接口没有提供跳转地址")?;
                let target = endpoint.join(location)?;
                anyhow::ensure!(
                    target.scheme() == "https"
                        && target.username().is_empty()
                        && target.password().is_none(),
                    "下载跳转需要有效的 HTTPS 文件地址"
                );
                return Ok(target);
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if response.status().is_success()
                && [
                    "application/epub+zip",
                    "application/pdf",
                    "application/octet-stream",
                    "text/plain",
                ]
                .iter()
                .any(|kind| mime.starts_with(kind))
            {
                return Ok(endpoint);
            }
            let value: Value = decode_response(response).await?;
            return download_link(&value, format);
        }
        Err(anyhow!(
            "download reference has neither a book hash nor an EAPI download path"
        ))
    }

    async fn download_headers_for(&self, url: &Url) -> Result<HeaderMap> {
        if url.origin() == self.origin().await.origin() {
            self.download_headers().await
        } else {
            Ok(HeaderMap::new())
        }
    }

    async fn download_headers(&self) -> Result<HeaderMap> {
        self.session
            .read()
            .await
            .as_ref()
            .ok_or(ProviderError::LoginRequired)?
            .headers()
    }
}

async fn decode_response<T: for<'de> Deserialize<'de>>(
    mut response: reqwest::Response,
) -> Result<T> {
    let status = response.status();
    anyhow::ensure!(
        !status.is_redirection(),
        "书源地址发生跳转，请在镜像设置中选择最终内容站"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 8 * 1024 * 1024,
            "书源接口 JSON 响应过大"
        );
        bytes.extend_from_slice(&chunk);
    }
    let beginning = String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
        .trim_start()
        .to_ascii_lowercase();
    anyhow::ensure!(
        !beginning.starts_with("<!doctype html") && !beginning.starts_with("<html"),
        "书源返回网页或验证页，接口暂不可用，请更换镜像"
    );
    serde_json::from_slice(&bytes).with_context(|| format!("书源响应格式不正确（HTTP {status}）"))
}

fn download_link(value: &Value, format: BookFormat) -> Result<Url> {
    if value.get("success").and_then(Value::as_i64) != Some(1) {
        return Err(rejected(
            value
                .pointer("/error/message")
                .or_else(|| value.get("error"))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string),
            "书源未提供下载文件，可能需要登录或已达到今日配额",
        ));
    }
    let file = value.get("file").context("下载接口未返回文件信息")?;
    if let Some(extension) = file.get("extension").and_then(Value::as_str) {
        anyhow::ensure!(
            BookFormat::parse(extension) == format,
            "书源实际返回的是 {extension}，请在详情中选择可用格式"
        );
    }
    let url = Url::parse(
        file.get("downloadLink")
            .and_then(Value::as_str)
            .context("下载接口没有返回真实文件地址")?,
    )?;
    anyhow::ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "书源返回的下载地址需要 HTTPS"
    );
    Ok(url)
}

#[derive(Debug, Deserialize)]
struct LoginEnvelope {
    success: i32,
    error: Option<String>,
    user: Option<LoginUser>,
}

#[derive(Debug, Deserialize)]
struct LoginUser {
    id: Value,
    remix_userkey: Option<String>,
    remix_user_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchEnvelope {
    success: i32,
    error: Option<String>,
    #[serde(default)]
    books: Vec<EapiBook>,
    #[serde(default)]
    pagination: EapiPagination,
}

#[derive(Debug, Deserialize)]
struct ProfileEnvelope {
    success: i32,
    error: Option<String>,
    user: Option<EapiProfileUser>,
}

#[derive(Debug, Deserialize)]
struct EapiProfileUser {
    #[serde(default)]
    downloads_today: Value,
    #[serde(default)]
    downloads_limit: Value,
    #[serde(default, rename = "dailyDownloadsCount")]
    daily_downloads_count: Value,
    #[serde(default, rename = "dailyDownloadLimit")]
    daily_download_limit: Value,
}

#[derive(Debug, Default, Deserialize)]
struct EapiPagination {
    #[serde(default)]
    current: u32,
    #[serde(default)]
    total_pages: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct EapiBook {
    id: Value,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    year: Value,
    #[serde(default)]
    identifier: Value,
    // Homepage cards carry the ISBN here instead of `identifier`.
    #[serde(default)]
    isbn: Value,
    #[serde(default)]
    language: String,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    filesize: Value,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "dl")]
    download_path: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    downloaded_at: String,
}

fn map_eapi_book(book: EapiBook) -> Option<(BookSummary, BookDetails, Option<String>)> {
    let id = json_string(&book.id);
    if id.is_empty() {
        return None;
    }
    let composite_id = if book.hash.is_empty() {
        id.clone()
    } else {
        format!("{id}:{}", book.hash)
    };
    let format = (!book.extension.is_empty()).then(|| BookFormat::parse(&book.extension));
    let authors = if book.author.trim().is_empty() {
        Vec::new()
    } else {
        vec![html_escape::decode_html_entities(&book.author).into_owned()]
    };
    let summary = BookSummary {
        id: composite_id,
        title: html_escape::decode_html_entities(&book.title).into_owned(),
        authors,
        year: json_string(&book.year).parse::<i32>().ok(),
        language: non_empty(book.language.clone()),
        format,
        available_formats: format.into_iter().collect(),
        size_bytes: json_u64(&book.filesize),
        cover_url: non_empty(book.cover.clone()),
    };
    let mut identifiers = Vec::new();
    let mut isbn = json_string(&book.identifier);
    if isbn.is_empty() {
        isbn = json_string(&book.isbn);
    }
    if !isbn.is_empty() {
        identifiers.push(("isbn".to_string(), isbn));
    }
    let acquisition_hint = non_empty(book.download_path.clone());
    let details = BookDetails {
        summary: summary.clone(),
        description: non_empty(
            html_escape::decode_html_entities(&crate::reader::html_to_text(&book.description))
                .into_owned(),
        ),
        identifiers,
    };
    Some((summary, details, acquisition_hint))
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn json_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => String::new(),
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn json_i64(value: &Value) -> i64 {
    match value {
        Value::Number(value) => value.as_i64().unwrap_or(0),
        Value::String(value) => value.parse().unwrap_or(0),
        _ => 0,
    }
}

fn first_non_zero(primary: &Value, fallback: &Value) -> i64 {
    let primary = json_i64(primary);
    if primary != 0 {
        primary
    } else {
        json_i64(fallback)
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn path_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod download_tests {
    use super::*;
    #[test]
    fn resolves_file_metadata_to_real_bytes_url_instead_of_downloading_json() {
        let metadata = serde_json::json!({"success":1,"file":{"downloadLink":"https://files.example.org/actual.epub?token=test","extension":"epub"}});
        assert_eq!(
            download_link(&metadata, BookFormat::Epub).unwrap().path(),
            "/actual.epub"
        );
        assert!(download_link(&metadata, BookFormat::Pdf).is_err());
        assert!(download_link(
            &serde_json::json!({"success":0,"error":"Please login"}),
            BookFormat::Epub
        )
        .is_err());
    }
    #[test]
    fn homepage_cards_do_not_invent_a_file_format() {
        let raw: EapiBook = serde_json::from_value(
            serde_json::json!({"id":42,"hash":"abc","title":"A &amp; B","author":"作者"}),
        )
        .unwrap();
        let (book, _, _) = map_eapi_book(raw).unwrap();
        assert_eq!(book.title, "A & B");
        assert!(book.format.is_none());
        assert!(book.available_formats.is_empty());
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    /// Minimal EAPI fixture: answers by path and records every request head it receives.
    async fn fixture(
        routes: &'static [(&'static str, &'static str)],
    ) -> (Url, Arc<StdMutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut buffer = [0_u8; 2048];
                while !head.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).await.unwrap_or(0);
                    if count == 0 {
                        break;
                    }
                    head.extend_from_slice(&buffer[..count]);
                }
                let head = String::from_utf8_lossy(&head).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                log.lock().unwrap().push(head);
                let body = routes
                    .iter()
                    .find(|(route, _)| path.starts_with(route))
                    .map(|(_, body)| *body)
                    .unwrap_or(r#"{"success":0,"error":"missing"}"#);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        (origin, seen)
    }

    async fn closed_origin() -> Url {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        drop(listener);
        origin
    }

    const POPULAR: &[(&str, &str)] = &[
        ("/eapi/info/domains", r#"{"success":1,"domains":[]}"#),
        (
            "/eapi/book/most-popular",
            r#"{"success":1,"books":[{"id":7,"hash":"ab12","title":"Fixture","isbn":"9780000000001"}]}"#,
        ),
        (
            "/eapi/book/search",
            r#"{"success":1,"books":[],"pagination":{"current":1,"total_pages":1}}"#,
        ),
    ];

    #[tokio::test]
    async fn unreachable_site_fails_over_to_a_working_mirror() {
        let dead = closed_origin().await;
        let (alive, _) = fixture(POPULAR).await;
        let provider = ZLibraryProvider::new(AppResolver::system().unwrap(), dead.clone()).unwrap();
        provider.set_fallback_origins(vec![dead.clone(), alive.clone()]);
        let mut changes = provider.origin_changes();
        let page = provider.discover().await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(provider.origin().await, alive);
        assert!(changes.has_changed().unwrap());
        assert_eq!(*changes.borrow_and_update(), alive);
    }

    #[tokio::test]
    async fn manual_site_reports_origin_unavailable_instead_of_switching() {
        let dead = closed_origin().await;
        let provider = ZLibraryProvider::new(AppResolver::system().unwrap(), dead.clone()).unwrap();
        let error = provider.discover().await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ProviderError>(),
            Some(ProviderError::OriginUnavailable)
        ));
        assert_eq!(provider.origin().await, dead);
    }

    #[tokio::test]
    async fn search_works_signed_out_and_sends_the_session_once_signed_in() {
        let (origin, seen) = fixture(POPULAR).await;
        let provider = ZLibraryProvider::new(AppResolver::system().unwrap(), origin).unwrap();
        let query = SearchQuery {
            text: "fixture".into(),
            page: 1,
            page_size: 10,
            formats: Vec::new(),
        };
        provider.search(query.clone()).await.unwrap();
        assert!(!seen.lock().unwrap()[0]
            .to_ascii_lowercase()
            .contains("remix_userkey"));
        provider
            .set_session(ZLibrarySession {
                user_id: "1".into(),
                user_key: "key".into(),
            })
            .await
            .unwrap();
        provider.search(query).await.unwrap();
        assert!(seen.lock().unwrap()[1].contains("remix_userkey=key"));
    }

    #[tokio::test]
    async fn signed_out_downloads_report_login_required() {
        let (origin, _) = fixture(POPULAR).await;
        let provider = ZLibraryProvider::new(AppResolver::system().unwrap(), origin).unwrap();
        let error = provider
            .acquisition_url("7:ab12", BookFormat::Epub)
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<ProviderError>(),
            Some(ProviderError::LoginRequired)
        ));
    }

    #[test]
    fn server_login_messages_become_login_required() {
        assert!(matches!(
            rejected(Some("Please login".into()), "x").downcast_ref::<ProviderError>(),
            Some(ProviderError::LoginRequired)
        ));
        assert_eq!(
            rejected(Some("Daily limit reached".into()), "x").to_string(),
            "Daily limit reached"
        );
        assert_eq!(rejected(None, "fallback").to_string(), "fallback");
    }
}

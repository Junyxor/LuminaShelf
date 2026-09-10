use super::{
    BookDetails, BookFormat, BookProvider, BookSummary, ProviderCapabilities, SearchQuery,
    SearchResult,
};
use crate::network::{AppResolver, ReqwestResolver};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, COOKIE},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZLibrarySession {
    pub user_id: String,
    pub user_key: String,
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
    client: Client,
    origin: Arc<RwLock<Url>>,
    session: Arc<RwLock<Option<ZLibrarySession>>>,
    cache: Arc<DashMap<String, BookDetails>>,
    acquisition_hints: Arc<DashMap<String, String>>,
}

impl ZLibraryProvider {
    pub fn new(resolver: AppResolver, origin: Url) -> Result<Self> {
        if origin.scheme() != "https" {
            return Err(anyhow!("Z-Library provider requires HTTPS"));
        }
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .user_agent("LuminaShelf/0.5 provider-zlibrary-eapi")
            .dns_resolver(Arc::new(ReqwestResolver::new(resolver)))
            .build()?;
        Ok(Self {
            client,
            origin: Arc::new(RwLock::new(origin)),
            session: Arc::new(RwLock::new(None)),
            cache: Arc::new(DashMap::new()),
            acquisition_hints: Arc::new(DashMap::new()),
        })
    }

    pub async fn set_origin(&self, origin: Url) -> Result<()> {
        if origin.scheme() != "https" {
            return Err(anyhow!("Z-Library provider requires HTTPS"));
        }
        *self.origin.write().await = origin;
        Ok(())
    }

    pub async fn origin(&self) -> Url {
        self.origin.read().await.clone()
    }

    pub async fn set_session(&self, session: ZLibrarySession) -> Result<()> {
        if session.user_id.trim().is_empty() || session.user_key.trim().is_empty() {
            return Err(anyhow!("session user id/key cannot be empty"));
        }
        *self.session.write().await = Some(session);
        Ok(())
    }

    pub async fn clear_session(&self) {
        *self.session.write().await = None;
    }

    pub async fn has_session(&self) -> bool {
        self.session.read().await.is_some()
    }

    pub async fn current_session(&self) -> Option<ZLibrarySession> {
        self.session.read().await.clone()
    }

    pub async fn profile(&self) -> Result<ZLibraryProfile> {
        let response: ProfileEnvelope = self.request_json("/eapi/user/profile", None, true).await?;
        if response.success != 1 {
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "profile request was rejected".to_string())));
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
        let response: SearchEnvelope = self.request_json(&path, None, true).await?;
        if response.success != 1 {
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "history request was rejected".to_string())));
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
            .request_json("/eapi/user/login", Some(body), false)
            .await?;
        if value.success != 1 {
            return Err(anyhow!(value
                .error
                .unwrap_or_else(|| "login was rejected".to_string())));
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
        auth: bool,
    ) -> Result<T> {
        let origin = self.origin().await;
        let endpoint = origin.join(path).context("build EAPI URL")?;
        let mut request = if let Some(body) = body {
            self.client
                .post(endpoint)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body)
        } else {
            self.client.get(endpoint)
        };
        request = request.header("accept", "application/json");
        if auth {
            let session = self
                .session
                .read()
                .await
                .clone()
                .ok_or_else(|| anyhow!("Z-Library account is not signed in"))?;
            request = request
                .header("remix-userid", &session.user_id)
                .header("remix-userkey", &session.user_key)
                .header(
                    "cookie",
                    format!(
                        "remix_userid={}; remix_userkey={}",
                        session.user_id, session.user_key
                    ),
                );
        }
        let response = request.send().await.context("EAPI request failed")?;
        let status = response.status();
        let bytes = response.bytes().await.context("read EAPI response")?;
        let decoded: T = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode EAPI response (HTTP {status})"))?;
        Ok(decoded)
    }
}

#[async_trait]
impl BookProvider for ZLibraryProvider {
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
        let response: SearchEnvelope = self
            .request_json("/eapi/book/search", Some(encoded), true)
            .await?;
        if response.success != 1 {
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "search was rejected".to_string())));
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
        self.cache
            .get(id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("book details are not cached; search for the book first"))
    }

    async fn acquisition_url(&self, id: &str, _format: BookFormat) -> Result<Url> {
        let origin = self.origin().await;
        if let Some((book_id, hash)) = id.split_once(':') {
            if book_id.is_empty() || hash.is_empty() {
                return Err(anyhow!("invalid download reference"));
            }
            return origin
                .join(&format!(
                    "/eapi/book/{}/{}/file",
                    path_component(book_id),
                    path_component(hash)
                ))
                .context("build download URL");
        }
        if let Some(hint) = self.acquisition_hints.get(id) {
            return origin
                .join(hint.value())
                .context("build hinted download URL");
        }
        Err(anyhow!(
            "download reference has neither a book hash nor an EAPI download path"
        ))
    }

    async fn download_headers(&self) -> Result<HeaderMap> {
        let session = self
            .session
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow!("Z-Library account is not signed in"))?;
        let mut headers = HeaderMap::new();
        let mut user_id =
            HeaderValue::from_str(&session.user_id).context("invalid session user id")?;
        let mut user_key =
            HeaderValue::from_str(&session.user_key).context("invalid session user key")?;
        let mut cookie = HeaderValue::from_str(&format!(
            "remix_userid={}; remix_userkey={}",
            session.user_id, session.user_key
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
    let format = BookFormat::parse(&book.extension);
    let authors = if book.author.trim().is_empty() {
        Vec::new()
    } else {
        vec![book.author.clone()]
    };
    let summary = BookSummary {
        id: composite_id,
        title: book.title.clone(),
        authors,
        year: json_string(&book.year).parse::<i32>().ok(),
        language: non_empty(book.language.clone()),
        format: Some(format),
        available_formats: vec![format],
        size_bytes: json_u64(&book.filesize),
        cover_url: non_empty(book.cover.clone()),
    };
    let mut identifiers = Vec::new();
    let isbn = json_string(&book.identifier);
    if !isbn.is_empty() {
        identifiers.push(("isbn".to_string(), isbn));
    }
    let acquisition_hint = non_empty(book.download_path.clone());
    let details = BookDetails {
        summary: summary.clone(),
        description: non_empty(book.description),
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

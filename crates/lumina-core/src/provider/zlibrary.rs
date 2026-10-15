use super::{
    BookDetails, BookFormat, BookProvider, BookSummary, ProviderCapabilities, SearchQuery,
    SearchResult,
};
use crate::network::{AppResolver, NetworkClient};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, COOKIE};
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
    client: NetworkClient,
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
        let client = NetworkClient::with_redirects(
            resolver,
            "LuminaShelf/0.6 ZLibrary",
            Some(Duration::from_secs(25)),
            false,
        )?;
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
        self.cache.clear();
        self.acquisition_hints.clear();
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
        let response: SearchEnvelope = self.request_json(&path, None, false).await?;
        if response.success != 1 {
            return Err(anyhow!(response
                .error
                .unwrap_or_else(|| "无法读取其他格式".into())));
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
        let value: Value = self.request_json(&path, None, false).await?;
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
        let value: Value = self.request_json(&path, None, false).await?;
        anyhow::ensure!(
            value.get("success").and_then(Value::as_i64) == Some(1),
            "书源未返回书籍详情"
        );
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
        let client = self.client.client()?;
        let mut request = if let Some(body) = body {
            client
                .post(endpoint)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(body)
        } else {
            client.get(endpoint)
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
        let response = request
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("EAPI request failed")?;
        decode_response(response).await
    }
}

#[async_trait]
impl BookProvider for ZLibraryProvider {
    async fn discover(&self) -> Result<super::DiscoveryPage> {
        let (path, title, authenticated) = if self.has_session().await {
            ("/eapi/user/book/recommended", "为你推荐", true)
        } else {
            ("/eapi/book/most-popular", "站点热门", false)
        };
        let response: SearchEnvelope = self.request_json(path, None, authenticated).await?;
        anyhow::ensure!(
            response.success == 1,
            "{}",
            response.error.unwrap_or_else(|| "无法读取书源首页".into())
        );
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
            let endpoint = self.origin().await.join(&path)?;
            let response = self
                .client
                .client()?
                .get(endpoint.clone())
                .headers(self.download_headers().await?)
                .send()
                .await
                .map_err(reqwest::Error::without_url)
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
    anyhow::ensure!(
        value.get("success").and_then(Value::as_i64) == Some(1),
        "{}",
        value
            .pointer("/error/message")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or("书源未提供下载文件，可能需要登录或已达到配额")
    );
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
    let isbn = json_string(&book.identifier);
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

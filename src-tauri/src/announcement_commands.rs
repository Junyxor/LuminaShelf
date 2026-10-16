use super::*;
use scraper::{Html, Selector};
use tauri_plugin_opener::OpenerExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    id: String,
    title: String,
    body: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    kind: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    app_items: Vec<Announcement>,
    site_items: Vec<Announcement>,
    site_origin: String,
    fetched_at_unix_ms: i64,
    app_source: String,
    site_error: Option<String>,
    app_error: Option<String>,
}
fn safe_url(raw: &str, base: &Url) -> Option<String> {
    let url = base.join(raw).ok()?;
    (url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}
fn parse_banner(raw: &str, origin: &Url) -> Vec<Announcement> {
    if raw.trim().is_empty() {
        return Vec::new();
    }
    let html = Html::parse_fragment(raw);
    let body = html.root_element().text().collect::<Vec<_>>().join(" ");
    let mut items = vec![Announcement {
        id: "source-banner".into(),
        title: "当前书源公告".into(),
        body: body.trim().into(),
        published_at: None,
        url: Some(origin.to_string()),
        kind: None,
    }];
    let selector = Selector::parse("a[href]").unwrap();
    for (index, link) in html.select(&selector).take(12).enumerate() {
        let title = link.text().collect::<Vec<_>>().join(" ").trim().to_string();
        if !title.is_empty() {
            if let Some(url) = link
                .value()
                .attr("href")
                .and_then(|raw| safe_url(raw, origin))
            {
                items.push(Announcement {
                    id: format!("source-banner-link-{index}"),
                    title,
                    body: String::new(),
                    published_at: None,
                    url: Some(url),
                    kind: None,
                });
            }
        }
    }
    items
}
async fn get_text(state: &AppState, url: Url) -> Result<String, String> {
    let mut response = state
        .zlibrary_probe
        .client()
        .map_err(|e| e.to_string())?
        .get(url)
        .header(
            "accept",
            "application/vnd.github.raw+json,application/json,text/html",
        )
        .send()
        .await
        .map_err(|e| format!("{:#}", e.without_url()))?
        .error_for_status()
        .map_err(|e| format!("{:#}", e.without_url()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 1024 * 1024 {
            return Err("公告内容超过 1 MB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "公告文本不是 UTF-8".into())
}
fn parse_app_items(raw: &str) -> Result<Vec<Announcement>, String> {
    #[derive(Deserialize)]
    struct Envelope {
        items: Vec<Announcement>,
    }
    let envelope: Envelope = serde_json::from_str(raw).map_err(|_| "项目公告格式不正确")?;
    Ok(envelope.items.into_iter().take(30).collect())
}

fn announcement_cache_key(origin: &Url) -> String {
    format!("announcements:{origin}")
}

fn cached_feed(store: &StateStore, origin: &Url) -> Result<Option<Feed>, String> {
    Ok(store
        .setting(&announcement_cache_key(origin))
        .map_err(|e| e.to_string())?
        .and_then(|raw| serde_json::from_str::<Feed>(&raw).ok())
        .filter(|feed| feed.site_origin == origin.as_str()))
}

#[tauri::command]
pub async fn announcement_feeds(state: State<'_, AppState>, refresh: bool) -> Result<Feed, String> {
    let origin = state.zlibrary.origin().await;
    let key = announcement_cache_key(&origin);
    let cached = cached_feed(&state.state_store, &origin)?;
    if !refresh {
        if let Some(feed) = cached
            .as_ref()
            .filter(|feed| now_unix_ms() - feed.fetched_at_unix_ms < 300_000)
        {
            return Ok(feed.clone());
        }
    }
    let app_url =
        "https://api.github.com/repos/Junyxor/LuminaShelf/contents/public/announcements.json?ref=main";
    let app_future = async {
        let raw = get_text(&state, Url::parse(app_url).unwrap()).await?;
        parse_app_items(&raw)
    };
    let site_future = async {
        let raw = get_text(
            &state,
            origin
                .join("/eapi/info?switchLanguage=chinese")
                .map_err(|e| e.to_string())?,
        )
        .await?;
        let value: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "书源公告接口未返回 JSON")?;
        if value.get("success").and_then(serde_json::Value::as_i64) != Some(1) {
            return Err("书源公告接口未返回成功结果".into());
        }
        let mut items = value
            .get("banner")
            .and_then(serde_json::Value::as_str)
            .map(|raw| parse_banner(raw, &origin))
            .unwrap_or_default();
        if items.is_empty() {
            let home = get_text(&state, origin.clone()).await?;
            let document = Html::parse_document(&home);
            let selector = Selector::parse("a[href]").unwrap();
            for link in document.select(&selector) {
                let title = link.text().collect::<Vec<_>>().join(" ").trim().to_string();
                let label = title.to_lowercase();
                if ["公告", "捐赠", "donat", "announcement", "news", "活动"]
                    .iter()
                    .any(|word| label.contains(word))
                {
                    if let Some(url) = link
                        .value()
                        .attr("href")
                        .and_then(|raw| safe_url(raw, &origin))
                    {
                        items.push(Announcement {
                            id: format!("source-home-{}", items.len()),
                            title,
                            body: String::new(),
                            published_at: None,
                            url: Some(url),
                            kind: None,
                        });
                        if items.len() >= 20 {
                            break;
                        }
                    }
                }
            }
        }
        Ok::<_, String>(items)
    };
    let (app, site) = tokio::join!(app_future, site_future);
    let (app_items, app_source, app_error) = match app {
        Ok(items) => (items, app_url.to_string(), None),
        Err(error) => {
            let items = cached
                .as_ref()
                .map(|feed| feed.app_items.clone())
                .unwrap_or_else(|| {
                    parse_app_items(include_str!("../../public/announcements.json"))
                        .unwrap_or_default()
                });
            (items, "应用内公告副本".into(), Some(error))
        }
    };
    let (site_items, site_error) = match site {
        Ok(items) => (items, None),
        Err(error) => (
            cached
                .as_ref()
                .map(|feed| feed.site_items.clone())
                .unwrap_or_default(),
            Some(error),
        ),
    };
    let feed = Feed {
        app_items,
        site_items,
        site_origin: origin.to_string(),
        fetched_at_unix_ms: now_unix_ms(),
        app_source,
        site_error,
        app_error,
    };
    state
        .state_store
        .set_setting(
            &key,
            &serde_json::to_string(&feed).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    Ok(feed)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateReport {
    status: String,
    current_version: String,
    latest_version: Option<String>,
    release_url: Option<String>,
    notes: Option<String>,
    published_at: Option<String>,
    apk_url: Option<String>,
    apk_bytes: Option<u64>,
    checked_at: i64,
}

#[tauri::command]
pub async fn check_app_update(state: State<'_, AppState>) -> Result<UpdateReport, String> {
    let current = env!("CARGO_PKG_VERSION");
    let response = state
        .zlibrary_probe
        .client()
        .map_err(|e| e.to_string())?
        .get("https://api.github.com/repos/Junyxor/LuminaShelf/releases/latest")
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("更新检查失败：{:#}", e.without_url()))?;
    let base = UpdateReport {
        status: "no_releases".into(),
        current_version: current.into(),
        latest_version: None,
        release_url: None,
        notes: None,
        published_at: None,
        apk_url: None,
        apk_bytes: None,
        checked_at: now_unix_ms(),
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(base);
    }
    if response.status() == reqwest::StatusCode::FORBIDDEN
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err("GitHub 请求受限，请稍后再检查更新".into());
    }
    let value: serde_json::Value = response
        .error_for_status()
        .map_err(|e| format!("{:#}", e.without_url()))?
        .json()
        .await
        .map_err(|_| "GitHub 更新响应格式不正确")?;
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or("发布没有版本号")?
        .trim_start_matches('v');
    let latest = semver::Version::parse(tag).map_err(|_| "发布版本号不符合版本规则")?;
    let installed = semver::Version::parse(current).map_err(|e| e.to_string())?;
    let assets = value
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let asset = assets.iter().find(|asset| {
        asset
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| name.ends_with("-universal-release.apk"))
    });
    let release_url = value
        .get("html_url")
        .and_then(serde_json::Value::as_str)
        .filter(|url| url.starts_with("https://github.com/Junyxor/LuminaShelf/releases/"))
        .map(str::to_string);
    Ok(UpdateReport {
        status: if latest > installed {
            "available"
        } else {
            "up_to_date"
        }
        .into(),
        latest_version: Some(latest.to_string()),
        release_url,
        notes: value
            .get("body")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        published_at: value
            .get("published_at")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        apk_url: asset
            .and_then(|asset| asset.get("browser_download_url"))
            .and_then(serde_json::Value::as_str)
            .filter(|url| {
                url.starts_with("https://github.com/Junyxor/LuminaShelf/releases/download/")
            })
            .map(str::to_string),
        apk_bytes: asset
            .and_then(|asset| asset.get("size"))
            .and_then(serde_json::Value::as_u64),
        ..base
    })
}

#[tauri::command]
pub fn open_announcement_link(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|_| "链接不正确")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("只支持 HTTPS 网页链接".into());
    }
    app.opener()
        .open_url(url, None::<String>)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcement_cache_cannot_reuse_another_sources_activity_links() {
        let store = StateStore::open_memory().unwrap();
        let first = Url::parse("https://books.example.org:8443/").unwrap();
        let second = Url::parse("https://books.example.org:9443/").unwrap();
        let feed = |origin: &Url| Feed {
            app_items: Vec::new(),
            site_items: parse_banner("<a href='/support'>捐赠活动</a>", origin),
            site_origin: origin.to_string(),
            fetched_at_unix_ms: now_unix_ms(),
            app_source: "test".into(),
            site_error: None,
            app_error: None,
        };
        let first_json = serde_json::to_string(&feed(&first)).unwrap();
        store
            .set_setting(&announcement_cache_key(&first), &first_json)
            .unwrap();
        assert_eq!(
            cached_feed(&store, &first).unwrap().unwrap().site_origin,
            first.as_str()
        );
        assert!(
            cached_feed(&store, &second).unwrap().is_none(),
            "A newly selected source must not reuse the previous source's feed"
        );
        // A mismatched persisted record must also be excluded from refresh/error fallback.
        store
            .set_setting(&announcement_cache_key(&second), &first_json)
            .unwrap();
        assert!(cached_feed(&store, &second).unwrap().is_none());
        store
            .set_setting(
                &announcement_cache_key(&second),
                &serde_json::to_string(&feed(&second)).unwrap(),
            )
            .unwrap();
        assert_eq!(
            cached_feed(&store, &first).unwrap().unwrap().site_origin,
            first.as_str()
        );
        let second_feed = cached_feed(&store, &second).unwrap().unwrap();
        assert_eq!(second_feed.site_origin, second.as_str());
        assert_eq!(
            second_feed.site_items[1].url.as_deref(),
            Some("https://books.example.org:9443/support")
        );
    }

    #[test]
    fn banner_links_are_read_from_source_and_unsafe_links_are_rejected() {
        let items=parse_banner("<div>活动公告 <a href='/support'>支持项目</a><a href='javascript:alert(1)'>bad</a></div>",&Url::parse("https://books.example.org").unwrap());
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[1].url.as_deref(),
            Some("https://books.example.org/support")
        );
        assert!(parse_banner("", &Url::parse("https://books.example.org").unwrap()).is_empty());
    }
}

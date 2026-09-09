use super::{BookDetails, BookFormat, BookProvider, BookSummary, ProviderCapabilities, SearchQuery, SearchResult};
use crate::network::{AppResolver, ReqwestResolver};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use url::Url;

#[derive(Clone)]
pub struct GutendexProvider {
    client: Client,
}

impl GutendexProvider {
    pub fn new(resolver: AppResolver) -> Result<Self> {
        let client = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(8))
            .timeout(std::time::Duration::from_secs(20))
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(6)
            .tcp_nodelay(true)
            .user_agent("LuminaShelf/0.5 provider-gutendex")
            .dns_resolver(Arc::new(ReqwestResolver::new(resolver)))
            .build()?;
        Ok(Self { client })
    }

    async fn fetch(&self, url: Url) -> Result<GutendexResponse> {
        self.client.get(url).send().await?.error_for_status()?.json().await.context("decode Gutendex response")
    }

    async fn fetch_one(&self, id: &str) -> Result<GutendexBook> {
        let mut url = Url::parse("https://gutendex.com/books/")?;
        url.query_pairs_mut().append_pair("ids", id);
        self.fetch(url).await?.results.into_iter().next().ok_or_else(|| anyhow!("book not found"))
    }
}

#[async_trait]
impl BookProvider for GutendexProvider {
    fn id(&self) -> &'static str { "gutendex" }
    fn name(&self) -> &'static str { "Project Gutenberg · Gutendex" }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities { authenticated: false, downloadable: true, searchable: true, paginated: true }
    }

    async fn search(&self, query: SearchQuery) -> Result<SearchResult> {
        let mut url = Url::parse("https://gutendex.com/books/")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("search", query.text.trim());
            if query.page > 1 { pairs.append_pair("page", &query.page.to_string()); }
        }
        let response = self.fetch(url).await?;
        let mut items = response.results.into_iter().map(to_summary).collect::<Vec<_>>();
        if !query.formats.is_empty() {
            items.retain(|item| item.available_formats.iter().any(|format| query.formats.contains(format)));
        }
        if query.page_size > 0 && items.len() > query.page_size as usize {
            items.truncate(query.page_size as usize);
        }
        Ok(SearchResult { items, page: query.page.max(1), has_next: response.next.is_some() })
    }

    async fn details(&self, id: &str) -> Result<BookDetails> {
        let book = self.fetch_one(id).await?;
        let summary = to_summary(book.clone());
        Ok(BookDetails {
            summary,
            description: book.subjects.first().cloned(),
            identifiers: vec![("gutenberg".to_string(), id.to_string())],
        })
    }

    async fn acquisition_url(&self, id: &str, format: BookFormat) -> Result<Url> {
        let book = self.fetch_one(id).await?;
        acquisition(&book.formats, format).ok_or_else(|| anyhow!("requested format is not available"))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GutendexResponse { next: Option<String>, results: Vec<GutendexBook> }

#[derive(Debug, Clone, Deserialize)]
struct GutendexBook {
    id: u64,
    title: String,
    #[serde(default)] authors: Vec<GutendexAuthor>,
    #[serde(default)] languages: Vec<String>,
    #[serde(default)] subjects: Vec<String>,
    #[serde(default)] formats: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GutendexAuthor { name: String }

fn to_summary(book: GutendexBook) -> BookSummary {
    let available_formats = available_formats(&book.formats);
    BookSummary {
        id: book.id.to_string(),
        title: book.title,
        authors: book.authors.into_iter().map(|author| author.name).collect(),
        year: None,
        language: book.languages.first().cloned(),
        format: available_formats.first().copied(),
        available_formats,
        size_bytes: None,
        cover_url: book.formats.get("image/jpeg").cloned(),
    }
}

fn available_formats(formats: &HashMap<String, String>) -> Vec<BookFormat> {
    let mut output = Vec::new();
    for format in [BookFormat::Epub, BookFormat::Pdf, BookFormat::Mobi, BookFormat::Txt] {
        if acquisition(formats, format).is_some() { output.push(format); }
    }
    output
}

fn acquisition(formats: &HashMap<String, String>, format: BookFormat) -> Option<Url> {
    let candidates: &[&str] = match format {
        BookFormat::Epub => &["application/epub+zip"],
        BookFormat::Pdf => &["application/pdf"],
        BookFormat::Mobi => &["application/x-mobipocket-ebook"],
        BookFormat::Txt => &["text/plain; charset=us-ascii", "text/plain; charset=utf-8", "text/plain"],
        _ => &[],
    };
    candidates.iter().find_map(|key| formats.get(*key)).and_then(|raw| Url::parse(raw).ok()).filter(|url| url.scheme() == "https" || url.scheme() == "http")
}

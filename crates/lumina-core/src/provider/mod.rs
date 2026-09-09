mod gutendex;
mod models;
mod zlibrary;

pub use gutendex::GutendexProvider;
pub use models::{
    BookDetails, BookFormat, BookSummary, ProviderCapabilities, ProviderDescriptor, SearchQuery,
    SearchResult,
};
pub use zlibrary::{
    ZLibraryHistoryItem, ZLibraryHistoryPage, ZLibraryProfile, ZLibraryProvider, ZLibrarySession,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

#[async_trait]
pub trait BookProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn search(&self, query: SearchQuery) -> Result<SearchResult>;
    async fn details(&self, id: &str) -> Result<BookDetails>;
    async fn acquisition_url(&self, id: &str, format: BookFormat) -> Result<url::Url>;
    async fn download_headers(&self) -> Result<reqwest::header::HeaderMap> {
        Ok(reqwest::header::HeaderMap::new())
    }
}

#[derive(Clone, Default)]
pub struct ProviderRegistry {
    providers: Arc<DashMap<String, Arc<dyn BookProvider>>>,
}

impl ProviderRegistry {
    pub fn register<P: BookProvider + 'static>(&self, provider: P) {
        self.providers
            .insert(provider.id().to_string(), Arc::new(provider));
    }
    pub fn register_shared<P: BookProvider + 'static>(&self, provider: Arc<P>) {
        self.providers.insert(provider.id().to_string(), provider);
    }
    pub fn descriptors(&self) -> Vec<ProviderDescriptor> {
        let mut items = self
            .providers
            .iter()
            .map(|entry| {
                let provider = entry.value();
                ProviderDescriptor {
                    id: provider.id().to_string(),
                    name: provider.name().to_string(),
                    capabilities: provider.capabilities(),
                }
            })
            .collect::<Vec<_>>();
        items.sort_by(|a, b| a.name.cmp(&b.name));
        items
    }
    pub async fn search(&self, provider_id: &str, query: SearchQuery) -> Result<SearchResult> {
        let provider = self
            .providers
            .get(provider_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("unknown provider: {provider_id}"))?;
        provider.search(query).await
    }
    pub async fn details(&self, provider_id: &str, id: &str) -> Result<BookDetails> {
        let provider = self
            .providers
            .get(provider_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("unknown provider: {provider_id}"))?;
        provider.details(id).await
    }
    pub async fn acquisition_url(
        &self,
        provider_id: &str,
        id: &str,
        format: BookFormat,
    ) -> Result<url::Url> {
        let provider = self
            .providers
            .get(provider_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("unknown provider: {provider_id}"))?;
        provider.acquisition_url(id, format).await
    }
    pub async fn download_headers(&self, provider_id: &str) -> Result<reqwest::header::HeaderMap> {
        let provider = self
            .providers
            .get(provider_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("unknown provider: {provider_id}"))?;
        provider.download_headers().await
    }
}

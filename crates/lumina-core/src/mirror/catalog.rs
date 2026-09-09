use super::{EndpointSource, MirrorRegistry, MirrorSourceKind, RemoteEndpointSource};
use crate::network::{AppResolver, ReqwestResolver};
use anyhow::{Context, Result};
use dashmap::DashMap;
use futures::{stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDefinition {
    pub id: String,
    pub label: String,
    pub url: Url,
    pub kind: MirrorSourceKind,
    pub enabled: bool,
    pub allow_insecure_http: bool,
    pub last_refresh_unix_ms: Option<u64>,
    pub discovered_count: usize,
    pub last_error: Option<String>,
}

impl SourceDefinition {
    pub fn new(label: impl Into<String>, url: Url, kind: MirrorSourceKind) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            label: label.into(),
            url,
            kind,
            enabled: true,
            allow_insecure_http: false,
            last_refresh_unix_ms: None,
            discovered_count: 0,
            last_error: None,
        }
    }
}

#[derive(Clone)]
pub struct SourceCatalog {
    client: Client,
    entries: Arc<DashMap<String, SourceDefinition>>,
}

impl SourceCatalog {
    pub fn new(resolver: AppResolver) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(4))
            .tcp_nodelay(true)
            .pool_idle_timeout(Duration::from_secs(45))
            .user_agent("LuminaShelf/0.2 source-discovery")
            .dns_resolver(Arc::new(ReqwestResolver::new(resolver)))
            .build()?;
        Ok(Self {
            client,
            entries: Arc::new(DashMap::new()),
        })
    }

    pub fn add(&self, source: SourceDefinition) -> String {
        let id = source.id.clone();
        self.entries.insert(id.clone(), source);
        id
    }

    pub fn remove(&self, id: &str) -> Option<SourceDefinition> {
        self.entries.remove(id).map(|(_, source)| source)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> bool {
        let Some(mut source) = self.entries.get_mut(id) else {
            return false;
        };
        source.enabled = enabled;
        true
    }

    pub fn snapshot(&self) -> Vec<SourceDefinition> {
        let mut sources: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        sources.sort_by(|a, b| {
            a.label
                .to_ascii_lowercase()
                .cmp(&b.label.to_ascii_lowercase())
        });
        sources
    }

    pub async fn refresh(&self, id: &str, registry: &MirrorRegistry) -> Result<usize> {
        let source = self
            .entries
            .get(id)
            .context("unknown mirror source")?
            .clone();
        if !source.enabled {
            return Ok(0);
        }
        let remote =
            RemoteEndpointSource::new(self.client.clone(), source.url.clone(), source.kind)
                .allow_insecure_http(source.allow_insecure_http);
        match remote.discover().await {
            Ok(endpoints) => {
                let count = endpoints.len();
                for endpoint in endpoints {
                    registry.upsert(endpoint);
                }
                if let Some(mut entry) = self.entries.get_mut(id) {
                    entry.last_refresh_unix_ms = Some(unix_ms());
                    entry.discovered_count = count;
                    entry.last_error = None;
                }
                Ok(count)
            }
            Err(error) => {
                if let Some(mut entry) = self.entries.get_mut(id) {
                    entry.last_refresh_unix_ms = Some(unix_ms());
                    entry.last_error = Some(error.to_string());
                }
                Err(error)
            }
        }
    }

    pub async fn refresh_all(&self, registry: &MirrorRegistry) -> Vec<(String, Result<usize>)> {
        let ids: Vec<String> = self
            .entries
            .iter()
            .filter(|entry| entry.value().enabled)
            .map(|entry| entry.key().clone())
            .collect();
        let concurrency = ids.len().clamp(1, 4);
        stream::iter(ids)
            .map(|id| {
                let catalog = self.clone();
                let registry = registry.clone();
                async move {
                    let result = catalog.refresh(&id, &registry).await;
                    (id, result)
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

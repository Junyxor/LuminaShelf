use super::{MirrorEndpoint, MirrorRegistry, MirrorSourceKind, ProbeConfig, ProbeEngine};
use crate::network::AppResolver;
use anyhow::{Context, Result};
use url::Url;

#[derive(Clone)]
pub struct MirrorRuntime {
    registry: MirrorRegistry,
    probe: ProbeEngine,
}

impl MirrorRuntime {
    pub fn new(resolver: AppResolver) -> Result<Self> {
        Ok(Self {
            registry: MirrorRegistry::default(),
            probe: ProbeEngine::with_resolver(ProbeConfig::default(), resolver)?,
        })
    }

    pub fn registry(&self) -> &MirrorRegistry {
        &self.registry
    }

    pub fn seed_builtin<I, S>(&self, origins: I) -> Result<usize>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut added = 0usize;
        for raw in origins {
            let raw = raw.as_ref().trim();
            if raw.is_empty() {
                continue;
            }
            let origin =
                Url::parse(raw).with_context(|| format!("invalid mirror origin: {raw}"))?;
            if origin.scheme() != "https" || origin.host_str().is_none() {
                anyhow::bail!("mirror origin must be an HTTPS host: {raw}");
            }
            let label = origin.host_str().unwrap_or(raw).to_string();
            self.registry.upsert(MirrorEndpoint::new(
                label,
                origin,
                MirrorSourceKind::Builtin,
            ));
            added += 1;
        }
        Ok(added)
    }

    pub async fn probe_all(&self) -> Vec<MirrorEndpoint> {
        self.probe.probe_registry(&self.registry).await;
        self.registry.snapshot()
    }

    pub fn ranked(&self) -> Vec<MirrorEndpoint> {
        self.registry.snapshot()
    }

    pub fn best(&self) -> Option<MirrorEndpoint> {
        self.registry.best()
    }

    pub fn best_origin(&self) -> Option<Url> {
        self.best().map(|endpoint| endpoint.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_origins_are_deduplicated_by_origin() {
        let runtime = MirrorRuntime::new(AppResolver::system().unwrap()).unwrap();
        runtime
            .seed_builtin(["https://example.com", "https://example.com/"])
            .unwrap();
        assert_eq!(runtime.ranked().len(), 1);
    }

    #[test]
    fn rejects_non_https_builtin_origin() {
        let runtime = MirrorRuntime::new(AppResolver::system().unwrap()).unwrap();
        assert!(runtime.seed_builtin(["http://example.com"]).is_err());
    }
}

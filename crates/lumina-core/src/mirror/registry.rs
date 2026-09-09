use super::{MirrorEndpoint, MirrorSourceKind, MirrorState};
use dashmap::DashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct MirrorRegistry {
    inner: Arc<DashMap<String, MirrorEndpoint>>,
    by_origin: Arc<DashMap<String, String>>,
}

impl MirrorRegistry {
    pub fn upsert(&self, mut endpoint: MirrorEndpoint) {
        if endpoint.observed_sources.is_empty() {
            endpoint.observed_sources.push(endpoint.source);
        }
        let origin_key = canonical_origin(&endpoint);
        if let Some(existing_id) = self
            .by_origin
            .get(&origin_key)
            .map(|entry| entry.value().clone())
        {
            if let Some(mut existing) = self.inner.get_mut(&existing_id) {
                let incoming_source = endpoint.source;
                if !existing.observed_sources.contains(&incoming_source) {
                    existing.observed_sources.push(incoming_source);
                }
                for source in endpoint.observed_sources.drain(..) {
                    if !existing.observed_sources.contains(&source) {
                        existing.observed_sources.push(source);
                    }
                }
                for ip in endpoint.address_hints.drain(..) {
                    if !existing.address_hints.contains(&ip) {
                        existing.address_hints.push(ip);
                    }
                }
                if source_rank(incoming_source) > source_rank(existing.source) {
                    existing.source = incoming_source;
                    existing.label = endpoint.label;
                }
                return;
            }
            self.by_origin.remove(&origin_key);
        }
        self.by_origin.insert(origin_key, endpoint.id.clone());
        self.inner.insert(endpoint.id.clone(), endpoint);
    }

    pub fn remove(&self, id: &str) -> Option<MirrorEndpoint> {
        let (_, endpoint) = self.inner.remove(id)?;
        self.by_origin.remove(&canonical_origin(&endpoint));
        Some(endpoint)
    }

    pub fn get(&self, id: &str) -> Option<MirrorEndpoint> {
        self.inner.get(id).map(|entry| entry.value().clone())
    }

    pub fn snapshot(&self) -> Vec<MirrorEndpoint> {
        let mut items: Vec<_> = self
            .inner
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        items.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| source_rank(b.source).cmp(&source_rank(a.source)))
        });
        items
    }

    pub fn best(&self) -> Option<MirrorEndpoint> {
        self.inner
            .iter()
            .filter(|entry| {
                matches!(
                    entry.value().state,
                    MirrorState::Healthy | MirrorState::Degraded
                )
            })
            .max_by(|a, b| a.value().score.total_cmp(&b.value().score))
            .map(|entry| entry.value().clone())
    }

    pub fn update<F>(&self, id: &str, mutate: F) -> bool
    where
        F: FnOnce(&mut MirrorEndpoint),
    {
        let Some(mut entry) = self.inner.get_mut(id) else {
            return false;
        };
        mutate(entry.value_mut());
        true
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

fn canonical_origin(endpoint: &MirrorEndpoint) -> String {
    endpoint.origin.as_str().trim_end_matches('/').to_owned()
}

fn source_rank(source: MirrorSourceKind) -> u8 {
    match source {
        MirrorSourceKind::Official => 5,
        MirrorSourceKind::Builtin => 4,
        MirrorSourceKind::Github => 3,
        MirrorSourceKind::Subscription => 2,
        MirrorSourceKind::Manual => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn duplicate_origin_merges_provenance_without_losing_probe_state() {
        let registry = MirrorRegistry::default();
        let mut manual = MirrorEndpoint::new(
            "manual",
            Url::parse("https://example.com").unwrap(),
            MirrorSourceKind::Manual,
        );
        manual.score = 88.0;
        manual.state = MirrorState::Healthy;
        registry.upsert(manual);
        registry.upsert(MirrorEndpoint::new(
            "official",
            Url::parse("https://example.com/").unwrap(),
            MirrorSourceKind::Official,
        ));
        let endpoint = registry.snapshot().pop().unwrap();
        assert_eq!(endpoint.score, 88.0);
        assert_eq!(endpoint.source, MirrorSourceKind::Official);
        assert_eq!(endpoint.observed_sources.len(), 2);
    }

    #[test]
    fn duplicate_origin_merges_address_hints() {
        let registry = MirrorRegistry::default();
        let mut first = MirrorEndpoint::new(
            "A",
            Url::parse("https://example.com").unwrap(),
            MirrorSourceKind::Github,
        );
        first.address_hints.push("203.0.113.1".parse().unwrap());
        let mut second = MirrorEndpoint::new(
            "B",
            Url::parse("https://example.com/").unwrap(),
            MirrorSourceKind::Subscription,
        );
        second.address_hints.push("203.0.113.2".parse().unwrap());
        registry.upsert(first);
        registry.upsert(second);
        let endpoint = registry.snapshot().pop().unwrap();
        assert_eq!(endpoint.address_hints.len(), 2);
    }
}

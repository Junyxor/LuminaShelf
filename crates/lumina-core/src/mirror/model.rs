use serde::{Deserialize, Serialize};
use std::{
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorSourceKind {
    Official,
    Builtin,
    Github,
    Subscription,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MirrorState {
    Healthy,
    Degraded,
    Offline,
    Probing,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProbeMetrics {
    pub dns_ms: Option<f64>,
    pub tcp_ms: Option<f64>,
    pub tls_ms: Option<f64>,
    pub ttfb_ms: Option<f64>,
    pub total_ms: Option<f64>,
    pub throughput_mbps: Option<f64>,
    pub status_code: Option<u16>,
    pub redirect_count: u8,
    pub resolved_addresses: Vec<IpAddr>,
    pub selected_address: Option<IpAddr>,
    pub via_app_hosts: bool,
    pub resolver_fallback: bool,
    pub via_address_hint: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorEndpoint {
    pub id: String,
    pub label: String,
    pub origin: Url,
    #[serde(default)]
    pub address_hints: Vec<IpAddr>,
    pub source: MirrorSourceKind,
    #[serde(default)]
    pub observed_sources: Vec<MirrorSourceKind>,
    pub state: MirrorState,
    pub score: f64,
    pub metrics: ProbeMetrics,
    pub success_streak: u32,
    pub failure_streak: u32,
    pub last_success_unix_ms: Option<u64>,
    pub last_probe_unix_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl MirrorEndpoint {
    pub fn new(label: impl Into<String>, origin: Url, source: MirrorSourceKind) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            label: label.into(),
            origin,
            address_hints: Vec::new(),
            source,
            observed_sources: vec![source],
            state: MirrorState::Probing,
            score: 0.0,
            metrics: ProbeMetrics::default(),
            success_streak: 0,
            failure_streak: 0,
            last_success_unix_ms: None,
            last_probe_unix_ms: None,
            last_error: None,
        }
    }

    pub fn mark_probe(&mut self, metrics: ProbeMetrics, healthy: bool, score: f64, error: Option<String>) {
        let now = unix_ms();
        self.metrics = metrics;
        self.last_probe_unix_ms = Some(now);
        self.score = score.clamp(0.0, 100.0);
        self.last_error = error;
        if healthy {
            self.success_streak = self.success_streak.saturating_add(1);
            self.failure_streak = 0;
            self.last_success_unix_ms = Some(now);
            self.state = if self.score >= 75.0 { MirrorState::Healthy } else { MirrorState::Degraded };
            self.last_error = None;
        } else {
            self.failure_streak = self.failure_streak.saturating_add(1);
            self.success_streak = 0;
            self.state = MirrorState::Offline;
        }
    }
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v01_persisted_endpoint_remains_readable() {
        let old = r#"{
          "id":"old-route",
          "label":"Legacy",
          "origin":"https://example.com/",
          "source":"manual",
          "state":"healthy",
          "score":88.0,
          "metrics":{"dnsMs":9.0,"tcpMs":25.0,"tlsMs":30.0,"ttfbMs":80.0,"totalMs":160.0,"throughputMbps":22.0,"statusCode":200,"redirectCount":0},
          "successStreak":3,
          "failureStreak":0,
          "lastSuccessUnixMs":1,
          "lastProbeUnixMs":1
        }"#;
        let endpoint: MirrorEndpoint = serde_json::from_str(old).unwrap();
        assert!(endpoint.address_hints.is_empty());
        assert!(endpoint.observed_sources.is_empty());
        assert!(endpoint.metrics.resolved_addresses.is_empty());
        assert!(!endpoint.metrics.via_address_hint);
        assert!(endpoint.last_error.is_none());
    }
}

use anyhow::{anyhow, Context, Result};
use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig, ResolverOpts, ServerOrderingStrategy},
    net::runtime::TokioRuntimeProvider,
    TokioResolver,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::RwLock, time::timeout};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolverMode {
    System,
    AppHosts,
    Doh,
    Dot,
    CustomDns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResolverPolicy {
    pub mode: ResolverMode,
    pub app_hosts: HashMap<String, Vec<IpAddr>>,
    pub upstream_ip: Option<IpAddr>,
    pub server_name: Option<String>,
    pub doh_path: Option<String>,
    pub fallback_to_system: bool,
    pub timeout_ms: u64,
}

impl Default for ResolverPolicy {
    fn default() -> Self {
        Self {
            mode: ResolverMode::System,
            app_hosts: HashMap::new(),
            upstream_ip: None,
            server_name: None,
            doh_path: Some("/dns-query".to_string()),
            fallback_to_system: true,
            timeout_ms: 3_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveResult {
    pub host: String,
    pub addresses: Vec<IpAddr>,
    pub elapsed_ms: f64,
    pub via_app_hosts: bool,
    pub used_fallback: bool,
    pub mode: ResolverMode,
}

#[derive(Clone)]
pub struct AppResolver {
    policy: Arc<RwLock<ResolverPolicy>>,
    selected: Arc<RwLock<Arc<TokioResolver>>>,
    system: Arc<TokioResolver>,
}

impl AppResolver {
    pub fn new(policy: ResolverPolicy) -> Result<Self> {
        validate_policy(&policy)?;
        let system = Arc::new(build_system_resolver()?);
        let selected = if matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) {
            system.clone()
        } else {
            Arc::new(build_resolver(&policy)?)
        };
        Ok(Self {
            policy: Arc::new(RwLock::new(policy)),
            selected: Arc::new(RwLock::new(selected)),
            system,
        })
    }

    pub fn system() -> Result<Self> {
        Self::new(ResolverPolicy::default())
    }

    pub async fn policy(&self) -> ResolverPolicy {
        self.policy.read().await.clone()
    }

    pub async fn set_policy(&self, policy: ResolverPolicy) -> Result<()> {
        validate_policy(&policy)?;
        let current = self.policy().await;
        if resolver_transport_changed(&current, &policy) {
            let next = if matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) {
                self.system.clone()
            } else {
                Arc::new(build_resolver(&policy)?)
            };
            *self.selected.write().await = next;
        }
        *self.policy.write().await = policy;
        Ok(())
    }

    pub async fn set_host_override(&self, host: impl Into<String>, addresses: Vec<IpAddr>) -> Result<()> {
        if addresses.is_empty() {
            return Err(anyhow!("at least one address is required"));
        }
        let host = normalize_host(&host.into())?;
        let mut policy = self.policy().await;
        policy.app_hosts.insert(host, addresses);
        self.set_policy(policy).await
    }

    pub async fn remove_host_override(&self, host: &str) -> Result<bool> {
        let host = normalize_host(host)?;
        let mut policy = self.policy().await;
        let removed = policy.app_hosts.remove(&host).is_some();
        self.set_policy(policy).await?;
        Ok(removed)
    }

    pub async fn resolve(&self, host: &str) -> Result<ResolveResult> {
        let host = normalize_host(host)?;
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(ResolveResult {
                host,
                addresses: vec![ip],
                elapsed_ms: 0.0,
                via_app_hosts: true,
                used_fallback: false,
                mode: self.policy.read().await.mode,
            });
        }

        let policy = self.policy().await;
        if let Some(addresses) = app_host_match(&policy.app_hosts, &host) {
            return Ok(ResolveResult {
                host,
                addresses,
                elapsed_ms: 0.0,
                via_app_hosts: true,
                used_fallback: false,
                mode: policy.mode,
            });
        }

        let started = Instant::now();
        let selected = self.selected.read().await.clone();
        let lookup = lookup_with_timeout(&selected, &host, policy.timeout_ms).await;
        match lookup {
            Ok(addresses) if !addresses.is_empty() => Ok(ResolveResult {
                host,
                addresses,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                via_app_hosts: false,
                used_fallback: false,
                mode: policy.mode,
            }),
            Ok(_) | Err(_) if policy.fallback_to_system && !matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) => {
                let addresses = lookup_with_timeout(&self.system, &host, policy.timeout_ms)
                    .await
                    .context("selected resolver failed and system fallback also failed")?;
                Ok(ResolveResult {
                    host,
                    addresses,
                    elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                    via_app_hosts: false,
                    used_fallback: true,
                    mode: policy.mode,
                })
            }
            Ok(_) => Err(anyhow!("DNS lookup returned no addresses for {host}")),
            Err(error) => Err(error),
        }
    }
}

fn resolver_transport_changed(before: &ResolverPolicy, after: &ResolverPolicy) -> bool {
    let before_system = matches!(before.mode, ResolverMode::System | ResolverMode::AppHosts);
    let after_system = matches!(after.mode, ResolverMode::System | ResolverMode::AppHosts);
    if before_system && after_system {
        return false;
    }
    before.mode != after.mode
        || before.upstream_ip != after.upstream_ip
        || before.server_name != after.server_name
        || before.doh_path != after.doh_path
}

fn build_system_resolver() -> Result<TokioResolver> {
    let mut builder = TokioResolver::builder_tokio().context("load system DNS configuration")?;
    builder.options_mut().server_ordering_strategy = ServerOrderingStrategy::QueryStatistics;
    builder.build().context("build system DNS resolver")
}

fn build_resolver(policy: &ResolverPolicy) -> Result<TokioResolver> {
    if matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) {
        return build_system_resolver();
    }
    let ip = policy.upstream_ip.context("custom resolver requires upstreamIp")?;
    let name_server = match policy.mode {
        ResolverMode::CustomDns => NameServerConfig::udp_and_tcp(ip),
        ResolverMode::Dot => {
            let server_name: Arc<str> = Arc::from(policy.server_name.as_deref().context("DoT requires serverName")?);
            NameServerConfig::tls(ip, server_name)
        }
        ResolverMode::Doh => {
            let server_name: Arc<str> = Arc::from(policy.server_name.as_deref().context("DoH requires serverName")?);
            let path: Arc<str> = Arc::from(policy.doh_path.as_deref().unwrap_or("/dns-query"));
            NameServerConfig::https(ip, server_name, Some(path))
        }
        ResolverMode::System | ResolverMode::AppHosts => unreachable!(),
    };
    let config = ResolverConfig::from_parts(None, Vec::new(), vec![name_server]);
    let mut builder = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
    let opts: &mut ResolverOpts = builder.options_mut();
    opts.server_ordering_strategy = ServerOrderingStrategy::UserProvidedOrder;
    builder.build().context("build configured DNS resolver")
}

fn validate_policy(policy: &ResolverPolicy) -> Result<()> {
    if policy.timeout_ms < 250 || policy.timeout_ms > 30_000 {
        return Err(anyhow!("timeoutMs must be between 250 and 30000"));
    }
    match policy.mode {
        ResolverMode::System | ResolverMode::AppHosts => {}
        ResolverMode::CustomDns => {
            policy.upstream_ip.context("custom DNS requires upstreamIp")?;
        }
        ResolverMode::Dot | ResolverMode::Doh => {
            policy.upstream_ip.context("encrypted DNS requires upstreamIp")?;
            let name = policy.server_name.as_deref().context("encrypted DNS requires serverName")?;
            if name.starts_with("*.") {
                return Err(anyhow!("encrypted DNS serverName cannot be a wildcard"));
            }
            normalize_host(name)?;
        }
    }
    for (host, addresses) in &policy.app_hosts {
        normalize_host(host)?;
        if addresses.is_empty() {
            return Err(anyhow!("host override {host} has no addresses"));
        }
    }
    Ok(())
}

async fn lookup_with_timeout(resolver: &TokioResolver, host: &str, timeout_ms: u64) -> Result<Vec<IpAddr>> {
    let fqdn = format!("{}.", host.trim_end_matches('.'));
    let response = timeout(Duration::from_millis(timeout_ms), resolver.lookup_ip(fqdn))
        .await
        .context("DNS lookup timed out")??;
    let mut addresses: Vec<IpAddr> = response.iter().collect();
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

fn app_host_match(entries: &HashMap<String, Vec<IpAddr>>, host: &str) -> Option<Vec<IpAddr>> {
    if let Some(addresses) = entries.get(host) {
        return Some(addresses.clone());
    }
    entries
        .iter()
        .filter_map(|(pattern, addresses)| {
            let suffix = pattern.strip_prefix("*.")?;
            (host != suffix && host.ends_with(&format!(".{suffix}"))).then_some((suffix.len(), addresses))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, addresses)| addresses.clone())
}

fn normalize_host(host: &str) -> Result<String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.contains('/') || host.contains(char::is_whitespace) {
        return Err(anyhow!("invalid host name"));
    }
    if let Some(suffix) = host.strip_prefix("*.") {
        if suffix.is_empty() || suffix.starts_with('.') {
            return Err(anyhow!("invalid wildcard host"));
        }
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_host_prefers_longest_suffix() {
        let mut hosts = HashMap::new();
        hosts.insert("*.example.org".to_string(), vec!["1.1.1.1".parse().unwrap()]);
        hosts.insert("*.api.example.org".to_string(), vec!["2.2.2.2".parse().unwrap()]);
        assert_eq!(app_host_match(&hosts, "v1.api.example.org").unwrap()[0], "2.2.2.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn exact_host_wins() {
        let mut hosts = HashMap::new();
        hosts.insert("api.example.org".to_string(), vec!["3.3.3.3".parse().unwrap()]);
        hosts.insert("*.example.org".to_string(), vec!["1.1.1.1".parse().unwrap()]);
        assert_eq!(app_host_match(&hosts, "api.example.org").unwrap()[0], "3.3.3.3".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn v01_resolver_policy_remains_readable() {
        let old = r#"{
          "mode":"system",
          "appHosts":{"legacy.local":["127.0.0.1"]},
          "endpoint":null,
          "fallbackToSystem":true
        }"#;
        let policy: ResolverPolicy = serde_json::from_str(old).unwrap();
        assert_eq!(policy.timeout_ms, 3_000);
        assert_eq!(policy.doh_path.as_deref(), Some("/dns-query"));
        assert!(policy.app_hosts.contains_key("legacy.local"));
    }
}

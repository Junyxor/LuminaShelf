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
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock as SyncRwLock,
    },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ResolverPolicy {
    pub mode: ResolverMode,
    pub app_hosts: HashMap<String, Vec<IpAddr>>,
    pub upstream_ip: Option<IpAddr>,
    pub bootstrap_ips: Vec<IpAddr>,
    pub server_name: Option<String>,
    pub doh_path: Option<String>,
    pub doh_url: Option<String>,
    pub upstream_port: Option<u16>,
    pub proxy_url: Option<String>,
    pub fallback_to_system: bool,
    pub timeout_ms: u64,
}

impl Default for ResolverPolicy {
    fn default() -> Self {
        Self {
            mode: ResolverMode::System,
            app_hosts: HashMap::new(),
            upstream_ip: None,
            bootstrap_ips: Vec::new(),
            server_name: None,
            doh_path: Some("/dns-query".to_string()),
            doh_url: None,
            upstream_port: None,
            proxy_url: None,
            fallback_to_system: true,
            timeout_ms: 3_000,
        }
    }
}

impl ResolverPolicy {
    pub fn validated(self) -> Result<Self> {
        let policy = normalize_policy(self)?;
        validate_policy(&policy)?;
        Ok(policy)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DnsProfile {
    pub id: String,
    pub name: String,
    pub policy: ResolverPolicy,
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
    selected: Arc<RwLock<Arc<DnsBackend>>>,
    system: Arc<DnsBackend>,
    revision: Arc<AtomicU64>,
    proxy: Arc<SyncRwLock<Option<String>>>,
}

enum DnsBackend {
    Hickory(Box<TokioResolver>),
    Doh(Box<super::doh::DohResolver>),
    #[cfg(any(target_os = "android", test))]
    Native,
}

impl AppResolver {
    pub fn new(policy: ResolverPolicy) -> Result<Self> {
        let policy = normalize_policy(policy)?;
        validate_policy(&policy)?;
        Self::from_system(policy, build_system_resolver()?)
    }

    fn from_system(policy: ResolverPolicy, system: DnsBackend) -> Result<Self> {
        let system = Arc::new(system);
        let selected = if matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) {
            system.clone()
        } else {
            Arc::new(build_resolver(&policy)?)
        };
        Ok(Self {
            revision: Arc::new(AtomicU64::new(0)),
            proxy: Arc::new(SyncRwLock::new(policy.proxy_url.clone())),
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

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn proxy_url(&self) -> Option<String> {
        self.proxy.read().ok().and_then(|proxy| proxy.clone())
    }

    pub async fn set_policy(&self, policy: ResolverPolicy) -> Result<()> {
        let policy = normalize_policy(policy)?;
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
        *self.proxy.write().map_err(|_| anyhow!("代理配置锁异常"))? = policy.proxy_url.clone();
        *self.policy.write().await = policy;
        self.revision.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub async fn set_host_override(
        &self,
        host: impl Into<String>,
        addresses: Vec<IpAddr>,
    ) -> Result<()> {
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
            Ok(_) | Err(_)
                if policy.fallback_to_system
                    && !matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) =>
            {
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
        || before.bootstrap_ips != after.bootstrap_ips
        || before.server_name != after.server_name
        || before.doh_path != after.doh_path
        || before.doh_url != after.doh_url
        || before.upstream_port != after.upstream_port
        || before.proxy_url != after.proxy_url
        || before.timeout_ms != after.timeout_ms
}

#[cfg(target_os = "android")]
fn build_system_resolver() -> Result<DnsBackend> {
    // Android can have no active network at startup. OS lookups are lazy and also
    // follow changes to the active network, VPN and Private DNS configuration.
    Ok(DnsBackend::Native)
}

#[cfg(not(target_os = "android"))]
fn build_system_resolver() -> Result<DnsBackend> {
    let mut builder = TokioResolver::builder_tokio().context("load system DNS configuration")?;
    builder.options_mut().server_ordering_strategy = ServerOrderingStrategy::QueryStatistics;
    Ok(DnsBackend::Hickory(Box::new(
        builder.build().context("build system DNS resolver")?,
    )))
}

fn build_resolver(policy: &ResolverPolicy) -> Result<DnsBackend> {
    if policy.mode == ResolverMode::Doh {
        return Ok(DnsBackend::Doh(Box::new(super::doh::DohResolver::new(
            policy,
        )?)));
    }
    if matches!(policy.mode, ResolverMode::System | ResolverMode::AppHosts) {
        return build_system_resolver();
    }
    let ip = policy
        .upstream_ip
        .context("custom resolver requires upstreamIp")?;
    let mut name_server = match policy.mode {
        ResolverMode::CustomDns => NameServerConfig::udp_and_tcp(ip),
        ResolverMode::Dot => {
            let server_name: Arc<str> = Arc::from(
                policy
                    .server_name
                    .as_deref()
                    .context("DoT requires serverName")?,
            );
            NameServerConfig::tls(ip, server_name)
        }
        ResolverMode::Doh => {
            let server_name: Arc<str> = Arc::from(
                policy
                    .server_name
                    .as_deref()
                    .context("DoH requires serverName")?,
            );
            let path: Arc<str> = Arc::from(policy.doh_path.as_deref().unwrap_or("/dns-query"));
            NameServerConfig::https(ip, server_name, Some(path))
        }
        ResolverMode::System | ResolverMode::AppHosts => unreachable!(),
    };
    if let Some(port) = policy.upstream_port {
        for connection in &mut name_server.connections {
            connection.port = port;
        }
    }
    let config = ResolverConfig::from_parts(None, Vec::new(), vec![name_server]);
    let mut builder = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
    let opts: &mut ResolverOpts = builder.options_mut();
    opts.server_ordering_strategy = ServerOrderingStrategy::UserProvidedOrder;
    Ok(DnsBackend::Hickory(Box::new(
        builder.build().context("build configured DNS resolver")?,
    )))
}

fn validate_policy(policy: &ResolverPolicy) -> Result<()> {
    if policy.upstream_port == Some(0) {
        return Err(anyhow!("DNS 端口不能为 0"));
    }
    if let Some(proxy) = &policy.proxy_url {
        let url = url::Url::parse(proxy).context("代理地址格式不正确")?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            || url.host_str().is_none()
        {
            return Err(anyhow!(
                "代理地址需为 http://、https://、socks5:// 或 socks5h:// 地址"
            ));
        }
    }
    if policy.timeout_ms < 250 || policy.timeout_ms > 30_000 {
        return Err(anyhow!("timeoutMs must be between 250 and 30000"));
    }
    match policy.mode {
        ResolverMode::System | ResolverMode::AppHosts => {}
        ResolverMode::CustomDns => {
            policy
                .upstream_ip
                .context("custom DNS requires upstreamIp")?;
        }
        ResolverMode::Doh => {
            super::doh::endpoint(policy)?;
        }
        ResolverMode::Dot => {
            policy
                .upstream_ip
                .context("encrypted DNS requires upstreamIp")?;
            let name = policy
                .server_name
                .as_deref()
                .context("encrypted DNS requires serverName")?;
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

fn normalize_policy(mut policy: ResolverPolicy) -> Result<ResolverPolicy> {
    if policy.mode == ResolverMode::Doh {
        if let Some(ip) = policy.upstream_ip.take() {
            policy.bootstrap_ips.push(ip);
        }
    }
    policy.bootstrap_ips.sort_unstable();
    policy.bootstrap_ips.dedup();
    if policy.bootstrap_ips.len() > 16 {
        return Err(anyhow!("最多填写 16 个引导 IP"));
    }
    policy.app_hosts = policy
        .app_hosts
        .into_iter()
        .map(|(host, mut addresses)| {
            addresses.sort_unstable();
            addresses.dedup();
            Ok((normalize_host(&host)?, addresses))
        })
        .collect::<Result<_>>()?;
    for field in [
        &mut policy.doh_url,
        &mut policy.proxy_url,
        &mut policy.server_name,
    ] {
        *field = field
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    Ok(policy)
}

async fn lookup_with_timeout(
    resolver: &DnsBackend,
    host: &str,
    timeout_ms: u64,
) -> Result<Vec<IpAddr>> {
    let deadline = Duration::from_millis(timeout_ms);
    let mut addresses: Vec<IpAddr> = match resolver {
        // Each address family has its own full-query deadline. An outer deadline
        // would discard a valid A response whenever AAAA finishes at the limit.
        DnsBackend::Doh(resolver) => resolver.lookup(host).await?,
        DnsBackend::Hickory(resolver) => {
            let fqdn = format!("{}.", host.trim_end_matches('.'));
            timeout(deadline, resolver.lookup_ip(fqdn))
                .await
                .context("DNS lookup timed out")??
                .iter()
                .collect()
        }
        #[cfg(any(target_os = "android", test))]
        DnsBackend::Native => timeout(deadline, tokio::net::lookup_host((host, 0)))
            .await
            .context("DNS lookup timed out")??
            .map(|address| address.ip())
            .collect(),
    };
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
            (host != suffix && host.ends_with(&format!(".{suffix}")))
                .then_some((suffix.len(), addresses))
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

    #[tokio::test]
    async fn pasted_hosts_rules_are_normalized_before_matching() {
        let resolver =
            AppResolver::from_system(ResolverPolicy::default(), DnsBackend::Native).unwrap();
        let mut policy = ResolverPolicy::default();
        policy.app_hosts.insert(
            "  BOOKS.INVALID.  ".into(),
            vec!["192.0.2.8".parse().unwrap()],
        );
        resolver.set_policy(policy).await.unwrap();
        let result = resolver.resolve("books.invalid").await.unwrap();
        assert!(result.via_app_hosts);
        assert_eq!(
            result.addresses,
            vec!["192.0.2.8".parse::<IpAddr>().unwrap()]
        );
    }

    #[tokio::test]
    async fn native_system_backend_initializes_and_resolves_localhost() {
        let resolver = AppResolver::from_system(ResolverPolicy::default(), DnsBackend::Native)
            .expect("native system DNS initialization must not require an active network");
        let result = resolver.resolve("localhost").await.unwrap();
        assert!(!result.addresses.is_empty());
        assert!(result.addresses.iter().all(IpAddr::is_loopback));
        assert!(!result.used_fallback);
        assert_eq!(result.mode, ResolverMode::System);
    }

    #[tokio::test]
    async fn native_system_backend_preserves_offline_app_hosts() {
        let resolver =
            AppResolver::from_system(ResolverPolicy::default(), DnsBackend::Native).unwrap();
        resolver
            .set_host_override("offline.invalid", vec!["192.0.2.1".parse().unwrap()])
            .await
            .unwrap();
        let result = resolver.resolve("offline.invalid").await.unwrap();
        assert_eq!(
            result.addresses,
            vec!["192.0.2.1".parse::<IpAddr>().unwrap()]
        );
        assert!(result.via_app_hosts);
    }

    #[test]
    fn wildcard_host_prefers_longest_suffix() {
        let mut hosts = HashMap::new();
        hosts.insert(
            "*.example.org".to_string(),
            vec!["1.1.1.1".parse().unwrap()],
        );
        hosts.insert(
            "*.api.example.org".to_string(),
            vec!["2.2.2.2".parse().unwrap()],
        );
        assert_eq!(
            app_host_match(&hosts, "v1.api.example.org").unwrap()[0],
            "2.2.2.2".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn exact_host_wins() {
        let mut hosts = HashMap::new();
        hosts.insert(
            "api.example.org".to_string(),
            vec!["3.3.3.3".parse().unwrap()],
        );
        hosts.insert(
            "*.example.org".to_string(),
            vec!["1.1.1.1".parse().unwrap()],
        );
        assert_eq!(
            app_host_match(&hosts, "api.example.org").unwrap()[0],
            "3.3.3.3".parse::<IpAddr>().unwrap()
        );
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

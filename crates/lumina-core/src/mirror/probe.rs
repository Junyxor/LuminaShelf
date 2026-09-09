use super::{score_endpoint, MirrorRegistry, ProbeMetrics, ScoreWeights};
use crate::network::AppResolver;
use anyhow::{anyhow, Context, Result};
use futures::{stream, StreamExt};
use reqwest::{redirect::Policy, Client};
use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{net::TcpStream, time::timeout};
use tokio_rustls::{
    rustls::{
        crypto::ring,
        pki_types::ServerName,
        ClientConfig, RootCertStore,
    },
    TlsConnector,
};

#[derive(Debug, Clone)]
pub struct ProbeConfig {
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub concurrency: usize,
    pub sample_bytes: usize,
    pub happy_eyeballs_delay: Duration,
    pub weights: ScoreWeights,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(6),
            connect_timeout: Duration::from_secs(3),
            concurrency: 12,
            sample_bytes: 192 * 1024,
            happy_eyeballs_delay: Duration::from_millis(45),
            weights: ScoreWeights::default(),
        }
    }
}

#[derive(Clone)]
pub struct ProbeEngine {
    resolver: AppResolver,
    tls: TlsConnector,
    config: ProbeConfig,
}

impl ProbeEngine {
    pub fn new(config: ProbeConfig) -> Result<Self> {
        Self::with_resolver(config, AppResolver::system()?)
    }

    pub fn with_resolver(config: ProbeConfig, resolver: AppResolver) -> Result<Self> {
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let tls_config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .context("configure TLS protocol versions")?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self {
            resolver,
            tls: TlsConnector::from(Arc::new(tls_config)),
            config,
        })
    }

    pub fn resolver(&self) -> &AppResolver {
        &self.resolver
    }

    pub async fn probe_registry(&self, registry: &MirrorRegistry) {
        let endpoints = registry.snapshot();
        let concurrency = self.config.concurrency.max(1);
        stream::iter(endpoints)
            .for_each_concurrent(concurrency, |endpoint| {
                let this = self.clone();
                let registry = registry.clone();
                async move {
                    registry.update(&endpoint.id, |item| {
                        item.state = super::MirrorState::Probing;
                        item.last_error = None;
                    });
                    match this.probe_endpoint(&endpoint).await {
                        Ok(metrics) => {
                            let healthy = matches!(metrics.status_code, Some(200..=399));
                            let score = score_endpoint(
                                &metrics,
                                endpoint.success_streak + u32::from(healthy),
                                endpoint.failure_streak,
                                this.config.weights,
                            );
                            registry.update(&endpoint.id, |item| item.mark_probe(metrics, healthy, score, None));
                        }
                        Err(error) => {
                            registry.update(&endpoint.id, |item| {
                                item.mark_probe(ProbeMetrics::default(), false, 0.0, Some(error.to_string()))
                            });
                        }
                    }
                }
            })
            .await;
    }

    pub async fn probe_endpoint(&self, endpoint: &super::MirrorEndpoint) -> Result<ProbeMetrics> {
        self.probe_route(&endpoint.origin, &endpoint.address_hints).await
    }

    pub async fn probe_one(&self, origin: &url::Url) -> Result<ProbeMetrics> {
        self.probe_route(origin, &[]).await
    }

    async fn probe_route(&self, origin: &url::Url, address_hints: &[std::net::IpAddr]) -> Result<ProbeMetrics> {
        let total_started = Instant::now();
        let host = origin.host_str().context("mirror URL has no host")?;
        let port = origin.port_or_known_default().ok_or_else(|| anyhow!("mirror URL has no known port"))?;

        let resolved = self.resolver.resolve(host).await;
        let mut addresses = address_hints.to_vec();
        let (dns_ms, via_app_hosts, resolver_fallback) = match resolved {
            Ok(result) => {
                for ip in result.addresses {
                    if !addresses.contains(&ip) { addresses.push(ip); }
                }
                (Some(result.elapsed_ms), result.via_app_hosts, result.used_fallback)
            }
            Err(error) if address_hints.is_empty() => return Err(error.context("DNS resolution failed")),
            Err(_) => (None, false, false),
        };
        if addresses.is_empty() {
            return Err(anyhow!("no addresses available for route"));
        }

        let tcp_started = Instant::now();
        let (tcp, selected_addr) = self.connect_race(&addresses, port).await.context("TCP connection failed")?;
        let tcp_ms = tcp_started.elapsed().as_secs_f64() * 1000.0;

        let tls_ms = if origin.scheme() == "https" {
            let tls_started = Instant::now();
            let server_name = ServerName::try_from(host.to_owned()).context("invalid TLS server name")?;
            timeout(self.config.connect_timeout, self.tls.connect(server_name, tcp))
                .await
                .context("TLS handshake timed out")??;
            Some(tls_started.elapsed().as_secs_f64() * 1000.0)
        } else {
            drop(tcp);
            None
        };

        let socket_overrides = [SocketAddr::new(selected_addr.ip(), 0)];
        let client = Client::builder()
            .timeout(self.config.timeout)
            .connect_timeout(self.config.connect_timeout)
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2)
            .tcp_nodelay(true)
            .redirect(Policy::none())
            .user_agent("LuminaShelf/0.2")
            .resolve_to_addrs(host, &socket_overrides)
            .build()?;

        let http_started = Instant::now();
        let response = client.get(origin.clone()).send().await?;
        let ttfb_ms = http_started.elapsed().as_secs_f64() * 1000.0;
        let status = response.status().as_u16();
        let redirect_count = u8::from(response.status().is_redirection());

        let body_started = Instant::now();
        let mut bytes = 0usize;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            bytes = bytes.saturating_add(chunk?.len());
            if bytes >= self.config.sample_bytes {
                break;
            }
        }
        let body_elapsed = body_started.elapsed().as_secs_f64().max(0.001);
        let throughput_mbps = (bytes as f64 * 8.0) / body_elapsed / 1_000_000.0;

        Ok(ProbeMetrics {
            dns_ms,
            tcp_ms: Some(tcp_ms),
            tls_ms,
            ttfb_ms: Some(ttfb_ms),
            total_ms: Some(total_started.elapsed().as_secs_f64() * 1000.0),
            throughput_mbps: Some(throughput_mbps),
            status_code: Some(status),
            redirect_count,
            resolved_addresses: addresses,
            selected_address: Some(selected_addr.ip()),
            via_app_hosts,
            resolver_fallback,
            via_address_hint: address_hints.contains(&selected_addr.ip()),
        })
    }

    async fn connect_race(&self, addresses: &[std::net::IpAddr], port: u16) -> Result<(TcpStream, SocketAddr)> {
        let mut attempts = futures::stream::FuturesUnordered::new();
        let ordered = interleave_ip_families(addresses);
        for (index, ip) in ordered.into_iter().enumerate() {
            let delay = self.config.happy_eyeballs_delay * index as u32;
            let per_attempt_timeout = self.config.connect_timeout;
            attempts.push(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let addr = SocketAddr::new(ip, port);
                let stream = timeout(per_attempt_timeout, TcpStream::connect(addr))
                    .await
                    .map_err(|_| anyhow!("connect timeout: {addr}"))??;
                stream.set_nodelay(true)?;
                Ok::<_, anyhow::Error>((stream, addr))
            });
        }

        let mut last_error = None;
        while let Some(result) = attempts.next().await {
            match result {
                Ok(connection) => return Ok(connection),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("no IP addresses could be connected")))
    }
}

fn interleave_ip_families(addresses: &[std::net::IpAddr]) -> Vec<std::net::IpAddr> {
    let mut v4 = addresses.iter().copied().filter(std::net::IpAddr::is_ipv4);
    let mut v6 = addresses.iter().copied().filter(std::net::IpAddr::is_ipv6);
    let prefer_v6 = addresses.first().is_some_and(std::net::IpAddr::is_ipv6);
    let mut out = Vec::with_capacity(addresses.len());
    loop {
        let pair = if prefer_v6 { (v6.next(), v4.next()) } else { (v4.next(), v6.next()) };
        if pair.0.is_none() && pair.1.is_none() {
            break;
        }
        if let Some(ip) = pair.0 { out.push(ip); }
        if let Some(ip) = pair.1 { out.push(ip); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn interleaves_address_families() {
        let items = vec![
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        ];
        let ordered = interleave_ip_families(&items);
        assert!(ordered[0].is_ipv6());
        assert!(ordered[1].is_ipv4());
        assert!(ordered[2].is_ipv6());
        assert!(ordered[3].is_ipv4());
    }
}

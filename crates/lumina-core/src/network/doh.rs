use anyhow::{anyhow, ensure, Context, Result};
use hickory_resolver::proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use reqwest::{header, Client};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

use super::ResolverPolicy;

#[derive(Clone)]
pub(super) struct DohResolver {
    client: Client,
    endpoint: Url,
    query_timeout: Duration,
    cache: Arc<Mutex<DohCache>>,
}

#[derive(Default)]
struct DohCache {
    entries: HashMap<String, (Instant, Vec<IpAddr>)>,
    flights: HashMap<String, Weak<Mutex<()>>>,
    epoch: u64,
}
struct Answer {
    ips: Vec<IpAddr>,
    ttl: u32,
}
fn has_local_route(addresses: &[IpAddr]) -> bool {
    addresses.iter().any(|ip| {
        let local = if ip.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
        // UDP connect performs a local route lookup without sending any packet.
        // A successful DNS answer is unusable if that address family has no route.
        std::net::UdpSocket::bind(local)
            .and_then(|socket| socket.connect(SocketAddr::new(*ip, 443)))
            .is_ok()
    })
}

pub(super) fn endpoint(policy: &ResolverPolicy) -> Result<Url> {
    let value = if let Some(value) = &policy.doh_url {
        value
            .trim()
            .trim_end_matches("{?dns}")
            .trim_end_matches("{&dns}")
            .to_string()
    } else {
        let host = policy
            .server_name
            .as_deref()
            .context("请填写完整的 DoH HTTPS 地址")?;
        let port = policy.upstream_port.unwrap_or(443);
        format!(
            "https://{host}:{port}{}",
            policy.doh_path.as_deref().unwrap_or("/dns-query")
        )
    };
    ensure!(!value.contains(['{', '}']), "DoH 地址包含不支持的模板变量");
    let url = Url::parse(&value).context("DoH 地址格式不正确")?;
    ensure!(
        url.scheme() == "https" && url.host_str().is_some(),
        "DoH 地址必须使用 HTTPS 并包含域名"
    );
    ensure!(url.fragment().is_none(), "DoH 地址不能包含 # 片段");
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "DoH 地址不支持用户名或密码，请使用服务商提供的完整查询地址"
    );
    ensure!(url.port() != Some(0), "DoH 端口不能为 0");
    Ok(url)
}

impl DohResolver {
    pub(super) fn new(policy: &ResolverPolicy) -> Result<Self> {
        let endpoint = endpoint(policy)?;
        // Intentionally use Reqwest's OS resolver here. Resolving the DoH service
        // through this same DoH service would recurse before the first query.
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_millis(policy.timeout_ms))
            .timeout(Duration::from_millis(policy.timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("LuminaShelf/{} DoH", env!("CARGO_PKG_VERSION")));
        let mut bootstrap: Vec<_> = policy
            .bootstrap_ips
            .iter()
            .copied()
            .map(|ip| SocketAddr::new(ip, 0))
            .collect();
        if let Some(ip) = policy.upstream_ip {
            bootstrap.push(SocketAddr::new(ip, 0));
        }
        if !bootstrap.is_empty() {
            builder = builder.resolve_to_addrs(endpoint.host_str().unwrap(), &bootstrap);
        }
        if let Some(proxy) = &policy.proxy_url {
            builder = builder.proxy(reqwest::Proxy::all(proxy).context("代理地址无效")?);
        }
        Ok(Self {
            client: builder.build()?,
            endpoint,
            query_timeout: Duration::from_millis(policy.timeout_ms),
            cache: Arc::new(Mutex::new(DohCache::default())),
        })
    }

    pub(super) async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>> {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let gate = {
            let mut cache = self.cache.lock().await;
            cache
                .entries
                .retain(|_, (expires, _)| *expires > Instant::now());
            if let Some((_, ips)) = cache.entries.get(&host) {
                return Ok(ips.clone());
            }
            cache.flights.retain(|_, gate| gate.strong_count() > 0);
            if let Some(gate) = cache.flights.get(&host).and_then(Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(Mutex::new(()));
                cache.flights.insert(host.clone(), Arc::downgrade(&gate));
                gate
            }
        };
        let _flight = gate.lock().await;
        if let Some((expires, ips)) = self.cache.lock().await.entries.get(&host) {
            if *expires > Instant::now() {
                return Ok(ips.clone());
            }
        }
        let epoch = self.cache.lock().await.epoch;
        let query = |kind| {
            let resolver = self.clone();
            let host = host.clone();
            tokio::spawn(async move { resolver.timed_query(&host, kind).await })
        };
        let mut a = query(RecordType::A);
        let mut aaaa = query(RecordType::AAAA);
        // Give the other family a short grace period once we already have usable
        // addresses. An unresponsive IPv6 resolver must not stall every HTTP request.
        let (first, mut pending) = tokio::select! {
            first = &mut a => {
                (first.context("DoH 查询任务中断")?, aaaa)
            },
            first = &mut aaaa => {
                (first.context("DoH 查询任务中断")?, a)
            }
        };
        let second = if first
            .as_ref()
            .is_ok_and(|answer| !answer.ips.is_empty() && has_local_route(&answer.ips))
        {
            tokio::time::timeout(Duration::from_millis(150), &mut pending)
                .await
                .ok()
        } else {
            Some((&mut pending).await)
        };
        let incomplete = second.is_none();
        let second =
            second.map(|result| result.context("DoH 查询任务中断").and_then(|answer| answer));
        let mut addresses = Vec::new();
        let mut errors = Vec::new();
        let mut ttl = 3600;
        for result in std::iter::once(first).chain(second) {
            match result {
                Ok(mut answer) => {
                    if !answer.ips.is_empty() {
                        ttl = ttl.min(answer.ttl);
                        addresses.append(&mut answer.ips);
                    }
                }
                Err(error) => errors.push(format!("{error:#}")),
            }
        }
        if addresses.is_empty() && errors.is_empty() {
            return Err(anyhow!("DoH 未返回此域名的 IP 地址"));
        }
        ensure!(!addresses.is_empty(), "DoH 解析失败：{}", errors.join("；"));
        addresses.sort_unstable();
        addresses.dedup();
        self.cache_answer(
            &host,
            Answer {
                ips: addresses.clone(),
                ttl,
            },
            epoch,
        )
        .await;
        if incomplete {
            let resolver = self.clone();
            tokio::spawn(async move {
                // Keep the slower query alive and merge its addresses. The first
                // usable family is fast without excluding another mobile route for a full TTL.
                if let Ok(Ok(answer)) = pending.await {
                    resolver.cache_answer(&host, answer, epoch).await;
                }
            });
        }
        Ok(addresses)
    }

    pub(super) async fn clear_cache(&self) {
        let mut cache = self.cache.lock().await;
        cache.entries.clear();
        cache.epoch += 1;
    }

    async fn cache_answer(&self, host: &str, mut answer: Answer, epoch: u64) {
        if answer.ttl == 0 || answer.ips.is_empty() {
            return;
        }
        let mut cache = self.cache.lock().await;
        if cache.epoch != epoch {
            return;
        }
        let mut expires = Instant::now() + Duration::from_secs(answer.ttl.into());
        if let Some((old_expires, ips)) = cache.entries.get(host) {
            if *old_expires > Instant::now() {
                expires = expires.min(*old_expires);
                answer.ips.extend(ips);
            }
        }
        answer.ips.sort_unstable();
        answer.ips.dedup();
        if cache.entries.len() >= 256 {
            if let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, (expires, _))| *expires)
                .map(|(host, _)| host.clone())
            {
                cache.entries.remove(&oldest);
            }
        }
        cache.entries.insert(host.to_owned(), (expires, answer.ips));
    }

    async fn timed_query(&self, host: &str, kind: RecordType) -> Result<Answer> {
        tokio::time::timeout(self.query_timeout, self.query(host, kind))
            .await
            .context("DoH 查询超时")?
    }

    async fn query(&self, host: &str, kind: RecordType) -> Result<Answer> {
        let mut name = Name::from_ascii(format!("{}.", host.trim_end_matches('.')))
            .context("域名格式不正确")?;
        let mut seen = HashSet::new();
        let mut ttl = 3600;
        for _ in 0..8 {
            ensure!(seen.insert(name.clone()), "DoH 返回了循环别名");
            let mut request = Message::new(0, MessageType::Query, OpCode::Query);
            request.metadata.recursion_desired = true;
            request.queries.push(Query::query(name.clone(), kind));
            let mut response = self
                .client
                .post(self.endpoint.clone())
                .header(header::ACCEPT, "application/dns-message")
                .header(header::CONTENT_TYPE, "application/dns-message")
                .body(request.to_vec()?)
                .send()
                .await
                .map_err(reqwest::Error::without_url)
                .context("无法连接 DoH 服务")?
                .error_for_status()
                .map_err(reqwest::Error::without_url)
                .context("DoH 服务返回 HTTP 错误")?;
            let age = response
                .headers()
                .get(header::AGE)
                .and_then(|v| v.to_str().ok()?.parse::<u32>().ok())
                .unwrap_or(0);
            let no_store = response
                .headers()
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| {
                    v.split(',')
                        .any(|part| part.trim().eq_ignore_ascii_case("no-store"))
                });
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(reqwest::Error::without_url)?
            {
                ensure!(
                    bytes.len() + chunk.len() <= 65_535,
                    "DoH 响应超过 DNS 消息大小限制"
                );
                bytes.extend_from_slice(&chunk);
            }
            let reply = Message::from_vec(&bytes).context("DoH 返回的不是有效 DNS 二进制消息")?;
            ensure!(
                reply.metadata.message_type == MessageType::Response
                    && reply.metadata.id == 0
                    && reply.metadata.op_code == OpCode::Query
                    && reply.queries == request.queries,
                "DoH 响应与查询不匹配"
            );
            ensure!(!reply.metadata.truncation, "DoH 返回了截断的 DNS 消息");
            ensure!(
                reply.metadata.response_code == ResponseCode::NoError,
                "DNS 服务返回 {}",
                reply.metadata.response_code
            );
            let mut owners = HashSet::from([name.clone()]);
            for _ in 0..8 {
                let aliases: Vec<_> = reply
                    .answers
                    .iter()
                    .filter_map(|record| {
                        if !owners.contains(&record.name) {
                            return None;
                        }
                        if let RData::CNAME(alias) = &record.data {
                            Some(alias.0.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                let mut changed = false;
                for alias in aliases {
                    changed |= owners.insert(alias);
                }
                if !changed {
                    break;
                }
            }
            let ips: Vec<_> = reply
                .answers
                .iter()
                .filter(|record| owners.contains(&record.name))
                .filter_map(|record| match &record.data {
                    RData::A(value) if kind == RecordType::A => Some(IpAddr::V4(value.0)),
                    RData::AAAA(value) if kind == RecordType::AAAA => Some(IpAddr::V6(value.0)),
                    _ => None,
                })
                .collect();
            for record in reply
                .answers
                .iter()
                .filter(|record| owners.contains(&record.name))
            {
                if record.record_type() == kind || matches!(record.data, RData::CNAME(_)) {
                    ttl = ttl.min(if no_store {
                        0
                    } else {
                        record.ttl.saturating_sub(age)
                    });
                }
            }
            if !ips.is_empty() {
                return Ok(Answer { ips, ttl });
            }
            match reply.answers.iter().find_map(|record| {
                if record.name == name {
                    if let RData::CNAME(alias) = &record.data {
                        return Some(alias.0.clone());
                    }
                }
                None
            }) {
                Some(alias) => name = alias,
                None => {
                    return Ok(Answer {
                        ips: Vec::new(),
                        ttl: 0,
                    })
                }
            }
        }
        Err(anyhow!("DoH 别名链超过 8 次"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::proto::rr::{rdata::A, Record};
    use std::sync::Arc;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    use tokio_rustls::{
        rustls::{
            crypto::ring,
            pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
            ServerConfig,
        },
        TlsAcceptor,
    };

    async fn service(
        code: ResponseCode,
        stall_aaaa: bool,
    ) -> (Url, reqwest::Certificate, tokio::task::JoinHandle<()>) {
        service_with_ttl(code, stall_aaaa, 60, 2).await
    }
    async fn service_with_ttl(
        code: ResponseCode,
        stall_aaaa: bool,
        ttl: u32,
        requests: usize,
    ) -> (Url, reqwest::Certificate, tokio::task::JoinHandle<()>) {
        service_delayed(code, stall_aaaa, ttl, requests, None).await
    }
    async fn service_delayed(
        code: ResponseCode,
        stall_aaaa: bool,
        ttl: u32,
        requests: usize,
        delayed: Option<RecordType>,
    ) -> (Url, reqwest::Certificate, tokio::task::JoinHandle<()>) {
        let certificate =
            rcgen::generate_simple_self_signed(vec!["dns.fixture.test".into()]).unwrap();
        let trust = reqwest::Certificate::from_pem(certificate.cert.pem().as_bytes()).unwrap();
        let mut config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate.cert.der().to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                    certificate.signing_key.serialize_der(),
                )),
            )
            .unwrap();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = Url::parse(&format!(
            "https://dns.fixture.test:{port}/custom/resolve?profile=unit"
        ))
        .unwrap();
        let task = tokio::spawn(async move {
            let mut stalled = Vec::new();
            let mut replies = Vec::new();
            for _ in 0..requests {
                let (tcp, _) = listener.accept().await.unwrap();
                let Ok(mut stream) = acceptor.accept(tcp).await else {
                    continue;
                };
                let mut bytes = Vec::new();
                let mut buffer = [0_u8; 4096];
                let body_at;
                let length;
                loop {
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..index]).to_ascii_lowercase();
                        assert!(header.starts_with("post /custom/resolve?profile=unit http/1.1"));
                        assert!(header.contains(&format!("host: dns.fixture.test:{port}")));
                        assert!(header.contains("content-type: application/dns-message"));
                        length = header
                            .lines()
                            .find_map(|line| {
                                line.strip_prefix("content-length: ")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap();
                        body_at = index + 4;
                        break;
                    }
                }
                while bytes.len() < body_at + length {
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let request = Message::from_vec(&bytes[body_at..body_at + length]).unwrap();
                assert_eq!(request.metadata.id, 0);
                if stall_aaaa && request.queries[0].query_type == RecordType::AAAA {
                    stalled.push(stream);
                    continue;
                }
                let mut response = Message::response(0, OpCode::Query);
                response.queries = request.queries.clone();
                response.metadata.response_code = code;
                if code == ResponseCode::NoError && request.queries[0].query_type == RecordType::A {
                    response.answers.push(Record::from_rdata(
                        request.queries[0].name.clone(),
                        ttl,
                        RData::A(A("203.0.113.20".parse().unwrap())),
                    ));
                }
                if delayed.is_some() && request.queries[0].query_type == RecordType::AAAA {
                    response.answers.push(Record::from_rdata(
                        request.queries[0].name.clone(),
                        ttl,
                        RData::AAAA(hickory_resolver::proto::rr::rdata::AAAA(
                            "2001:db8::20".parse().unwrap(),
                        )),
                    ));
                }
                let payload = response.to_vec().unwrap();
                let is_delayed = delayed == Some(request.queries[0].query_type);
                replies.push(tokio::spawn(async move {
                if is_delayed { tokio::time::sleep(Duration::from_millis(400)).await; }
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).as_bytes()).await.unwrap();
                stream.write_all(&payload).await.unwrap();
                let _ = stream.shutdown().await;
                }));
            }
            for reply in replies {
                reply.await.unwrap();
            }
            if !stalled.is_empty() {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        (url, trust, task)
    }

    fn fixture_client(url: Url, trust: reqwest::Certificate) -> DohResolver {
        let client = Client::builder()
            .add_root_certificate(trust)
            .resolve_to_addrs("dns.fixture.test", &["127.0.0.1:0".parse().unwrap()])
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        DohResolver {
            client,
            endpoint: url,
            query_timeout: Duration::from_secs(3),
            cache: Arc::new(Mutex::new(DohCache::default())),
        }
    }

    #[tokio::test]
    async fn custom_https_port_query_and_dns_wire_reach_real_server() {
        let (url, trust, server) = service(ResponseCode::NoError, false).await;
        let addresses = fixture_client(url, trust)
            .lookup("books.fixture.test")
            .await
            .unwrap();
        assert_eq!(addresses, vec!["203.0.113.20".parse::<IpAddr>().unwrap()]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_200_with_dns_nxdomain_is_an_error() {
        let (url, trust, server) = service(ResponseCode::NXDomain, false).await;
        assert!(fixture_client(url, trust)
            .lookup("missing.fixture.test")
            .await
            .is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn untrusted_https_certificate_is_rejected() {
        let (url, _, server) = service(ResponseCode::NoError, false).await;
        let policy = ResolverPolicy {
            mode: super::super::resolver::ResolverMode::Doh,
            doh_url: Some(url.to_string()),
            upstream_ip: Some("127.0.0.1".parse().unwrap()),
            timeout_ms: 3000,
            ..Default::default()
        };
        assert!(DohResolver::new(&policy)
            .unwrap()
            .lookup("books.fixture.test")
            .await
            .is_err());
        server.abort();
    }
    #[tokio::test]
    async fn unanswered_aaaa_does_not_discard_successful_a_response() {
        let (url, trust, server) = service(ResponseCode::NoError, true).await;
        let mut client = fixture_client(url, trust);
        client.query_timeout = Duration::from_secs(3);
        let result = tokio::time::timeout(
            Duration::from_millis(450),
            client.lookup("books.fixture.test"),
        )
        .await;
        server.abort();
        assert_eq!(
            result
                .expect("A response lost while waiting for AAAA")
                .unwrap(),
            vec!["203.0.113.20".parse::<IpAddr>().unwrap()]
        );
    }
    #[tokio::test]
    async fn cache_survives_doh_service_disconnection_and_coalesces_eight_callers() {
        let (url, trust, server) = service(ResponseCode::NoError, false).await;
        let client = Arc::new(fixture_client(url, trust));
        let mut requests = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            requests.push(tokio::spawn(async move {
                client.lookup("books.fixture.test").await.unwrap()
            }));
        }
        for request in requests {
            assert_eq!(
                request.await.unwrap(),
                vec!["203.0.113.20".parse::<IpAddr>().unwrap()]
            );
        }
        server.await.unwrap();
        assert!(
            client.lookup("BOOKS.fixture.test.").await.is_ok(),
            "TTL cache must avoid another wire request after service disconnects"
        );
        client.clear_cache().await;
        assert!(
            client.lookup("books.fixture.test").await.is_err(),
            "Applying settings must allow a fresh diagnostic query"
        );
    }
    #[tokio::test]
    async fn zero_ttl_and_expired_answers_are_not_reused() {
        let (url, trust, server) = service_with_ttl(ResponseCode::NoError, false, 0, 4).await;
        let client = fixture_client(url, trust);
        client.lookup("books.fixture.test").await.unwrap();
        client.lookup("books.fixture.test").await.unwrap();
        server.await.unwrap();
        assert!(client.lookup("books.fixture.test").await.is_err());
        let (url, trust, server) = service_with_ttl(ResponseCode::NoError, false, 1, 2).await;
        let client = fixture_client(url, trust);
        client.lookup("books.fixture.test").await.unwrap();
        server.await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(
            client.lookup("books.fixture.test").await.is_err(),
            "Expired TTL must trigger an actual new query"
        );
    }
    #[tokio::test]
    async fn slower_address_family_is_merged_without_another_wire_lookup() {
        for delayed in [RecordType::A, RecordType::AAAA] {
            let (url, trust, server) =
                service_delayed(ResponseCode::NoError, false, 60, 2, Some(delayed)).await;
            let resolver = fixture_client(url, trust);
            let first = tokio::time::timeout(
                Duration::from_millis(650),
                resolver.lookup("books.fixture.test"),
            )
            .await
            .unwrap()
            .unwrap();
            assert!(!first.is_empty());
            server.await.unwrap();
            let both = tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    let ips = resolver.lookup("books.fixture.test").await.unwrap();
                    if ips.len() == 2 {
                        break ips;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(both.contains(&"203.0.113.20".parse::<IpAddr>().unwrap()));
            assert!(both.contains(&"2001:db8::20".parse::<IpAddr>().unwrap()));
        }
    }
    #[test]
    fn complete_endpoint_preserves_custom_port_path_and_query() {
        let policy = ResolverPolicy {
            doh_url: Some("https://dns.example.org:8443/private/resolve?key=test".into()),
            ..Default::default()
        };
        let url = endpoint(&policy).unwrap();
        assert_eq!(url.port(), Some(8443));
        assert_eq!(url.path(), "/private/resolve");
        assert_eq!(url.query(), Some("key=test"));
        assert!(
            DohResolver::new(&policy).is_ok(),
            "constructing a custom endpoint must not perform network I/O"
        );
    }
    #[test]
    fn insecure_endpoint_is_rejected() {
        let policy = ResolverPolicy {
            doh_url: Some("http://dns.example.org/query".into()),
            ..Default::default()
        };
        assert!(endpoint(&policy).is_err());
    }
}

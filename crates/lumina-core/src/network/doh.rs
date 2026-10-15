use anyhow::{anyhow, ensure, Context, Result};
use hickory_resolver::proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RData, RecordType},
};
use reqwest::{header, Client};
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::Url;

use super::ResolverPolicy;

pub(super) struct DohResolver {
    client: Client,
    endpoint: Url,
    query_timeout: Duration,
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
        })
    }

    pub(super) async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>> {
        let (v4, v6) = tokio::join!(
            self.timed_query(host, RecordType::A),
            self.timed_query(host, RecordType::AAAA)
        );
        let mut addresses = Vec::new();
        let mut errors = Vec::new();
        for result in [v4, v6] {
            match result {
                Ok(mut ips) => addresses.append(&mut ips),
                Err(error) => errors.push(format!("{error:#}")),
            }
        }
        if addresses.is_empty() && errors.is_empty() {
            return Err(anyhow!("DoH 未返回此域名的 IP 地址"));
        }
        ensure!(!addresses.is_empty(), "DoH 解析失败：{}", errors.join("；"));
        addresses.sort_unstable();
        addresses.dedup();
        Ok(addresses)
    }

    async fn timed_query(&self, host: &str, kind: RecordType) -> Result<Vec<IpAddr>> {
        tokio::time::timeout(self.query_timeout, self.query(host, kind))
            .await
            .context("DoH 查询超时")?
    }

    async fn query(&self, host: &str, kind: RecordType) -> Result<Vec<IpAddr>> {
        let mut name = Name::from_ascii(format!("{}.", host.trim_end_matches('.')))
            .context("域名格式不正确")?;
        let mut seen = HashSet::new();
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
            if !ips.is_empty() {
                return Ok(ips);
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
                None => return Ok(Vec::new()),
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
            for _ in 0..2 {
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
                        60,
                        RData::A(A("203.0.113.20".parse().unwrap())),
                    ));
                }
                let payload = response.to_vec().unwrap();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/dns-message\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).as_bytes()).await.unwrap();
                stream.write_all(&payload).await.unwrap();
                let _ = stream.shutdown().await;
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
        client.query_timeout = Duration::from_millis(250);
        let result = tokio::time::timeout(
            Duration::from_millis(500),
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

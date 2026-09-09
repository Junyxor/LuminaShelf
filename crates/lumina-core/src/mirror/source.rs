use super::{MirrorEndpoint, MirrorSourceKind};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use std::{collections::HashMap, net::IpAddr};
use url::Url;

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_DISCOVERED_ENDPOINTS: usize = 4096;

#[async_trait]
pub trait EndpointSource: Send + Sync {
    async fn discover(&self) -> Result<Vec<MirrorEndpoint>>;
}

pub struct StaticEndpointSource {
    source: MirrorSourceKind,
    entries: Vec<(String, Url)>,
}

impl StaticEndpointSource {
    pub fn new(source: MirrorSourceKind, entries: Vec<(String, Url)>) -> Self {
        Self { source, entries }
    }
}

#[async_trait]
impl EndpointSource for StaticEndpointSource {
    async fn discover(&self) -> Result<Vec<MirrorEndpoint>> {
        Ok(self
            .entries
            .iter()
            .map(|(label, origin)| MirrorEndpoint::new(label.clone(), origin.clone(), self.source))
            .collect())
    }
}

pub struct RemoteEndpointSource {
    client: Client,
    url: Url,
    kind: MirrorSourceKind,
    allow_insecure_http: bool,
}

impl RemoteEndpointSource {
    pub fn new(client: Client, url: Url, kind: MirrorSourceKind) -> Self {
        Self {
            client,
            url,
            kind,
            allow_insecure_http: false,
        }
    }

    pub fn allow_insecure_http(mut self, allow: bool) -> Self {
        self.allow_insecure_http = allow;
        self
    }
}

#[async_trait]
impl EndpointSource for RemoteEndpointSource {
    async fn discover(&self) -> Result<Vec<MirrorEndpoint>> {
        let response = self
            .client
            .get(self.url.clone())
            .send()
            .await?
            .error_for_status()?;
        if let Some(size) = response.content_length() {
            if size > MAX_SOURCE_BYTES as u64 {
                return Err(anyhow!("endpoint source is too large: {size} bytes"));
            }
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or(16 * 1024)
                .min(MAX_SOURCE_BYTES as u64) as usize,
        );
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_SOURCE_BYTES {
                return Err(anyhow!(
                    "endpoint source exceeded {} bytes",
                    MAX_SOURCE_BYTES
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let text = std::str::from_utf8(&bytes).context("endpoint source is not UTF-8")?;
        parse_endpoint_list(text, self.kind, self.allow_insecure_http)
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum JsonEntry {
    Url(String),
    Object {
        label: Option<String>,
        url: String,
        #[serde(default)]
        ip: Option<String>,
        #[serde(default)]
        ips: Vec<String>,
    },
}

pub fn parse_endpoint_list(
    text: &str,
    kind: MirrorSourceKind,
    allow_insecure_http: bool,
) -> Result<Vec<MirrorEndpoint>> {
    let trimmed = text.trim();
    let mut result: Vec<MirrorEndpoint> = Vec::new();
    let mut by_origin: HashMap<String, usize> = HashMap::new();
    let mut invalid_candidates = 0usize;

    if trimmed.starts_with('[') {
        for entry in serde_json::from_str::<Vec<JsonEntry>>(trimmed)? {
            let parsed = match entry {
                JsonEntry::Url(raw) => push_candidate(
                    &mut result,
                    &mut by_origin,
                    None,
                    &raw,
                    Vec::new(),
                    kind,
                    allow_insecure_http,
                ),
                JsonEntry::Object {
                    label,
                    url,
                    ip,
                    ips,
                } => {
                    let mut hints = Vec::new();
                    if let Some(ip) = ip {
                        if let Ok(parsed) = ip.parse::<IpAddr>() {
                            hints.push(parsed);
                        }
                    }
                    hints.extend(
                        ips.into_iter()
                            .filter_map(|value| value.parse::<IpAddr>().ok()),
                    );
                    push_candidate(
                        &mut result,
                        &mut by_origin,
                        label,
                        &url,
                        hints,
                        kind,
                        allow_insecure_http,
                    )
                }
            };
            if parsed.is_err() {
                invalid_candidates = invalid_candidates.saturating_add(1);
            }
            if result.len() >= MAX_DISCOVERED_ENDPOINTS {
                break;
            }
        }
    } else {
        for line in trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let without_comment = line.split('#').next().unwrap_or("").trim();
            if without_comment.is_empty() {
                continue;
            }
            let fields: Vec<&str> = without_comment.split_whitespace().collect();
            if fields.len() >= 2 {
                if let Ok(ip) = fields[0].parse::<IpAddr>() {
                    if !is_safe_remote_hint(ip) {
                        continue;
                    }
                    for host in &fields[1..] {
                        if host.eq_ignore_ascii_case("localhost") || host.contains('/') {
                            continue;
                        }
                        let raw = format!("https://{host}");
                        if push_candidate(
                            &mut result,
                            &mut by_origin,
                            None,
                            &raw,
                            vec![ip],
                            kind,
                            allow_insecure_http,
                        )
                        .is_err()
                        {
                            invalid_candidates = invalid_candidates.saturating_add(1);
                        }
                        if result.len() >= MAX_DISCOVERED_ENDPOINTS {
                            break;
                        }
                    }
                    continue;
                }
            }
            if push_candidate(
                &mut result,
                &mut by_origin,
                None,
                without_comment,
                Vec::new(),
                kind,
                allow_insecure_http,
            )
            .is_err()
            {
                invalid_candidates = invalid_candidates.saturating_add(1);
            }
            if result.len() >= MAX_DISCOVERED_ENDPOINTS {
                break;
            }
        }
    }

    if result.len() > MAX_DISCOVERED_ENDPOINTS {
        result.truncate(MAX_DISCOVERED_ENDPOINTS);
    }
    if result.is_empty() && invalid_candidates > 0 {
        return Err(anyhow!("endpoint source contained no valid candidates"));
    }
    Ok(result)
}

fn push_candidate(
    result: &mut Vec<MirrorEndpoint>,
    by_origin: &mut HashMap<String, usize>,
    label: Option<String>,
    raw: &str,
    hints: Vec<IpAddr>,
    kind: MirrorSourceKind,
    allow_insecure_http: bool,
) -> Result<()> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(());
    }
    let normalized = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let mut url =
        Url::parse(&normalized).with_context(|| format!("invalid endpoint URL/domain: {raw}"))?;
    if url.scheme() != "https" && !(allow_insecure_http && url.scheme() == "http") {
        return Ok(());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Ok(());
    }
    let Some(host) = url.host_str().map(str::to_owned) else {
        return Ok(());
    };
    if host
        .parse::<IpAddr>()
        .is_ok_and(|ip| !is_safe_remote_hint(ip))
    {
        return Ok(());
    }
    let hints: Vec<IpAddr> = hints
        .into_iter()
        .filter(|ip| is_safe_remote_hint(*ip))
        .collect();
    url.set_query(None);
    url.set_fragment(None);
    let canonical = url.as_str().trim_end_matches('/').to_owned();
    if let Some(index) = by_origin.get(&canonical).copied() {
        for ip in hints {
            if !result[index].address_hints.contains(&ip) {
                result[index].address_hints.push(ip);
            }
        }
        return Ok(());
    }
    let mut endpoint = MirrorEndpoint::new(label.unwrap_or(host), url, kind);
    for ip in hints {
        if !endpoint.address_hints.contains(&ip) {
            endpoint.address_hints.push(ip);
        }
    }
    by_origin.insert(canonical, result.len());
    result.push(endpoint);
    Ok(())
}

fn is_safe_remote_hint(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, d] = ip.octets();
            !(ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
                || [a, b, c, d] == [255, 255, 255, 255]
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let first = segments[0];
            !(ip.is_unspecified()
                || ip.is_loopback()
                || (first & 0xff00) == 0xff00
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_deduplicates() {
        let data = r#"["https://example.com", {"label":"A","url":"https://example.com/","ip":"1.1.1.1"}, "http://insecure.test"]"#;
        let endpoints = parse_endpoint_list(data, MirrorSourceKind::Github, false).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].origin.host_str(), Some("example.com"));
        assert_eq!(
            endpoints[0].address_hints,
            vec!["1.1.1.1".parse::<IpAddr>().unwrap()]
        );
    }

    #[test]
    fn parses_text_urls_domains_and_hosts() {
        let data = "# comment\nhttps://a.example\nb.example\n1.0.0.1 c.example c-alt.example\n";
        let endpoints = parse_endpoint_list(data, MirrorSourceKind::Subscription, false).unwrap();
        assert_eq!(endpoints.len(), 4);
        let c = endpoints
            .iter()
            .find(|item| item.origin.host_str() == Some("c.example"))
            .unwrap();
        assert_eq!(c.address_hints[0], "1.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn ignores_reserved_remote_host_hints_and_bad_lines() {
        let data = "127.0.0.1 local.example\nnot a valid url %%%\nhttps://safe.example\n";
        let endpoints = parse_endpoint_list(data, MirrorSourceKind::Github, false).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].origin.host_str(), Some("safe.example"));
    }
}

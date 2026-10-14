use super::*;
use lumina_core::network::NetworkClient;
use std::time::Instant;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkReport {
    host: String,
    dns: Option<ResolveResult>,
    dns_error: Option<String>,
    http_status: Option<u16>,
    http_error: Option<String>,
    elapsed_ms: f64,
    usable: bool,
    via_proxy: bool,
}

async fn check(policy: ResolverPolicy, host: String) -> Result<NetworkReport, String> {
    let started = Instant::now();
    let resolver = AppResolver::new(policy.clone()).map_err(|error| format!("{error:#}"))?;
    let mut report = NetworkReport {
        host: host.clone(),
        dns: None,
        dns_error: None,
        http_status: None,
        http_error: None,
        elapsed_ms: 0.0,
        usable: false,
        via_proxy: policy.proxy_url.is_some(),
    };
    match resolver.resolve(&host).await {
        Ok(result) => report.dns = Some(result),
        Err(error) => report.dns_error = Some(format!("{error:#}")),
    }
    if report.dns.is_none() && !report.via_proxy {
        report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        return Ok(report);
    }
    let target = if host == "gutendex.com" {
        "https://gutendex.com/books/?ids=11".to_string()
    } else {
        format!("https://{host}/")
    };
    let client = NetworkClient::new(
        resolver,
        "LuminaShelf/0.6 diagnostic",
        Some(Duration::from_secs(15)),
    )
    .and_then(|client| client.client())
    .map_err(|error| format!("{error:#}"))?;
    match client.get(target).send().await {
        Ok(response) => {
            report.http_status = Some(response.status().as_u16());
            if response.status().is_success() {
                if host == "gutendex.com" {
                    match response.json::<serde_json::Value>().await {
                        Ok(value)
                            if value
                                .get("results")
                                .is_some_and(serde_json::Value::is_array) =>
                        {
                            report.usable = true
                        }
                        _ => report.http_error = Some("书源没有返回有效书籍数据".into()),
                    }
                } else {
                    report.usable = true;
                }
            } else {
                report.http_error = Some(format!("书源返回 HTTP {}", response.status().as_u16()));
            }
        }
        Err(error) => report.http_error = Some(format!("{:#}", error.without_url())),
    }
    report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(report)
}

#[tauri::command]
pub async fn test_network(policy: ResolverPolicy, host: String) -> Result<NetworkReport, String> {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host.contains(['/', ' ', '@', ':']) {
        return Err("测试域名只填写域名，例如 gutendex.com".into());
    }
    check(policy, host).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Optimization {
    policy: ResolverPolicy,
    report: NetworkReport,
    label: String,
}

#[tauri::command]
pub async fn optimize_network(state: State<'_, AppState>) -> Result<Optimization, String> {
    use futures::{stream, StreamExt};
    let existing = state.resolver.policy().await;
    let mut candidates = Vec::new();
    let mut current = existing.clone();
    current.fallback_to_system = false;
    candidates.push(("当前配置".to_string(), current));
    for (label, mode, endpoint) in [
        ("系统网络", lumina_core::ResolverMode::System, None),
        (
            "阿里加密 DNS",
            lumina_core::ResolverMode::Doh,
            Some("https://dns.alidns.com/dns-query"),
        ),
        (
            "腾讯加密 DNS",
            lumina_core::ResolverMode::Doh,
            Some("https://doh.pub/dns-query"),
        ),
    ] {
        let policy = ResolverPolicy {
            mode,
            doh_url: endpoint.map(str::to_string),
            app_hosts: existing.app_hosts.clone(),
            proxy_url: existing.proxy_url.clone(),
            fallback_to_system: false,
            timeout_ms: 5000,
            ..Default::default()
        };
        candidates.push((label.to_string(), policy));
    }
    let mut reports = stream::iter(candidates)
        .map(|(label, policy)| async move {
            let report = check(policy.clone(), "gutendex.com".into()).await;
            (label, policy, report)
        })
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    reports.retain(|(_, _, report)| report.as_ref().is_ok_and(|report| report.usable));
    reports.sort_by(|a, b| {
        a.2.as_ref()
            .unwrap()
            .elapsed_ms
            .total_cmp(&b.2.as_ref().unwrap().elapsed_ms)
    });
    let (label, mut policy, report) = reports
        .into_iter()
        .next()
        .ok_or("没有配置完成实际书源连接；请检查网络或配置可用代理")?;
    policy.fallback_to_system = existing.fallback_to_system;
    state
        .resolver
        .set_policy(policy)
        .await
        .map_err(|error| format!("{error:#}"))?;
    let applied = state.resolver.policy().await;
    if let Err(error) = save_resolver_policy(&state.resolver_policy_path, &applied) {
        let _ = state.resolver.set_policy(existing).await;
        return Err(error);
    }
    Ok(Optimization {
        label,
        policy: applied,
        report: report.unwrap(),
    })
}

#[tauri::command]
pub async fn book_cover_bytes(
    state: State<'_, AppState>,
    url: String,
) -> Result<tauri::ipc::Response, String> {
    let target = Url::parse(&url).map_err(|_| "封面地址无效")?;
    if !matches!(target.scheme(), "http" | "https") {
        return Err("封面地址须为 HTTP(S) 地址".into());
    }
    let client = state
        .zlibrary_probe
        .client()
        .map_err(|error| format!("{error:#}"))?;
    let mut response = client
        .get(target)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("{:#}", error.without_url()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| error.to_string())? {
        if bytes.len() + chunk.len() > 3 * 1024 * 1024 {
            return Err("封面图片过大".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(tauri::ipc::Response::new(bytes))
}

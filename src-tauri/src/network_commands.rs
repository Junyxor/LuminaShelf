use super::*;
use lumina_core::network::DnsProfile;
use lumina_core::network::NetworkClient;
use std::time::Instant;

#[tauri::command]
pub async fn dns_profiles(state: State<'_, AppState>) -> Result<Vec<DnsProfile>, String> {
    if state
        .state_store
        .setting("dns-profiles-migrated")
        .map_err(|e| e.to_string())?
        .is_none()
    {
        let policy = state.resolver.policy().await;
        if !matches!(
            policy.mode,
            lumina_core::ResolverMode::System | lumina_core::ResolverMode::AppHosts
        ) {
            state
                .state_store
                .save_dns_profile(DnsProfile {
                    id: "legacy-dns-v06".into(),
                    name: "原有 DNS 配置".into(),
                    policy,
                })
                .map_err(|e| e.to_string())?;
        }
        state
            .state_store
            .set_setting("dns-profiles-migrated", "1")
            .map_err(|e| e.to_string())?;
    }
    state
        .state_store
        .dns_profiles()
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn save_dns_profile(
    state: State<'_, AppState>,
    profile: DnsProfile,
) -> Result<DnsProfile, String> {
    state
        .state_store
        .save_dns_profile(profile)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub async fn remove_dns_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<ResolverPolicy, String> {
    let current = state.resolver.policy().await;
    if state
        .state_store
        .dns_profiles()
        .map_err(|e| e.to_string())?
        .iter()
        .any(|profile| profile.id == id && profile.policy == current)
    {
        let next = ResolverPolicy {
            mode: lumina_core::ResolverMode::System,
            upstream_ip: None,
            bootstrap_ips: Vec::new(),
            doh_url: None,
            server_name: None,
            upstream_port: None,
            ..current
        };
        set_resolver_policy(state.clone(), next).await?;
    }
    state
        .state_store
        .remove_dns_profile(&id)
        .map_err(|error| format!("{error:#}"))?;
    Ok(state.resolver.policy().await)
}

#[tauri::command]
pub async fn apply_dns_profile(
    state: State<'_, AppState>,
    id: String,
) -> Result<ResolverPolicy, String> {
    let profile = state
        .state_store
        .dns_profiles()
        .map_err(|error| format!("{error:#}"))?
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or("DNS 配置不存在")?;
    set_resolver_policy(state, profile.policy).await
}

#[derive(Debug, Clone, Serialize)]
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
    check_path(policy, host, None).await
}

/// `path` probes an API route instead of the site root, which may be a browser challenge.
async fn check_path(
    policy: ResolverPolicy,
    host: String,
    path: Option<&str>,
) -> Result<NetworkReport, String> {
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
        format!("https://{host}{}", path.unwrap_or("/"))
    };
    let client = NetworkClient::new(
        resolver,
        &format!("LuminaShelf/{} diagnostic", env!("CARGO_PKG_VERSION")),
        Some(Duration::from_secs(15)),
    )
    .map_err(|error| format!("{error:#}"))?;
    match client
        .get(Url::parse(&target).map_err(|error| error.to_string())?)
        .await
    {
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
        Err(error) => report.http_error = Some(format!("{error:#}")),
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
    zlibrary: Option<NetworkReport>,
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
    for profile in state
        .state_store
        .dns_profiles()
        .map_err(|error| format!("{error:#}"))?
    {
        let mut policy = profile.policy;
        policy.fallback_to_system = false;
        candidates.push((profile.name, policy));
    }
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
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();
    for (label, policy) in candidates {
        let key = serde_json::to_value(&policy)
            .map_err(|error| error.to_string())?
            .to_string();
        if seen.insert(key) {
            unique.push((label, policy));
        }
    }
    // Both book sources are tested: a DNS that reaches Gutenberg can still be poisoned for
    // Z-Library, which is the site most users need the setting for.
    let zlibrary_host = state.zlibrary.origin().await.host_str().map(str::to_string);
    let mut reports = stream::iter(unique)
        .map(|(label, policy)| {
            let zlibrary_host = zlibrary_host.clone();
            async move {
                let (gutendex, zlibrary) =
                    tokio::join!(check(policy.clone(), "gutendex.com".into()), async {
                        match zlibrary_host {
                            Some(host) => {
                                check_path(policy.clone(), host, Some("/eapi/info/domains"))
                                    .await
                                    .ok()
                            }
                            None => None,
                        }
                    });
                (label, policy, gutendex.ok(), zlibrary)
            }
        })
        .buffer_unordered(2)
        .collect::<Vec<_>>()
        .await;
    let reached = |report: &Option<NetworkReport>| report.as_ref().is_some_and(|r| r.usable);
    let score = |gutendex: &Option<NetworkReport>, zlibrary: &Option<NetworkReport>| {
        u8::from(reached(gutendex)) + 2 * u8::from(reached(zlibrary))
    };
    let elapsed =
        |report: &Option<NetworkReport>| report.as_ref().map(|r| r.elapsed_ms).unwrap_or(f64::MAX);
    reports.retain(|(_, _, gutendex, zlibrary)| score(gutendex, zlibrary) > 0);
    reports.sort_by(|a, b| {
        score(&b.2, &b.3)
            .cmp(&score(&a.2, &a.3))
            .then((elapsed(&a.2) + elapsed(&a.3)).total_cmp(&(elapsed(&b.2) + elapsed(&b.3))))
    });
    let (label, mut policy, gutendex, zlibrary) = reports
        .into_iter()
        .next()
        .ok_or("没有配置完成实际书源连接；请检查网络或配置可用代理")?;
    let report = gutendex
        .into_iter()
        .chain(zlibrary.clone())
        .find(|report| report.usable)
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
        report,
        zlibrary,
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

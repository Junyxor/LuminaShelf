use lumina_core::provider::BookProvider;
use lumina_core::{AppResolver, GutendexProvider, ResolverMode, ResolverPolicy, SearchQuery};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut modes = vec![
        ("system", ResolverMode::System, None, None),
        (
            "aliyun-dns",
            ResolverMode::CustomDns,
            Some("223.5.5.5"),
            None,
        ),
        (
            "aliyun-doh",
            ResolverMode::Doh,
            None,
            Some("https://dns.alidns.com/dns-query"),
        ),
        (
            "tencent-doh",
            ResolverMode::Doh,
            None,
            Some("https://doh.pub/dns-query"),
        ),
        (
            "cloudflare-doh",
            ResolverMode::Doh,
            None,
            Some("https://cloudflare-dns.com/dns-query"),
        ),
    ];
    let mut success = false;
    let selected = std::env::args().nth(1);
    if let Some(endpoint) = selected
        .as_deref()
        .filter(|value| value.starts_with("https://"))
    {
        modes = vec![("custom-doh", ResolverMode::Doh, None, Some(endpoint))];
    }
    for (name, mode, ip, endpoint) in modes {
        if selected
            .as_ref()
            .is_some_and(|selected| !selected.starts_with("https://") && selected != name)
        {
            continue;
        }
        let policy = ResolverPolicy {
            mode,
            upstream_ip: ip.map(str::parse).transpose()?,
            doh_url: endpoint.map(str::to_string),
            fallback_to_system: false,
            timeout_ms: 5000,
            ..Default::default()
        };
        let resolver = AppResolver::new(policy)?;
        match resolver.resolve("gutendex.com").await {
            Ok(result) => println!(
                "{name}: DNS OK {:?} ({:.0}ms)",
                result.addresses, result.elapsed_ms
            ),
            Err(error) => {
                println!("{name}: DNS FAIL {error:#}");
                continue;
            }
        }
        let provider = GutendexProvider::new(resolver)?;
        let query = SearchQuery {
            text: "Alice".into(),
            page: 1,
            page_size: 5,
            formats: vec![],
        };
        match provider.search(query).await {
            Ok(result) if !result.items.is_empty() => {
                println!("{name}: SEARCH OK {} books", result.items.len());
                success = true;
            }
            Ok(_) => println!("{name}: SEARCH FAIL empty response"),
            Err(error) => println!("{name}: SEARCH FAIL {error:#}"),
        }
    }
    anyhow::ensure!(success, "no network route completed a real book search");
    Ok(())
}

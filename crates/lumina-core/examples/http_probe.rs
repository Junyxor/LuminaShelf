//! Manual HTTP protocol/DNS diagnostic. Sends only public book metadata GETs.
use lumina_core::{AppResolver, ReqwestResolver, ResolverMode, ResolverPolicy};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let resolver = AppResolver::new(ResolverPolicy {
        mode: ResolverMode::Doh,
        doh_url: Some("https://dns.alidns.com/dns-query".into()),
        fallback_to_system: false,
        timeout_ms: 5000,
        ..Default::default()
    })?;
    for (label, http1, system) in [
        ("app-auto", false, false),
        ("app-http1", true, false),
        ("system-auto", false, true),
        ("system-http1", true, true),
    ] {
        let mut builder = reqwest::Client::builder()
            .user_agent("LuminaShelf/0.6 Gutenberg")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
        if http1 {
            builder = builder.http1_only();
        }
        if !system {
            builder = builder.dns_resolver(Arc::new(ReqwestResolver::new(resolver.clone())));
        }
        let client = builder.build()?;
        for suffix in ["ids=11", "search=Alice"] {
            let started = Instant::now();
            match client
                .get(format!("https://gutendex.com/books/?{suffix}"))
                .send()
                .await
            {
                Ok(response) => println!(
                    "{label} {suffix}: {} {:?}, {}ms",
                    response.status(),
                    response.version(),
                    started.elapsed().as_millis()
                ),
                Err(error) => println!(
                    "{label} {suffix}: {:#}, {}ms",
                    anyhow::Error::new(error.without_url()),
                    started.elapsed().as_millis()
                ),
            }
        }
    }
    Ok(())
}

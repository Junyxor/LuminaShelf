use super::AppResolver;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{io, net::SocketAddr};

#[derive(Clone)]
pub struct ReqwestResolver {
    inner: AppResolver,
}

impl ReqwestResolver {
    pub fn new(inner: AppResolver) -> Self { Self { inner } }
}

impl Resolve for ReqwestResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolver = self.inner.clone();
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let result = resolver.resolve(&host).await.map_err(|error| {
                Box::new(io::Error::other(error.to_string())) as Box<dyn std::error::Error + Send + Sync>
            })?;
            let addrs: Addrs = Box::new(result.addresses.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

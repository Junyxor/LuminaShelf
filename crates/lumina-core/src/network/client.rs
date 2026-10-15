use super::{AppResolver, ReqwestResolver};
use anyhow::{Context, Result};
use reqwest::{Client, ClientBuilder};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct NetworkClient {
    resolver: AppResolver,
    user_agent: Arc<str>,
    timeout: Option<Duration>,
    allow_redirects: bool,
    cached: Arc<Mutex<(u64, Client)>>,
}

impl NetworkClient {
    pub fn new(resolver: AppResolver, user_agent: &str, timeout: Option<Duration>) -> Result<Self> {
        Self::with_redirects(resolver, user_agent, timeout, true)
    }
    pub fn with_redirects(
        resolver: AppResolver,
        user_agent: &str,
        timeout: Option<Duration>,
        allow_redirects: bool,
    ) -> Result<Self> {
        let client = Self::build(&resolver, user_agent, timeout, allow_redirects)?;
        Ok(Self {
            cached: Arc::new(Mutex::new((resolver.revision(), client))),
            resolver,
            user_agent: user_agent.into(),
            timeout,
            allow_redirects,
        })
    }
    pub fn client(&self) -> Result<Client> {
        let revision = self.resolver.revision();
        let mut cache = self
            .cached
            .lock()
            .map_err(|_| anyhow::anyhow!("网络客户端锁异常"))?;
        if cache.0 != revision {
            *cache = (
                revision,
                Self::build(
                    &self.resolver,
                    &self.user_agent,
                    self.timeout,
                    self.allow_redirects,
                )?,
            );
        }
        Ok(cache.1.clone())
    }
    pub async fn get(&self, url: url::Url) -> Result<reqwest::Response> {
        for attempt in 0..2 {
            match self.client()?.get(url.clone()).send().await {
                Ok(response) => return Ok(response),
                Err(error)
                    if attempt == 0
                        && (error.is_connect() || error.is_timeout() || error.is_request()) =>
                {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                Err(error) => {
                    return Err(anyhow::Error::new(error.without_url()).context("HTTPS 连接失败"))
                }
            }
        }
        unreachable!()
    }
    fn build(
        resolver: &AppResolver,
        user_agent: &str,
        timeout: Option<Duration>,
        allow_redirects: bool,
    ) -> Result<Client> {
        let mut builder: ClientBuilder = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .user_agent(user_agent)
            .redirect(if allow_redirects {
                reqwest::redirect::Policy::limited(5)
            } else {
                reqwest::redirect::Policy::none()
            })
            .dns_resolver(Arc::new(ReqwestResolver::new(resolver.clone())));
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(proxy) = resolver.proxy_url() {
            builder = builder.proxy(reqwest::Proxy::all(&proxy).context("代理地址无效")?);
        }
        builder.build().context("创建网络客户端失败")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    #[tokio::test]
    async fn account_clients_keep_redirects_disabled_after_network_changes() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 1024];
            let received = socket.read(&mut buffer).await.unwrap();
            assert!(received > 0);
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/should-not-receive-account-data\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let resolver = AppResolver::system().unwrap();
        let client = NetworkClient::with_redirects(
            resolver.clone(),
            "test",
            Some(Duration::from_secs(2)),
            false,
        )
        .unwrap();
        resolver
            .set_host_override("redirect.invalid", vec!["127.0.0.1".parse().unwrap()])
            .await
            .unwrap();
        assert_eq!(
            client
                .client()
                .unwrap()
                .get(format!("http://redirect.invalid:{port}/"))
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FOUND
        );
        server.await.unwrap();
    }

    async fn endpoint(
        ip: &str,
        port: u16,
        body: &'static str,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind((ip, port)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut input = [0_u8; 1024];
            let _ = stream.read(&mut input).await.unwrap();
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
            // A stale pooled connection would be reused here instead of the new IP.
            if stream.read(&mut input).await.unwrap_or(0) > 0 {
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        (port, task)
    }

    #[tokio::test]
    async fn saved_hosts_change_reaches_new_server_instead_of_old_connection() {
        let (port, first) = endpoint("127.0.0.1", 0, "first").await;
        let (_, second) = endpoint("127.0.0.2", port, "second").await;
        let resolver = AppResolver::system().unwrap();
        resolver
            .set_host_override("pool-test.invalid", vec!["127.0.0.1".parse().unwrap()])
            .await
            .unwrap();
        let client =
            NetworkClient::new(resolver.clone(), "test", Some(Duration::from_secs(3))).unwrap();
        let url = format!("http://pool-test.invalid:{port}/");
        assert_eq!(
            client
                .client()
                .unwrap()
                .get(&url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "first"
        );
        resolver
            .set_host_override("pool-test.invalid", vec!["127.0.0.2".parse().unwrap()])
            .await
            .unwrap();
        assert_eq!(
            client
                .client()
                .unwrap()
                .get(&url)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "second"
        );
        first.abort();
        second.abort();
    }
}

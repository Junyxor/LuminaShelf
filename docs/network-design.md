# 应用内 DNS 与 DoH 实现依据

核对日期：2026-10-13。以下结论来自 RFC、已锁定的 Hickory 0.26.3 / Reqwest 0.12.28 源码和服务商官方文档。

## 配置与引导

DoH 配置应保存完整 HTTPS 端点，例如 `https://dns.example.com:8443/custom/query?profile=reader`。解析 URL 后保留主机名、显式端口、路径和静态查询参数；默认 HTTPS 端口为 443。不能只保存 IP 和 `/dns-query`，也不能把用户输入的端点重新拼成固定路径。应用可将接口限定为具体 HTTPS URL；如果另行支持 RFC URI Template，则必须真正执行模板展开：POST 使用未定义变量，GET 将 `dns` 定义为查询报文的 base64url 编码；已有静态 query 时，模板使用 `{&dns}`。应用的具体 URL 模式不要声称支持任意模板。[1][2]

引导地址和 HTTPS 身份是两个不同的值。连接可以使用指定 IP，但 TLS 验证与 HTTP authority 必须使用端点 URL 的主机名。没有指定引导 IP 时，仅通过独立的系统解析器解析 DoH 端点域名；该 HTTP 客户端不能使用 `AppResolver` / `ReqwestResolver` 解析自己的上游域名，否则会产生循环依赖。RFC 明确允许配置初始地址、使用有对应 IP 证书的 IP URL，或者通过传统 DNS / 其他 DoH 服务引导后再验证 HTTPS 身份。[1, §10]

建议使用独立 Reqwest 客户端承载完整 DoH URL。显式引导 IP 可通过 `ClientBuilder::resolve_to_addrs(endpoint_host, addresses)` 注入，保留原 URL，使 HTTPS 主机名和 HTTP authority 仍由 URL 生成。其官方源码说明，URL 中的端口优先于引导 `SocketAddr` 的端口；引导地址端口设为 0 即可。[7] 端点引导与“查询失败后回退系统 DNS”是不同的操作，用户配置的回退开关应控制业务域名查询的回退，并在结果中报告实际回退。

配置校验建议要求 `https`、有效主机名和有效端口；拒绝 URL 用户信息、fragment 与未实现的模板语法。保存端点后再建立传输对象，避免不合法配置到查询阶段才失败。这些是应用的接口约束，而非 RFC 新增要求。

## Hickory 0.26.3 的边界与证书根库

`NameServerConfig::https(ip, server_name, path)` 配置 HTTP/2 POST，默认端口为 443。`path` 实际按 HTTP `PathAndQuery` 解析，因此能够包含静态查询参数。`ConnectionConfig.port` 能改变 TCP 连接端口。[3][4]

但是 Hickory 的 `server_name` 同时用于 Rustls 的 `ServerName` 和 HTTP `:authority`。只修改 `ConnectionConfig.port` 不会给 HTTP authority 加上自定义端口；将 `host:port` 填入 `server_name` 又不符合 TLS 主机名类型。因此完整自定义 URL，尤其非 443 端口，需要独立 HTTP 传输，或者上游明确拆分 TLS 主机名与 HTTP authority。不要仅把 URL 拆成现有 `upstreamIp`、`serverName`、`dohPath` 后宣称完整支持。[4][5]

Hickory 的默认 feature 只有 `system-config` 和 `tokio`。启用 `tls-ring` / `https-ring` 提供加密能力，但不会自动提供受信任 CA。`hickory-net::tls::client_config()` 在没有 `rustls-platform-verifier` 时创建空 `RootCertStore`，只有启用 `webpki-roots` 才填入 `webpki_roots::TLS_SERVER_ROOTS`。该库提供 Mozilla 信任的根证书；`rustls-platform-verifier` 则使用平台验证器。[6][8]

因此当前 Hickory DoT/DoH 依赖应显式启用 `webpki-roots`（或者选用平台验证器），保持证书链和主机名验证开启。Reqwest 的 `rustls-tls` feature 自带 `rustls-tls-webpki-roots`，但这不等同于启用了 Hickory 自身的 `webpki-roots` feature。[6][7] 本次排查的现场验证显示：缺少 Hickory 根库时 AliDNS / Cloudflare 的加密解析失败，加入该 feature 后 AliDNS DoH 与真实 Gutendex 搜索通过；该结果与源码原因一致。

## RFC 8484 请求与响应

- POST：请求体是单条 DNS wire message 的原始字节，设置 `Content-Type: application/dns-message` 和 `Accept: application/dns-message`；不能发送 JSON，也不能附加 DNS over TCP / DoT 使用的两个长度字节。Hickory 0.26.3 客户端采用此格式。[1, §§4.1, 6][4][5]
- GET：将同一条 DNS wire message 编码为 **无 `=` padding 的 base64url**，放入 `dns` 查询变量。保留端点已有的静态参数。RFC 要求服务端支持 GET 和 POST，客户端选择 POST 即可。[1, §§4.1, 6][2]
- 使用 DNS ID 0 符合 RFC 的缓存建议；HTTP 已负责匹配请求与响应。Hickory 会将请求 ID 设为 0。[1, §4.1][5]
- HTTP 2xx 可以携带 NXDOMAIN / SERVFAIL，不能将 HTTP 成功直接当作 DNS 成功。应解析 DNS 响应与 RCODE，提取有效 A / AAAA 地址，处理 CNAME，并区分没有地址、格式错误、超时与传输错误。RFC 的 DNS wire payload 上限是 65535 字节。[1, §§4.2.1, 6]

## 服务商预设

阿里云官方公共 DNS 首页列出 `dns.alidns.com` 作为 DoH/DoT 地址，IPv4 为 `223.5.5.5`、`223.6.6.6`；官方接入文档确认这两个地址是该域名的 A 记录。DoH 使用 `https://dns.alidns.com/dns-query`；本机核对当天也解析到这两个地址。官方 DoH 文档支持 GET 与 POST，并给出了 `/dns-query` 二进制协议示例。`/resolve?name=...&type=...` 是另一套 JSON API，不能与 RFC 8484 wire transport 混用。用户配置的账号专属域名应完整保留。[9][10][11]

腾讯 DNSPod 官方产品页给出的加密端点是 `https://doh.pub/dns-query`（DoH）和 `dot.pub`（DoT），同时将 `119.29.29.29` 列为普通 IPv4 Public DNS 地址。不能直接把传统 UDP DNS 预设的 `119.29.29.29` 沿用为 `doh.pub` 的固定 HTTPS 引导地址。[12]

DNSPod 官方公告曾列出 IP 形式的 `https://1.12.12.12/dns-query` 与 `https://120.53.53.53/dns-query`，但从 2024-09-29 起推荐域名接入；公告说明 IP 接入当时暂未下线。本机系统解析 `doh.pub` 在核对当天返回 `1.12.12.12`、`120.53.53.53`。因此腾讯预设应优先使用完整 `doh.pub` URL 与独立系统引导；如果保留这两个静态种子，应当作为可更新的引导地址，连接时仍验证 `doh.pub`，不可承诺 IP 永久有效。[13]

官方还明确下线了旧 DoH/DoT 测试集群 `162.14.21.5`、`162.14.21.178`、`175.24.219.66`、`120.53.80.136`，要求清除相关域名固定映射，并建议通过公共 DNS 解析 `doh.pub` / `dot.pub`。不要把这些旧地址放入新的预设。[14]

## 实现验收要点

验证一个自定义 HTTPS 端点确实收到原路径、静态 query 和自定义端口的 authority；验证指定引导 IP 时仍校验 URL 主机名；验证未指定引导 IP 时使用独立解析器引导上游域名且不递归调用应用解析器。真实服务检查应关闭业务域名查询回退，确保成功确实来自所选 DoH；另行验证开启回退时准确报告 `usedFallback`。无效 DNS wire response 和带 NXDOMAIN / SERVFAIL 的 HTTP 200 必须形成正确的 DNS 错误。

## 一手来源

1. [RFC 8484 — DNS Queries over HTTPS](https://www.rfc-editor.org/rfc/rfc8484.html)，§§3、4.1、4.2.1、6、10。
2. [RFC 6570 — URI Template](https://www.rfc-editor.org/rfc/rfc6570.html)，§§3.2.8、3.2.9。
3. [Hickory Resolver 0.26.3 config.rs](https://docs.rs/hickory-resolver/0.26.3/src/hickory_resolver/config.rs.html)，`NameServerConfig::https`、`ConnectionConfig`、`ProtocolConfig`。
4. [Hickory Net 0.26.3 http.rs](https://docs.rs/hickory-net/0.26.3/src/hickory_net/http.rs.html#21-74)，`RequestContext::build`。
5. [Hickory Net 0.26.3 h2.rs](https://docs.rs/hickory-net/0.26.3/src/hickory_net/h2.rs.html)，`send_message`、`connect`、`send`。
6. [Hickory Net 0.26.3 tls.rs](https://docs.rs/hickory-net/0.26.3/src/hickory_net/tls.rs.html#251-269)，`client_config`；[Resolver feature 定义](https://docs.rs/crate/hickory-resolver/0.26.3/source/Cargo.toml)。
7. [Reqwest 0.12.28 ClientBuilder](https://docs.rs/reqwest/0.12.28/reqwest/struct.ClientBuilder.html#method.resolve_to_addrs)；[官方 client.rs 源码](https://docs.rs/reqwest/0.12.28/src/reqwest/async_impl/client.rs.html)；[官方 feature 定义](https://docs.rs/crate/reqwest/0.12.28/source/Cargo.toml)。
8. [webpki-roots 1.0.9](https://docs.rs/webpki-roots/1.0.9/webpki_roots/)；[Rustls 0.23.45 RootCertStore](https://docs.rs/rustls/0.23.45/rustls/struct.RootCertStore.html)。
9. [阿里云公共 DNS 官网](https://www.alidns.com/)。
10. [阿里云官方接入配置](https://www.alidns.com/knowledge?type=SETTING_DOCS#company_doh)；该页面的[第一方内容 API](https://www.alidns.com/api/get_article_detail?id=6018321800a44d0e45e90d39)。
11. [阿里云 — DNS over HTTPs (DoH)](https://help.aliyun.com/zh/dns/dns-over-https)。
12. [DNSPod Public DNS 官方产品页](https://www.dnspod.cn/products/publicdns)。
13. [DNSPod — 免费版 DoH、DoT 不再公开 IP 接入的公告](https://docs.dnspod.cn/notices/mian-fei-ban-dot-dohbu-zai-gong-kai-ipjie-ru-de-gong-gao/)，更新于 2024-11-04。
14. [DNSPod — 关于 DoH/DoT 独立测试 IP 下线的公告](https://docs.dnspod.cn/notices/guan-yu-doh-dotdu-li-ce-shi-ipxia-xian-de-gong-gao/)，更新于 2025-09-28。公告的节点下线年份有内部不一致，因此本文仅使用其明确列出的地址与域名接入建议。

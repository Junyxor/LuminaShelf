# FluxDO 源码核对与 LuminaShelf 可采用的交互

核对日期：2026-10-14。主库固定为 [`e1bd839ef87825347c062fb588751a1a62e02372`](https://github.com/Lingyan000/fluxdo/commit/e1bd839ef87825347c062fb588751a1a62e02372)，其 `core/doh_proxy` 子模块固定为 [`08d3468f0a1eb2858840c525d61f8aa888696522`](https://github.com/Lingyan000/fluxdo_doh/commit/08d3468f0a1eb2858840c525d61f8aa888696522)。网络传输中断使浅克隆未完成；本次通过 GitHub 一手 tree API 和固定版本 raw 源码核对，选定文件保存在被忽略的 `.references/fluxdo/`，该目录不是完整 Git checkout。未启动浏览器、安装证书、修改系统网络或复制实现到产品中。

## 直接对应用户反馈的发现

FluxDO 是 Linux.do 的 Flutter 论坛客户端，并非电子书应用；README 明确其用途和 GPL-3.0 协议。不能将它描述成已有书源镜像抓取或图书推荐方案的项目。[项目用途](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/README.md#L1-L8) · [协议声明](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/README.md#L162-L164)

它真正值得采用的是“服务器列表”交互：每个自定义 DoH 保存名称、完整 URL、可选 Bootstrap IP 列表；预设与自定义条目同时展示，当前选中项单独保存。页面明确提供“添加”、逐项测速、全部测速，以及复制、编辑、删除；点击整行即可选用服务器。这比只有一个覆盖式地址输入框更符合“我自己添加 DNS”的期望。这里是多个已保存服务器、单个活动服务器，并非同时对多个服务器竞速的配置。[模型与字段](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/network_settings_service.dart#L27-L119) · [列表与操作](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/pages/network_settings_page/doh_detail_settings_page.dart#L160-L355) · [添加表单](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/pages/network_settings_page/doh_detail_settings_page.dart#L606-L690)

自定义列表通过 SharedPreferences 中的 JSON 持久化；选用服务器会更新解析器、清理旧解析缓存并触发网络设置版本变更。编辑当前条目的 URL 会同步更新当前选择；删除后若原选择不在列表中，回到首个预设。已核对的设置页提供 DoH 配置，没有普通 UDP DNS / DoT 的多协议服务器表单。[载入和选用](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/network_settings_service.dart#L268-L336) · [增改删](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/network_settings_service.dart#L339-L429) · [失效选择处理](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/network_settings_service.dart#L1277-L1294)

## 传输实现的适用边界

Bootstrap IP 是可选列表；Dart 客户端将指定 IP 用于连接，同时保留原域名进行 TLS/SNI，留空则由系统引导连接上游。解析器另有受开关控制的系统 DNS 回退，并实现 TTL 缓存及同域名请求合并。[引导连接](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/bootstrap_doh_client.dart#L101-L163) · [缓存与回退](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/doh_resolver.dart#L89-L188)

不能照搬其“测速成功”的含义：`testLatency` 只测 TCP + TLS 握手，源码明确不发送 HTTP 请求，所以不能据此确认 DoH DNS 报文、URL 路径或实际搜书可用。全部测速同时发出所有请求，也不适合本次限制并发为 2 的运行要求。[连接层测速](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/doh_resolver.dart#L191-L245) · [全部测速](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/pages/network_settings_page/doh_detail_settings_page.dart#L364-L398)

完整自定义 URL 也不能直接复用这套实现。Dart 客户端只读取 `uri.path`，然后自行拼 `?dns=...`，丢弃原静态查询参数，手写 Host 也没加自定义端口。Rust 子模块对包含已知供应商域名的输入直接改成固定预设；一般 URL 分支保留 URI，但手动 GET 同样用 `uri.path()` 重建，丢弃 query。它对自定义路径只是告警，并非拒绝；不要误报。LuminaShelf 应保留现有按完整 URL 发送原始 DNS POST 的 Reqwest 传输。[Dart URL 与请求](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/bootstrap_doh_client.dart#L61-L66) · [Dart GET](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/network/doh/bootstrap_doh_client.dart#L174-L204) · [Rust URL 处理](https://github.com/Lingyan000/fluxdo_doh/blob/08d3468f0a1eb2858840c525d61f8aa888696522/src/dns.rs#L282-L442) · [Rust GET](https://github.com/Lingyan000/fluxdo_doh/blob/08d3468f0a1eb2858840c525d61f8aa888696522/src/dns.rs#L1393-L1409)

FluxDO 还包含 WebView MITM / 安装 CA 的专用分支。LuminaShelf 的应用内 HTTP 请求可以直接使用自有解析器，无须为借鉴设置交互引入这套证书授权流程。[Windows 证书分支](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/pages/network_settings_page/widgets/doh_settings_card.dart#L115-L126) · [证书安装引导](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/pages/network_settings_page/widgets/doh_settings_card.dart#L371-L405)

## 首页与书源镜像要求

虽不是图书项目，它的首页数据管线可作为架构参考：GET 站点首页，提取 `data-preloaded` 脚本中的 JSON（兼容旧属性形式），从中解析真实首屏话题列表；列表服务优先使用这些内容，无可用首屏数据时请求 `/latest.json`。因此可借鉴“取站点真实内容 → 来源适配器解析 → 本地缓存 → 明确错误/重试”的流程，而不是生成占位推荐。[获取首页](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/preloaded_data_service.dart#L526-L550) · [提取数据](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/preloaded_data_service.dart#L566-L598) · [首屏列表](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/preloaded_data_service.dart#L837-L862) · [站点 API 回退](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/lib/services/discourse/_topics.dart#L21-L42)

此处未发现图书源官方/GitHub 镜像清单抓取及图书首页推荐适配实现；这两项仍须在 LuminaShelf 的图书提供方中完成。建议镜像清单保存发布地址、更新时间和已发现端点，去重后实际请求兼容接口，显示状态/延迟并允许选用；刷新失败保留上次清单及用户自填地址。找书首页显示所选书源的真实推荐，来源不支持或受限时明确显示状态，并继续提供搜索。不要把 DNS 服务器列表当作书源镜像列表。

## 最小具体改动建议

1. 设置首页增加明确的“DNS 服务器”列表和“添加 DNS / DoH”按钮，条目显示名称、协议、地址和当前使用标记；完整点击行和按钮保持至少 48px。
2. 自定义条目使用稳定 ID，支持添加、改名、编辑、删除、单项测试及“使用”；保存多条，不再用一个地址框覆盖之前的条目。普通 DNS、DoT、DoH 均沿用 LuminaShelf 既有解析模式；DoH 主输入始终是完整 HTTPS URL，可选填写多个 IPv4/IPv6 引导地址。
3. 测试按该条目草稿创建临时解析器，关闭业务域名回退，执行真实 A/AAAA 查询和书源 HTTPS 请求，分别显示结果。批量测试并发最多 2；测试操作不自动覆盖当前选择。
4. 当前条目的任意传输字段修改（包括仅修改引导地址）均刷新解析器及业务连接池；删除活动自定义条目后切换到明确的系统模式或另一有效条目并持久化。迁移 0.6 的既有自定义配置为第一条已保存条目。
5. 独立完成可刷新镜像清单、可用性检测/选择和真实书源发现首页；两者共用已选择的应用网络配置。

## 许可证与复用结论

FluxDO 主库 LICENSE 为 GPL-3.0；复制主库实现形成派生作品后分发，须满足相应 GPL 授权和对应源码要求，不能把摘录源码继续当成 MIT 实现。此次仅参考行为与组织方式，未复制 Dart UI、素材或实现。[GPL 第 5 节](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/LICENSE#L208-L243) · [第 6 节](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/LICENSE#L245-L286)

`core/doh_proxy` 是单独仓库，固定版本 README 末尾声明 MIT，但该版本 tree 中没有单独 LICENSE 文件，Cargo manifest 也没有 `license` 字段。若未来实际引入其代码，应保留作者版权与 MIT 许可文本并逐项核对第三方依赖；当前任务没有引入，现有 Hickory/Reqwest 已覆盖必需传输能力。[子模块来源](https://github.com/Lingyan000/fluxdo/blob/e1bd839ef87825347c062fb588751a1a62e02372/.gitmodules#L1-L3) · [子模块 MIT 声明](https://github.com/Lingyan000/fluxdo_doh/blob/08d3468f0a1eb2858840c525d61f8aa888696522/README.md#L262-L264) · [Cargo manifest](https://github.com/Lingyan000/fluxdo_doh/blob/08d3468f0a1eb2858840c525d61f8aa888696522/Cargo.toml#L1-L10)

## 补充：可实际拉取的社区镜像发布源

另行核对了 [`dongyubin/Awesome-Zlibrary`](https://github.com/dongyubin/Awesome-Zlibrary)，其 README 确实含 Z-Library 镜像地址表。最近一次 README 提交为 [`74bc1569f270536f124db7306491b626ef30af38`](https://github.com/dongyubin/Awesome-Zlibrary/commit/74bc1569f270536f124db7306491b626ef30af38)，提交时间 2026-10-14；核对时 GitHub 显示 639 stars。发布源应标注“社区”，README 自称部分地址为官方不构成本项目对隶属关系的确认。

可保存的原发布 URL 为 `https://raw.githubusercontent.com/dongyubin/Awesome-Zlibrary/main/README.md`。本机 raw 域名当时连接超时，已通过公开 GitHub Contents API 成功读取同一文件：`https://api.github.com/repos/dongyubin/Awesome-Zlibrary/contents/README.md?ref=main`，请求头 `Accept: application/vnd.github.raw+json`。固定版本核对时将 `ref` 设为上述 commit；API 返回的文件 blob SHA 为 `98f85a4cc15a0d2e7798a3a96a1110c8378d8935`。这些是地址发布内容的验证，没有向镜像发送凭据或执行登录。

应只解析 `## Zlibrary官方最新网址合集` 节中标题为 `| Z-Library 最新官方/镜像网址 | 国内网络是否能访问 |` 的连续表格，遇到 `### 官方网址最新（已失效）` 结束；逐行只接受 `✅`，同时跳过 HTTP。固定版本第 60–72 行得到 13 个 HTTPS 候选，其中包括 `https://z-lib.fm/`、`https://zh.z-lib.gd/`、`https://z-library.website/`、`https://z-library.qa/`、`https://z-library.im/`、`https://zh.zlib.li/` 和 `https://zh.z-library.sk/`。[已核对的完整表格与边界](https://github.com/dongyubin/Awesome-Zlibrary/blob/74bc1569f270536f124db7306491b626ef30af38/README.md#L57-L80)

不能对整个 README 直接提取所有 URL：同一张表第 73 行起已有 `🚫` 行，后面另有过期地址、钓鱼域名说明、捐赠和非书源链接。候选地址仅说明“该社区在发布”，仍须通过无账户的书源接口检测，才能显示“可用”；不得依据 README 的 `✅` 直接向用户承诺可访问或 EAPI 兼容。[失效部分](https://github.com/dongyubin/Awesome-Zlibrary/blob/74bc1569f270536f124db7306491b626ef30af38/README.md#L73-L101)

已排除 `bipinkrish/Zlibrary-API` README 作为镜像发布源：它是 API 包使用说明，不能仅因项目名相关便作为可工作的镜像订阅。此次只推荐上述一个已核对含实际列表且仍维护的社区源；没有将未证实的“官方发布地址”编入清单。

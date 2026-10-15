# LuminaShelf · 星书

LuminaShelf is a high-performance cross-platform ebook client built around a Rust core and a thin Tauri + React UI.

## 0.8.0 — 可恢复的下载队列

下载任务保存在 SQLite，断线自动按已保存字节重试；暂停、取消或应用进程退出后，
可在“下载”页继续。继续时重新获取书源链接，并校验文件版本与 Content-Range。
下载页提供直接可见的暂停、继续和管理按钮，也支持长按整条任务；移除记录保留书籍，
删除未完成文件与从头下载会明确确认。可分别设置每本 1–8 个连接、同时下载 1–3 本，
任务显示真实活动连接数、峰值连接数、速度和续传能力。服务器不支持 Range 时安全使用单流。

Android 下载使用 dataSync 前台服务保持后台传输，完成或暂停后停止；不申请 VPN、
证书、通知运行时权限或整个存储空间权限。系统强制终止后保留任务，用户主动继续。
旧版未带版本信息的部分文件会先逐字节核对再采用；无法确认同一文件时从头下载。
ZIP 书包可保留为压缩文件导出或交给解压应用，不自动展开远程压缩包。

DoH 增加遵循 TTL 的有界缓存、并发查询合并，以及快速地址族先返回、慢地址族后台补全。
应用 DNS 设置时清空解析缓存并重建连接池；完整自定义地址、引导 IP 与严格 TLS 校验保留。
参考 Olib Mobile 的任务管理交互，未复制其代码或私有依赖，见 [参考核查](docs/olib-mobile-reference.md)。

GitHub Actions 分别提供 x86（32位）、x86_64（64位）和通用 APK。
通用版包含 arm64-v8a、armeabi-v7a、x86、x86_64；三份包使用相同应用标识与签名。
普通 ARM 手机可使用通用版，Intel/AMD Android 设备可选对应的较小独立包。

## 0.7.0 — 收藏、书源与下载管理

“收藏”只保存远程书籍条目，“下载”另外保存离线文件。找书页直接读取古登堡热门、
Z-Library 网站热门或已登录账户的推荐，点击卡片再读取真实详情和可用格式。
下载会先解析书源的文件描述符，再请求真正的文件，并检查内容后入库；不把 JSON、
登录页或验证页伪装成 EPUB。已下载书籍支持长按管理、重新获取、导出、本地 TXT ↔ EPUB
文字转换和删除应用管理的副本。Android 支持通过系统文件授权交给外部阅读器，
也可使用系统分享菜单发送给已安装的邮箱或云盘应用。

DNS 设置采用明确的“添加、编辑、测试、使用、删除”列表，可保存多组配置和多个可选引导 IP；
0.6 的既有自定义配置会迁移进列表。镜像设置内置候选站点、书源域名目录和带来源标记的
GitHub 社区镜像表；支持拉取、去重、限量检测、手动选择及自己的发布清单。
社区表仅读取当前有效区，不采集失效区、钓鱼黑名单或无关外链。

公告区分别显示本项目公告和当前书源实际提供的公告/活动链接；站点未提供时如实显示状态。
更新检查读取本项目 GitHub Releases，区分“没有已发布版本”、连接失败和有可用更新。
官网转码、邮箱/Kindle/Google Drive 等服务的具体操作仍使用官网入口，受网站和账户权限约束；
未把未核实接口包装成本地发送成功。本地转换只支持经回读验证的 TXT/EPUB 文字内容。

参考与边界：[FluxDO 核对](docs/fluxdo-reference.md)、[书源协议与下载问题](docs/book-source-discovery.md)、
[公告和更新来源](docs/announcements-update-sources.md)。

## 浅色界面与可配置网络

默认浅色书架；手机底部固定提供“书架、找书、下载、设置”四个文字入口。
设置入口高 62 px，主要操作触控区域至少 48 px。阅读器默认纸白色，可主动切换护眼或夜间配色。

在“设置 → 网络连接”选择 DoH 后，可直接粘贴完整 HTTPS 地址，保留端口、路径和查询参数。
引导 IP 可留空，另支持代理、DoT、普通 DNS 和应用内 Hosts。保存后会重建请求连接池，
使设置实际应用到搜索、下载和封面；测试会检查 DNS 与真实 HTTPS，并明确报告系统回退。
“自动检测并选择”只采用完成实际书源连接的配置，保留用户的回退偏好。

本地导入使用系统文档选择器，支持 EPUB、PDF、TXT；仅在点击“导入书籍”时打开。
Android 内容 URI 会复制进应用私有书库，无需请求整个存储空间的访问权限。
基于官方插件、Radix Dialog 和 Lucide 实现，见 [开源说明](OPEN_SOURCE_NOTICES.md)。

网络实现依据 RFC 8484 及服务商官方文档，详见 [网络实现调研](docs/network-design.md)。
实际验证范围与未验证项记录于 [构建验证](BUILD_VERIFICATION.md)。

## Current state

The main desktop workflow is now connected end to end:

- search through pluggable Rust `BookProvider` implementations
- authenticated Z-Library EAPI account, profile, history, search, details and acquisition
- Project Gutenberg / Gutendex public provider
- app-local System DNS / custom DNS / DoH / DoT resolver and App Hosts rules
- native resumable segmented downloader with bounded HTTP Range concurrency
- persisted local library and reading progress in SQLite
- automatic library insertion after successful downloads
- EPUB 2 / EPUB 3 navigation parsing
- metadata-first EPUB / TXT reading with chapter-on-demand loading and bounded caches
- in-chapter text search and adjacent chapter prefetch
- built-in PDF reader with Rust IPC range loading and lazy-loaded PDF.js runtime
- Android arm64 debug APK CI artifact
- signed Android release APK build with persistent local release keys
- Windows NSIS installer packaging workflow

## Architecture

```text
Tauri 2 + React UI
        │
        ▼
Rust command bridge
        │
        ├── BookProvider registry
        │      ├── Gutendex
        │      └── Z-Library EAPI
        │
        ├── AppResolver (Hosts / DNS / DoH / DoT)
        ├── Native segmented downloader
        ├── SQLite library / reading state
        └── EPUB / TXT / PDF reader bridge
```

## Boundaries

- Z-Library integration uses the EAPI provider path; no HTML/JS anti-bot challenge solver is included.
- Remote hosts lists only become candidate address hints; they are never silently written to OS Hosts.
- TLS hostname validation stays enabled even when an address hint is used.
- Account passwords are not stored in generic SQLite metadata.

## Development

Requires a current Rust stable toolchain and Node.js 24+.

```bash
npm ci
npm run build
cargo test -p lumina-core
cargo clippy -p lumina-core --all-targets -- -D warnings
cargo test -p lumina-shelf-desktop --lib
npm run test:ui
```

Run the desktop app locally with:

```bash
npm run tauri -- dev
```

## Packaging

A Windows installer can be built with:

```bash
npm run tauri -- build --bundles nsis
```

The `Desktop Package` GitHub Actions workflow can also be started manually. Pushing a `v*` tag builds the Windows installer and publishes it to the matching GitHub Release.

Builds default to two Cargo jobs and two Gradle workers. UI checks run headlessly;
`scripts/check-native.mjs` exercises a hidden Windows WebView without opening file dialogs.

Android CI produces signed x86, x86_64 and universal APK artifacts and can optionally publish a prerelease from `workflow_dispatch`.

For a signed Android release, install JDK 17, Android SDK Platform 36, Build Tools 36.0.0,
NDK 27.2.12479018 and the Rust Android targets, then run:

```bash
npm run android:release
```

The default release includes arm64-v8a, armeabi-v7a, x86 and x86_64. The signed APK,
SHA-256 checksum and public certificate are written to `artifacts/android/`.
The first run generates a release keystore and credentials in `.signing/`; subsequent
builds reuse that identity. Both directories are ignored by Git. See
[Android release instructions](ANDROID_RELEASE.md) for environment setup and key backups.

The `Android CI` workflow runs checks, signs all three APK variants with repository
Secrets, uploads each APK/checksum/public certificate separately, and can boot the universal and x86_64 packages in a headless
Android emulator. Manual builds default to no GitHub Release publication. Publishing
requires the workflow's explicit release input or a pushed `v*` tag.

## Status

LuminaShelf is in the integration and release-hardening stage. The core search → details → download → local library → built-in reader path is implemented; remaining work is focused on real-device testing, packaging polish and targeted reliability fixes rather than another architecture rewrite.

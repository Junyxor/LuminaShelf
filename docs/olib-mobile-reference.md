# Olib Mobile 下载、管理与网络实现核对

核对日期：2026-10-15。用户提供项目为 [`shiyi-0x7f/olib-mobile`](https://github.com/shiyi-0x7f/olib-mobile)，本次浅克隆并固定核对提交 [`0deb406d6da8d7b802e8d131755eba3eb1866b99`](https://github.com/shiyi-0x7f/olib-mobile/commit/0deb406d6da8d7b802e8d131755eba3eb1866b99)。副本位于已忽略的 `.references/olib-mobile/`，未构建或运行该应用、访问其账户、启动浏览器、索取私有依赖权限或复制源码/素材。以下结论来自固定版本源码；不能将 README 的功能介绍等同于完整实现或实机验证。

## 对当前反馈的直接结论

Olib 可参考的是下载页的任务分组、可见重试/取消按钮、更多菜单、文件存在检查及一次性下载链接的明确提示。**公开版本不能作为已经完成断点续传、单文件多连接分段下载或持久后台任务的参考实现**。其公开下载器失败后删除临时文件，重试从新任务开始；真正的 Z-Library HTTP 客户端又是未公开的私有依赖，不能推测其中支持了上述能力。[任务状态与可重试条件](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L13-L35) · [临时文件与传输](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L305-L351) · [失败清理](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L391-L419) · [私有依赖](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/pubspec.yaml#L76-L79)

用户截图表明大文件传输在约 5.5 MB / 34 MB 时流读取中止，并且下载项缺少恢复入口。`error decoding response body … error reading a body from connection: connection aborted` 本身不足以证明是 DoH、EPUB 内容或压缩解码错误；需要下载引擎保留字节、记录可恢复任务并处理流中断。只修 DNS 或把失败文字翻译成中文，都不能解决已下载部分丢失和不能继续的问题。

## 公开代码实际具备什么

### 下载任务与持久化

- `DownloadStatus` 仅有 `pending/downloading/completed/error/cancelled`，没有 `paused`。`DownloadTask` 保存书籍、进度、路径、错误、下载时间、大小和一次性链接/缺失文件标记，没有已下载字节偏移、分段表、ETag 或 Last-Modified。[模型](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L13-L78)
- 启动时恢复的是已完成下载历史；即使本地文件缺失也只是将历史转为错误。持久化内容是 `bookId → title/author/filePath/cover/extension/downloadTime`，并不保存进行中或失败任务、临时文件位置、下载 URL 或书籍 hash。由历史重建的最小 Book 也没有 hash，所以文件丢失的历史项不允许重试。[启动恢复](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L91-L140) · [历史存储字段](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/services/storage_service.dart#L84-L113)
- 每次开始创建新的微秒时间戳 `.part`，下载成功才重命名为最终书名；取消或失败的 `finally` 删除该临时文件。这样避免了半本书出现在完成区，但丢弃了断点，不能用作恢复机制。[写临时文件并提交](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L305-L390) · [清理](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L409-L419)
- 去重只排除同一个书籍 ID 的准备/下载中任务；多个书籍可以分别异步调用 `startDownload`。公开下载器没有队列并发上限，也没有单个文件的 Range 分片调度。多任务同时传输与单文件分段下载必须分开设计和验收。[任务开始与去重](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L255-L278) · [一次传输调用](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L319-L351)

### 链接、重试与账户错误

普通账号路径调用 `ZLibraryApi.downloadBook(id, hash, temporaryPath, onProgress, cancelToken)`。错误/取消后可见的“重试”只是再次调用 `startDownload(task.book)`；公开层没有自动保留偏移或指数退避重试。[调用](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L337-L360) · [重试按钮](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L268-L289)

`presetUrl` 使用独立无账户头的 `Dio()` 直接下载，且将任务标为 `needsFreshLink`。这类任务不展示普通重试，提示回原书页获取新链接；书籍详情页向自家后端 `/books/download-url` 取得签名 URL 后重新发起下载。该后端协议是 Olib 自家服务，不是 Z-Library 的公开下载接口，不能照搬其域名、token 或配额机制。[一次性链接](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L268-L277) · [不带源账户下载](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L322-L336) · [后端协议](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/services/backend_books_api.dart#L16-L67) · [详情重新获取链接](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/book_detail/widgets/book_action_bar.dart#L292-L323)

普通下载失败时会再查询账号资料辨认当日额度是否用尽，以 `quotaExceeded` 标记并显示专门提示，而不是让所有业务拒绝都表现为连接失败。值得借鉴错误分类；不能把一次查询不到额度误认为一定网络问题。[额度确认与错误分类](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L352-L430)

### 下载管理交互

Olib 下载页把准备/下载/失败/取消放在“进行中”，已完成单独显示并按时间倒序；每个卡片明确提供重试或取消，完成项提供打开文件。右侧可见更多菜单提供详情、分享、微信读书导入、局域网传电脑、移除记录/删除文件；打开/分享前重新检查本地文件是否存在。可以参考这套可发现的管理入口。[分组](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L24-L85) · [可见操作](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L265-L324) · [文件检查/外部打开](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L382-L413) · [删除确认](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L483-L520)

但它的**下载页卡片没有长按回调**。文档里的“长按多选整理”说的是收藏页；不能将收藏页支持长按作为下载页已经可管理的证据。LuminaShelf 应在用户截图所在的下载页实现菜单和长按，并验证失败任务也能操作。[下载卡片完整结构](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/downloads/local_downloads_screen.dart#L133-L331) · [收藏与下载文档](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/dev_docs/shelf-and-downloads.md#L3-L17)

### DNS / DoH 与线路测试

公开设置页面名为网络设置，实际是书源域名选择；`dns` 只是图标名称。`DomainNotifier` 保存域名、调用私有 API 的 `setDomain`、重新校验账户并使书源缓存失效。未在公开 `lib/`、Android/iOS 源码中找到自定义 DoH/DoT 解析器、DoH URL/Bootstrap 配置、DNS TTL 缓存或自定义 `HttpClient` 解析替换实现。私有 API 内部行为不在此次可核对范围内，不能断言其二进制完全没有 DoH，也不能把“多线路”误报为 DoH 支持。[网络设置](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/screens/settings/widgets/network_section.dart#L12-L59) · [域名切换](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/domain_provider.dart#L78-L131)

线路测速使用系统 Dart `HttpClient` 请求 `https://<domain>/eapi/info/languages`，最多同时 8 个。耗时包含连接、响应头和读取正文，优于单测 TLS 握手；但它将任何 HTTP 2xx 都判为可用，即使正文不是有效 EAPI JSON，因此不要复制这条健康判断。LuminaShelf 应继续对响应结构/业务成功标志进行验证，并限制批量测试并发。[并发调度](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/speed_test_provider.dart#L55-L106) · [实际探测与宽松判定](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/speed_test_provider.dart#L126-L156)

内置域名表是硬编码候选，不带发布者来源或独立官方隶属证明。本次未向这些站点发送任何凭据，也未将其批量加入 LuminaShelf。[候选列表](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/domain_provider.dart#L8-L70)

### Android 生命周期与存储

公开 Android Manifest 没有为下载声明前台服务；公开依赖中也未提供 WorkManager、后台下载任务插件或系统下载器封装。公开层是 Riverpod 生命周期内的异步网络请求，加已完成历史恢复，不能据此承诺应用被杀后自动继续后台下载。此次没有运行其 APK，也未检查第三方插件编译后合并 Manifest，因此结论限定为公开应用层。[Manifest](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/android/app/src/main/AndroidManifest.xml#L1-L85) · [依赖](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/pubspec.yaml#L30-L80) · [初始化](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L80-L92)

Olib 11+ Android 下载主动请求 `MANAGE_EXTERNAL_STORAGE`，以写公共 Download/Olib；它另有根据当前用户外部目录计算路径及退回应用目录的处理。用户当前明确不希望被突然授权打断，LuminaShelf 不应为借鉴这一交互引入全盘存储授权；保留应用管理目录，用户主动导出时才调用文档选择器即可。[权限请求](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L280-L303) · [多用户存储路径与退回](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/lib/providers/download_provider.dart#L147-L185)

## LuminaShelf 本轮应完成的行为

以下是基于用户反馈、上述源码缺口与 HTTP 标准的实现建议，不声称 Olib 已经提供。

1. 将下载任务、源提供方、书籍标识/格式、已完成字节、临时文件/分片位置、文件总长、校验信息及暂停/失败状态持久化。应用重新启动时将未完成任务恢复为可继续状态；网络断流保留部分文件，只有用户明确删除/重新下载或校验表明文件版本变化时才丢弃旧部分。完成后继续做现有格式校验，再原子提交至书库。
2. 下载页逐项提供“暂停 / 继续 / 重试 / 取消 / 移除”及明确更多菜单；长按整项打开同一管理菜单，失败项也能管理。把“保留进度暂停”和“取消/删除部分文件”的语义区分清楚。错误主体显示可采取的行动，技术详情可展开，避免截图里英文长错误占据整个卡片。
3. 续传按真实磁盘长度发起 `Range: bytes=<offset>-`。单段 206 必须有有效 `Content-Range`，并核对起点、终点、总长度、实际读取量及文件身份。`200` 表示服务器忽略 Range 或资源版本变化，不能 append 到旧部分；应安全切换到重新下载。`416` 也不能直接判完成，须核对服务端总长及完整文件。HTTP 标准允许服务器忽略 Range；界面应如实显示“不支持续传，将从头下载”。[Range](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.2) · [Content-Range](https://www.rfc-editor.org/rfc/rfc9110.html#section-14.4) · [206 单段](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.3.7.1) · [416](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.17)
4. 持久化强 ETag 或符合标准的时间验证值，并在可用时使用 `If-Range`；跨请求合并部分响应必须确认表示同一版本。刷新签名 URL 不代表内容身份已确认，源站/总长度相同也不能替代强验证。缺少服务端验证时，应采取保守策略，不能把新旧内容静默拼接成损坏文件。[If-Range](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.5) · [合并部分响应的强验证要求](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.3.7.3)
5. 单文件分段下载先实测 Range 支持，再按有界连接数请求不重叠分段，每段独立验证 206/范围/字节数/同一资源身份。失败保留已完成分段；服务端忽略 Range、拒绝分段或缺乏安全合并条件时自动退回单连接，并显示实际模式。多个文件队列并发与单文件连接数分别控制，不能仅增加按钮或配置数字。
6. 普通连接中断采用有限重试、退避并保留断点；签名链接的 401/403/404 等可能失效时通过原提供方重新取得链接，避免无限重试旧链接。登录失效、额度不足、验证码、无 Range 支持、文件版本变化与普通网络故障分别呈现。书源账户头不能泄漏给外部 CDN，签名 URL 不写公共日志。
7. DoH 优化继续参考已核对的 FluxDO 行为与本项目解析器：测试真实 DNS 响应及实际书源/CDN HTTPS 请求，区分“解析成功”和“文件传输稳定”；切换网络配置须更新解析缓存/连接池，对解析 TTL 和同域并发查询进行合理处理。不要把镜像延迟列表当作 DoH 实现，也不要在已有传输中断时只轮换 DNS 掩盖未实现的续传。
8. 必须增加真实本地 HTTP 夹具的中断/恢复验收：读取若干 MB 后强制断流 → 保留字节 → 再请求含正确 Range → 完成文件逐字节一致；以及进程重建恢复、暂停期间无持续写入、并发分段确实同时运行、不支持 Range 的 200、错误 Content-Range/长度、资源 ETag 变化、416 和签名链接刷新。下载页应独立验证触摸长按与可见按钮，不能沿用书架长按测试宣称下载管理通过。

## 许可证与不可核对范围

仓库 README 的 MIT 徽章与实际 LICENSE 不能混为一谈：固定版本文件标题为 **“MIT License with Additional Disclaimers”**，MIT 授权文本外还附有教育/个人用途、用户责任、免责、补偿等附加条款；GitHub API 因而标为 `NOASSERTION`。本次只记录行为并提出独立实现建议，没有复制其 Dart UI、源码或图片。以后若实质复制，需完整保留作者版权及原许可文本并核对附加条款，不能简单标成纯 MIT。[实际 LICENSE](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/LICENSE#L1-L57) · [README 声明](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/README_ZH.md#L113-L121)

`olib_api_plugin` 被 `.gitignore` 排除，注明来自 `shiyi-0x7f/olib-api-plugin` 私有仓库；CI 需要专用 SSH deploy key 才拉取。未读取该私有库，不能借由公开 UI 证明其 DNS、重定向、分段、格式校验或自动重试细节。也不应为了参考这个项目向用户弹授权或尝试绕过私有访问限制。[忽略与私有依赖说明](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/.gitignore#L85-L93) · [CI 私有依赖要求](https://github.com/shiyi-0x7f/olib-mobile/blob/0deb406d6da8d7b802e8d131755eba3eb1866b99/.github/workflows/build-android.yml#L13-L24)

本次未运行该项目测试/APK，未对其任何内置镜像进行登录下载，未作后台续传实机结论。Android 官方后台工作文档请求在本机超时，因此未用未读取页面充当后台行为的证据；后台实现仍应另行按实际产品代码和 Android 生命周期验证。

## 补充：本项目 Tauri 2 / Android 后台下载接入依据

补充核对日期：2026-10-15。随后通过 Android 官方中国镜像 `developer.android.google.cn` 成功读取英文文档；此前 `developer.android.com` 超时不再构成本节的资料缺口。本节是接入指导，没有添加服务实现、执行构建或运行模拟器。当前项目 `Cargo.lock` 固定 `tauri 2.12.3`；已直接核对本机 Cargo registry 的 Kotlin/Rust 原码，对应上游标签提交 [`5b23c82ea5ba6b9aec420dd71f60493edc486aa9`](https://github.com/tauri-apps/tauri/commit/5b23c82ea5ba6b9aec420dd71f60493edc486aa9)。Android 16 AOSP 权限/Service 源码另固定为 [`99b01a65cc4c104933788b3143285ab6bae65827`](https://github.com/aosp-mirror/platform_frameworks_base/commit/99b01a65cc4c104933788b3143285ab6bae65827)。

### targetSdk 36 的前台服务规则

- 用户主动开始的书籍上传/下载符合 `dataSync` 类型。Manifest 应声明 `android.permission.FOREGROUND_SERVICE`、`android.permission.FOREGROUND_SERVICE_DATA_SYNC`，服务声明 `android:foregroundServiceType="dataSync"` 与 `android:exported="false"`；运行时提升服务传入 `ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC`。这类服务没有额外运行时权限前提，两个权限均为正常权限，无须权限弹窗。[dataSync 用途与权限](https://developer.android.google.cn/develop/background-work/services/fgs/service-types?hl=en#data-sync) · [服务声明](https://developer.android.google.cn/develop/background-work/services/fgs/declare?hl=en) · [AOSP 正常权限](https://github.com/aosp-mirror/platform_frameworks_base/blob/99b01a65cc4c104933788b3143285ab6bae65827/core/res/AndroidManifest.xml#L7908-L7942)
- 必须从可见应用中用户点击“下载/继续”时启动服务，再开始可能耗时的下载链接查询和流传输。Android 12+ 一般禁止应用已在后台时新建前台服务；迟至 Activity `onPause/onStop` 再启动容易抛 `ForegroundServiceStartNotAllowedException`。启动过程是 `startForegroundService` → 服务尽快在 `onStartCommand` 调 `ServiceCompat.startForeground`，后者需要正整数通知 ID、有小图标的合法通知以及已声明的类型。不要等网络成功后才提升；官方排错文档要求几秒内完成。[后台启动限制](https://developer.android.google.cn/develop/background-work/services/fgs/restrictions-bg-start?hl=en) · [启动与提升](https://developer.android.google.cn/develop/background-work/services/fgs/launch?hl=en) · [未及时提升的错误](https://developer.android.google.cn/develop/background-work/services/fgs/troubleshooting?hl=en#internal-exception-fgs-did-not-start-in-time)
- **不需要申请 `POST_NOTIFICATIONS` 才能启动前台服务。**即使未获通知权限，仍须创建通知渠道、构建通知并交给 `startForeground`；Android 13+ 不在通知抽屉展示时，系统仍在 Task Manager 展示前台服务提示。targetSdk 36 不需要为了启动服务调用任何通知运行时授权请求。使用低重要性渠道/至少 `PRIORITY_LOW`、无声音/震动的进度通知即可，不申请 VPN、CA、全盘存储或电池白名单。[通知权限与 FGS 的明确例外](https://developer.android.google.cn/develop/ui/views/notifications/notification-permission?hl=en#declare) · [拒绝后的 Task Manager 行为](https://developer.android.google.cn/develop/ui/views/notifications/notification-permission?hl=en#exemptions) · [通知优先级](https://developer.android.google.cn/develop/background-work/services/fgs/launch?hl=en)
- target 35+ 的 `dataSync` 在后台累计最多 **6 小时 / 24 小时**，同一应用所有该类型服务共用；用户将应用带回前台会重置计时。API 35 新增 `Service.onTimeout(startId: Int, fgsType: Int)`，系统回调后只有几秒可 `stopSelf()`，否则导致内部异常/应用崩溃。须立即通知 Rust 暂停、释放 CPU 锁并停止服务；不能在主线程等待长时间网络清理。应依靠平时的下载检查点保障断点，而非等超时回调才保存。target 35+ 也不能从 `BOOT_COMPLETED` 接收器启动 `dataSync` 服务。[超时规则](https://developer.android.google.cn/develop/background-work/services/fgs/timeout?hl=en) · [dataSync 开机限制](https://developer.android.google.cn/develop/background-work/services/fgs/service-types?hl=en#data-sync) · [AOSP 回调签名](https://github.com/aosp-mirror/platform_frameworks_base/blob/99b01a65cc4c104933788b3143285ab6bae65827/core/java/android/app/Service.java#L1192-L1225)
- 前台服务不能让已经死亡的 Rust future 自动复活，也不能抵抗用户强制停止。Android Task Manager 的“停止”会移除整个应用进程且**不会发送回调**。任务元信息和实际部分文件必须在传输过程中持续保留；下次进程初始化把中断项恢复为可继续的暂停状态。仅作为当前 Rust 引擎保活外壳时，可用 `START_NOT_STICKY`，避免系统只重启一个没有 Rust 下载上下文的空服务；如将来要自动重建下载，需要另建完整持久调度入口，不能只改为 `START_STICKY` 就宣称支持。[用户停止语义](https://developer.android.google.cn/develop/background-work/services/fgs/handle-user-stopping?hl=en) · [AOSP 两种重启模式](https://github.com/aosp-mirror/platform_frameworks_base/blob/99b01a65cc4c104933788b3143285ab6bae65827/core/java/android/app/Service.java#L391-L437)

### CPU 唤醒锁与释放

需要熄屏继续实际传输时，服务可以持有 `PowerManager.PARTIAL_WAKE_LOCK`，Manifest 另加正常权限 `android.permission.WAKE_LOCK`；这不会点亮屏幕，也不需要用户授权。采用 `acquire(timeoutMillis)`，超时值有界，且仅在仍有有效传输时续租；最后一项停止、所有任务暂停、致命错误、`onTimeout`、`onDestroy` 均显式释放。不能无限期持锁或为了简单而常驻；官方特别要求 `try/finally` 释放，并建议把 CPU 锁放在有用户可见前台服务的工作内。[设置及有界 acquire](https://developer.android.google.cn/develop/background-work/background-tasks/awake/wakelock/set?hl=en) · [释放与最佳实践](https://developer.android.google.cn/develop/background-work/background-tasks/awake/wakelock/best-practices?hl=en) · [AOSP WAKE_LOCK 正常权限](https://github.com/aosp-mirror/platform_frameworks_base/blob/99b01a65cc4c104933788b3143285ab6bae65827/core/res/AndroidManifest.xml#L2761-L2768)

### 已安装 Tauri 2.12.3 的准确接口

Kotlin 使用 `app.tauri.annotation.TauriPlugin`、`app.tauri.annotation.Command`、`app.tauri.plugin.Plugin`、`app.tauri.plugin.Invoke`、`app.tauri.plugin.JSObject`。插件类的最小形状是带 `@TauriPlugin` 的 public 类，构造参数必须为一个 `android.app.Activity`，并继承 `Plugin(activity)`；Rust 注册通过 JNI 调用的构造签名正是 `(Landroid/app/Activity;)V`。可把类及服务编入本应用的 `app.luminashelf.client` 包，不必额外发布一个插件仓库。[注解](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/src/main/java/app/tauri/annotation/TauriPlugin.kt#L5-L19) · [Command](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/src/main/java/app/tauri/annotation/PluginMethod.kt#L5-L8) · [Plugin 基类](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/src/main/java/app/tauri/plugin/Plugin.kt#L45-L49) · [注册与构造](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin/mobile.rs#L206-L286)

命令形状为 `@Command fun start(invoke: Invoke)` 等 public 方法。参数可直接用 `invoke.getArgs(): JSObject`，或 `invoke.parseArgs(MyArgs::class.java)`；返回有 `invoke.resolve()`、`invoke.resolve(data: JSObject?)`、`invoke.resolveObject(data: Any)`，失败用 `invoke.reject(msg: String?)`。其中无参 `resolve()` 返回 JSON `null`，适配 Rust `::<()>`；返回对象则用匹配的反序列化结构或 `serde_json::Value`，不要让空对象与 Rust unit 类型互配。命令名是方法原名，不自动把 `start_download` 转为 `startDownload`。[Invoke 完整签名](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/src/main/java/app/tauri/plugin/Invoke.kt#L20-L91) · [按方法名索引与执行](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/src/main/java/app/tauri/plugin/PluginHandle.kt#L148-L167)

Rust 注册接口位于插件的 setup API，准确签名为 `PluginApi<R, C>::register_android_plugin(&self, plugin_identifier: &str, class_name: &str) -> Result<PluginHandle<R>, PluginInvokeError>`，仅 Android 编译。`tauri::plugin::Builder::new(name).setup(...)` 的闭包签名是 `FnOnce(&AppHandle<R>, PluginApi<R, C>) -> Result<(), Box<dyn std::error::Error>> + Send + 'static`；在这里注册 Kotlin 类，将得到的可 Clone `PluginHandle<R>` 包装到 managed state，再由下载控制器调用。Android 注册必须 `#[cfg(target_os = "android")]`，桌面路径保持可编译。[Plugin Builder setup](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin.rs#L432-L439) · [Clone handle](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin.rs#L124-L138) · [Android 注册签名](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin/mobile.rs#L206-L212)

同步调用准确签名为 `PluginHandle<R>::run_mobile_plugin<T: DeserializeOwned>(&self, command: impl AsRef<str>, payload: impl Serialize) -> Result<T, PluginInvokeError>`；它在 `rx.recv()` 阻塞当前线程。当前版本还提供同签名参数的 `async fn run_mobile_plugin_async<T: DeserializeOwned>(...) -> Result<T, PluginInvokeError>`，内部用 oneshot await，**异步下载入口优先用该方法**。同步方法不应在 Android 主线程调用；所有 Kotlin 命令尽快完成，不在命令内执行下载循环或文件大读写。[异步方法](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin/mobile.rs#L290-L321) · [同步阻塞方法](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin/mobile.rs#L323-L349) · [Android 调度](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/src/plugin/mobile.rs#L439-L508)

Tauri 的消费端 ProGuard 规则已保留 public `@TauriPlugin` 类的构造方法和 public `@Command` 方法；保持正确注解和可见性，不需要用新增 JS 全局桥绕开插件调用。[已有 R8 保留规则](https://github.com/tauri-apps/tauri/blob/5b23c82ea5ba6b9aec420dd71f60493edc486aa9/crates/tauri/mobile/android/consumer-rules.pro#L19-L29)

### 本仓库的最小接入顺序与验收边界

1. 将 Kotlin 插件/服务文件保存在跟踪的 `src-tauri/android/` 下，更新 `scripts/android-prepare.mjs` 复制这些文件与 Manifest 权限/服务声明；不能只改被忽略的 `src-tauri/gen/android/`，否则 Actions 干净 checkout 丢失实现。插件通过应用中已有 Tauri runtime 编入，不需要访问 Olib 私有插件或复制其实现。
2. 可见页面主动下发开始/恢复 → Rust await Kotlin `start` 短命令 → Kotlin 用 application context 启动服务，服务立即显示低重要性通知及按需 CPU 锁 → Rust 开始描述符获取和已有可恢复传输引擎。插件只传任务 ID/数量/进度，不在 Intent、通知或日志塞账户凭据、签名 URL 或临时文件内容。
3. 管理器维护“实际传输的任务 ID 集合”，第一项时启动，最后一项完成/暂停/失败/取消后停止并释放；不要只在开始时计数或只靠某个成功分支释放。并发任务结束顺序、重复点击、暂停全部、重试失败及前台服务启动失败都要覆盖，避免虚假后台支持状态和空服务常驻。
4. Rust 数据流与任务状态属于 Rust 管理器；Kotlin 服务只是当前进程的保活和系统通知外壳。超时/用户停止/进程死亡时须以持久任务和 `.part` 为准恢复；不得宣称不受 Android、厂商省电或强制停止影响。若 native callback 用来通知 Rust 暂停，其信号须线程安全且不阻塞 Android 主线程。
5. 需要在 API 33+ 拒绝/未申请通知权限、API 35/36 服务超时、切后台/熄屏、应用进程被终止再启动等情形验证“无授权弹窗、服务存在、锁按时释放、已下载字节保留、重新继续取得完整相同文件”。已有桌面 HTTP 夹具测试能证明恢复协议，不能证明 Android 服务生命周期已实测；构建成功和一个通知图标也不能替代这些验收。

## 补充：CI Android 下载暂停测试的模拟器限速

2026-10-15 阅读 Android 官方模拟器控制台文档，并核对本机 SDK `adb help` 与 AOSP 实现。未启动本地模拟器或执行任何限速。可在模拟器已经运行、应用页面已经加载时执行以下**宿主机控制命令**，无需设备 root、应用运行时权限或授权对话框：`adb -s emulator-5554 emu network speed 32`；同理 `64`；恢复为 `adb -s emulator-5554 emu network speed full`，读取配置为 `adb -s emulator-5554 emu network status`。设备序列号应使用当前 CI 的实际值。数值单位是 **kilobits/sec（千比特每秒）**，一个数值同时设置上/下行；`full` 为不施加模拟限速，实际仍受宿主网络限制。官方明确支持应用运行中动态修改。[官方 network speed/status](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network) · [adb emu 转发入口](https://github.com/aosp-mirror/platform_system_core/blob/5aa0e4241029c80617a8e98b07f7f48ab6b3a3d7/adb/client/commandline.cpp#L1631-L1632)

`adb emu` 通过本机模拟器控制台连接，并自动读取本地 `.emulator_console_auth_token` 完成协议认证；这是 CI 宿主进程的本地认证，不是 Android 应用授权弹窗。不要手工输出 token，也不需要执行 `adb root`。[AOSP 自动构造认证命令](https://github.com/aosp-mirror/platform_system_core/blob/5aa0e4241029c80617a8e98b07f7f48ab6b3a3d7/adb/client/console.cpp#L30-L52) · [控制台连接与执行](https://github.com/aosp-mirror/platform_system_core/blob/5aa0e4241029c80617a8e98b07f7f48ab6b3a3d7/adb/client/console.cpp#L94-L160)

**关键限制是网络后端。**当前官方文档把 `network` 控制标为 **Ethernet and Cellular only**，并注明 Emulator 36.5+ 默认 Wi-Fi 流量交给 netsim，传统控制台抓包只覆盖 QEMU slirp 的 cellular/WAN。不能假设默认 Wi-Fi 下载会因 `network speed` 返回 OK 就被限速。CI 应先确认模拟器使用受控的蜂窝/以太网路径，或明确选择并记录支持限速的后端；若关闭模拟 Wi-Fi，则要确认蜂窝数据真的可联网。新的高级网络文档另列 netsim 配置，不应自行虚构其“限速”参数。[官方后端限制和 36.5 说明](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network) · [netsim 官方配置](https://developer.android.google.cn/studio/run/emulator-networking-advanced?hl=en)

限速本身不区分 HTTP 和 HTTPS：已核对的传统实现修改收发两侧 `netshaper`，其调度按数据包字节数计算，不解析应用层正文或 TLS，所以**走受控外网路径的 HTTPS 下载同样受限**，不需要 CA、代理解密或 VPN。该源码固定为 Emulator 35.6.3 时期提交 `ae9d18d2b6261179fbd57fffec720a04f7bfb053`，不能用它保证新 netsim 后端也相同行为。传统实现还绕过源/目标都在 `10.x.x.x` 的 IPv4 内部流量，因此经 `10.0.2.2` 的本机夹具或代理可能不受这条限速影响；外网 Gutenberg 下载比本地 URL 更适合本项验证。[动态修改 shaper](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/console.cpp#L869-L897) · [按包字节调度及内部流量例外](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L28-L59) · [运行时变速和发送路径](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L195-L237)

建议 CI 顺序为：全速载入真实书籍详情 → 设置 32 或 64 并确认 OK/状态 → 点下载，观察真实已接收字节缓慢增长 → 暂停并确认字节停止增长/状态改变 → 下载页长按及可见菜单管理 → 继续并核对延续已有进度 → 最终始终恢复 `full`。约 189 KB 的文件在理想 32/64 kbps 下需约 47/24 秒，能比全速提供更稳定的操作窗口，但网络栈仍可能有初始突发，不能只靠固定 sleep 判断暂停成功。必须记录实际字节、状态和耗时；若几乎瞬间下载完毕，则本轮限速未被证明有效，应明确失败或改用已验证的可控夹具，不能将限速命令成功当作暂停/恢复功能通过。

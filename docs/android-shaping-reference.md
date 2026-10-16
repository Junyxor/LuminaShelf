# Android Emulator 限速后 TLS 连接失败：证据与鉴别实验

核对日期：2026-10-16。范围仅限 Android 官方文档与 AOSP 源码；本次没有运行模拟器、执行云端测试、改动下载实现/脚本或降低 TLS 校验。下文严格区分已有运行记录、已核对源码及尚待验证的假说。

## 当前运行现象与尚不能得出的结论

主任务提供的云端记录：Emulator `37.2.12.0`、build ID `16428233`，headless API 35，关闭 Wi-Fi 后使用 cellular；同一签名 release APK 不限速可完成约 795 KB 的 Gutenberg Moby EPUB，执行 `adb emu network speed 256` 后新连接反复在文件正文 0 B 时关闭。HTTP/2 报 `hyper Canceled: connection closed`，强制 HTTP/1 后仍有 rustls `peer closed connection without sending TLS close_notify`；连接/探测超时约为 25/30 秒。

这是“限速条件与失败相关”的有效对照，但不足以证明 Emulator 37 存在已确认回归，也不足以确认 Rust HTTP 栈、CDN 或队列中的哪一方首先出错。正文 0 B 不等于没有 TCP/TLS 流量；HTTP/1 仍失败使“仅 HTTP/2 协议兼容问题”的解释缺乏支持。不要因 `close_notify` 缺失关闭证书/主机名验证，也不要只靠扩大超时将测试变绿。

## 官方确认的限速规则与版本边界

`network speed <num>` 官方支持模拟器运行中动态调整。单位为 kilobits/sec，单个 `256` 同时设置上下行；`up:down` 可分别设置，`full` 为无限速；`network status` 只说明配置值，不能证明实际吞吐已经受控。已核对 AOSP 参数解析将数值乘以 1000，`full` 对应两方向 0，因此 `256` 是 256,000 bit/s，约 32,000 byte/s，并非 256 KB/s。795 KiB 的正文理想串行传输已约需 25.4 秒，另有 TLS、请求、协议头和竞争流量；这不解释握手必然失败，却说明 30 秒窗口不适合同时承载任意排队和整本下载。[官方控制台文档](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network) · [AOSP 解析实现](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/cmdline/src/android/network/constants.c#L20-L67)

当前官方文档明确将 `network` 控制标为 **Ethernet and Cellular only**；Emulator 36.5+ 默认 Wi-Fi 使用 netsim，传统控制台抓包仅覆盖 QEMU slirp 的 cellular/WAN。当前测试关闭 Wi-Fi 后走蜂窝，是合理的受控路径，但还应记录实际默认路由和 `network status`，不能仅以设置页图标作为后端证据。高级 netsim 文档没有给出可替换此命令的通用 TLS 限速参数。[控制台后端限制](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network) · [官方高级网络文档](https://developer.android.google.cn/studio/run/emulator-networking-advanced?hl=en)

官方发布说明确认 `37.2.12 Stable` 发布于 2026-10-01，其列出的修复是 macOS 截图、休眠和音频问题，没有列出“限速导致 TLS 失败”的修复或已知限制。`36.5.10` 发布说明把新网络栈描述为解决传统栈的数据丢失 `#150758736` 与连接掉线 `#449817856`；这证明网络后端差异有官方依据，**不能把这些历史问题认定为本次的同一个 bug**。本次未独立验证 build ID 与某个源码提交的对应关系。[37.2.12 发布说明](https://developer.android.google.cn/studio/releases/emulator?hl=en#37-2-12) · [36.5.10 网络栈变化](https://developer.android.google.cn/studio/releases/emulator?hl=en#36-5-10)

## 已核对的传统 netshaper 行为

以下源码固定为 AOSP GitHub 镜像 `emu-master-dev` 的提交 [`ae9d18d2b6261179fbd57fffec720a04f7bfb053`](https://github.com/aosp-mirror/platform_external_qemu/commit/ae9d18d2b6261179fbd57fffec720a04f7bfb053)，提交时间 2025-03-27，说明为 Bump Emulator to 35.6.3 Canary。该官方镜像没有可供本次直接核对的 37.2.12 分支；本机请求 android.googlesource 当前分支超时。**这是已确认的旧版本实现，不是对 37.2.12 二进制的反编译或精确源码核验。**

- 两个全局 shaper 分别控制输入与输出。调度以数据包字节数和实时时钟计算，包占用时间是 `bytes × 8 / rate`；同方向后续包排队等待定时器。实现没有按 TCP 连接公平分配，也不识别 HTTPS/TLS。系统后台流量会与应用共用相同的方向预算；不能把 256 kbps 理解成“单个下载独享”。[全局实例与调度说明](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L21-L60) · [排队路径](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L220-L269)
- 已到期包由定时器发送，仍有排队包则重新安排定时器；`num_packets` 只计数，所核对的这段路径没有按队列长度丢包的上限。大量竞争流量可能造成排队延迟，这是实现推导，尚无本次运行的队列/抓包证据。[到期发送](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L141-L170) · [入队及计数](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L241-L269)
- **动态变速会立即发出所有已有排队包，再更新速率/启用标记并重置 `block_until`。**因此它会改变正在进行连接的包节奏，可能形成突发；没有看到主动关闭 TCP socket、发送 RST 或重建 TLS 的操作。“改速必然破坏已有连接”没有源码依据；反过来，也不能假定变速瞬间完全没有影响。对照实验应在没有活动请求时先设速，再建立新连接。[控制台同时修改两方向](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/console.cpp#L869-L897) · [flush 与预算重置](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L195-L218)
- 对源/目标地址均为 `10.x.x.x` 的 IPv4 包，传统实现直接绕过限速。使用 `10.0.2.2` 的本机夹具或内部代理时，必须另验是否真正受控；外网 Gutenberg 请求更适合此次相同目标对照。[内部流量条件](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L28-L46) · [绕过位置](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L225-L230)

另发现一个**仅能确定存在于上述旧源码中的字段初始化风险**：`NetShaperRec` 含 `do_copy`，`netshaper_create(int do_copy, ...)` 使用 `malloc` 后没有将参数写入 `shaper->do_copy`；后续排队却读取该字段，决定复制包数据还是仅保留原始指针。源代码本身的这条未初始化读取可以定位，但是否影响当前 37.2.12、调用路径是否仍相同、是否实际导致本次 TLS 字节异常均未证实，不能据此宣布当前根因，也不应在应用层绕开 TLS 完整性检查。[字段](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L106-L118) · [构造函数完整范围](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L174-L192) · [复制/引用分支](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L71-L95) · [字段读取](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L241-L245)

## 现有云端 signed APK 的快速鉴别顺序

这些是待执行实验建议，不是已经通过的验收结果；无需重新构建 APK、设备 root、通知授权、VPN 或 CA。所有分支结束都恢复 `network speed full`，重新查询状态，并继续原应用验收；诊断失败不能替代或跳过真正的暂停/续传测试。

1. **先做平台 TLS 对照。**现有 `HttpsURLConnection` 诊断在同一 guest、同一固定 HTTPS 目标，以相同连接/读取超时，依次 `full → 256 → full` 完整读取正文并记录字节数、耗时、SHA-256、异常链；设速时确保没有在途请求，再建立新连接。若平台和 APK 都仅在 256 失败，应优先调查共同模拟网络/目标行为；若平台稳定通过而 APK 失败，再缩小到 Rust 客户端配置、连接池、DNS/地址与请求差异，不能立即断言某个库有错。确保最终响应是相同书籍字节，而不是 HTML 错误页。
2. **区分“启用 shaper”与“确实太慢”。**仅在需要时补测 `network speed 100000`（100 Mbps，仍启用 limiter）或 `8192`，对比 `full`。若很高的显式速率也失败而 `full` 成功，普通低带宽超时假说会减弱；若只有低速失败，继续看竞争流量、排队、服务端/客户端时限。命令返回 OK 和配置数值本身不是有效吞吐证据。[数值与 full 的差别](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/cmdline/src/android/network/constants.c#L20-L67) · [active 条件](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/android-emu/android/shaper.c#L209-L215)
3. **区分方向。**必要时分别测 `network speed 100000:256` 与 `256:100000`；语法为上行:下行。其结果可区分下行服务器握手/正文排队与上行 ClientHello/ACK 路径问题；因为两方向都不是 full，结果仍需结合上一个高显式速率对照解释。[官方双向速率语法](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network)
4. **区分稳态限速与改速瞬间。**同一模拟器空闲时先设 256 再重启应用/平台客户端，使请求从一开始就在稳态 limiter 下；与已建立连接中途变速的情况分开记录。若稳态也新连接 0 B，不能仅归因“动态改速打断已建 TLS”。不使用反复快速改速清队列作为正式验收绕过。
5. **需要进一步证据时抓取 cellular pcap。**控制台 `network capture start <bare-filename>`、`network capture stop` 无需 guest root；当前关闭 Wi-Fi 的路径适合传统抓包。抓握手前后，观察 TCP 重传/停顿、FIN/RST 发起侧和时序，再对齐平台/应用日志；TLS 加密记录不解密，也不从单个 FIN 推断证书验证或服务器主动策略。后台总流量明显增长而应用正文为 0 B，支持共享带宽竞争的调查方向；尚不能证明具体队列长度。pcap 也不应公开包含其他无关用户流量。[官方捕获命令与存放约束](https://developer.android.google.cn/studio/run/emulator-console?hl=en#network)

若平台对照同样失败，短期可保留明确失败的限速诊断，并用已经通过的本地可控 HTTP 夹具覆盖协议断流/Range 恢复、全速 release APK 覆盖实际 Android 下载/管理；但两者不能合并表述成“本次 Android 低速暂停续传已通过”。要声称完成该项，还需在有效限速环境中观察真实进度、暂停后字节稳定、继续后取得逐字节相同的完整文件。

## 资料访问限制

本次访问 android.googlesource 当前分支及上述 Google Issue Tracker 详情超时；搜索工具亦不可用。因此没有声称核对 37.2.12 的精确源码或关联缺陷详情。可用的一手证据为当前 Android 官方发布/控制台文档与明确固定的旧 AOSP 源码。旧源码用于解释机制和设计实验，不作为当前版本已确认漏洞或故障根因的证明。

## 补充：宿主回环 HTTP CONNECT relay 可作为独立限速路径

主任务新增运行证据：宿主 `tc` 的 pref 321 规则没有命中，原 `clsact` 已存在 pref 1 direct-action BPF `tc_ingress_traffic`；不调整、替换或插队该规则。应用全速约一秒下载完成后前台服务正常退出，稍后检查服务存在性的断言失败，不能据此判为产品 FGS 故障。替代方案应使传输真正持续足够时间后再检查服务，而不是让服务空转迎合断言。

**当前 Android 官方文档支持在模拟器启动时配置 `-http-proxy http://127.0.0.1:18888`。**这是 Emulator Proxy，在 Android OS 以下的网络层转发全部 TCP，不依赖应用是否遵循 Android 系统 HTTP 代理设置，因此覆盖 guest 内 Rust reqwest、原始 TCP/TLS 和 `HttpsURLConnection` 的 HTTPS/TCP 请求。官方明确 HTTPS 通过未解密 TCP 隧道传递，不能用它做 SSL inspection；不支持 UDP 重定向。无需安装 CA、关闭 TLS 校验、申请 VPN 或弹出应用授权。`127.0.0.1` 在这个参数中属于宿主 emulator 进程地址空间；与 emulator 同一宿主/网络命名空间中的 relay 可仅绑定回环。参数在启动时使用，不要求重编签名 APK。[官方命令行参数](https://developer.android.google.cn/studio/run/emulator-commandline?hl=en#network) · [官方明确区分 Emulator Proxy / Android System Proxy](https://developer.android.google.cn/studio/run/emulator-networking-proxy?hl=en)

已核对的旧 AOSP CONNECT 实现用 `sock_address_to_string(&root->address, ...)` 生成请求行，再连接配置的代理地址；`root->address` 来自待连接 socket 地址，而不是原始 URL。因此应预期 **CONNECT authority 为已经解析的数值 IP:port**，不能依赖收到原始书源域名。relay 需按首行 authority 解析 IPv4/IPv6 地址及端口、连接原目标；不进行反向 DNS 替换。这个具体编码结论来自固定旧源码，37.2.12 应通过实际 relay 首行记录核实。[保存目标 socket 地址](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_common.c#L62-L88) · [CONNECT 请求与代理连接](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_http_connector.c#L48-L79)

这一路径保留“guest 先解析目标，再请求 TCP 连接”的行为，而不是将普通目标域名交给 relay 解析。UDP DNS 不在 TCP 代理覆盖范围；应用自有 DoH 仍是 HTTPS/TCP 隧道，DoT 仍是 TCP 隧道。relay 只转发字节时，原 guest TLS ClientHello/SNI、证书链和主机名验证仍属于 guest 与源服务器之间的端到端 TLS，不需要 relay 知道 HTTP Host 或下载 URL。它改变了宿主侧建连路径，**不能称为完全相同的 TCP 链路**，但保留了实际 APK 的请求、DNS策略和 TLS验证；用于验收时应明确记录该替代网络环境。[官方 TCP 隧道与 UDP 边界](https://developer.android.google.cn/studio/run/emulator-networking-proxy?hl=en) · [只接受 TCP socket](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_http.c#L32-L64)

relay 接入注意事项：旧实现 CONNECT 生成过程不保证额外 `Host` 头，不能要求 Host 才接受隧道；应以请求行 authority 为准。在上游连接成功后返回合法的 2xx CONNECT 响应和头结束空行，再按序双向原样转发所有后续字节，保留 EOF/半关闭语义，不自行生成 TLS 记录。旧 QEMU AUTO 对目标 80 端口使用 HTTP rewriter，其余端口使用 CONNECT；因此 **CONNECT-only relay 适用于本次 HTTPS 测试，但不是完整的通用 HTTP 代理**。普通 HTTP/Android 网络连通性探测若经过此路径，需支持正常 HTTP 转发或至少记录拒绝情况，避免把代理能力缺口当作应用故障。[CONNECT 请求/响应处理](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_http_connector.c#L58-L62) · [响应 2xx 与头结束](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_http_connector.c#L118-L150) · [80 端口特殊路径](https://github.com/aosp-mirror/platform_external_qemu/blob/ae9d18d2b6261179fbd57fffec720a04f7bfb053/android/emu/proxy/src/android/proxy/proxy_http.c#L49-L64)

该方案无需任何 `tc`/BPF 规则变更：保持 legacy `network speed full`，仅在宿主 loopback relay 对所有隧道的**共享下行密文字节**实行约 32 KB/s 预算，上行正常转发以减少 ACK/ClientHello 的人为干扰；若目标是双向共享总预算，须另外明确，不能把每连接 32 KB/s 误报为全局限速。relay 的计数字节含 TLS 开销，不能与 APK 正文进度直接要求完全相等。先用同一目标做 relay 不限速对照，确认平台客户端与 APK 都能完整读取，再启用预算；记录 CONNECT 目标、已转发字节与时间，观察实际应用进度/服务状态，暂停、后台、继续后校验完整书籍。这样获得的是有独立吞吐证据的替代限速验收，不是对旧 shaper 正常工作的证明。此段是基于已核对代理机制的实验设计，本次未运行 relay 或宣称测试通过。

## 补充：持续进度更新时避免 uiautomator dump 的等待

新增运行记录来自主任务：relay 的 `limitedBytes=824937`，服务实际从 `06:29:28.9` 持续到 `06:29:55.2`，书籍慢速传输约 27 秒完成；系统 `uiautomator dump` 到 `06:29:57` 才返回完成状态的 UI 树，错过活动期。这些记录已证明本轮存在有效限速及实际服务持续时间；晚到的 UI 快照不能否认传输期间服务存在，也尚未证明暂停/继续交互已通过。

直接核对 Android 15/API 35 AOSP 标签 `android-15.0.0_r1`，固定提交 [`4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3`](https://github.com/aosp-mirror/platform_frameworks_base/commit/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3)：`DumpCommand` 在连接自动化桥后，**无条件调用 `uiAutomation.waitForIdle(1000, 1000 * 10)`**，之后才获取活动窗口根节点。官方定义的 idle 是连续一段时间没有收到无障碍事件；不断发出的进度/内容变化事件可能使一秒安静窗口一直不成立。单次等待全局最多十秒，随后抛 TimeoutException，系统命令输出不能获得 idle 的错误；27 秒总延迟不能只归于一次十秒调用，还可能含调用启动和外层重试。[DumpCommand 等待及超时处理](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/cmds/uiautomator/src/com/android/commands/uiautomator/DumpCommand.java#L77-L118) · [官方 waitForIdle 契约](https://developer.android.google.cn/reference/android/app/UiAutomation?hl=en#waitForIdle(long,long)) · [AOSP 等待实现](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/core/java/android/app/UiAutomation.java#L1223-L1259)

拟议的 shell DEX 路径有官方实现依据：系统 `uiautomator.sh` 本来就把 `/system/framework/android.test.runner.jar` 和 `/system/framework/uiautomator.jar` 放入 CLASSPATH，再通过 `app_process` 启动命令。前置本次自有 DEX，保留这两个系统 jar，在 `adb shell` 的 shell 身份中执行独立导出程序，无须安装新 APK、设备 root 或用户授权弹窗。它依赖系统工具内部类，适用于这份 AOSP API 35 系统镜像；不是给普通第三方 APK 承诺的稳定公开 SDK。[官方 shell 启动 classpath](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/cmds/uiautomator/uiautomator.sh#L61-L61) · [官方 app_process 入口](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/cmds/uiautomator/uiautomator.sh#L127-L129)

准确类名 **`com.android.uiautomator.core.UiAutomationShellWrapper`** 存在，为 public、无显式构造器，具有隐式 public 无参构造器；public 方法有 `connect()`、`getUiAutomation(): android.app.UiAutomation`、`disconnect()`、`setCompressedLayoutHierarchy(boolean)`。其源码实际位于 `library/testrunner-src/`，不是 `core-src/`，并由 `uiautomator.core` 静态编入系统工具。`connect()` 内部已经调用 `Looper.prepareMainLooper()`，自有 `main` 不要预先重复调用；连接后直接取 `getUiAutomation().getRootInActiveWindow()`，**不调用 waitForIdle**。[wrapper 类与 connect](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/library/testrunner-src/com/android/uiautomator/core/UiAutomationShellWrapper.java#L14-L37) · [其余准确方法](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/library/testrunner-src/com/android/uiautomator/core/UiAutomationShellWrapper.java#L66-L85) · [工具静态依赖](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/cmds/uiautomator/Android.bp#L27-L32)

`UiAutomation.getRootInActiveWindow()` 自身没有调用 idle 等待，而是向 `AccessibilityInteractionClient` 获取窗口树，因此可绕开上述人为十秒等待；它仍可能返回 null、在桥刚连接时抛异常或有 IPC 延迟，不能保证瞬时返回。采用短而有界的 root 获取重试，记录快照开始/结束时间，最终 `disconnect()` 释放桥，避免与另一个同时运行的 dump/自动化会话争用。获取过程中界面仍可变化，所得树不应被称为所有节点同一瞬间的原子状态。[根节点 API 实现](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/core/java/android/app/UiAutomation.java#L916-L947) · [系统命令关于桥刚连接的提示](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/cmds/uiautomator/src/com/android/commands/uiautomator/DumpCommand.java#L86-L100)

自有 `XmlSerializer` 可独立递归输出 `text=getText()`、`content-desc=getContentDescription()`、`class=getClassName()`、`enabled=isEnabled()`，以 `getBoundsInScreen(Rect)` 输出 bounds；这些字段都有官方 dumper 的对应依据。为保持用于点击测试的含义，还应过滤不可见子节点、处理 null 子节点及 null 文本，并确保 XML 字符转义有效。官方 dumper 的 bounds 会裁剪至可见屏幕，直接输出原始 bounds 不完全等价，点击时须校验坐标确实在屏幕内。此处只核对方案，没有改动测试脚本或执行本地 UI。[官方节点字段与可见性处理](https://github.com/aosp-mirror/platform_frameworks_base/blob/4e43ad12e1b211f15152e9c5b16b0fe88e6b93f3/cmds/uiautomator/library/core-src/com/android/uiautomator/core/AccessibilityNodeInfoDumper.java#L197-L235)

## 实验设施与首轮执行边界

工作流可选 `network_diagnostics` 编译一个独立 Android 平台 TLS 探针，在同一 guest 对公开 Moby EPUB 分别执行单字节 Range 探测和整本下载，记录 HTTP 状态、Content-Range、字节、SHA-256 和耗时。首轮运行 `38857754038` 的宿主 Gutendex 元数据请求遭 HTTP403，平台探针尚未执行；可选诊断失败被记录，原 APP 验收仍执行并在传统限速下失败。不能将这轮结果表述成平台 TLS 同样失败。随后去掉元数据依赖，固定官方 API 刚核对的公开 EPUB 地址。

运行 `38858663120` 的平台对照实际完成：全速 Range 返回206、1字节、538ms；整本返回200、814023字节、639ms，SHA-256为`1bc88e79ef7cf9d5325abe0071e6302265b708df56d3af07db124c81e84197f6`。传统256kbps下两次请求均0字节，约22.5秒后出现`java.io.IOException: unexpected end of stream`；异常链来自 Android 系统 OkHttp，见该运行的`network-control/`和`logcat.txt`。这支持共同限速路径/目标行为的调查方向，不能证明具体模拟器源码缺陷。

另提供 `bandwidth_shaper=host`，在一次性 Linux Actions runner 创建自有 IFB和独立 ingress filter，通过内核 TBF 将实际 IPv4 HTTPS 下行限制为256kbit/s。它不修改 APK、DNS 或 TLS；保留已有 ingress/clsact及其过滤规则，仅清理本次标记为自有的资源，创建新规则前拒绝占用的优先级或设备名。原有实际字节、后台进度、暂停、进程恢复、长按和阅读断言保留，另检查实际 tc 流量计数。首轮因已有 ingress 而主动拒绝覆盖；改为共存后该后端是否可用、是否有效限速以及 APP 是否通过，仍须以新的云端运行和证据为准。

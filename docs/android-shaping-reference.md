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

## 实验设施与首轮执行边界

工作流可选 `network_diagnostics` 编译一个独立 Android 平台 TLS 探针，在同一 guest 对公开 Moby EPUB 分别执行单字节 Range 探测和整本下载，记录 HTTP 状态、Content-Range、字节、SHA-256 和耗时。首轮运行 `38857754038` 的宿主 Gutendex 元数据请求遭 HTTP403，平台探针尚未执行；可选诊断失败被记录，原 APP 验收仍执行并在传统限速下失败。不能将这轮结果表述成平台 TLS 同样失败。随后去掉元数据依赖，固定官方 API 刚核对的公开 EPUB 地址。

另提供 `bandwidth_shaper=host`，在一次性 Linux Actions runner 创建自有 IFB/ingress，通过内核 TBF 将实际 IPv4 HTTPS 下行限制为256kbit/s。它不修改 APK、DNS 或 TLS；拒绝覆盖已有策略，仅清理本次标记为自有的资源。原有实际字节、后台进度、暂停、进程恢复、长按和阅读断言保留，另检查实际 tc 流量计数。该后端是否可用、是否有效限速以及 APP 是否通过，仍须以新的云端运行和证据为准。

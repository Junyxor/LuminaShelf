# LuminaShelf · 星书

面向 Android 的电子书应用，使用 Tauri 2 + React 界面和 Rust 核心。
0.6.0 将搜索、可恢复下载、本地书库与 EPUB / TXT / PDF 阅读整合到手机端。

本地交付包：`artifacts/android/LuminaShelf_0.6.0_arm64-debug.apk`。
Android 15 模拟器验收记录见 [docs/ANDROID-VALIDATION.md](docs/ANDROID-VALIDATION.md)。

## 手机上的主要流程

- 「搜索」查找书名、作者或关键词。公开书源 Gutendex 无需登录；
  Z-Library 需要可用的 EAPI 地址和账户。
- 「下载」查看持久任务，在连接或传输期间暂停/取消，重启后继续未完成任务。
- 「书库 → 导入电子书」打开 Android 系统文件选择器，支持本地文件和云文件提供者。
  导入会复制文件，不依赖长期的 content URI 授权，也不修改原文件。
- 在书库阅读 EPUB/TXT/PDF。章节或页码会保存；安卓返回键先关闭阅读器或详情页。
- 文件保存在应用私有目录，无需“所有文件访问”权限。卸载应用会移除应用内书籍和进度。

## 下载与账户

下载任务、书库、阅读位置存于 SQLite。重启后未完成任务恢复为暂停状态；
继续时重新获取下载地址，复用已完成的数据。同一任务在旧写入者完全停止并保存
状态前不能启动下一次下载。

大文件使用有界并发 Range 下载。HTTP 长度和 Content-Range 会被检查，EPUB/PDF
容器验证通过后才进入书库。临时文件不会被扫描成已完成的书，已有不同内容的
文件不会被覆盖；无效内容会被清理，以便下一次真正重新下载。

Android 的系统 DNS 使用平台解析接口，不依赖桌面上的 resolv.conf。
应用还支持自定义 DNS、DoH、DoT 和应用内 Hosts。TLS 主机名校验保持启用，
应用内 Hosts 不修改操作系统配置。

Android 会话通过 Keystore 保护的加密存储保存。密码不持久化。
保存/删除操作会检查磁盘提交结果，退出登录后清除会话。
下载队列不保存临时 URL 或请求头，新分段清单保存 URL 指纹。

## 构建 APK

需要 Node.js 24+、当前 Rust stable、JDK 17、Android SDK Platform 36、
Build Tools 35+、Android NDK，以及对应 Rust target：

```bash
npm ci
rustup target add aarch64-linux-android
npm run android:init
npm run android:apk
```

设置 `JAVA_HOME`、`ANDROID_HOME`、`NDK_HOME`。
初始化脚本会补充 Android Keystore 所需的 JNI 初始化，不要跳过
`scripts/patch-android-keyring.mjs`。

Windows 没有符号链接权限时，使用复制原生库的构建脚本，无需修改系统权限：

```powershell
npm run android:apk:windows
# 额外生成供模拟器验证的 x86_64 版本
./scripts/build-android-windows.ps1 -Targets aarch64,x86_64
```

APK 输出：`src-tauri/gen/android/app/build/outputs/apk/arm64/debug/app-arm64-debug.apk`。
这是签名调试包，适合安装测试。正式商店发布需要发布签名和发行配置。
现有 Android CI 可生成 arm64 APK artifact；仅明确选择发布时才创建 prerelease。

## 验证

```bash
npm test
npm run build
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Android 设备测试在已安装调试 APK、启用 ADB 的设备或模拟器运行：

```bash
# ANDROID_SERIAL 选择测试设备，默认 emulator-5556
npm run test:android
```

测试操作实际 Android DocumentsUI 和 WebView，覆盖 SAF 导入、三种阅读器、
返回键、强制停止后的书库/位置恢复，以及独立非敏感测试项的 Keystore
保存/恢复/删除。测试生成的书籍、截图和结果位于 `artifacts/android/smoke/`。
安全存储探针仅在 Android debug 构建可用，不接触账户凭据。

## 当前边界

- 尚无 Android 前台下载服务；系统终止应用后传输停止，下次打开可继续。
- EPUB 当前侧重文本与章节阅读，尚未完整呈现插图、媒体和原版排版。
- MOBI / AZW3 / CBZ / DjVu 可导入登记，尚无内置阅读器。
- 公开与认证书源依赖外部服务；不包含 HTML/JS 反机器人挑战绕过。
- 「移除记录」仅删除任务历史，不删除已下载书籍或保留的断点数据。
- 桌面代码保留兼容，但本次交付和验收目标是 Android APK。

验收范围见 [docs/DELIVERY.md](docs/DELIVERY.md)。

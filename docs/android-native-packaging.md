# 原生库压缩与 16 KB 页兼容备忘

核对日期：2026-10-16。结论：APK 可使用 `jniLibs.useLegacyPackaging = true` 压缩原生库，但这只改变打包/安装方式，不能替代原生库自身的 16 KB ELF 对齐。

`useLegacyPackaging=true` 表示压缩 APK 中的 `.so`。AGP 8.11 官方 API 说明：该选项为空且 `minSdk >= 23` 时，默认使用未压缩、页对齐的 `.so`。Manifest 的 `android:extractNativeLibs` 控制安装器是否将库提取到文件系统；`false` 对应未压缩库直接从 APK 加载。Android 官方要求 AGP 4.2 起优先用 Gradle DSL 配置，替代手写这个 Manifest 属性。因此应让 DSL 与最终 Manifest 的提取行为一致，并核对产物，不能只改 Manifest 就宣称 `.so` 已压缩。[AGP 8.11 API](https://developer.android.google.cn/reference/tools/gradle-api/8.11/com/android/build/api/dsl/JniLibsPackaging?hl=en#useLegacyPackaging()) · [Manifest 属性说明](https://developer.android.google.cn/guide/topics/manifest/application-element?hl=en#extractNativeLibs)

16 KB 兼容有两个独立检查：

- 未压缩 `.so` 从 APK 直接映射时，需要 ZIP 内文件起点的 16 KB 对齐。官方推荐 AGP 8.5.1+；也明确列出 `useLegacyPackaging=true` 压缩后安装时提取，作为避免未对齐 ZIP 原生库安装问题的替代方式。[官方打包要求](https://developer.android.google.cn/guide/practices/page-sizes?hl=en#update-packaging)
- 提取后的 `.so` **仍必须满足 16 KB ELF 段对齐及运行时要求**，预编译依赖也要核查。压缩或 `extractNativeLibs=true` 不会修复 ELF。`zipalign -P 16` 针对未压缩 `.so`，所以压缩包通过 zipalign 不能证明 ELF 兼容；应解出各 ABI 的实际库单独检查，再做设备运行验证。[官方 ELF 要求](https://developer.android.google.cn/guide/practices/page-sizes?hl=en#compile-16-kb-alignment) · [zipalign 适用范围](https://developer.android.google.cn/tools/zipalign?hl=en)

代价是**下载 APK 可变小，安装占用不能按 APK 大小估算**：压缩库安装时要解压并复制到磁盘，官方明确警告这会增加安装空间需求，低剩余空间设备可能更容易安装失败。不能据此承诺固定增加多少 MB；当前包须实际测量。ARM64/ARMv7 独立包可减少随包携带的无关 ABI 内容，但不会取消所选 ABI 的解压成本。当前工程生成配置是 AGP 8.11.0，压缩属于体积与安装空间的取舍，不是因为 AGP 太旧而必须使用的兼容修复。[官方空间代价警告](https://developer.android.google.cn/guide/practices/page-sizes?hl=en#agp_version_85_or_lower)

来源边界：本次成功读取的是上述 Android 官方中国镜像中的页面、AGP 8.11 API 和 zipalign 文档，并只读核对了工程 AGP 版本。`developer.android.com` 原域超时来自主任务记录，本次未将原域视为已读取；没有取得或引用 AGP `JniLibsPackaging.kt` 的具体实现源码，也未对 0.8.1 构建产物作验证。上述结论只针对本轮 APK 打包，不能直接当作 AAB/Play 生成 APK 的验收结果。

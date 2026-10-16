# 应用公告、书源活动与版本更新的数据来源

核对日期：2026-10-14。研究只做匿名公开读取，没有登录、捐赠、发送、安装或修改外部数据。网页尝试采用 GPU 禁用的无窗口 Chromium；没有启动可见浏览器或弹出权限窗口。

## 已取得的书源公告字段

当前配置内可响应的 `https://zlib.bz/eapi/info` 现场根字段为 `success, books, articles, plans, languages, extensions, banner, service`。默认语言与 `?switchLanguage=chinese` 均返回以下部分结构。[1][2]

```json
{
  "success": 1,
  "banner": "",
  "service": {
    "supportEmail": "support@z-lib.fm",
    "desktopApps": {
      "windows": "https://dln1.ncdn.ec/general-files/soft/desktop/latest.yml",
      "macos": "https://dln1.ncdn.ec/general-files/soft/desktop/latest-macos.yml",
      "linux": "https://dln1.ncdn.ec/general-files/soft/desktop/latest-linux.yml"
    }
  }
}
```

`banner` 是当前唯一已证实的公告候选字段，现场为空，不能编造“正在募捐”等活动。其非空值的实际内容类型、HTML 结构、发布时间和活动链接本次未取得样本；不能把它当作已有固定文章列表 API。`service.desktopApps` 指向 **Z-Library 自己的桌面应用**，不能拿它判断 LuminaShelf 版本。`supportEmail` 是客服联系方式，不能当作收款地址或支付指引。[1]

请求和显示应保留所选书源域名、完整响应来源、刷新时间和错误状态。当前响应域名及目录中的全部域名没有在本次研究中逐一独立证明所有权；UI 可以准确称为“当前书源公告”，不应无依据为每个用户自填站点颁发“官方认证”。

网站首页、`/blog` 以及博客子域公开 HTML 当前遇到浏览器验证/拒绝；普通无窗口浏览器最终只取得“Try again later or contact us support@z-lib.fm”，没有公告、博客、RSS 或捐赠导航链接。本次未发现可一手核实的新闻列表、博客 RSS 或活动接口。不能硬编码猜测的活动标题或支付目的地。[3]

## 捐赠和活动的边界

匿名读取 `/eapi/user/donations` 返回 HTTP 400、JSON `{success:0,error:"Please login"}`。它是用户相关接口，不能当作公开募捐活动列表。[4] `/eapi/info/plans` 返回方案能力和捐赠金额对应权益的说明，但没有当前活动标题、时间、活动 URL、支付账户或付款码；方案金额不是正在举办活动的证据。[5]

研究中猜测的旧路线 `/howtodonate.php` 返回 HTTP 404，因此不应作为可用捐赠入口出货。[6] 在没有真实公告链接时，可以展示“当前没有取得书源公告”并提供所选书源首页；不能把首页包装成已验证的专用捐赠页面。

后续如果真实 `banner` 或实际首页导航公布了活动，应从原文提取标题、正文和链接，显示原始来源与更新时间。HTML 必须清理，不执行服务器返回的脚本、iframe 或内联事件；链接仅接受可展示给用户的 HTTPS 地址。捐赠/账户/支付类链接应由用户主动打开，不由应用在刷新时跳转、登录、付款或启动授权。书源活动、LuminaShelf 开发者公告与 LuminaShelf 支持方式应各自注明主体，不能混用捐赠目标。

## LuminaShelf 自己的公告来源

GitHub REST 确认 `Junyxor/LuminaShelf` 是公开仓库，默认分支 `main`，目前没有设置仓库 homepage，Discussions 未启用。[7] 递归检查当前 `main` 文件树没有应用公告/通知/新闻订阅文件。[8] 因此不存在已经发布且可声称“已接入”的应用公告 JSON。

可新增由项目维护的静态公告文件，并在包内提供相同格式的初始副本；远端 URL 必须等该文件真实发布后才能视为在线来源。建议字段为 `id`、`title`、`body`、`publishedAt`、可选 `expiresAt`、`url` 与 `kind`。这是 **LuminaShelf 自有格式建议**，不是 Z-Library 的现有接口。公告可以准确说明当前版本的功能、已知限制和更新说明；没有实际发布的功能不能写成已经上线。

GitHub Release 的 `body` 也可作为本应用发布公告，但需要真实存在的 Release，不能把本地构建成功当作 GitHub 已发布。应用公告可显示缓存并注明上次成功刷新；刷新失败不应清空最后的有效公告，也不能显示“没有公告”掩盖网络失败。

## LuminaShelf 检查更新

可读取固定仓库端点：

- `GET https://api.github.com/repos/Junyxor/LuminaShelf/releases/latest`：当前现场 HTTP 404、`Not Found`。[9]
- `GET https://api.github.com/repos/Junyxor/LuminaShelf/releases?per_page=5`：当前现场 HTTP 200、空数组 `[]`。[10]

结合公开仓库存在的事实，这代表当前没有可供客户端检查的已发布 Release，不能显示“0.6.0 已是官网最新版”，也不能把 404 等同于断网。网络超时、权限错误、限流和“尚无发布版本”应分别显示。

GitHub 官方协议给出 Release 的 `tag_name`、`name`、`body`、`draft`、`prerelease`、`published_at`、`html_url` 和 `assets`。资产包含 `name`、`state`、`size`、`content_type`、`browser_download_url` 以及可用时的 `digest`（例如 `sha256:...`）。`latest` 只针对非草稿、非预发布版本；GitHub 文档说明排序依据涉及 `created_at`，不能自行假定返回“最后上传文件”的版本。[11]

实现建议：固定项目 owner/repo；解析语义版本后与当前应用版本比较，不能按字符串比较 `0.10.0` 与 `0.9.0`；选择上传完成且适合 Android 的实际 `.apk` 资产，展示版本、发布日期、大小、变更说明、来源与下载按钮。缺少 APK 时显示“该发布没有 Android 安装包”。`digest` 存在时核验下载内容；系统仍会要求更新 APK 与已安装应用签名身份兼容，必须沿用当前项目签名证书。[11][12]

检查更新本身只做 HTTP，不调用安装器、不弹权限对话框。下载与安装应是用户明确触发的后续动作，刷新公告或进入设置不应自动启动它。不要误用 Z-Library 的 `latest.yml`、其他项目 Release 或第三方下载中转作为本应用更新来源。

GitHub 官方建议使用条件请求（`ETag` / `If-None-Match` 等），并在限流时遵循 `Retry-After`、`x-ratelimit-reset`，避免持续请求。[13] 设置里的“检查更新”可手动刷新并复用短期缓存；后台自动检查不应抢焦点。

## 来源

1. [当前书源 EAPI 信息](https://zlib.bz/eapi/info)，匿名现场 JSON。
2. [中文参数下的 EAPI 信息](https://zlib.bz/eapi/info?switchLanguage=chinese)，匿名现场 `banner:""`。
3. [当前书源首页](https://zlib.bz/)，现场 HTML 浏览器验证与无窗口读取结果；没有取得可引用的公告链接。
4. [匿名 donations 接口](https://zlib.bz/eapi/user/donations)，现场 HTTP 400 `Please login`。
5. [书源方案信息](https://zlib.bz/eapi/info/plans)，匿名现场功能与金额说明。
6. [研究中核对的旧路线](https://zlib.bz/howtodonate.php)，现场 HTTP 404；不是建议使用的 URL。
7. [LuminaShelf 仓库元数据](https://api.github.com/repos/Junyxor/LuminaShelf)，GitHub 官方 REST。
8. [LuminaShelf main 文件树](https://api.github.com/repos/Junyxor/LuminaShelf/git/trees/main?recursive=1)，GitHub 官方 REST。
9. [LuminaShelf 最新发布接口](https://api.github.com/repos/Junyxor/LuminaShelf/releases/latest)，现场 HTTP 404。
10. [LuminaShelf 发布列表](https://api.github.com/repos/Junyxor/LuminaShelf/releases?per_page=5)，现场空数组。
11. [GitHub REST — Releases](https://docs.github.com/en/rest/releases/releases#get-the-latest-release)，最新发布规则与 Release/asset 字段。
12. [Android — Sign your app，官方中国站](https://developer.android.google.cn/studio/publish/app-signing?hl=en)，现场核对更新与签名密钥身份；主站读取超时。
13. [GitHub REST 最佳实践](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api)，条件请求与限流处理。

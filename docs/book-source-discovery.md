# 书源发现、首页与下载协议核对

核对日期：2026-10-14。只发送匿名公开 HTTP 读取请求，没有登录账户、发送邮件、保存网盘、修改远端资料、打开浏览器窗口或修改系统网络。以下区分现场服务器响应、服务维护者文档和第三方客户端线索；当前网络可达性不是域名所有权证明。

## Z-Library 域名目录与可信来源

本机当前配置内的 `https://zlib.bz/eapi/info/domains` 返回 HTTP 200、`application/json`，正文如下。[1]

```json
{"success":1,"domains":[
  {"domain":"z-library.sk","contentAvailable":true,"isRedirector":false},
  {"domain":"1lib.sk","contentAvailable":true,"isRedirector":false},
  {"domain":"article.sk","contentAvailable":true,"isRedirector":false},
  {"domain":"articles.sk","contentAvailable":true,"isRedirector":false},
  {"domain":"proxy.zlibraryproxies.workers.dev","contentAvailable":true,"isRedirector":false}
]}
```

这是服务器提供的域名目录，其条目不是字符串数组。解析应检查 `success == 1`、`domains` 数组与非空有效主机名，保留 `contentAvailable`、`isRedirector`。`success:0`、HTML 挑战页或任意非空 JSON 不能判为可用。设置列表应记录响应源 URL、拉取时间、发现方式、是否内容站/跳转站，以及随后逐站探测的结果；是否被目录列出和是否实际可达是不同信息。

不能仅凭一个站点返回这个 JSON 就把该站点及所有结果标为“已独立核实官方”。本次对其他原有种子 `z-lib.gd`、`z-lib.gl`、`z-library.ec` 以及返回目录内若干站点的匿名 HTTPS 请求未连通，不能宣称已验证所有镜像。`zlib.bz` 的首页与公开书籍 HTML 页返回 HTTP 503 的浏览器验证页，而 EAPI 公共读取正常。[1][2]

本次没有找到具有可验证第一方反向链接的 Z-Library 官方 GitHub 域名发布仓库。GitHub 上的社区地址列表可以作为用户自行添加的订阅，必须显示仓库/文件 URL 与“社区来源”，不能自动升级为官方目录。以用户选定的可信书源目录为起点，逐层发现、去重、限量健康检查即可，不应递归抓取网络中所有外链。探测和目录拉取无需携带账户标识或 cookie。

## Z-Library 首页与书籍详情

以下端点在当前响应站点进行了匿名实测。[3][4][5][6]

- `GET /eapi/book/most-popular` 返回 `{success:1,books:[...]}`，现场为 100 张真实热门书籍卡片。每项有 `id`、`hash`、`title`、`author`、`cover`、`url`、`isbn`；**没有**文件格式、文件大小或语言。可作为“网站热门”首页数据，不能直接假定为 EPUB。
- `GET /eapi/user/book/recommended` 返回 `{success:0,error:"Please login"}`。这是账户推荐接口，匿名状态应显示登录提示，不得拿热门或普通搜索结果冒充个人推荐。
- `GET /eapi/book/recently` 在现场只返回 `{success:1}`，无 `books`。不能从端点名称推断为“最新上架”，也不能虚构该栏的数据。
- `GET /eapi/book/{id}/{hash}` 返回 `{success:1,book:{...}}`。样本 `28704955/49edaa` 包含 `extension:"epub"`、`filesize:2343976`、`sha256`、`md5`、`dl:"/dl/0j0OJaZpgD"`、`href`、`description`、`kindleAvailable:false`、`sendToEmailAvailable:false`、`readOnlineAvailable:true`。也包含签名在线阅读 URL；这类临时 URL 不宜写入公共日志或文档。
- `GET /eapi/book/{id}/{hash}/formats` 的两个现场样本均返回 `{success:1,books:[]}`。已确认它是 `books` 信封；没有证据表明这是当前书籍可转换的目标格式列表。空结果不得生成虚假的 PDF/MOBI 转换按钮。

实现上，首页卡片进入详情时应读取真实详情接口，再依据真实文件格式、可用动作与书籍标识发起下载。当前仅从搜索缓存读取详情的逻辑无法完整支持上述首页卡片。标题中现场存在 `&amp;` 等 HTML 实体，展示时应解码为纯文字；描述 HTML 需要经过现有清理流程，不能作为可信页面脚本执行。

## 下载失败的直接原因与协议边界

安全的无效 ID 匿名读取 `GET /eapi/book/0/000000/file` 返回 HTTP 400、`Content-Type: application/json; charset=UTF-8`，正文为 `{success:0,error:"Please login"}`。没有使用账户或取得书籍文件。[7] 因此 `/file` 至少会返回业务错误 JSON；不能把接口响应直接命名为 `.epub` 后交给阅读器。

两个独立的公开客户端都将成功的 `/eapi/book/{id}/{hash}/file` 作为 **JSON 下载描述符** 处理：读取 `file.downloadLink`，然后再次 GET 该链接取得文件；`file.extension`、`file.description`、`file.author` 用于格式与文件名。[8][9] 这两项是第三方实现的交叉印证，成功描述符尚未通过真实账户现场核实，不能称为官方文档保证。

建议下载链路明确为：检查 EAPI HTTP 状态与 JSON `success` → 验证 `file.downloadLink` 和真实格式 → 请求实际文件 → 验证内容类型/文件结构及可用的长度或摘要 → 成功后入库。至少应拒绝 JSON、HTML 登录/验证页面伪装的 EPUB/PDF；EPUB 需要 ZIP/容器结构，PDF 需要有效 PDF 文件，且阅读器必须实际能解析。兼容服务若直接返回二进制或重定向，也需要按实际响应分支验证，不能只看扩展名。

`downloadLink` 可能是独立 CDN 的签名 URL。原书源的账户头和 cookie 不应无条件转发给这个外部主机；URL 中的签名参数也不应出现在日志。下载错误必须保留可读原因，避免把“未登录/额度不足/验证页”最终变成“文件打不开”。

## 官方站点能力与尚未证实的动作

当前服务的公共 `GET /eapi/info` 与 `/eapi/info/plans` 返回真实功能标记。[10][11] 现场 `plans.basic.converter.value` 与 `plans.premium.converter.value` 均为 `true`；`send_to_email`、`send_to_gdrive`、`send_to_kindle` 的 `value` 在 basic 为 `false`，premium 为 `true`。这是服务方案能力，不能替代用户当前权限、邮箱/Kindle 配置、Google Drive 授权和具体书籍的可用标记。书籍详情已有 `kindleAvailable`、`sendToEmailAvailable`，需与账户情况一起判断。

`GET /eapi/info/extensions` 返回 `pdf, fb2, epub, lit, mobi, djvu, djv, txt, rtf, azw, azw3, cbz`。[12] 这是目录格式列表，不能据此宣称能够在任意两种格式之间转换。

社区 EAPI 文档列出 `/eapi/book/{id}/{hash}/send-to-{type}` 与更新 `kindle_email` 的账户接口。[13] 这些是待核实线索：本次没有第一方公开脚本证明允许的 `type` 全集、收件地址/格式参数、转换任务协议、网盘绑定协议或成功响应结构；也没有调用任何发送、转换、保存或账户修改接口。不能据此编造一个“万能转换/邮件/网盘”实现。尚未接通的能力应给出真实状态和网站入口，不能在本地直接显示“发送成功”。应用自己的本地转换应作为独立能力，只列出已能生成并被阅读器验证的输入/输出组合。

公共 `/eapi/info.service.desktopApps` 还指向第一方分发目录内的 Windows/macOS/Linux 更新元数据；Windows `latest.yml` 现场版本为 `3.2.1`，安装包约 222 MB。[10][14] 未为研究运行、安装或下载整个客户端，也未绕过站点浏览器挑战，因此上述动作参数仍未取得一手确认。

## Project Gutenberg 与 Gutendex

Gutendex 维护者文档明确说明它是可自托管的 Gutenberg 元数据 API；Gutenberg 本身没有相同的 JSON API。生产长期使用建议自托管 Gutendex，而公共 `gutendex.com` 可用于试用。[15][16]

`GET https://gutendex.com/books/?sort=popular` 是按 Gutenberg 下载次数排序的真实热门列表，返回 `count`、`next`、`previous`、0–32 项 `results`。现场首项为 Gutenberg ID 1342《Pride and Prejudice》，前五项与 Gutenberg 官网首页的 Most Popular 栏一致。[15][17][18] 应标为“古登堡热门”，不能称为个性化推荐或编辑精选。`sort=descending` 按 ID 逆序，也不能严格冒充发布时间排序。可通过 `languages=zh` 获取中文书籍热门，但需要准确显示过滤条件。[15]

官网首页目前有 Recent Releases 与 Most Popular 两个真实书架；前者链接 `ebooks/search/?sort_order=release_date`，后者链接 `ebooks/search/?sort_order=downloads`。[18] 官网另有第一方新书 RSS `https://www.gutenberg.org/cache/epub/feeds/today.rss`，文档说明包含最近 24 小时新增或更新的书，夜间更新。[19] 若实现该栏目应沿用“新增或更新”，不能宣称完全是新上架。

Gutenberg 官网维护 `https://www.gutenberg.org/MIRRORS.ALL`，2026-10-14 现场 12 条，包含 HTTP、HTTPS、FTP、rsync。[20] 官方镜像文档明确只提供书籍目录及可下载目录元数据，不分发其中央搜索软件/网页。[21] 所以这些是 **文件镜像**，不是 Gutendex 的备用 API，不可把 Gutendex 请求路径直接替换成该镜像 origin。镜像还有 `gutenberg-epub/`、`dirs/` 等路径，必须保留基路径并根据主集合/生成文件集合的映射判断特定文件是否存在；仅探测首页 HTTP 200 不足以证明支持 EPUB 下载。

本次不以自动化方式镜像整个网站。首页刷新、RSS、目录拉取和健康检查应限量、缓存、按用户点击触发；批量元数据同步应走官方提供的机器可读目录，遵循官网机器人访问与镜像说明。[21][22]

## 来源

1. [当前响应书源的 EAPI 域名目录](https://zlib.bz/eapi/info/domains)，匿名现场 JSON；域名所有权未独立证明。
2. [当前响应书源首页](https://zlib.bz/)，匿名现场 HTTP 503 浏览器验证页面。
3. [网站热门 EAPI](https://zlib.bz/eapi/book/most-popular)，匿名现场 JSON。
4. [账户推荐 EAPI](https://zlib.bz/eapi/user/book/recommended)，匿名现场 `Please login`。
5. [Recently EAPI](https://zlib.bz/eapi/book/recently)，匿名现场无书籍数组。
6. [公开书籍详情 EAPI](https://zlib.bz/eapi/book/28704955/49edaa) 与[格式 EAPI](https://zlib.bz/eapi/book/28704955/49edaa/formats)，匿名现场 JSON。
7. [无效 ID 的文件接口](https://zlib.bz/eapi/book/0/000000/file)，匿名现场 HTTP 400 JSON。
8. [bipinkrish/Zlibrary-API 的公开客户端源码](https://github.com/bipinkrish/Zlibrary-API/blob/main/Zlibrary.py)，`__getBookFile`，第三方线索。
9. [baroxyton/zlibrary-CLI 的公开客户端源码](https://github.com/baroxyton/zlibrary-CLI/blob/master/src/api.js)，`getDownloadLink`、`downloadFile`，第三方线索。
10. [服务信息 EAPI](https://zlib.bz/eapi/info)，匿名现场 JSON。
11. [功能方案 EAPI](https://zlib.bz/eapi/info/plans)，匿名现场 JSON。
12. [文件扩展名 EAPI](https://zlib.bz/eapi/info/extensions)，匿名现场 JSON。
13. [baroxyton 的非官方 EAPI 文档](https://github.com/baroxyton/zlibrary-eapi-documentation)，明确标注 Inofficial，只用于发现待验证路由。
14. [服务信息内发布的 Windows 客户端更新元数据](https://dln1.ncdn.ec/general-files/soft/desktop/latest.yml)，匿名现场 YAML。
15. [Gutendex 自己的 API 文档](https://gutendex.com/)，列表、格式与排序协议。
16. [Gutendex 维护者源码 readme](https://github.com/garethbjohnson/gutendex/blob/268b181ad0ba958ccd237043f6c1f50c3bf7aa30/readme.md)，自托管与元数据来源。
17. [Gutendex 现场热门列表](https://gutendex.com/books/?sort=popular)。
18. [Project Gutenberg 官方首页](https://www.gutenberg.org/)，现场 Recent Releases / Most Popular 结构。
19. [Project Gutenberg — Feeds of New eBooks](https://www.gutenberg.org/ebooks/feeds.html) 与[官方 RSS](https://www.gutenberg.org/cache/epub/feeds/today.rss)。
20. [Project Gutenberg 官方镜像列表](https://www.gutenberg.org/MIRRORS.ALL)。
21. [Project Gutenberg — Mirroring How-To](https://www.gutenberg.org/help/mirroring.html)，镜像边界、元数据与生成集合。
22. [Project Gutenberg — Robot Access](https://www.gutenberg.org/policy/robot_access.html)。

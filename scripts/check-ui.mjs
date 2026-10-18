import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawn } from "node:child_process";
import { chromium } from "@playwright/test";

const output = resolve("artifacts/testing/v08"); mkdirSync(output, { recursive: true });
const server = spawn(process.execPath, [resolve("node_modules/vite/bin/vite.js"), "preview", "--host", "127.0.0.1", "--port", "1420", "--strictPort"], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
let serverLogs = ""; server.stdout.on("data", (chunk) => { serverLogs += chunk; }); server.stderr.on("data", (chunk) => { serverLogs += chunk; });
for (let attempt = 0; attempt < 60; attempt++) {
  try { if ((await fetch("http://127.0.0.1:1420/")).ok) break; } catch {}
  if (attempt === 59) { server.kill(); throw new Error(`UI server did not start: ${serverLogs}`); }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || undefined, args: ["--disable-gpu", "--disable-background-networking"] });
const results = [];
let currentPage;
try {
  for (const width of [320, 390, 768, 1280]) {
    const context = await browser.newContext({ viewport: { width, height: 844 }, colorScheme: "dark", hasTouch: true });
    const page = currentPage = await context.newPage(); const errors = []; page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      const initial = { mode: "system", appHosts: { "multi.example": ["203.0.113.1", "203.0.113.2"] }, upstreamIp: null, bootstrapIps: [], serverName: null, dohPath: "/dns-query", dohUrl: null, upstreamPort: null, proxyUrl: null, fallbackToSystem: false, timeoutMs: 5000 };
      const read = (key, fallback) => JSON.parse(sessionStorage.getItem(key) || JSON.stringify(fallback));
      const write = (key, value) => sessionStorage.setItem(key, JSON.stringify(value));
      let policy = read("ui-policy", initial); let profiles = read("ui-profiles", []); let favorites = read("ui-favorites", []);
      let downloads = read("ui-downloads", [{ id: "interrupted", generation: "fixture-1", title: "断线后等待继续的测试书", providerId: "gutendex", bookId: "interrupted", format: "epub", destination: "partial.epub", state: "failed", totalBytes: 34 * 1024 * 1024, downloadedBytes: 5.5 * 1024 * 1024, bytesPerSecond: 0, connections: 4, activeConnections: 0, peakConnections: 4, supportsResume: true, retryCount: 4, error: "下载中断，已保留可续传的进度。检查网络后点击继续下载", errorDetails: "error decoding response body: connection aborted", createdAtUnixMs: 1, updatedAtUnixMs: 1, revision: 1 }]);
      let downloadSettings = read("ui-download-settings", { connections: 4, simultaneousBooks: 2 });
      let callbackId = 0; const callbacks = new Map(); const listeners = new Map();
      const emitTask = (task) => { for (const [id, listener] of listeners) if (listener.event === "download-task") callbacks.get(listener.handler)?.({ event: listener.event, id, payload: structuredClone(task) }); };
      const saveTask = (task) => { write("ui-downloads", downloads); emitTask(task); return structuredClone(task); };
      const book = { id: "remote-book", title: "一部用于收藏与下载验证的远程书籍", authors: ["测试作者"], format: "epub", availableFormats: ["epub", "txt"] };
      const configuration = { selectedOrigin: "https://zlib.bz/", lastRefresh: null, subscriptions: [{ id: "source", name: "书源目录", url: "https://zlib.bz/eapi/info/domains", enabled: true }], mirrors: [{ id: "builtin", origin: "https://zlib.bz/", sources: ["应用内置候选"], available: null, contentAvailable: true, redirector: false }] };
      window.__uiCalls = []; window.__uiMaxTests = 0; let activeTests = 0;
      window.__TAURI_INTERNALS__ = {
        transformCallback(callback) { const id = ++callbackId; callbacks.set(id, callback); return id; }, unregisterCallback(id) { callbacks.delete(id); }, convertFileSrc(path) { return path; },
        async invoke(command, args = {}) {
          window.__uiCalls.push({ command, args });
          if (command === "core_status") return { version: "0.8.0" };
          if (command === "provider_descriptors") return [{ id: "gutendex", name: "Gutenberg", capabilities: { authenticated: false, searchable: true, downloadable: true } }, { id: "zlibrary", name: "Z-Library", capabilities: { authenticated: true, searchable: true, downloadable: true } }];
          if (command === "zlibrary_status") return { signedIn: false, origin: "https://zlib.bz/", originMode: "automatic" };
          if (command === "library_items") return read("ui-library", []);
          if (command === "favorite_books") return structuredClone(favorites);
          if (command === "set_book_favorite") {
            favorites = favorites.filter((item) => item.providerId !== args.providerId || item.bookId !== args.book.id);
            if (args.favorite) favorites.push({ providerId: args.providerId, bookId: args.book.id, title: args.book.title, authors: args.book.authors, format: args.book.format, addedAtUnixMs: Date.now() });
            write("ui-favorites", favorites); return null;
          }
          if (command === "resolver_policy") return structuredClone(policy);
          if (command === "set_resolver_policy") { policy = structuredClone(args.policy); write("ui-policy", policy); return structuredClone(policy); }
          if (command === "dns_profiles") return structuredClone(profiles);
          if (command === "save_dns_profile") { const saved = { ...args.profile, id: args.profile.id || `profile-${Date.now()}` }; profiles = [...profiles.filter((item) => item.id !== saved.id), saved]; write("ui-profiles", profiles); return structuredClone(saved); }
          if (command === "apply_dns_profile") { policy = structuredClone(profiles.find((item) => item.id === args.id).policy); write("ui-policy", policy); return structuredClone(policy); }
          if (command === "remove_dns_profile") { const removed = profiles.find((item) => item.id === args.id); profiles = profiles.filter((item) => item.id !== args.id); if (removed && JSON.stringify(removed.policy) === JSON.stringify(policy)) policy = structuredClone(initial); write("ui-profiles", profiles); write("ui-policy", policy); return structuredClone(policy); }
          if (command === "test_network") { activeTests++; window.__uiMaxTests = Math.max(window.__uiMaxTests, activeTests); await new Promise((resolve) => setTimeout(resolve, 20)); activeTests--; return { host: args.host, dns: { addresses: ["203.0.113.20"], elapsedMs: 1, usedFallback: false }, httpStatus: 200, elapsedMs: 2, usable: true, viaProxy: false }; }
          if (command === "mirror_configuration") return structuredClone(configuration);
          if (command === "refresh_book_mirrors") { configuration.lastRefresh = Date.now(); configuration.mirrors[0].available = true; configuration.mirrors[0].latencyMs = 120; return structuredClone(configuration); }
          if (command === "discover_books") return { title: "古登堡热门", sourceUrl: "https://gutendex.com/books/?sort=popular", items: [book], fetchedAtUnixMs: Date.now(), stale: false };
          if (command === "book_details") return { ...book, description: "真实操作流程的界面测试数据", identifiers: [] };
          if (command === "book_download_options") return { variants: [book, { ...book, format: "txt" }], siteFeatures: {} };
          if (command === "download_tasks") return structuredClone(downloads);
          if (command === "download_settings") return structuredClone(downloadSettings);
          if (command === "save_download_settings") { downloadSettings = structuredClone(args.settings); write("ui-download-settings", downloadSettings); return structuredClone(downloadSettings); }
          if (command === "enqueue_download_book") {
            const item = { id: "downloaded", title: args.title, authors: book.authors, format: args.format, path: `download.${args.format}`, sizeBytes: 1000 }; write("ui-library", [item]);
            const task = { ...downloads[0], id: "new-task", generation: "new-generation", title: args.title, bookId: args.bookId, format: args.format, destination: item.path, state: "completed", downloadedBytes: 1000, totalBytes: 1000, libraryId: item.id, createdAtUnixMs: Date.now(), revision: 2, error: null, errorDetails: null };
            downloads = [...downloads.filter((task) => task.id !== "new-task"), task]; return saveTask(task);
          }
          if (["resume_download", "pause_download", "cancel_download", "restart_download"].includes(command)) {
            const task = downloads.find((task) => task.id === args.id); task.revision++; task.error = null; task.errorDetails = null;
            if (command === "resume_download") { task.state = "downloading"; task.downloadedBytes += 4096; task.activeConnections = 3; task.bytesPerSecond = 65536; }
            else { task.state = command === "cancel_download" ? "cancelled" : "paused"; task.activeConnections = 0; task.bytesPerSecond = 0; }
            return saveTask(task);
          }
          if (command === "remove_download") { downloads = downloads.filter((task) => task.id !== args.id); write("ui-downloads", downloads); return null; }
          if (command === "pause_all_downloads") { for (const task of downloads.filter((task) => ["queued", "downloading"].includes(task.state))) { task.state = "paused"; task.activeConnections = 0; task.revision++; saveTask(task); } return null; }
          if (command === "library_cover" || command === "book_cover_bytes") return new ArrayBuffer(0);
          if (command === "reading_progress") return null;
          if (command === "save_reading_progress" || command === "validate_library_file") return args;
          if (command === "open_local_book_metadata") return { title: "一部用来验证狭窄屏幕工具栏的长书名", chapters: [{ id: "text", title: "全文" }] };
          if (command === "open_local_book_chapter") return { id: "text", title: "全文", text: "这是用于检查阅读界面布局的文字。\n\n".repeat(60) };
          if (command === "announcement_feeds") return { appItems: [{ id: "release", title: "星书 0.7 功能说明", body: "收藏与下载独立管理", publishedAt: "2026-10-14" }], siteItems: [], siteOrigin: "https://zlib.bz/", appSource: "应用内公告副本", fetchedAtUnixMs: Date.now() };
          if (command === "check_app_update") return { status: "no_releases", currentVersion: "0.8.0", checkedAt: Date.now() };
          if (command === "plugin:event|listen") { const id = ++callbackId; listeners.set(id, args); return id; }
          if (command === "plugin:event|unlisten") { listeners.delete(args.eventId); return null; }
          throw new Error(`Unexpected UI fixture command: ${command}`);
        },
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    });
    await page.goto("http://127.0.0.1:1420/", { waitUntil: "domcontentloaded" });
    await page.getByRole("heading", { name: "我的书架", exact: true }).waitFor({ timeout: 15000 });
    const nav = page.getByRole("navigation", { name: width <= 720 ? "手机主导航" : "主导航", exact: true });
    const settings = nav.getByRole("button", { name: "设置", exact: true }); const size = await settings.boundingBox();
    assert(size && size.height >= 48 && size.width >= 48);
    await settings.click(); await page.getByRole("button", { name: /DNS 服务器与网络/ }).click();
    await page.getByRole("button", { name: "添加 DNS", exact: true }).click();
    await page.getByLabel("配置名称", { exact: true }).fill("我的专属 DNS");
    const endpoint = "https://my-dns.example:8443/private/resolve?profile=mine";
    await page.getByLabel("DoH 地址", { exact: true }).fill(endpoint);
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
    await page.getByText("已保存并应用到搜索、下载和封面连接", { exact: true }).waitFor();
    let saved = await page.evaluate(() => window.__uiCalls.findLast((call) => call.command === "save_dns_profile"));
    assert.equal(saved.args.profile.policy.dohUrl, endpoint); assert.deepEqual(saved.args.profile.policy.bootstrapIps, []);
    assert.deepEqual(saved.args.profile.policy.appHosts, { "multi.example": ["203.0.113.1", "203.0.113.2"] });
    await page.getByRole("button", { name: "添加 DNS", exact: true }).click();
    await page.getByLabel("配置名称", { exact: true }).fill("备用普通 DNS");
    await page.getByLabel("DNS 解析方式", { exact: true }).selectOption("custom_dns");
    await page.getByLabel("DNS 服务器 IP", { exact: true }).fill("203.0.113.53");
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
    await page.getByRole("button", { name: "使用 备用普通 DNS", exact: true }).waitFor();
    await page.getByRole("button", { name: "使用 我的专属 DNS", exact: true }).click();
    await page.getByRole("button", { name: "编辑 我的专属 DNS", exact: true }).click();
    await page.getByLabel("引导 IP", { exact: true }).fill("203.0.113.1\n2001:db8::1");
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
    await page.getByText("已保存并应用到搜索、下载和封面连接", { exact: true }).waitFor();
    saved = await page.evaluate(() => window.__uiCalls.findLast((call) => call.command === "save_dns_profile"));
    assert.deepEqual(saved.args.profile.policy.bootstrapIps, ["203.0.113.1", "2001:db8::1"]);
    await page.getByRole("button", { name: "测试全部", exact: true }).click();
    await page.getByRole("button", { name: "测试全部", exact: true }).waitFor();
    assert((await page.evaluate(() => window.__uiMaxTests)) <= 2);
    await page.getByRole("button", { name: "测试配置", exact: true }).click(); await page.getByText("实际连接正常", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `DNS overflow at ${width}`);
    assert.equal(await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme), "light");
    await page.screenshot({ path: resolve(output, `dns-${width}.png`), fullPage: true });
    await page.getByRole("button", { name: "删除 备用普通 DNS", exact: true }).click();
    await page.reload(); await nav.getByRole("button", { name: "设置", exact: true }).click(); await page.getByRole("button", { name: /DNS 服务器与网络/ }).click();
    await page.getByRole("button", { name: "编辑 我的专属 DNS", exact: true }).waitFor(); assert.equal(await page.getByRole("button", { name: "编辑 备用普通 DNS", exact: true }).count(), 0);
    // The same DNS URL can be saved with different Hosts/proxy/fallback policies.
    await page.evaluate(() => {
      const items = JSON.parse(sessionStorage.getItem("ui-profiles"));
      const original = items[0];
      const other = { id: "same-endpoint", name: "同地址另一配置", policy: { ...original.policy, appHosts: { "other.example": ["203.0.113.5"] }, proxyUrl: "http://127.0.0.1:8080", fallbackToSystem: true, timeoutMs: 9000 } };
      sessionStorage.setItem("ui-profiles", JSON.stringify([original, other]));
      sessionStorage.setItem("ui-policy", JSON.stringify(other.policy));
    });
    await page.reload(); await nav.getByRole("button", { name: "设置", exact: true }).click(); await page.getByRole("button", { name: /DNS 服务器与网络/ }).click();
    await page.getByRole("button", { name: "编辑 同地址另一配置", exact: true }).waitFor();
    assert.equal(await page.getByLabel("配置名称", { exact: true }).inputValue(), "同地址另一配置", "Same-endpoint profiles selected the wrong profile for editing");
    assert.equal(await page.locator(".profile-current").count(), 1);
    await page.getByRole("button", { name: "保存并应用", exact: true }).click(); await page.getByText("已保存并应用到搜索、下载和封面连接", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.__uiCalls.findLast((call) => call.command === "save_dns_profile").args.profile.id), "same-endpoint");
    await page.getByRole("button", { name: "删除 我的专属 DNS", exact: true }).click();
    await page.getByText("已移除 DNS 配置", { exact: true }).waitFor();
    assert.equal(await page.locator(".profile-current").count(), 1);
    await page.getByRole("button", { name: "删除 同地址另一配置", exact: true }).click(); await page.getByText("已移除配置，当前使用系统 DNS", { exact: true }).waitFor();
    await nav.getByRole("button", { name: "找书", exact: true }).click();
    await page.getByRole("heading", { name: "古登堡热门", exact: true }).waitFor();
    await page.getByRole("button", { name: "收藏 一部用于收藏与下载验证的远程书籍", exact: true }).click();
    await page.getByText("已收藏到书架", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.__uiCalls.some((call) => call.command === "enqueue_download_book")), false, "Favoriting unexpectedly downloaded a file");
    assert.deepEqual(await page.evaluate(() => JSON.parse(sessionStorage.getItem("ui-library") || "[]")), []);
    await nav.getByRole("button", { name: "书架", exact: true }).click(); await page.getByRole("tab", { name: /收藏/ }).click();
    await page.getByRole("button", { name: "查看收藏 一部用于收藏与下载验证的远程书籍", exact: true }).click();
    await page.getByLabel("可用文件格式", { exact: true }).selectOption("1"); await page.getByRole("button", { name: "下载 TXT", exact: true }).click();
    await page.locator('.download-item[data-state="completed"]').filter({ hasText: "一部用于收藏与下载验证的远程书籍" }).waitFor();
    const download = await page.evaluate(() => window.__uiCalls.findLast((call) => call.command === "enqueue_download_book")); assert.equal(download.args.format, "txt");
    await nav.getByRole("button", { name: "下载", exact: true }).click();
    let interrupted = page.locator(".download-item").filter({ hasText: "断线后等待继续的测试书" });
    await interrupted.getByRole("button", { name: "继续下载 断线后等待继续的测试书", exact: true }).waitFor();
    assert.match(await interrupted.textContent(), /5.5 MB.*34 MB/);
    await interrupted.locator("h3").scrollIntoViewIfNeeded();
    const touch = await context.newCDPSession(page); const rect = await interrupted.locator("h3").boundingBox();
    await touch.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x: rect.x + Math.min(rect.width / 2, 50), y: rect.y + rect.height / 2 }] });
    await page.waitForTimeout(650);
    await touch.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
    await page.locator(".download-task-dialog").waitFor();
    await page.getByRole("button", { name: "移除下载记录", exact: false }).waitFor();
    await page.screenshot({ path: resolve(output, `task-long-press-${width}.png`), fullPage: true });
    await page.getByRole("button", { name: "关闭任务管理", exact: true }).click();
    await interrupted.getByRole("button", { name: "继续下载 断线后等待继续的测试书", exact: true }).click();
    await interrupted.getByText(/^3 个连接 · 上限 4 · 支持续传/).waitFor();
    await interrupted.getByRole("button", { name: "暂停", exact: true }).click();
    await interrupted.getByText("已暂停", { exact: true }).waitFor();
    const pausedBytes = await page.evaluate(() => JSON.parse(sessionStorage.getItem("ui-downloads")).find((task) => task.id === "interrupted").downloadedBytes);
    assert(pausedBytes > 5.5 * 1024 * 1024);
    await page.locator(".download-preferences summary").click();
    await page.getByLabel("单文件连接数", { exact: true }).selectOption("8");
    await page.getByLabel("同时下载书籍", { exact: true }).selectOption("1");
    await page.reload(); await nav.getByRole("button", { name: "下载", exact: true }).click();
    interrupted = page.locator(".download-item").filter({ hasText: "断线后等待继续的测试书" });
    await interrupted.getByText("已暂停", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => JSON.parse(sessionStorage.getItem("ui-downloads")).find((task) => task.id === "interrupted").downloadedBytes), pausedBytes);
    await page.locator(".download-preferences summary").click();
    assert.equal(await page.getByLabel("单文件连接数", { exact: true }).inputValue(), "8");
    assert.equal(await page.getByLabel("同时下载书籍", { exact: true }).inputValue(), "1");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, `Download queue overflow at ${width}`);
    const downloadTargets = await interrupted.locator("button").evaluateAll((buttons) => buttons.map((button) => ({ width: button.offsetWidth, height: button.offsetHeight })));
    assert(downloadTargets.every((target) => target.width >= 48 && target.height >= 48));
    await page.screenshot({ path: resolve(output, `download-queue-${width}.png`), fullPage: true });
    await interrupted.getByRole("button", { name: "管理下载 断线后等待继续的测试书", exact: true }).click();
    await page.getByRole("button", { name: /移除下载记录/ }).click();
    assert.equal(await interrupted.count(), 0);
    assert.equal((await page.evaluate(() => JSON.parse(sessionStorage.getItem("ui-library")))).length, 1, "Removing a task deleted the library file");
    await page.evaluate(() => sessionStorage.setItem("ui-library", JSON.stringify([{ id: "layout", title: "一部用来验证狭窄屏幕工具栏的长书名", path: "layout.txt", format: "txt", authors: [], sizeBytes: 1000 }])));
    await page.reload(); const cover = page.getByRole("button", { name: "阅读 一部用来验证狭窄屏幕工具栏的长书名", exact: true });
    const coverElement = await cover.elementHandle();
    await coverElement.dispatchEvent("pointerdown", { pointerType: "touch", clientX: 20, clientY: 200 }); await page.waitForTimeout(600); await coverElement.dispatchEvent("pointerup");
    await page.getByRole("button", { name: /使用其他应用打开/ }).waitFor(); await page.screenshot({ path: resolve(output, `download-management-${width}.png`), fullPage: true });
    await page.getByRole("button", { name: "关闭下载管理", exact: true }).click();
    await cover.click(); if (!(await page.locator(".reader-shell").count())) await cover.click();
    await page.locator(".reader-text").filter({ hasText: "这是用于检查" }).waitFor();
    const targets = await page.locator(".reader-toolbar button").evaluateAll((buttons) => buttons.map((button) => { const rect = button.getBoundingClientRect(); return { label: button.textContent, width: rect.width, height: rect.height, visible: rect.left >= 0 && rect.right <= innerWidth }; }));
    assert(targets.every((target) => target.visible && target.width >= 48 && target.height >= 48), `Reader toolbar clipped at ${width}: ${JSON.stringify(targets)}`);
    // Re-apply the scroll until the reader's one-frame position restore has finished.
    await page.waitForFunction(() => { const body = document.querySelector(".reader-body"); body.scrollTop = body.scrollHeight; return document.querySelector(".reader-position small")?.textContent === "100%"; }, null, { polling: 100 });
    await page.waitForFunction(() => window.__uiCalls.some((call) => call.command === "save_reading_progress" && JSON.parse(call.args.locator).offset === 1));
    await page.screenshot({ path: resolve(output, `reader-${width}.png`) }); await page.getByRole("button", { name: "← 书架", exact: true }).click();
    await nav.getByRole("button", { name: "设置", exact: true }).click(); await page.getByRole("button", { name: /书源镜像与发布源/ }).click();
    await page.getByRole("button", { name: "拉取并检测镜像", exact: true }).click(); await page.getByText("接口可用 · 120 ms", { exact: true }).waitFor();
    await nav.getByRole("button", { name: "设置", exact: true }).click(); await page.getByRole("button", { name: /公告与检查更新/ }).click();
    await page.getByRole("button", { name: "检查更新", exact: true }).click(); await page.getByRole("heading", { name: "项目尚无已发布版本信息", exact: true }).waitFor();
    await page.getByRole("tab", { name: "当前书源公告", exact: true }).click(); await page.getByRole("heading", { name: "暂未取得当前书源公告", exact: true }).waitFor();
    assert.deepEqual(errors, [], "Frontend exceptions");
    results.push({ width, settingsTarget: size, namedDnsProfilesPersisted: true, sameEndpointProfilesDistinct: true, bootstrapList: true, favoritesIndependentOfDownloads: true, formatSelected: "txt", downloadTaskTouchLongPress: true, pausedProgressPersists: true, downloadSettingsPersist: true, removeTaskPreservesLibrary: true, downloadTargets, longPressManagement: true, mirrors: true, updateAbsenceHandled: true, lightTheme: true, readerTargets: targets });
    await context.close(); currentPage = null;
  }
} catch (error) {
  if (currentPage) { await currentPage.screenshot({ path: resolve(output, "ui-failure.png"), fullPage: true }).catch(() => {}); writeFileSync(resolve(output, "ui-failure.html"), await currentPage.content()); }
  throw error;
} finally { await browser.close(); server.kill(); }
writeFileSync(resolve(output, "ui-results.json"), JSON.stringify({ fixtureOnly: true, results }, null, 2));
console.log(JSON.stringify(results));

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { chromium } from "@playwright/test";

const directory = resolve("artifacts/testing/v07");
mkdirSync(directory, { recursive: true });
const data = resolve(directory, `native-data-${Date.now()}`); mkdirSync(data, { recursive: true });
const fixture = resolve(directory, "星书阅读验证.txt");
writeFileSync(fixture, "星书阅读验证\n\n" + Array.from({ length: 100 }, (_, index) => `第 ${index + 1} 段：这是用于验证本地导入、中文显示和阅读进度恢复的测试文字。阅读界面应保持清晰，操作按钮应当足够大。\n\n`).join(""));
const pdfFixture = resolve(directory, "PDF阅读验证.pdf");
const stream = (page) => { const content = `BT /F1 24 Tf 30 700 Td (LuminaShelf PDF test page ${page}) Tj ET`; return `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`; };
const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
  ...[6, 7].map((content) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 840] /Resources << /Font << /F1 5 0 R >> >> /Contents ${content} 0 R >>`),
  "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", stream(1), stream(2)];
let pdfSource = "%PDF-1.4\n"; const offsets = [0];
objects.forEach((object, index) => { offsets.push(Buffer.byteLength(pdfSource)); pdfSource += `${index + 1} 0 obj\n${object}\nendobj\n`; });
const xref = Buffer.byteLength(pdfSource);
pdfSource += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.slice(1).map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
writeFileSync(pdfFixture, pdfSource);
const child = spawn(resolve("target/debug/lumina-shelf-desktop.exe"), [], {
  windowsHide: true, stdio: ["pipe", "pipe", "pipe"],
  env: { ...process.env, LUMINA_QA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: resolve(directory, "webview-data"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=9234 --disable-gpu --disable-background-networking --disable-background-timer-throttling" },
});
let logs = "";
child.stdout.on("data", (chunk) => { logs = (logs + chunk).slice(-50000); });
child.stderr.on("data", (chunk) => { logs = (logs + chunk).slice(-50000); writeFileSync(resolve(directory, "native-progress.log"), logs); });
let browser;
const report = { platform: "Windows native Tauri (hidden window)", dataDirectory: data, steps: [] };
const recordStep = report.steps.push.bind(report.steps);
report.steps.push = (...steps) => { for (const step of steps) console.log(`PASS ${step}`); return recordStep(...steps); };
try {
  let ready = false;
  for (let attempt = 0; attempt < 100; attempt++) {
    try { if ((await fetch("http://127.0.0.1:9234/json/version")).ok) { ready = true; break; } } catch {}
    if (child.exitCode !== null) throw new Error(`Native application exited: ${child.exitCode}\n${logs}`);
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  assert(ready, `Native WebView did not start\n${logs}`);
  browser = await chromium.connectOverCDP("http://127.0.0.1:9234");
  const context = browser.contexts()[0];
  let page;
  for (let attempt = 0; attempt < 50; attempt++) {
    page = context.pages().find((page) => page.url().includes("tauri.localhost"));
    if (page) break;
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  assert(page, "Native main page is missing");
  await page.waitForFunction(() => window.__TAURI_INTERNALS__ && document.querySelector("h1"));
  const call = (command, args = {}) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  assert.equal((await call("core_status")).version, "0.7.0");
  report.steps.push("native startup and IPC");
  await call("dns_profiles");
  const policy = { ...(await call("resolver_policy")), mode: "doh", dohUrl: "https://dns.alidns.com/dns-query", upstreamIp: null, serverName: null, proxyUrl: null, fallbackToSystem: false, timeoutMs: 5000 };
  const profile = await call("save_dns_profile", { profile: { id: "qa-ali", name: "我的阿里 DNS", policy } });
  const custom = await call("save_dns_profile", { profile: { id: "qa-custom", name: "我的完整地址 DNS", policy: { ...policy, dohUrl: "https://dns.example.invalid:8443/private/query?profile=mine", bootstrapIps: ["203.0.113.1", "2001:db8::1"] } } });
  assert.equal(custom.policy.dohUrl, "https://dns.example.invalid:8443/private/query?profile=mine"); assert.equal(custom.policy.bootstrapIps.length, 2);
  assert((await call("dns_profiles")).some((item) => item.id === profile.id));
  await call("remove_dns_profile", { id: custom.id });
  await call("apply_dns_profile", { id: profile.id });
  report.steps.push("native named DNS save/edit/list/apply/delete with full URL and multiple bootstrap IPs");
  const connection = await call("test_network", { policy, host: "gutendex.com" });
  report.network = connection;
  assert(connection.usable, `Real HTTPS source check failed: ${JSON.stringify(connection)}`);
  assert.equal(connection.dns.usedFallback, false);
  report.steps.push("full DoH URL without bootstrap IP, DNS and real source HTTPS");
  const optimized = await call("optimize_network");
  assert(optimized.report.usable); assert.equal(optimized.policy.fallbackToSystem, false);
  report.optimization = { selected: optimized.label, usable: optimized.report.usable, fallbackPreserved: true };
  report.steps.push("automatic optimization verifies a real source and preserves disabled fallback");
  await call("set_resolver_policy", { policy });
  const books = await call("search_books", { providerId: "gutendex", text: "Alice", page: 1, pageSize: 3, formats: ["epub"] });
  assert(books.items.length > 0); report.search = { books: books.items.map((book) => book.title) };
  report.steps.push("real book search through configured DoH");
  const selected = books.items[0];
  const countBeforeFavorite = (await call("library_items")).length;
  await call("set_book_favorite", { providerId: "gutendex", book: selected, favorite: true });
  assert((await call("favorite_books")).some((item) => item.bookId === selected.id));
  assert.equal((await call("library_items")).length, countBeforeFavorite);
  report.steps.push("native SQLite favorite creates no downloaded file or library item");
  const popular = await call("discover_books", { providerId: "gutendex", refresh: true });
  assert(popular.items.length > 0); assert(popular.sourceUrl.includes("sort=popular"));
  report.steps.push("real Gutenberg popularity feed, independent of search");
  const receipt = await call("download_book", { providerId: "gutendex", bookId: selected.id, title: selected.title, format: "epub", downloadDir: resolve(directory, "downloads") });
  const metadata = await call("open_local_book_metadata", { path: receipt.item.path });
  assert(metadata.chapters.length > 0);
  const chapter = await call("open_local_book_chapter", { path: receipt.item.path, chapterId: metadata.chapters[0].id });
  assert(chapter.text.length > 0);
  report.download = { title: receipt.item.title, bytes: receipt.item.sizeBytes, chapters: metadata.chapters.length };
  report.steps.push("real EPUB download, library insertion and chapter reading");
  const imported = await call("import_paths", { paths: [fixture, pdfFixture] });
  assert.equal(imported.length, 2); assert.equal(imported[0].format, "txt"); assert.equal(imported[1].format, "pdf");
  await call("save_reading_progress", { libraryId: imported[1].id, locator: "page:1", fraction: 0 });
  report.steps.push("local UTF-8 text and PDF import without opening a file dialog");
  await page.reload();
  await page.waitForFunction(() => document.querySelectorAll(".shelf-book").length >= 3);
  await page.screenshot({ path: resolve(directory, "native-shelf.png"), fullPage: true });
  // DOM actions keep the native window hidden and never request OS focus.
  await page.evaluate(() => [...document.querySelectorAll(".shelf-book")].find((item) => item.textContent.includes("星书阅读验证")).querySelector(".cover-button").click());
  await page.waitForFunction(() => document.querySelector(".reader-text")?.textContent.includes("第 100 段"));
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => requestAnimationFrame(resolve)))));
  await page.evaluate(() => { const body = document.querySelector(".reader-body"); body.scrollTop = 1500; body.dispatchEvent(new Event("scroll")); });
  await new Promise((resolve) => setTimeout(resolve, 700));
  const progress = await call("reading_progress", { libraryId: imported[0].id });
  assert(progress && JSON.parse(progress.locator).offset > 0, "Reader did not save its scroll offset");
  report.steps.push("reader scroll position saved to native SQLite");
  await page.screenshot({ path: resolve(directory, "native-reader.png"), fullPage: true });
  await page.evaluate(() => window.luminaBack());
  await page.waitForFunction(() => !document.querySelector(".reader-shell"));
  await page.evaluate(() => [...document.querySelectorAll(".shelf-book")].find((item) => item.textContent.includes("PDF阅读验证")).querySelector(".cover-button").click());
  await page.waitForFunction(() => document.querySelector(".pdf-status.error") || (document.querySelector(".pdf-canvas-wrap canvas")?.width > 0 && !document.querySelector(".pdf-canvas-wrap.rendering")));
  assert.equal(await page.locator(".pdf-status.error").count(), 0, await page.locator(".pdf-stage").innerText());
  await page.evaluate(() => [...document.querySelectorAll(".pdf-footer button")].find((button) => button.textContent.includes("下一页")).click());
  await page.waitForFunction(() => document.querySelector(".pdf-title small")?.textContent.includes("第 2 / 2 页"));
  let pdfProgress;
  for (let attempt = 0; attempt < 20; attempt++) {
    pdfProgress = await call("reading_progress", { libraryId: imported[1].id });
    if (pdfProgress?.locator === "page:2") break;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  assert.equal(pdfProgress?.locator, "page:2");
  report.steps.push("native PDF binary IPC, real PDF.js worker rendering and saved page navigation");
  const zoomBefore = await page.locator(".pdf-zoom-controls strong").innerText();
  await page.evaluate(() => document.querySelector('[aria-label="缩小 PDF"]').click());
  await page.waitForFunction((before) => document.querySelector(".pdf-zoom-controls strong")?.textContent !== before, zoomBefore);
  assert(parseInt(await page.locator(".pdf-zoom-controls strong").innerText()) < parseInt(zoomBefore));
  await page.screenshot({ path: resolve(directory, "native-pdf.png"), fullPage: true });
  await page.evaluate(() => window.luminaBack());
  const converted = await call("convert_library_book", { libraryId: imported[0].id, target: "epub" });
  const convertedMeta = await call("open_local_book_metadata", { path: converted.path });
  const convertedText = await call("open_local_book_chapter", { path: converted.path, chapterId: convertedMeta.chapters[0].id });
  assert(convertedText.text.includes("第 100 段"));
  const backToText = await call("convert_library_book", { libraryId: converted.id, target: "txt" });
  assert.equal(backToText.format, "txt"); await call("validate_library_file", { libraryId: backToText.id });
  report.steps.push("real local Chinese TXT to EPUB to TXT conversion, reopened through native reader");
  await call("delete_library_download", { libraryId: converted.id });
  assert.equal(existsSync(converted.path), false); assert(!(await call("library_items")).some((item) => item.id === converted.id));
  report.steps.push("explicit deletion removes only a QA-owned managed download");
  const mirrors = await call("mirror_configuration"); assert(mirrors.mirrors.length >= 6);
  await call("select_book_mirror", { id: mirrors.mirrors[0].id });
  const refreshed = await call("refresh_book_mirrors");
  const available = refreshed.mirrors.filter((item) => item.available);
  assert.equal(new Set(refreshed.mirrors.map((item) => item.origin)).size, refreshed.mirrors.length, "Duplicate mirror origins");
  assert.equal(new Set(refreshed.mirrors.map((item) => item.id)).size, refreshed.mirrors.length, "Duplicate mirror IDs");
  assert(available.length > 0, "No mirror passed real EAPI validation");
  const mirror = available.find((item) => item.origin.includes("zlib.bz")) || available[0];
  await call("select_book_mirror", { id: mirror.id });
  const sourceHome = await call("discover_books", { providerId: "zlibrary", refresh: true });
  assert(sourceHome.items.length > 0); assert(sourceHome.sourceUrl.includes("most-popular"));
  const siteBook = sourceHome.items.find((book) => book.id.includes(":"));
  const siteDetails = await call("book_details", { providerId: "zlibrary", bookId: siteBook.id });
  assert(siteDetails.format && siteDetails.format !== "other");
  const options = await call("book_download_options", { providerId: "zlibrary", bookId: siteBook.id }); assert(options.variants.length > 0);
  report.mirrors = { total: refreshed.mirrors.length, available: available.length, selected: mirror.origin, publishedSources: refreshed.subscriptions.map((source) => ({ name: source.name, error: source.lastError })) };
  report.sourceHome = { books: sourceHome.items.length, firstTitle: siteDetails.title, actualFormat: siteDetails.format, variants: options.variants.length };
  report.steps.push("live mirror directory refresh, bounded health checks, selection and real anonymous Z-Library home/details/formats");
  const feed = await call("announcement_feeds", { refresh: true }); assert(feed.appItems.length > 0);
  const update = await call("check_app_update"); assert(["no_releases", "available", "up_to_date"].includes(update.status));
  report.announcements = { localItems: feed.appItems.length, sourceItems: feed.siteItems.length, sourceError: feed.siteError, updateStatus: update.status };
  report.steps.push("live announcement availability and GitHub release-status checks without opening links");
  await page.reload();
  assert((await call("dns_profiles")).some((item) => item.id === "qa-ali"));
  assert((await call("favorite_books")).some((item) => item.bookId === selected.id));
  report.steps.push("DNS profiles and independent favorites remain after UI reload");
  await page.waitForFunction(() => document.querySelector(".shelf-tabs"));
  await page.evaluate(() => [...document.querySelectorAll('[aria-label="手机主导航"] button')].find((button) => button.textContent.includes("找书")).click());
  await page.waitForFunction(() => document.querySelector(".source-home .shelf-book"));
  await page.evaluate(() => [...document.querySelectorAll('.source-picker button')].find((button) => button.textContent.includes("Z-Library")).click());
  await page.waitForFunction(() => document.querySelector(".source-home h2")?.textContent === "站点热门");
  await page.screenshot({ path: resolve(directory, "native-source-home.png") });
  report.passed = true;
} finally {
  writeFileSync(resolve(directory, "native-output.log"), logs);
  writeFileSync(resolve(directory, "native-results.json"), JSON.stringify(report, null, 2));
  if (browser) await browser.close();
  if (child.exitCode === null) child.kill();
}
console.log(JSON.stringify(report));

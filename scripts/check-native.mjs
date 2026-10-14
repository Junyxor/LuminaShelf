import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { chromium } from "@playwright/test";

const directory = resolve("artifacts/testing/v06");
mkdirSync(directory, { recursive: true });
const data = resolve(directory, "native-data"); mkdirSync(data, { recursive: true });
const fixture = resolve(directory, "星书阅读验证.txt");
writeFileSync(fixture, "星书阅读验证\n\n" + Array.from({ length: 100 }, (_, index) => `第 ${index + 1} 段：这是用于验证本地导入、中文显示和阅读进度恢复的测试文字。阅读界面应保持清晰，操作按钮应当足够大。\n\n`).join(""));
const pdfFixture = resolve(directory, "PDF阅读验证.pdf");
const stream = (page) => { const content = `BT /F1 18 Tf 30 350 Td (LuminaShelf PDF test page ${page}) Tj ET`; return `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`; };
const objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
  ...[6, 7].map((content) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 420] /Resources << /Font << /F1 5 0 R >> >> /Contents ${content} 0 R >>`),
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
child.stderr.on("data", (chunk) => { logs = (logs + chunk).slice(-50000); });
let browser;
const report = { platform: "Windows native Tauri (hidden window)", steps: [] };
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
  assert.equal((await call("core_status")).version, "0.6.0");
  report.steps.push("native startup and IPC");
  const policy = { ...(await call("resolver_policy")), mode: "doh", dohUrl: "https://dns.alidns.com/dns-query", upstreamIp: null, serverName: null, proxyUrl: null, fallbackToSystem: false, timeoutMs: 5000 };
  await call("set_resolver_policy", { policy });
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
  const receipt = await call("download_book", { providerId: "gutendex", bookId: selected.id, title: selected.title, format: "epub", downloadDir: resolve(directory, "downloads") });
  const metadata = await call("open_local_book_metadata", { path: receipt.item.path });
  assert(metadata.chapters.length > 0);
  const chapter = await call("open_local_book_chapter", { path: receipt.item.path, chapterId: metadata.chapters[0].id });
  assert(chapter.text.length > 0);
  report.download = { title: receipt.item.title, bytes: receipt.item.sizeBytes, chapters: metadata.chapters.length };
  report.steps.push("real EPUB download, library insertion and chapter reading");
  const imported = await call("import_paths", { paths: [fixture, pdfFixture] });
  assert.equal(imported.length, 2); assert.equal(imported[0].format, "txt"); assert.equal(imported[1].format, "pdf");
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
  await page.screenshot({ path: resolve(directory, "native-pdf.png"), fullPage: true });
  report.passed = true;
} finally {
  writeFileSync(resolve(directory, "native-output.log"), logs);
  writeFileSync(resolve(directory, "native-results.json"), JSON.stringify(report, null, 2));
  if (browser) await browser.close();
  if (child.exitCode === null) child.kill();
}
console.log(JSON.stringify(report));

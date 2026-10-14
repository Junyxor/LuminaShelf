import assert from "node:assert/strict";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { spawn } from "node:child_process";
import { chromium } from "@playwright/test";

const output = resolve("artifacts/testing/v06"); mkdirSync(output, { recursive: true });
const server = spawn(process.execPath, [resolve("node_modules/vite/bin/vite.js"), "--host", "127.0.0.1", "--port", "1420", "--strictPort"], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
let serverLogs = "";
server.stdout.on("data", (chunk) => { serverLogs += chunk; });
server.stderr.on("data", (chunk) => { serverLogs += chunk; });
for (let attempt = 0; attempt < 50; attempt++) {
  try { if ((await fetch("http://127.0.0.1:1420/")).ok) break; } catch {}
  if (attempt === 49) throw new Error("UI server did not start");
  await new Promise((resolve) => setTimeout(resolve, 100));
}
const browser = await chromium.launch({ headless: true, args: ["--disable-gpu", "--disable-background-networking"] });
const results = [];
try {
  for (const width of [320, 390, 768, 1280]) {
    const context = await browser.newContext({ viewport: { width, height: 844 }, colorScheme: "dark" });
    const page = await context.newPage();
    const errors = []; page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      const policy = { mode: "system", appHosts: { "multi.example": ["203.0.113.1", "203.0.113.2"] }, upstreamIp: null, serverName: null, dohPath: "/dns-query", dohUrl: null, upstreamPort: null, proxyUrl: null, fallbackToSystem: true, timeoutMs: 5000 };
      window.__uiCalls = [];
      window.__TAURI_INTERNALS__ = {
        transformCallback() { return 1; },
        convertFileSrc(path) { return path; },
        async invoke(command, args = {}) {
          window.__uiCalls.push({ command, args });
          if (command === "core_status") return { version: "0.6.0" };
          if (command === "provider_descriptors") return [
            { id: "gutendex", name: "Gutenberg", capabilities: { authenticated: false, searchable: true, downloadable: true } },
            { id: "zlibrary", name: "Z-Library", capabilities: { authenticated: true, searchable: true, downloadable: true } },
          ];
          if (command === "zlibrary_status") return { signedIn: false, origin: "https://z-lib.gd", originMode: "automatic" };
          if (command === "library_items") return JSON.parse(sessionStorage.getItem("ui-library") || "[]");
          if (command === "reading_progress") return null;
          if (command === "save_reading_progress") return args;
          if (command === "open_local_book_metadata") return { title: "一部用来验证狭窄屏幕工具栏的长书名", chapters: [{ id: "text", title: "全文" }] };
          if (command === "open_local_book_chapter") return { id: "text", title: "全文", text: "这是用于检查阅读界面布局的文字。\n\n".repeat(60) };
          if (command === "resolver_policy") return structuredClone(policy);
          if (command === "set_resolver_policy") { Object.assign(policy, args.policy); return structuredClone(policy); }
          if (command === "test_network") return { host: args.host, dns: { addresses: ["203.0.113.20"], elapsedMs: 1, usedFallback: false }, httpStatus: 200, elapsedMs: 2, usable: true, viaProxy: false };
          if (command === "plugin:event|listen") return 1;
          if (command === "plugin:event|unlisten") return null;
          if (command === "search_books") return { items: [], page: 1, hasNext: false };
          throw new Error(`Unexpected UI fixture command: ${command}`);
        },
      };
      window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
    });
    await page.goto("http://127.0.0.1:1420/");
    await page.getByRole("heading", { name: "我的书架" }).waitFor({ timeout: 15000 }).catch(async (error) => {
      await page.screenshot({ path: resolve(output, `ui-failure-${width}.png`) });
      throw new Error(`${error}\nPage errors: ${errors.join("\n")}\n${serverLogs}\n${(await page.content()).slice(0, 3000)}`);
    });
    const nav = page.getByRole("navigation", { name: width <= 720 ? "手机主导航" : "主导航", exact: true });
    const settings = nav.getByRole("button", { name: "设置", exact: true });
    const size = await settings.boundingBox();
    assert(size && size.height >= 48 && size.width >= 48, `Settings target too small at ${width}`);
    await settings.click();
    await page.getByLabel("DNS 解析方式", { exact: true }).selectOption("doh");
    const url = "https://my-dns.example:8443/private/resolve?profile=mine";
    await page.getByLabel("DoH 地址", { exact: true }).fill(url);
    await page.getByRole("button", { name: "保存并应用", exact: true }).click();
    await page.getByText("已保存并应用到搜索、下载和封面连接", { exact: true }).waitFor();
    const call = await page.evaluate(() => window.__uiCalls.findLast((call) => call.command === "set_resolver_policy"));
    assert.equal(call.args.policy.dohUrl, url);
    assert.equal(call.args.policy.upstreamIp, null, "Custom DoH must not require a bootstrap IP");
    assert.deepEqual(call.args.policy.appHosts, { "multi.example": ["203.0.113.1", "203.0.113.2"] }, "Multiple Hosts addresses must survive an unchanged save");
    await page.getByRole("button", { name: "测试配置", exact: true }).click();
    await page.getByText("实际连接正常", { exact: true }).waitFor();
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
    assert.equal(overflow, false, `Page overflow at ${width}`);
    const scheme = await page.evaluate(() => getComputedStyle(document.documentElement).colorScheme);
    assert.equal(scheme, "light", "The UI must stay light even when the OS uses dark mode");
    await page.screenshot({ path: resolve(output, `settings-${width}.png`), fullPage: true });
    await nav.getByRole("button", { name: "书架", exact: true }).click();
    await page.screenshot({ path: resolve(output, `shelf-empty-${width}.png`), fullPage: true });
    await page.evaluate(() => sessionStorage.setItem("ui-library", JSON.stringify([{ id: "layout", title: "一部用来验证狭窄屏幕工具栏的长书名", path: "layout.txt", format: "txt", authors: [], sizeBytes: 1000 }])));
    await page.reload();
    await page.getByRole("button", { name: "阅读 一部用来验证狭窄屏幕工具栏的长书名", exact: true }).click();
    await page.locator(".reader-text").filter({ hasText: "这是用于检查" }).waitFor();
    const readerTargets = await page.locator(".reader-toolbar button").evaluateAll((buttons) => buttons.map((button) => {
      const rect = button.getBoundingClientRect();
      return { label: button.textContent, width: rect.width, height: rect.height, visible: rect.left >= 0 && rect.right <= innerWidth };
    }));
    assert(readerTargets.every((target) => target.visible && target.width >= 48 && target.height >= 48), `Reader toolbar is clipped at ${width}: ${JSON.stringify(readerTargets)}`);
    await page.screenshot({ path: resolve(output, `reader-${width}.png`), fullPage: true });
    assert.deepEqual(errors, [], "Browser console errors");
    results.push({ width, settingsTarget: size, customDohSaved: true, overflow: false, scheme, readerTargets });
    await context.close();
  }
} finally { await browser.close(); server.kill(); }
writeFileSync(resolve(output, "ui-results.json"), JSON.stringify({ fixtureOnly: true, results }, null, 2));
console.log(JSON.stringify(results));

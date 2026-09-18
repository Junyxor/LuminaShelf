import { expect } from "@playwright/test";
import { _android as android } from "playwright";
import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { createFixtures } from "./smoke-fixtures.mjs";

const serial = process.env.ANDROID_SERIAL || "emulator-5556";
const sdk = process.env.ANDROID_HOME || path.resolve(".android-sdk");
const adbPath = path.join(sdk, "platform-tools", process.platform === "win32" ? "adb.exe" : "adb");
const app = "app.luminashelf.client";
const output = path.resolve("artifacts/android/smoke");
const fixtures = path.join(output, "fixtures");
const checks = [];
const errors = [];
let device, page;
await mkdir(output, { recursive: true });
await createFixtures(fixtures);
function adb(...args) { return execFileSync(adbPath, ["-s", serial, ...args], { encoding: "utf8", windowsHide: true, timeout: 30000 }).trim(); }
async function launch() {
  adb("shell", "am", "start", "-n", app + "/.MainActivity");
  let pid;
  await expect.poll(() => { try { pid = adb("shell", "pidof", app).split(" ")[0]; return pid; } catch { return ""; } }, { timeout: 30000 }).toBeTruthy();
  await expect.poll(() => adb("shell", "cat", "/proc/net/unix").includes("webview_devtools_remote_" + pid), { timeout: 30000 }).toBe(true);
  if (!device) {
    const devices = await android.devices();
    device = devices.find((candidate) => candidate.serial() === serial);
    // Enumeration also opens handles for other ADB transports (including a
    // duplicate TCP connection to the same emulator). Release every unused one.
    await Promise.all(devices.filter((candidate) => candidate !== device).map((candidate) => candidate.close()));
  }
  if (!device) throw new Error("Android emulator is unavailable");
  const webview = await device.webView({ pkg: app });
  page = await webview.page();
  page.on("pageerror", (error) => errors.push(error.message));
  await expect(page.locator(".version-chip")).toHaveText("v0.6.0", { timeout: 30000 });
  await expect.poll(() => page.evaluate(() => window.__TAURI_INTERNALS__.invoke("core_status"))).toMatchObject({ platform: "android", rustCore: true });
}
async function restart() {
  await page?.context().close().catch(() => {});
  adb("shell", "am", "force-stop", app);
  await launch();
}
async function screenshot(name) {
  const png = execFileSync(adbPath, ["-s", serial, "exec-out", "screencap", "-p"], { windowsHide: true });
  await writeFile(path.join(output, name + ".png"), png);
}
function layout() {
  adb("shell", "uiautomator", "dump", "/sdcard/window-lumina.xml");
  return adb("shell", "cat", "/sdcard/window-lumina.xml");
}
async function tapNode(match) {
  let node;
  await expect.poll(() => {
    node = layout().match(/<node\b[^>]*>/g)?.find((node) => match.test(node));
    return Boolean(node);
  }, { timeout: 20000 }).toBe(true);
  const bounds = node.match(/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/).slice(1).map(Number);
  adb("shell", "input", "tap", String(Math.round((bounds[0] + bounds[2]) / 2)), String(Math.round((bounds[1] + bounds[3]) / 2)));
}
async function importBook(name) {
  await page.getByRole("button", { name: "导入电子书", exact: true }).click();
  // Exercise the real Android DocumentsUI, including granting a content:// URI.
  let xml = layout();
  if (!xml.includes(name)) {
    await tapNode(/content-desc="Show roots"|content-desc="显示根目录"|content-desc="打开导航抽屉"/);
    await tapNode(/text="Downloads"|text="下载"/);
  }
  await tapNode(new RegExp('text="' + name.replace(/[.*+?^$()|[\]\\]/g, "\\$&") + '"'));
  await expect(page.getByRole("status")).toContainText("已导入", { timeout: 20000 });
}

try {
  adb("shell", "am", "force-stop", app);
  adb("shell", "input", "keyevent", "82");
  adb("shell", "mkdir", "-p", "/sdcard/Download");
  for (const name of ["Lumina TXT.txt", "Lumina EPUB.epub", "Lumina PDF.pdf"]) {
    adb("push", path.join(fixtures, name), "/sdcard/Download/" + name);
  }
  await launch();
  await expect(page.getByRole("alert")).toHaveCount(0);
  checks.push("Android native IPC startup and secure-session bootstrap");
  await page.getByRole("button", { name: "书库", exact: true }).last().click();
  await expect(page.getByRole("textbox", { name: "书库目录" })).toHaveCount(0);
  for (const name of ["Lumina TXT.txt", "Lumina EPUB.epub", "Lumina PDF.pdf"]) await importBook(name);
  checks.push("SAF imports of TXT, EPUB and PDF");
  await screenshot("library");
  const open = async (title) => {
    await page.locator(".library-card").filter({ has: page.getByRole("heading", { name: title, exact: true }) }).first().getByRole("button", { name: "阅读 / 继续阅读" }).click();
  };
  await open("Lumina TXT");
  await expect(page.getByText(/Offline reading on Android/)).toBeVisible();
  await screenshot("txt-reader");
  adb("shell", "input", "keyevent", "4");
  await expect(page.locator(".reader-backdrop")).toHaveCount(0);
  checks.push("TXT reader and Android Back");
  await open("Lumina EPUB");
  await expect(page.getByText(/Android EPUB import works/)).toBeVisible();
  await page.getByRole("button", { name: /下一章/ }).click();
  await expect(page.getByText(/Continue the story after restarting/)).toBeVisible();
  await screenshot("epub-reader");
  adb("shell", "input", "keyevent", "4");
  await expect(page.locator(".reader-backdrop")).toHaveCount(0);
  await open("Lumina PDF");
  await expect(page.locator(".pdf-title")).toContainText("第 1 / 2 页", { timeout: 30000 });
  await page.getByRole("button", { name: "下一页", exact: true }).click();
  await expect(page.locator(".pdf-title")).toContainText("第 2 / 2 页");
  await expect.poll(() => page.locator(".pdf-canvas-wrap canvas").evaluate((canvas) => canvas.width)).toBeGreaterThan(0);
  await screenshot("pdf-reader");
  await expect.poll(() => page.evaluate(async () => {
    const books = await window.__TAURI_INTERNALS__.invoke("library_items");
    const pdf = books.find((book) => book.title === "Lumina PDF");
    return (await window.__TAURI_INTERNALS__.invoke("reading_progress", { libraryId: pdf.id }))?.locator;
  })).toBe("page:2");
  checks.push("EPUB chapters and PDF ranged rendering");
  const probe = (action) => page.evaluate((action) => window.__TAURI_INTERNALS__.invoke("debug_secure_store_probe", { action }), action);
  expect(await probe("write")).toBe(true);
  await restart();
  expect(await probe("verify")).toBe(true);
  expect(await probe("clear")).toBe(true);
  await page.getByRole("button", { name: "书库", exact: true }).last().click();
  await open("Lumina EPUB");
  await expect(page.getByText(/Continue the story after restarting/)).toBeVisible();
  adb("shell", "input", "keyevent", "4");
  await expect(page.locator(".reader-backdrop")).toHaveCount(0);
  await open("Lumina PDF");
  await expect(page.locator(".pdf-title")).toContainText("第 2 / 2 页", { timeout: 30000 });
  checks.push("Library, EPUB chapter and PDF page persist after force-stop");
  await restart();
  expect(await probe("verify")).toBe(false);
  checks.push("Android Keystore save/restore/delete across process restarts");
  await page.getByRole("button", { name: "设置", exact: true }).last().click();
  await expect(page.getByText(/下载和导入的书籍保存在应用书库/)).toBeVisible();
  await screenshot("settings");
  expect(errors).toEqual([]);
  await writeFile(path.join(output, "result.json"), JSON.stringify({ passed: true, serial, android: adb("shell", "getprop", "ro.build.version.release"), checks }, null, 2));
  console.log("Android smoke passed: " + checks.join("; "));
} catch (error) {
  await screenshot("failure").catch(() => {});
  await writeFile(path.join(output, "failure.json"), JSON.stringify({ checks, errors, message: String(error), dom: await page?.locator("body").innerText().catch(() => "") }, null, 2));
  throw error;
} finally {
  await page?.context().close().catch(() => {});
  await device?.close();
}

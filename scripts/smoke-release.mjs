// Native accessibility smoke for the actual non-debuggable, R8-minified APK.
import { execFileSync } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { createFixtures } from "./smoke-fixtures.mjs";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";
const sdk = process.env.ANDROID_HOME || path.resolve(".android-sdk");
const serial = process.env.ANDROID_SERIAL || "emulator-5556";
const adbPath = path.join(sdk, "platform-tools", process.platform === "win32" ? "adb.exe" : "adb");
const output = path.resolve("artifacts/android/release-smoke");
await mkdir(output, { recursive: true });
const fixtures = path.join(output, "fixtures");
await createFixtures(fixtures);
function adb(...args) { return execFileSync(adbPath, ["-s", serial, ...args], { windowsHide: true, encoding: "utf8", timeout: 30000, maxBuffer: 8 * 1024 * 1024 }).trim(); }
function layout() { adb("shell", "uiautomator", "dump", "/sdcard/lumina-release.xml"); return adb("shell", "cat", "/sdcard/lumina-release.xml"); }
async function find(match) {
  for (let attempt = 0; attempt < 12; attempt++) {
    const node = layout().match(/<node\b[^>]*>/g)?.find((node) => match.test(node));
    if (node) return node;
    await delay(400);
  }
  throw new Error("Native UI not found: " + match);
}
async function tap(match) {
  const node = await find(match);
  const bounds = node.match(/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/).slice(1).map(Number);
  adb("shell", "input", "tap", String(Math.round((bounds[0] + bounds[2]) / 2)), String(Math.round((bounds[1] + bounds[3]) / 2)));
}
async function shot(name) {
  await writeFile(path.join(output, name + ".png"), execFileSync(adbPath, ["-s", serial, "exec-out", "screencap", "-p"], { windowsHide: true, maxBuffer: 16 * 1024 * 1024 }));
}
const checks = [];
try {
  adb("shell", "input", "keyevent", "82");
  adb("shell", "mkdir", "-p", "/sdcard/Download");
  adb("push", path.join(fixtures, "Lumina TXT.txt"), "/sdcard/Download/Lumina TXT.txt");
  adb("shell", "am", "force-stop", "app.luminashelf.client");
  adb("shell", "am", "start", "-n", "app.luminashelf.client/.MainActivity");
  await find(/text="书库"/);
  if (layout().includes("安全 Session 恢复失败")) throw new Error("Release Keystore bootstrap failed");
  checks.push("Signed release startup");
  await tap(/text="书库"/);
  await tap(/text="导入电子书"/);
  if (!layout().includes("Lumina TXT.txt")) {
    await tap(/content-desc="Show roots"/);
    await tap(/text="Downloads"/);
  }
  await tap(/text="Lumina TXT.txt"/);
  await find(/text="Lumina TXT"/);
  await tap(/text="阅读 \/ 继续阅读"/);
  await find(/text="[^"]*Offline reading on Android/);
  await shot("release-txt");
  checks.push("SAF import and TXT reading in R8 build");
  adb("shell", "input", "keyevent", "4");
  await tap(/text="管理 Lumina TXT"/);
  await tap(/text="分享"/);
  let chooser = false;
  for (let attempt = 0; attempt < 10; attempt++) {
    chooser = adb("shell", "dumpsys", "activity", "activities").includes("ChooserActivity");
    if (chooser) break;
    await delay(400);
  }
  if (!chooser) throw new Error("Native Android share chooser did not open");
  await shot("release-share");
  checks.push("Rust-to-Kotlin custom plugin and FileProvider share");
  adb("shell", "input", "keyevent", "4");
  adb("shell", "input", "keyevent", "4");
  adb("shell", "am", "force-stop", "app.luminashelf.client");
  adb("shell", "am", "start", "-n", "app.luminashelf.client/.MainActivity");
  await tap(/text="书库"/);
  await find(/text="Lumina TXT"/);
  checks.push("Release library persists after restart");
  await shot("release-library");
  await writeFile(path.join(output, "result.json"), JSON.stringify({ passed: true, serial, checks }, null, 2));
  console.log("Release smoke passed: " + checks.join("; "));
} catch (error) {
  await shot("failure");
  await writeFile(path.join(output, "failure.xml"), layout());
  throw error;
}

import { createServer } from "node:http";
import { expect } from "@playwright/test";
import { writeFile } from "node:fs/promises";
import path from "node:path";

export async function testBackground({ page, adb, tapNode, screenshot, output }) {
  const body = Buffer.alloc(2 * 1024 * 1024, "A");
  let transferred = 0;
  const ranges = [];
  const server = createServer((request, response) => {
    response.setHeader("Accept-Ranges", "bytes");
    if (request.method === "HEAD") {
      response.setHeader("Content-Length", body.length);
      response.end();
      return;
    }
    const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range || "");
    const start = range ? Number(range[1]) : 0;
    const end = range?.[2] ? Number(range[2]) : body.length - 1;
    ranges.push([start, end]);
    response.statusCode = range ? 206 : 200;
    response.setHeader("Content-Length", end - start + 1);
    if (range) response.setHeader("Content-Range", `bytes ${start}-${end}/${body.length}`);
    let position = start;
    const timer = setInterval(() => {
      const next = Math.min(end + 1, position + 8192);
      response.write(body.subarray(position, next));
      transferred += next - position;
      position = next;
      if (position > end) { clearInterval(timer); response.end(); }
    }, 70);
    response.on("close", () => clearInterval(timer));
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const url = `http://10.0.2.2:${server.address().port}/background.txt`;
  const invoke = (command, args) => page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args });
  try {
    const task = await invoke("debug_enqueue_download", { url });
    await expect.poll(() => transferred, { timeout: 20000 }).toBeGreaterThan(131072);
    expect(adb("shell", "dumpsys", "activity", "services", "app.luminashelf.client")).toContain("isForeground=true");
    adb("shell", "input", "keyevent", "3");
    adb("shell", "input", "keyevent", "26");
    const before = transferred;
    await expect.poll(() => transferred, { timeout: 10000 }).toBeGreaterThan(before + 131072);
    adb("shell", "input", "keyevent", "26");
    adb("shell", "input", "keyevent", "82");
    adb("shell", "cmd", "statusbar", "expand-notifications");
    await screenshot("download-notification");
    await tapNode(/text="全部暂停"/);
    adb("shell", "cmd", "statusbar", "collapse");
    adb("shell", "am", "start", "-n", "app.luminashelf.client/.MainActivity");
    const status = async () => (await invoke("download_tasks")).find((item) => item.id === task.id);
    await expect.poll(async () => (await status()).state).toBe("paused");
    const partial = (await status()).downloadedBytes;
    expect(partial).toBeGreaterThan(0);
    expect(partial).toBeLessThan(body.length);
    await invoke("enqueue_download", { providerId: "smoke", bookId: url, title: "后台下载测试", format: "txt", downloadDir: null });
    await expect.poll(async () => (await status()).state, { timeout: 45000, intervals: [500,1000] }).toBe("completed");
    expect(ranges.some(([start]) => start > 0)).toBe(true);
    await expect.poll(() => adb("shell", "dumpsys", "activity", "services", "app.luminashelf.client").includes("isForeground=true")).toBe(false);
    await writeFile(path.join(output, "background-result.json"), JSON.stringify({
      passed: true, backgroundTransfer: true, notificationPause: true,
      resumedFromPartial: partial, totalBytes: body.length, requestedRanges: ranges,
    }, null, 2));
  } finally { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); }
}

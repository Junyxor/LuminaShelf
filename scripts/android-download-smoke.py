"""Drive the signed release through Android accessibility; runs only on CI's emulator."""
import json
import os
import re
import subprocess
import time
import xml.etree.ElementTree as ET
from pathlib import Path

OUT = Path("smoke")
PACKAGE = "app.luminashelf.client"
steps = []
DISPLAY = [1080, 2400]


def adb(*args, check=True):
    return subprocess.run(["adb", *args], capture_output=True, text=True, timeout=25, check=check).stdout


def network_speed(speed, check=True):
    if os.environ.get("LUMINA_SMOKE_SHAPER") == "proxy":
        return subprocess.run(["python3", "scripts/android-proxy.py", speed], capture_output=True, text=True, timeout=10, check=check).stdout
    if os.environ.get("LUMINA_SMOKE_SHAPER") == "host":
        result = subprocess.run(["bash", "scripts/android-bandwidth.sh", speed], capture_output=True, text=True, timeout=25)
        if result.returncode and check:
            (OUT / "host-shaping-error.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
            result.check_returncode()
        return result.stdout
    return adb("emu", "network", "speed", speed, check=check)


def tree():
    result = adb("shell", "uiautomator", "dump", "--compressed", "/sdcard/lumina-download-test.xml", check=False)
    if "dumped" not in result:
        raise RuntimeError("Android accessibility tree was not ready: " + result)
    return ET.fromstring(adb("shell", "cat", "/sdcard/lumina-download-test.xml"))


def label(node):
    return node.get("text", "") + " " + node.get("content-desc", "")


def bounds(node):
    values = list(map(int, re.findall(r"\d+", node.get("bounds", ""))))
    return values if len(values) == 4 else [0, 0, 0, 0]


def wait_node(predicate, seconds=75, scroll=False, source_metadata=False):
    end = time.monotonic() + seconds
    error = ""
    while time.monotonic() < end:
        try:
            root = tree()
            for node in root.iter("node"):
                box = bounds(node)
                if predicate(node) and box[2] > box[0] and box[3] > box[1] and (box[1] + box[3]) // 2 < DISPLAY[1]:
                    return node, root
            if source_metadata and any("书源 HTTPS 连接失败" in label(n) for n in root.iter("node")):
                raise AssertionError("Source metadata connection failed")
            if any(n.get("text") == "下载中断" for n in root.iter("node")):
                details = next((n for n in root.iter("node") if n.get("text") == "错误详情"), None)
                if details is not None:
                    click_node(details)
                    root = evidence("download-error")
                message = " | ".join(label(n).strip() for n in root.iter("node") if label(n).strip())[-8000:]
                (OUT / "download-error.txt").write_text(message, encoding="utf-8")
                raise AssertionError("Native download failed: " + message)
            error = " | ".join(label(n).strip() for n in root.iter("node") if label(n).strip())[-4000:]
            if scroll:
                adb("shell", "input", "swipe", str(DISPLAY[0] // 2), str(int(DISPLAY[1] * 0.72)), str(DISPLAY[0] // 2), str(int(DISPLAY[1] * 0.35)), "300")
        except (RuntimeError, subprocess.TimeoutExpired) as exc:
            error = str(exc)
        time.sleep(0.4)
    raise AssertionError("Timed out waiting for Android UI: " + error)


def click_node(node):
    x1, y1, x2, y2 = bounds(node)
    adb("shell", "input", "tap", str((x1 + x2) // 2), str((y1 + y2) // 2))


def click(text, seconds=75):
    node, root = wait_node(lambda n: n.get("enabled") != "false" and (n.get("text", "").strip() == text or n.get("content-desc", "").strip() == text), seconds)
    click_node(node)
    return root


def evidence(name):
    root = tree()
    ET.ElementTree(root).write(OUT / (name + ".xml"), encoding="utf-8", xml_declaration=True)
    adb("shell", "screencap", "-p", "/sdcard/lumina-step.png")
    adb("pull", "/sdcard/lumina-step.png", str(OUT / (name + ".png")))
    return root


def byte_progress(text):
    match = re.search(r"EPUB\s*·\s*([\d.]+)\s*(B|KB|MB|GB)\s*/", text)
    if match:
        return float(match[1]) * 1024 ** ["B", "KB", "MB", "GB"].index(match[2])
    return 0


def progress(root):
    return max((byte_progress(label(node)) for node in root.iter("node")), default=0)


try:
    DISPLAY = list(map(int, re.findall(r"(\d+)x(\d+)", adb("shell", "wm", "size"))[-1]))
    # Use cellular IPv4 for both the host-kernel and legacy emulator shapers.
    adb("shell", "svc", "wifi", "disable")
    adb("shell", "svc", "data", "enable")
    adb("emu", "gsm", "data", "home", check=False)
    click("找书")
    search, _ = wait_node(lambda n: n.get("class") == "android.widget.EditText")
    click_node(search)
    adb("shell", "input", "text", "Moby%sDick")
    click("搜索")
    evidence("search-submitted")
    cover, _ = wait_node(lambda n: label(n).strip().startswith("查看 ") and "Moby" in label(n), 100, scroll=True)
    title = label(cover).strip().removeprefix("查看 ")
    click_node(cover)
    for metadata_attempt in range(3):
        try:
            download, _ = wait_node(lambda n: "下载 EPUB" in label(n) and n.get("enabled") == "true", 120, scroll=True, source_metadata=True)
            break
        except AssertionError as error:
            if "Source metadata connection failed" not in str(error) or metadata_attempt == 2:
                raise
            evidence("metadata-retry-" + str(metadata_attempt + 1))
            steps.append({"step": "retry transient source metadata connection", "attempt": metadata_attempt + 1})
            click("关闭书籍详情")
            cover, _ = wait_node(lambda n: label(n).strip().startswith("查看 ") and "Moby" in label(n), 45, scroll=True)
            click_node(cover)
    # Establish an unshaped control before testing a slow transfer. Connection
    # failures must be distinguished from shaping and saved-progress behavior.
    network_speed("full")
    click_node(download)
    wait_node(lambda n: n.get("text") == "已完成", 150)
    evidence("download-unshaped-control")
    steps.append({"step": "unshaped real HTTPS download control", "title": title})
    manage, _ = wait_node(lambda n: label(n).strip().startswith("管理下载 "))
    click_node(manage)
    restart, _ = wait_node(lambda n: label(n).strip().startswith("从头重新下载"))
    click_node(restart)
    network_speed("256")
    shaping = network_speed("status") if os.environ.get("LUMINA_SMOKE_SHAPER") in ["host", "proxy"] else adb("emu", "network", "status")
    (OUT / "network-shaping.txt").write_text(shaping + adb("shell", "dumpsys", "connectivity"), encoding="utf-8")
    started = time.monotonic()
    click("重新下载")
    _, root = wait_node(lambda n: byte_progress(label(n)) >= 8192, 240)
    before = progress(root)
    service = adb("shell", "dumpsys", "activity", "services", PACKAGE)
    (OUT / "download-service-active.txt").write_text(service, encoding="utf-8")
    assert "DownloadService" in service and "isForeground=true" in service, "Download foreground service did not start"
    adb("shell", "input", "keyevent", "KEYCODE_HOME")
    time.sleep(5)
    service = adb("shell", "dumpsys", "activity", "services", PACKAGE)
    assert "DownloadService" in service and "isForeground=true" in service, "Download service stopped when app entered background"
    adb("shell", "am", "start", "-n", PACKAGE + "/.MainActivity")
    # Pause through the visible control; the transfer must checkpoint before returning.
    click("暂停", 100)
    _, root = wait_node(lambda n: n.get("text") == "已暂停", 60)
    saved = progress(root)
    assert saved > 0, "No partial bytes saved"
    assert saved > before, "No byte progress while app was in background"
    assert saved / max(1, time.monotonic() - started) < 64 * 1024, "Network shaping did not slow actual HTTP bytes"
    if os.environ.get("LUMINA_SMOKE_SHAPER") == "host":
        traffic = network_speed("status")
        (OUT / "host-shaping-active.txt").write_text(traffic, encoding="utf-8")
        assert any(int(n) > 0 for n in re.findall(r"Sent (\d+) bytes", traffic)), "No actual traffic passed the host bandwidth limiter"
    if os.environ.get("LUMINA_SMOKE_SHAPER") == "proxy":
        traffic = json.loads(network_speed("status"))
        (OUT / "proxy-shaping-active.json").write_text(json.dumps(traffic), encoding="utf-8")
        assert traffic["limitedBytes"] > 0, "No actual bytes passed the limited HTTPS tunnel"
    evidence("download-paused")
    steps.append({"step": "real HTTPS download, foreground service, background and visible pause", "title": title, "savedBytesRounded": saved, "elapsedSeconds": time.monotonic() - started})
    # Kill the actual signed process, not merely its WebView, to prove SQLite recovery.
    adb("shell", "am", "force-stop", PACKAGE)
    adb("shell", "am", "start", "-n", PACKAGE + "/.MainActivity")
    click("下载")
    _, recovered = wait_node(lambda n: n.get("text") == "已暂停", 60)
    assert progress(recovered) == saved, "Partial progress did not survive process death"
    # Real Android long touch, then release: must open management without activating a destructive row.
    title_node, _ = wait_node(lambda n: n.get("text", "").strip() == title)
    x1, y1, x2, y2 = bounds(title_node)
    x, y = (x1 + x2) // 2, (y1 + y2) // 2
    adb("shell", "input", "touchscreen", "swipe", str(x), str(y), str(x), str(y), "750")
    wait_node(lambda n: "移除下载记录" in label(n), 30)
    evidence("download-long-press")
    click("关闭任务管理")
    network_speed("full")
    node, _ = wait_node(lambda n: label(n).strip().startswith("继续下载 ") or n.get("text") == "继续下载")
    click_node(node)
    wait_node(lambda n: n.get("text") == "已完成", 150)
    evidence("download-completed")
    click("阅读")
    wait_node(lambda n: "书架" in label(n) and "←" in label(n), 60)
    wait_node(lambda n: len(n.get("text", "")) > 160, 60)
    evidence("download-reader")
    steps.append({"step": "process recovery, actual Android long press, resume, completion and built-in EPUB reader", "savedBytesRounded": saved})
finally:
    network_speed("full", check=False)
    (OUT / "download-smoke.json").write_text(json.dumps({"platform": "signed Android release, headless API35 emulator", "fixture": False, "bandwidthShaper": os.environ.get("LUMINA_SMOKE_SHAPER", "emulator"), "steps": steps}, ensure_ascii=False, indent=2), encoding="utf-8")

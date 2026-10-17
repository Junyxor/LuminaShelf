"""Dismiss only the cloud emulator's Quickstep ANR; never dismiss app failures."""
import re
import subprocess
import sys
import xml.etree.ElementTree as ET


def launcher_close_button(root):
    system_nodes = [node for node in root.iter("node") if node.get("package") == "android"]
    if not any(node.get("text") in ("Quickstep isn't responding", "Quickstep isn’t responding") for node in system_nodes):
        return None
    buttons = [node for node in system_nodes if node.get("text") == "Close app" and node.get("class") == "android.widget.Button"]
    if len(buttons) != 1:
        raise RuntimeError("Ambiguous emulator launcher dialog")
    box = list(map(int, re.findall(r"-?\d+", buttons[0].get("bounds", ""))))
    if len(box) != 4 or box[2] <= box[0] or box[3] <= box[1]:
        raise RuntimeError("Invalid emulator launcher dialog bounds")
    return (box[0] + box[2]) // 2, (box[1] + box[3]) // 2


if __name__ == "__main__":
    target = launcher_close_button(ET.parse(sys.argv[1]).getroot())
    if target is not None:
        subprocess.run(["adb", "shell", "input", "tap", str(target[0]), str(target[1])], check=True, timeout=10)
        print("Dismissed the emulator Quickstep launcher ANR")

#!/usr/bin/env bash
set -Eeuo pipefail

mkdir -p smoke

collect_artifacts() {
  adb logcat -d > smoke/logcat.txt || true
  if timeout 10s adb shell uiautomator dump /sdcard/luminashelf-ui.xml; then
    adb pull /sdcard/luminashelf-ui.xml smoke/ui.xml || true
  fi
  if adb shell screencap -p /sdcard/luminashelf.png; then
    adb pull /sdcard/luminashelf.png smoke/screenshot.png || true
  fi
}

finish() {
  local exit_code=$?
  trap - EXIT
  collect_artifacts
  exit "$exit_code"
}
trap finish EXIT

apk_path="$(find package -name '*-universal-release.apk' -type f -print -quit)"
test -n "$apk_path"
adb install -r "$apk_path"
adb logcat -c
adb shell am start -W -n app.luminashelf.client/.MainActivity

ui_ready=false
for attempt in {1..20}; do
  if timeout 10s adb shell uiautomator dump /sdcard/luminashelf-ui.xml &&
    adb pull /sdcard/luminashelf-ui.xml smoke/ui.xml &&
    grep -q '我的书架' smoke/ui.xml; then
    ui_ready=true
    break
  fi
  sleep 1
done
test "$ui_ready" = true
python3 scripts/android-download-smoke.py
adb shell pidof app.luminashelf.client > smoke/pid.txt
adb logcat -d > smoke/logcat.txt
if grep -q 'Process: app.luminashelf.client.*PID:' smoke/logcat.txt; then
  exit 1
fi

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

apk_variant="${LUMINA_SMOKE_APK_VARIANT:-universal}"
case "$apk_variant" in universal|x86_64) ;; *) exit 1 ;; esac
apk_path="$(find package -name "*-${apk_variant}-release.apk" -type f -print -quit)"
test -n "$apk_path"
adb install -r "$apk_path"
adb logcat -c
if [[ "${LUMINA_RUN_NETWORK_DIAGNOSTICS:-0}" == "1" ]]; then
  diagnostics_status=0
  bash scripts/android-network-diagnostics.sh > smoke/network-diagnostics.log 2>&1 || diagnostics_status=$?
  echo "$diagnostics_status" > smoke/network-diagnostics-exit.txt
  adb emu network speed full
fi
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
if [[ "${LUMINA_SMOKE_UNIVERSAL_ONLY:-0}" != "1" ]]; then
single_apk="$(find package/x86_64 -name '*-x86_64-release.apk' -type f -print -quit)"
test -n "$single_apk"
adb install -r "$single_apk"
adb shell am start -W -n app.luminashelf.client/.MainActivity
x64_ready=false
for attempt in {1..15}; do
  if timeout 10s adb shell uiautomator dump /sdcard/luminashelf-x64.xml &&
    adb pull /sdcard/luminashelf-x64.xml smoke/x86_64-ui.xml &&
    grep -q '我的书架' smoke/x86_64-ui.xml; then
    x64_ready=true
    break
  fi
  sleep 1
done
test "$x64_ready" = true
fi
adb shell pidof app.luminashelf.client > smoke/pid.txt
adb logcat -d > smoke/logcat.txt
if grep -q 'Process: app.luminashelf.client.*PID:' smoke/logcat.txt; then
  exit 1
fi

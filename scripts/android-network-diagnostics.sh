#!/usr/bin/env bash
set -Eeuo pipefail

# Runs only on the headless cloud emulator. No APK, trust store or app data changes.
probe_dir="$PWD/smoke/network-control"
mkdir -p "$probe_dir"
trap 'adb emu network speed full >/dev/null 2>&1 || true' EXIT

android_jar="$(find "$ANDROID_HOME/platforms" -name android.jar -type f | sort -V | tail -n 1)"
d8_path="$(find "$ANDROID_HOME/build-tools" -name d8 -type f | sort -V | tail -n 1)"
test -n "$android_jar"
test -n "$d8_path"
javac --release 8 -d "$probe_dir" scripts/AndroidNetworkProbe.java
"$d8_path" --min-api 24 --lib "$android_jar" --output "$probe_dir" "$probe_dir/AndroidNetworkProbe.class"
adb push "$probe_dir/classes.dex" /data/local/tmp/lumina-network-probe.dex

# Fixed public Moby Dick EPUB; avoid a separate host-side metadata/WAF dependency.
book_url='https://www.gutenberg.org/ebooks/2701.epub3.images'
adb shell svc wifi disable
adb shell svc data enable
adb emu gsm data home || true

for speed in full 256; do
  adb emu network speed "$speed"
  adb emu network status > "$probe_dir/$speed-network.txt"
  for mode in probe file; do
    exit_code=0
    timeout 150s adb shell CLASSPATH=/data/local/tmp/lumina-network-probe.dex app_process /system/bin AndroidNetworkProbe "$book_url" "$mode" > "$probe_dir/$speed-$mode.txt" 2>&1 || exit_code=$?
    echo "exitCode=$exit_code" >> "$probe_dir/$speed-$mode.txt"
    cat "$probe_dir/$speed-$mode.txt"
  done
done

#!/usr/bin/env bash
set -Eeuo pipefail

output_dir="$PWD/smoke/ui-dump"
mkdir -p "$output_dir"
android_jar="$(find "$ANDROID_HOME/platforms" -name android.jar -type f | sort -V | tail -n 1)"
d8_path="$(find "$ANDROID_HOME/build-tools" -name d8 -type f | sort -V | tail -n 1)"
test -n "$android_jar"
test -n "$d8_path"
javac --release 8 -cp "$android_jar" -d "$output_dir" scripts/AndroidUiDump.java
"$d8_path" --min-api 24 --lib "$android_jar" --output "$output_dir" "$output_dir/AndroidUiDump.class"
adb push "$output_dir/classes.dex" /data/local/tmp/lumina-ui-dump.dex

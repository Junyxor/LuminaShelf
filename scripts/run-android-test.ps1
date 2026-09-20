# Cargo runner for native core tests on a selected Android emulator.
$ErrorActionPreference = "Stop"
$serial = if ($env:ANDROID_SERIAL) { $env:ANDROID_SERIAL } else { "emulator-5556" }
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path (Split-Path $PSScriptRoot -Parent) ".android-sdk" }
$adb = Join-Path $sdk "platform-tools/adb.exe"
$binary = (Resolve-Path -LiteralPath $args[0]).Path
$remote = "/data/local/tmp/luminashelf-tests"
& $adb -s $serial shell mkdir -p "$remote/tmp" | Out-Null
& $adb -s $serial push $binary "$remote/test" | Out-Null
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $adb -s $serial shell chmod 700 "$remote/test"
& $adb -s $serial shell "TMPDIR=$remote/tmp $remote/test"
exit $LASTEXITCODE

param(
    [string[]]$Targets = @("aarch64"),
    [switch]$Release
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $env:ANDROID_HOME -or -not $env:NDK_HOME -or -not $env:JAVA_HOME) {
    throw "Set ANDROID_HOME, NDK_HOME and JAVA_HOME before building."
}
$env:PATH = "$env:JAVA_HOME\bin;$env:PATH"
if (-not (Test-Path -LiteralPath "src-tauri/gen/android")) {
    cargo fetch --locked --target aarch64-linux-android
    if ($LASTEXITCODE -ne 0) { throw "Cargo dependency fetch failed" }
    npm run tauri -- android init --ci
    if ($LASTEXITCODE -ne 0) { throw "Android project initialization failed" }
}
$env:TAURI_ANDROID_PROJECT_PATH = (Resolve-Path "src-tauri/gen/android").Path
$env:TAURI_ANDROID_PACKAGE_UNESCAPED = "app.luminashelf.client"
$toolchain = Join-Path $env:NDK_HOME "toolchains/llvm/prebuilt/windows-x86_64/bin"
$profileName = if ($Release) { "release" } else { "debug" }
$profileTask = if ($Release) { "Release" } else { "Debug" }
npm run build
if ($LASTEXITCODE -ne 0) { throw "Frontend build failed" }
node scripts/patch-android-keyring.mjs
if ($LASTEXITCODE -ne 0) { throw "Android initialization patch failed" }
foreach ($target in $Targets) {
    switch ($target) {
        "aarch64" { $triple = "aarch64-linux-android"; $abi = "arm64-v8a"; $arch = "arm64"; $taskArch = "Arm64" }
        "x86_64" { $triple = "x86_64-linux-android"; $abi = "x86_64"; $arch = "x86_64"; $taskArch = "X86_64" }
        default { throw "Unsupported target: $target" }
    }
    $targetKey = $triple.Replace("-", "_")
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_" + $targetKey.ToUpperInvariant() + "_LINKER", (Join-Path $toolchain "$($triple)24-clang.cmd"), "Process")
    [Environment]::SetEnvironmentVariable("CC_" + $targetKey, (Join-Path $toolchain "$($triple)24-clang.cmd"), "Process")
    [Environment]::SetEnvironmentVariable("AR_" + $targetKey, (Join-Path $toolchain "llvm-ar.exe"), "Process")
    [Environment]::SetEnvironmentVariable("CARGO_TARGET_" + $targetKey.ToUpperInvariant() + "_RUSTFLAGS", "-C link-arg=-Wl,-z,max-page-size=16384", "Process")
    $cargoArgs = @("build", "-p", "lumina-shelf-desktop", "--lib", "--target", $triple, "--features", "custom-protocol")
    if ($Release) { $cargoArgs += "--release" }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed for $target" }
    $jniDir = "src-tauri/gen/android/app/src/main/jniLibs/$abi"
    New-Item -ItemType Directory -Path $jniDir -Force | Out-Null
    Copy-Item -LiteralPath "target/$triple/$profileName/liblumina_shelf_desktop.so" -Destination "$jniDir/liblumina_shelf_desktop.so" -Force
    # Keep full symbols in target/ for debugging; ship only runtime symbols in APK.
    & (Join-Path $toolchain "llvm-strip.exe") --strip-unneeded "$jniDir/liblumina_shelf_desktop.so"
    if ($LASTEXITCODE -ne 0) { throw "Native library stripping failed" }
    # Gradle's incremental ZIP updates can retain obsolete .so bytes in an APK.
    # Recreate this generated output to keep the installable artifact compact.
    $apkOutput = "src-tauri/gen/android/app/build/outputs/apk/$arch/$profileName/app-$arch-$profileName.apk"
    if (Test-Path -LiteralPath $apkOutput) { Remove-Item -LiteralPath $apkOutput }
    Push-Location src-tauri/gen/android
    try {
        & .\gradlew.bat ":app:assemble$taskArch$profileTask" "-x" ":app:rustBuild$taskArch$profileTask" "-PabiList=$abi" "-ParchList=$arch" "-PtargetList=$target" "--console=plain"
        if ($LASTEXITCODE -ne 0) { throw "Gradle APK build failed for $target" }
    } finally { Pop-Location }
}

if ($Release -and ($Targets -contains "aarch64")) {
    $releaseDirectory = "src-tauri/gen/android/app/build/outputs/apk/arm64/release"
    $releaseApk = Get-ChildItem -LiteralPath $releaseDirectory -Filter "*.apk" -File |
        Where-Object { $_.Name -notmatch "androidTest" } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (-not $releaseApk) { throw "arm64 release APK was not produced" }
    $version = (Get-Content -Raw src-tauri/tauri.conf.json | ConvertFrom-Json).version
    $signedOutput = "artifacts/android/LuminaShelf_$($version)_arm64-release.apk"
    & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/sign-android.ps1 -Apk $releaseApk.FullName -Output $signedOutput
    if ($LASTEXITCODE -ne 0) { throw "Stable release signing failed" }
    Write-Host "Signed Android release: $signedOutput"
}

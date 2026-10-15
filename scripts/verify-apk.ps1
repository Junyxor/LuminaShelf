param(
    [string]$ApkPath,
    [string]$BuildInfoPath,
    [string]$ReportPath,
    [string]$ExpectedCommit
)

$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$package = Get-Content -LiteralPath (Join-Path $repository 'package.json') -Raw | ConvertFrom-Json
$version = [string]$package.version
$parts = $version.Split('.')
$versionCode = [int]$parts[0] * 1000000 + [int]$parts[1] * 1000 + [int]$parts[2]
if (-not $ApkPath) { $ApkPath = Join-Path $repository "artifacts/android/LuminaShelf-$version-universal-release.apk" }
if (-not $BuildInfoPath) { $BuildInfoPath = Join-Path $repository 'artifacts/android/universal-build-info.json' }
if (-not $ReportPath) { $ReportPath = Join-Path $repository 'artifacts/android/apk-verification.json' }
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { $env:ANDROID_SDK_ROOT }
if (-not $sdk -or -not $env:JAVA_HOME) { throw 'Set ANDROID_HOME and JAVA_HOME before verification' }
$ndk = if ($env:NDK_HOME) { $env:NDK_HOME } else { Join-Path $sdk 'ndk/27.2.12479018' }
$java = Join-Path $env:JAVA_HOME 'bin/java.exe'
$aapt = Join-Path $sdk 'build-tools/36.0.0/aapt2.exe'
$signer = Join-Path $sdk 'build-tools/36.0.0/lib/apksigner.jar'
$nm = Join-Path $ndk 'toolchains/llvm/prebuilt/windows-x86_64/bin/llvm-nm.exe'
$kotlin = Get-Content -LiteralPath (Join-Path $repository 'src-tauri/gen/android/app/src/main/java/app/luminashelf/client/generated/Rust.kt') -Raw
$nativeMethods = [regex]::Matches($kotlin, '@JvmStatic external fun (\w+)\(')
if (-not $nativeMethods.Count) { throw 'Generated Kotlin native declarations were not found' }
$badging = & $aapt dump badging $ApkPath
if ($LASTEXITCODE -ne 0) { throw 'APK manifest could not be read' }
$manifest = $badging -join "`n"
$identity = "package: name='app.luminashelf.client' versionCode='$versionCode' versionName='$version'"
if (-not $manifest.Contains($identity)) { throw 'Wrong package or version' }
if ($manifest -notmatch "sdkVersion:'24'" -or $manifest -notmatch "targetSdkVersion:'36'") { throw 'Wrong minimum or target SDK' }
if ($manifest -match 'application-debuggable') { throw 'Release APK is debuggable' }
$signature = & $java -jar $signer verify --verbose --print-certs $ApkPath
if ($LASTEXITCODE -ne 0) { throw 'APK signature verification failed' }
$signatureText = $signature -join "`n"
if ($signatureText -notmatch 'Verified using v2 scheme .*: true' -or $signatureText -notmatch 'Verified using v3 scheme .*: true') { throw 'APK v2 and v3 signatures are required' }
$certificate = ($signature | Select-String 'certificate SHA-256 digest:').ToString().Split(':')[-1].Trim()
if ($certificate -ne 'c8ccb838c8e3ca4460c0a18132bfe7a67a9a4d26365b33f3749aab199167e9ff') { throw 'Signing certificate differs from the previous release' }
& (Join-Path $sdk 'build-tools/36.0.0/zipalign.exe') -c -P 16 4 $ApkPath
if ($LASTEXITCODE -ne 0) { throw 'APK ZIP alignment failed' }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead([IO.Path]::GetFullPath($ApkPath))
$libraries = @()
$temporary = Join-Path ([IO.Path]::GetTempPath()) ('luminashelf-apk-check-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    if ($null -eq $archive.GetEntry('assets/open-source-notices.md')) { throw 'Open source notices are missing from APK assets' }
    foreach ($entry in $archive.Entries) {
        if ($entry.FullName -notmatch '^lib/([^/]+)/liblumina_shelf_desktop\.so$') { continue }
        $abi = $Matches[1]
        $architecture = @{
            'arm64-v8a' = @{ elfClass = 2; machine = 183 }
            'armeabi-v7a' = @{ elfClass = 1; machine = 40 }
            'x86_64' = @{ elfClass = 2; machine = 62 }
        }[$abi]
        if ($null -eq $architecture) { throw "Unexpected native architecture: $abi" }
        $destination = Join-Path $temporary "$abi.so"
        [System.IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $true)
        $bytes = [IO.File]::ReadAllBytes($destination)
        if ([BitConverter]::ToUInt32($bytes, 0) -ne 0x464c457f) { throw "$abi is not ELF" }
        if ($bytes[4] -ne $architecture.elfClass -or $bytes[5] -ne 1 -or [BitConverter]::ToUInt16($bytes, 18) -ne $architecture.machine) { throw "$abi contains the wrong ELF architecture" }
        $elf64 = $bytes[4] -eq 2
        if ($elf64) {
            $offset = [BitConverter]::ToUInt64($bytes, 32)
            $size = [BitConverter]::ToUInt16($bytes, 54)
            $count = [BitConverter]::ToUInt16($bytes, 56)
        } else {
            $offset = [BitConverter]::ToUInt32($bytes, 28)
            $size = [BitConverter]::ToUInt16($bytes, 42)
            $count = [BitConverter]::ToUInt16($bytes, 44)
        }
        $alignments = @()
        for ($index = 0; $index -lt $count; $index++) {
            $header = [int]($offset + $index * $size)
            if ([BitConverter]::ToUInt32($bytes, $header) -ne 1) { continue }
            $alignment = if ($elf64) { [BitConverter]::ToUInt64($bytes, $header + 48) } else { [BitConverter]::ToUInt32($bytes, $header + 28) }
            if ($alignment -lt 16384 -or $alignment % 16384 -ne 0) { throw "$abi contains a LOAD segment without 16 KB alignment" }
            $alignments += $alignment
        }
        if (-not $alignments.Count) { throw "$abi contains no LOAD segments" }
        $exports = & $nm --dynamic --defined-only $destination
        if ($LASTEXITCODE -ne 0) { throw "$abi exports could not be read" }
        foreach ($method in $nativeMethods) {
            $symbol = 'Java_app_luminashelf_client_Rust_' + $method.Groups[1].Value
            if (-not ($exports -match ($symbol + '$'))) { throw "$abi Android JNI export is missing: $symbol" }
        }
        $libraries += @{ abi = $abi; loadAlignments = $alignments; jniExportsVerified = $true; nativeMethods = $nativeMethods.Count; bytes = $entry.Length }
    }
} finally {
    $archive.Dispose()
    # Only explicit files in this invocation's unique temporary directory are removed.
    foreach ($abi in @('arm64-v8a', 'armeabi-v7a', 'x86_64')) {
        $extracted = Join-Path $temporary "$abi.so"
        if (Test-Path -LiteralPath $extracted) { Remove-Item -LiteralPath $extracted }
    }
    Remove-Item -LiteralPath $temporary
}
$abis = @($libraries | ForEach-Object { $_.abi } | Sort-Object)
if (($abis -join ',') -ne 'arm64-v8a,armeabi-v7a,x86_64') { throw "Wrong ABI list: $abis" }
$hash = (Get-FileHash -LiteralPath $ApkPath -Algorithm SHA256).Hash.ToLowerInvariant()
$build = Get-Content -LiteralPath $BuildInfoPath -Raw | ConvertFrom-Json
if ($build.sha256 -ne $hash -or $build.sourceDirty -or $build.version -ne $version -or $build.bytes -ne (Get-Item -LiteralPath $ApkPath).Length) { throw 'Build metadata differs from the verified artifact or source was dirty' }
if (-not $ExpectedCommit) {
    $ExpectedCommit = & git -C $repository rev-parse HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine the expected source revision' }
    $ExpectedCommit = $ExpectedCommit.Trim()
}
if ($build.sourceCommit -ne $ExpectedCommit) { throw 'Build metadata does not match the expected source revision' }
$report = @{ passed = $true; version = $version; versionCode = $versionCode; minSdk = 24; targetSdk = 36; debuggable = $false; signatureV2 = $true; signatureV3 = $true; zipAligned16KB = $true; sha256 = $hash; certificateSha256 = $certificate; libraries = $libraries; sourceCommit = $build.sourceCommit; bytes = $build.bytes }
New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($ReportPath))) -Force | Out-Null
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ReportPath -Encoding utf8
$report | ConvertTo-Json -Depth 8

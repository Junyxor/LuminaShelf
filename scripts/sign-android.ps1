param(
    [Parameter(Mandatory=$true)][string]$Apk,
    [string]$Output
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $env:JAVA_HOME -or -not $env:ANDROID_HOME) { throw "Set JAVA_HOME and ANDROID_HOME." }

$keyDirectory = Join-Path (Get-Location) ".signing"
$keystore = Join-Path $keyDirectory "luminashelf-release.p12"
$protectedPassword = Join-Path $keyDirectory "password.dpapi"
New-Item -ItemType Directory -Path $keyDirectory -Force | Out-Null
Add-Type -AssemblyName System.Security
$callerSecret = $env:LUMINASHELF_SIGNING_PASSWORD

function Save-ProtectedPassword([string]$Secret) {
    $plain = [Text.Encoding]::UTF8.GetBytes($Secret)
    try {
        $encrypted = [Security.Cryptography.ProtectedData]::Protect(
            $plain,
            $null,
            [Security.Cryptography.DataProtectionScope]::CurrentUser
        )
        [IO.File]::WriteAllBytes($protectedPassword, $encrypted)
    } finally {
        [Array]::Clear($plain, 0, $plain.Length)
    }
}

if (-not (Test-Path -LiteralPath $keystore)) {
    if (Test-Path -LiteralPath $protectedPassword) {
        throw "A saved signing password exists without its keystore. Restore the key before continuing."
    }

    if ([string]::IsNullOrWhiteSpace($callerSecret)) {
        $random = New-Object byte[] 32
        $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
        try { $generator.GetBytes($random) } finally { $generator.Dispose() }
        $signingSecret = [Convert]::ToBase64String($random)
        [Array]::Clear($random, 0, $random.Length)
    } else {
        $signingSecret = $callerSecret
    }

    Save-ProtectedPassword $signingSecret
    $env:LUMINASHELF_SIGNING_PASSWORD = $signingSecret
    try {
        & (Join-Path $env:JAVA_HOME "bin/keytool.exe") -genkeypair -keystore $keystore -storetype PKCS12 -alias luminashelf -keyalg RSA -keysize 3072 -validity 10000 -dname "CN=LuminaShelf" -storepass:env LUMINASHELF_SIGNING_PASSWORD -keypass:env LUMINASHELF_SIGNING_PASSWORD
        if ($LASTEXITCODE -ne 0) {
            throw "Signing key generation failed. The saved password has been preserved for recovery."
        }
    } finally {
        Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue
        $signingSecret = $null
    }
} elseif (-not (Test-Path -LiteralPath $protectedPassword)) {
    if ([string]::IsNullOrWhiteSpace($callerSecret)) {
        throw "The local DPAPI password cache is missing. Set LUMINASHELF_SIGNING_PASSWORD to the backed-up release-key password to restore this machine; do not generate a replacement key."
    }
    $env:LUMINASHELF_SIGNING_PASSWORD = $callerSecret
    try {
        & (Join-Path $env:JAVA_HOME "bin/keytool.exe") -list -keystore $keystore -alias luminashelf -storepass:env LUMINASHELF_SIGNING_PASSWORD *> $null
        if ($LASTEXITCODE -ne 0) {
            throw "The supplied release-key password cannot open the existing keystore."
        }
        Save-ProtectedPassword $callerSecret
    } finally {
        Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue
    }
}

if (-not (Test-Path -LiteralPath $protectedPassword)) {
    throw "The signing password cache is missing; restore it instead of replacing the key."
}
$secretBytes = [Security.Cryptography.ProtectedData]::Unprotect(
    [IO.File]::ReadAllBytes($protectedPassword),
    $null,
    [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$env:LUMINASHELF_SIGNING_PASSWORD = [Text.Encoding]::UTF8.GetString($secretBytes)

try {
    $version = (Get-Content -Raw src-tauri/tauri.conf.json | ConvertFrom-Json).version
    if (-not $Output) { $Output = "artifacts/android/LuminaShelf_$($version)_arm64.apk" }
    $toolsDir = Get-ChildItem (Join-Path $env:ANDROID_HOME "build-tools") -Directory |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1
    if (-not $toolsDir) { throw "Android build-tools are not installed." }

    New-Item -ItemType Directory -Path (Split-Path $Output -Parent) -Force | Out-Null
    $alignedInput = Join-Path $env:TEMP ("luminashelf-aligned-" + [Guid]::NewGuid().ToString("N") + ".apk")
    try {
        # zipalign must run before apksigner. -P 16 also aligns uncompressed
        # native libraries for modern Android 16 KiB page-size requirements.
        & (Join-Path $toolsDir.FullName "zipalign.exe") -f -P 16 4 $Apk $alignedInput
        if ($LASTEXITCODE -ne 0) { throw "APK zipalign failed before signing" }
        & (Join-Path $toolsDir.FullName "apksigner.bat") sign --ks $keystore --ks-key-alias luminashelf --ks-pass env:LUMINASHELF_SIGNING_PASSWORD --out $Output $alignedInput
        if ($LASTEXITCODE -ne 0) { throw "APK signing failed" }
    } finally {
        Remove-Item -LiteralPath $alignedInput -Force -ErrorAction SilentlyContinue
    }

    & (Join-Path $toolsDir.FullName "apksigner.bat") verify --verbose --print-certs $Output
    if ($LASTEXITCODE -ne 0) { throw "APK signature verification failed" }
    & (Join-Path $toolsDir.FullName "zipalign.exe") -c -P 16 4 $Output
    if ($LASTEXITCODE -ne 0) { throw "APK alignment verification failed" }

    $hash = Get-FileHash -LiteralPath $Output -Algorithm SHA256
    $hash | Select-Object Path,Hash
    $hash.Hash | Set-Content -LiteralPath ($Output + ".sha256") -Encoding ascii
} finally {
    Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue
    [Array]::Clear($secretBytes, 0, $secretBytes.Length)
}

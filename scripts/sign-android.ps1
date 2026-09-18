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
if (-not (Test-Path -LiteralPath $keystore)) {
    if (Test-Path -LiteralPath $protectedPassword) { throw "A saved signing password exists without its keystore. Restore the key before continuing." }
    $random = New-Object byte[] 32
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    $generator.GetBytes($random)
    $generator.Dispose()
    $signingSecret = [Convert]::ToBase64String($random)
    $encrypted = [Security.Cryptography.ProtectedData]::Protect([Text.Encoding]::UTF8.GetBytes($signingSecret), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [IO.File]::WriteAllBytes($protectedPassword, $encrypted)
    $env:LUMINASHELF_SIGNING_PASSWORD = $signingSecret
    try {
        & (Join-Path $env:JAVA_HOME "bin/keytool.exe") -genkeypair -keystore $keystore -storetype PKCS12 -alias luminashelf -keyalg RSA -keysize 3072 -validity 10000 -dname "CN=LuminaShelf" -storepass:env LUMINASHELF_SIGNING_PASSWORD -keypass:env LUMINASHELF_SIGNING_PASSWORD
        if ($LASTEXITCODE -ne 0) { throw "Signing key generation failed. The saved password has been preserved for recovery." }
    } finally { Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue }
}
if (-not (Test-Path -LiteralPath $protectedPassword)) { throw "The signing password file is missing; restore it instead of replacing the key." }
$secretBytes = [Security.Cryptography.ProtectedData]::Unprotect([IO.File]::ReadAllBytes($protectedPassword), $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
$env:LUMINASHELF_SIGNING_PASSWORD = [Text.Encoding]::UTF8.GetString($secretBytes)
try {
    $version = (Get-Content -Raw src-tauri/tauri.conf.json | ConvertFrom-Json).version
    if (-not $Output) { $Output = "artifacts/android/LuminaShelf_$($version)_arm64.apk" }
    $toolsDir = Get-ChildItem (Join-Path $env:ANDROID_HOME "build-tools") -Directory | Sort-Object Name -Descending | Select-Object -First 1
    New-Item -ItemType Directory -Path (Split-Path $Output -Parent) -Force | Out-Null
    & (Join-Path $toolsDir.FullName "apksigner.bat") sign --ks $keystore --ks-key-alias luminashelf --ks-pass env:LUMINASHELF_SIGNING_PASSWORD --out $Output $Apk
    if ($LASTEXITCODE -ne 0) { throw "APK signing failed" }
    & (Join-Path $toolsDir.FullName "apksigner.bat") verify --verbose --print-certs $Output
    if ($LASTEXITCODE -ne 0) { throw "APK signature verification failed" }
    & (Join-Path $toolsDir.FullName "zipalign.exe") -c -P 16 4 $Output
    if ($LASTEXITCODE -ne 0) { throw "APK alignment verification failed" }
    Get-FileHash -LiteralPath $Output -Algorithm SHA256 | Select-Object Path,Hash
} finally {
    Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue
    [Array]::Clear($secretBytes,0,$secretBytes.Length)
}

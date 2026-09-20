param(
    [switch]$Show
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $env:JAVA_HOME) { throw "Set JAVA_HOME before exporting signing recovery information." }
Add-Type -AssemblyName System.Security

$keyDirectory = Join-Path (Get-Location) ".signing"
$keystore = Join-Path $keyDirectory "luminashelf-release.p12"
$protectedPassword = Join-Path $keyDirectory "password.dpapi"
$marker = Join-Path $keyDirectory "recovery-confirmed"

if (-not (Test-Path -LiteralPath $keystore)) { throw "Release keystore does not exist yet. Build a signed release first." }
if (-not (Test-Path -LiteralPath $protectedPassword)) { throw "Local DPAPI password cache is missing. Restore LUMINASHELF_SIGNING_PASSWORD first." }

$secretBytes = [Security.Cryptography.ProtectedData]::Unprotect(
    [IO.File]::ReadAllBytes($protectedPassword),
    $null,
    [Security.Cryptography.DataProtectionScope]::CurrentUser
)
$secret = [Text.Encoding]::UTF8.GetString($secretBytes)
$env:LUMINASHELF_SIGNING_PASSWORD = $secret

try {
    Write-Host "=== LuminaShelf Android release certificate ==="
    & (Join-Path $env:JAVA_HOME "bin/keytool.exe") -list -v -keystore $keystore -alias luminashelf -storepass:env LUMINASHELF_SIGNING_PASSWORD
    if ($LASTEXITCODE -ne 0) { throw "The release keystore could not be verified." }

    if ($Show) {
        Write-Warning "The next line is the release-key password. Store it in a trusted password manager/offline recovery record and do not commit it."
        Write-Output $secret
    } elseif (Get-Command Set-Clipboard -ErrorAction SilentlyContinue) {
        Set-Clipboard -Value $secret
        Write-Host "Release-key password copied to the clipboard. Save it now in a trusted password manager/offline recovery record."
    } else {
        throw "Set-Clipboard is unavailable. Re-run with -Show and securely record the displayed password."
    }

    $keystoreHash = (Get-FileHash -LiteralPath $keystore -Algorithm SHA256).Hash
    @(
        "confirmedAt=$([DateTime]::UtcNow.ToString("o"))",
        "keystoreSha256=$keystoreHash"
    ) | Set-Content -LiteralPath $marker -Encoding ascii

    Write-Host ""
    Write-Host "Recovery confirmation recorded for this exact keystore."
    Write-Host "Back up BOTH files outside this computer:"
    Write-Host "  $keystore"
    Write-Host "  the password just copied/displayed"
    Write-Host "Keystore SHA-256: $keystoreHash"
} finally {
    Remove-Item Env:LUMINASHELF_SIGNING_PASSWORD -ErrorAction SilentlyContinue
    $secret = $null
    [Array]::Clear($secretBytes, 0, $secretBytes.Length)
}

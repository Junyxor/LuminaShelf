param(
    [string]$Tag,
    [string]$Apk
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)

if (-not $env:ANDROID_HOME) { throw "Set ANDROID_HOME before publishing." }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "GitHub CLI (gh) is required." }

$keyDirectory = Join-Path (Get-Location) ".signing"
$keystore = Join-Path $keyDirectory "luminashelf-release.p12"
$recoveryMarker = Join-Path $keyDirectory "recovery-confirmed"
if (-not (Test-Path -LiteralPath $keystore)) { throw "The stable Android release keystore is missing." }
if (-not (Test-Path -LiteralPath $recoveryMarker)) {
    throw "Signing-key recovery has not been confirmed. Run npm run android:signing:recovery and back up the keystore plus password before publishing."
}
$markerValues = @{}
Get-Content -LiteralPath $recoveryMarker | ForEach-Object {
    $parts = $_ -split "=", 2
    if ($parts.Length -eq 2) { $markerValues[$parts[0]] = $parts[1] }
}
$currentKeystoreHash = (Get-FileHash -LiteralPath $keystore -Algorithm SHA256).Hash
if ($markerValues["keystoreSha256"] -ne $currentKeystoreHash) {
    throw "The release keystore changed after recovery confirmation. Re-run npm run android:signing:recovery and back up the current key before publishing."
}

$config = Get-Content -Raw src-tauri/tauri.conf.json | ConvertFrom-Json
$version = [string]$config.version
$expectedTag = "v$version"
if (-not $Tag) { $Tag = $expectedTag }
if ($Tag -ne $expectedTag) { throw "Tag $Tag does not match application version $version ($expectedTag)." }
if (-not $Apk) { $Apk = "artifacts/android/LuminaShelf_$($version)_arm64-release.apk" }
$Apk = (Resolve-Path -LiteralPath $Apk).Path

$toolsDir = Get-ChildItem (Join-Path $env:ANDROID_HOME "build-tools") -Directory |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
if (-not $toolsDir) { throw "Android build-tools are not installed." }

$apksigner = Join-Path $toolsDir.FullName "apksigner.bat"
$zipalign = Join-Path $toolsDir.FullName "zipalign.exe"
$aapt2 = Join-Path $toolsDir.FullName "aapt2.exe"

& $apksigner verify --verbose --print-certs $Apk
if ($LASTEXITCODE -ne 0) { throw "APK signature verification failed." }
& $zipalign -c -P 16 4 $Apk
if ($LASTEXITCODE -ne 0) { throw "APK 16 KiB/ZIP alignment verification failed." }

$badging = (& $aapt2 dump badging $Apk | Select-Object -First 1)
if ($LASTEXITCODE -ne 0 -or -not $badging) { throw "Could not read APK package metadata." }
$package = [regex]::Match($badging, "name='([^']+)'").Groups[1].Value
$versionName = [regex]::Match($badging, "versionName='([^']+)'").Groups[1].Value
$versionCode = [regex]::Match($badging, "versionCode='([^']+)'").Groups[1].Value
$parts = $version.Split(".") | ForEach-Object { [int]$_ }
$expectedVersionCode = $parts[0] * 1000000 + $parts[1] * 1000 + $parts[2]
if ($package -ne "app.luminashelf.client") { throw "Unexpected Android package: $package" }
if ($versionName -ne $version) { throw "APK versionName $versionName does not match $version." }
if ([int64]$versionCode -ne $expectedVersionCode) { throw "APK versionCode $versionCode does not match $expectedVersionCode." }

$head = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw "Cannot resolve Git HEAD." }
$tagCommit = (git rev-list -n 1 $Tag 2>$null).Trim()
if (-not $tagCommit) { throw "Tag $Tag does not exist locally. Create/push the final tag only after the RC is accepted." }
if ($tagCommit -ne $head) { throw "Tag $Tag points to $tagCommit, but this checkout is $head." }

gh auth status | Out-Host
if ($LASTEXITCODE -ne 0) { throw "GitHub CLI is not authenticated." }

$hash = Get-FileHash -LiteralPath $Apk -Algorithm SHA256
$hashFile = "$Apk.sha256"
("$($hash.Hash)  $([IO.Path]::GetFileName($Apk))") | Set-Content -LiteralPath $hashFile -Encoding ascii

$releaseExists = $true
gh release view $Tag --repo Junyxor/LuminaShelf *> $null
if ($LASTEXITCODE -ne 0) { $releaseExists = $false }

if ($releaseExists) {
    gh release upload $Tag $Apk $hashFile --repo Junyxor/LuminaShelf --clobber
    if ($LASTEXITCODE -ne 0) { throw "Uploading Android release assets failed." }
} else {
    gh release create $Tag $Apk $hashFile --repo Junyxor/LuminaShelf --target $head --title "LuminaShelf $Tag" --generate-notes
    if ($LASTEXITCODE -ne 0) { throw "Creating the GitHub release failed." }
}

Write-Host "Published $Tag Android release asset."
Write-Host "SHA-256: $($hash.Hash)"

# Android release builds

The release uses application ID `app.luminashelf.client`, Android API 24 or newer,
and target API 36. The universal release contains arm64-v8a, armeabi-v7a, x86
and x86_64. GitHub Actions also provides separate x86 and x86_64 APKs.

## Requirements

- Node.js 24 or newer and a current Rust stable toolchain
- JDK 17
- Android SDK Platform 36, Build Tools 36.0.0, Platform Tools
- Android NDK 27.2.12479018

Install the Rust targets with:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
npm ci
```

Set `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` to the installed tools. On Windows,
use an ASCII path for `CARGO_TARGET_DIR` to avoid the NDK batch linker corrupting
non-ASCII paths. When omitted on Windows, the build script uses a project-specific
directory under the temporary directory. Keep the SDK and NDK paths ASCII too.

For example, in PowerShell:

```powershell
$env:JAVA_HOME = 'C:/Tools/jdk-17'
$env:ANDROID_HOME = 'C:/Tools/android-sdk'
$env:NDK_HOME = "$env:ANDROID_HOME/ndk/27.2.12479018"
$env:CARGO_TARGET_DIR = 'C:/Tools/luminashelf-target'
npm run android:release
```

On Windows, the build script compiles Rust and copies its JNI libraries before
running Gradle, so Windows Developer Mode and symbolic link privileges are optional.
It also disables Kotlin incremental compilation on Windows because the Cargo
registry and project can reside on different drives. Android project preparation
adds network state access, the project icons and the compatible Kotlin compiler.
Preparation also copies tracked light-theme and Activity templates from
`src-tauri/android/`, including system-bar/cutout/keyboard insets and hardware back
navigation. Cargo and Gradle default to two workers.
Repository text files use LF line endings so Tauri's first Android initialization
does not appear to modify `Cargo.toml` on a Windows checkout. CI runs that
initialization and checks for a clean source tree before compiling the ABIs.
Android System DNS uses lazy operating-system lookups, so launching the reader
does not require an active network and DNS follows VPN/Private DNS/network changes.

## Output and signing identity

`artifacts/android/LuminaShelf-0.8.0-universal-release.apk` is the universal release.
The independent packages use the `-x86-release.apk` and `-x86_64-release.apk`
suffixes. All variants have the same version, application ID and signing identity.
The same folder contains its `.sha256` checksum, `release-certificate.pem` and
`universal-build-info.json`. Release builds use R8 and optimized, stripped Rust
libraries; native ELF segments and APK native library entries are aligned for
16 KB pages.

After building, run `./scripts/verify-apk.ps1` in PowerShell with the same SDK/JDK
environment. It verifies the manifest, non-debuggable flag, the exact expected ABI set, original
certificate, v2/v3 signatures, 16 KB ZIP/ELF alignment and every JNI method declared
in the generated Kotlin runtime. CI runs this check and uploads
`universal-apk-verification.json` alongside the APK. For independent packages,
use `./scripts/verify-apk.ps1 -Variant x86` or `-Variant x86_64`; reports and build
metadata are named separately for each variant.

The first release creates `.signing/luminashelf-release.p12` (a PKCS#12 keystore
with a 3072-bit RSA key) and `.signing/release.json` (alias and generated password).
Back up both files together. Keep that backup private and retain it for future
updates: Android requires the same signing certificate for an in-place upgrade.
The public certificate can be shared with the APK. Signing passwords are passed
through process environment variables, not command-line arguments.

For an arm64-only release:

```bash
npm run android:release -- aarch64
```

For the independent x86 and x86_64 signed releases:

```bash
npm run android:release -- i686
npm run android:release -- x86_64
```

For a debug APK:

```bash
npm run android:apk
```

For an x86_64 emulator debug APK:

```bash
node scripts/android-build.mjs --debug x86_64
```

Install an APK with Android Platform Tools:

```bash
adb install -r artifacts/android/LuminaShelf-0.8.0-universal-release.apk
```

The app-local default download folder on Android is under
`Android/data/app.luminashelf.client/files/Download/LuminaShelf`. The library page
supports EPUB/PDF/TXT imports using Android's Storage Access Framework picker.
Selected content URIs are streamed into private app storage; no broad storage
permission is requested. The picker opens only after an explicit import action.
Z-Library account tests require a user-owned account and a reachable EAPI origin.

## GitHub Actions and signing Secrets

The `Android CI` workflow checks TypeScript, Rust libraries, formatting, Clippy and
the production frontend at four widths before building three signed APK variants.
It uses Windows with SDK 36 / NDK 27.2 / JDK 17 and two build workers. The subsequent
Ubuntu job boots the actual signed APK in a headless Android 35 emulator and
checks the universal package's real HTTPS download, background service, pause,
process recovery, Android long press, resume and EPUB reader. It then installs
the independent x86_64 package over the same app and verifies startup. The x86
package receives signature/ELF/JNI/alignment checks; it is not runtime-tested on
the x86_64 emulator. The job collects UI trees, screenshots, process ID and logcat.
The emulator action invokes one standalone shell script
so APK path variables survive between commands. A launch check does not substitute for
account, reader-file, sharing and document-provider acceptance on real devices.

The following repository Actions Secrets hold the existing release identity:

- `LUMINASHELF_ANDROID_KEYSTORE`: base64-encoded PKCS#12 keystore.
- `LUMINASHELF_ANDROID_STORE_PASSWORD`: keystore password.
- `LUMINASHELF_ANDROID_KEY_ALIAS`: original alias.

`scripts/upload-signing-secrets.mjs` uploads them using authenticated GitHub CLI
stdin without printing values. `scripts/restore-signing.mjs` reconstructs the private
files only on the build runner. Private files are excluded from Git/artifacts and
removed in the final workflow step. CI refuses to generate a replacement key, and
release signing verifies the existing public certificate fingerprint.

Run the workflow manually on the desired branch. Keep `publish_prerelease` false
for an artifact-only build; turn it on and supply an explicit `v` tag only when
publishing a prerelease. A pushed `v*` tag publishes a release. The uploaded artifact
names are `LuminaShelf-signed-universal`, `LuminaShelf-signed-x86` and
`LuminaShelf-signed-x86_64`; the Android acceptance evidence is
`Android-release-smoke`.

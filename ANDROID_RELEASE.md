# Android release builds

The release uses application ID `app.luminashelf.client`, Android API 24 or newer,
and target API 36. A normal release contains arm64-v8a, armeabi-v7a and x86_64.

## Requirements

- Node.js 24 or newer and a current Rust stable toolchain
- JDK 17
- Android SDK Platform 36, Build Tools 36.0.0, Platform Tools
- Android NDK 27.2.12479018

Install the Rust targets with:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
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
Android System DNS uses lazy operating-system lookups, so launching the reader
does not require an active network and DNS follows VPN/Private DNS/network changes.

## Output and signing identity

`artifacts/android/LuminaShelf-0.6.0-universal-release.apk` is the installable release.
The same folder contains its `.sha256` checksum, `release-certificate.pem` and
`universal-build-info.json`. Release builds use R8 and optimized, stripped Rust
libraries; native ELF segments and APK native library entries are aligned for
16 KB pages.

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
adb install -r artifacts/android/LuminaShelf-0.6.0-universal-release.apk
```

The app-local default download folder on Android is under
`Android/data/app.luminashelf.client/files/Download/LuminaShelf`. The library page
supports EPUB/PDF/TXT imports using Android's Storage Access Framework picker.
Selected content URIs are streamed into private app storage; no broad storage
permission is requested. The picker opens only after an explicit import action.
Z-Library account tests require a user-owned account and a reachable EAPI origin.

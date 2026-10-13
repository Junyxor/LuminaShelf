# Release validation

Prepared on 2026-10-13 from upstream commit `8bf3baa` plus this branch's release work.

The delivered APK is a release build of LuminaShelf 0.5.0 with application ID
`app.luminashelf.client`, version code 5000, minimum SDK 24 and target SDK 36.
It includes arm64-v8a, armeabi-v7a and x86_64. Release signing uses the locally
generated RSA-3072 certificate kept in `.signing/`.

## Completed checks

- Clean frontend dependency installation from the committed npm lockfile.
- TypeScript checking and production Vite build.
- Rust library test suite: 31 core tests and 3 Tauri bridge tests.
- Rust formatting and core Clippy checks with warnings treated as errors.
- Android release compilation for all three ABIs and Gradle/R8 release packaging.
- Android release Lint Vital checks.
- APK signature schemes v2 and v3 verified with Android `apksigner`.
- Native library ZIP alignment checked with `zipalign -c -P 16 4`.
- arm64 and x86_64 ELF LOAD segment alignment checked as `0x4000` (16 KB).
- Packaged manifest, application version, supported ABIs and release debug flag checked.
- JNI exports checked against the generated Android package name.

The Android system resolver now defers OS DNS lookups until a request is made,
avoiding an eager JNI query for an active network during application startup.
Host tests cover the native resolver path and offline App Hosts overrides.

## Remaining runtime validation

No physical Android device was connected. This Windows host's Android emulator
crashed before reaching an ADB boot, with exit status `0xC0000005`; this happened
with current and archived emulator builds, software rendering, acceleration
disabled and a fresh minimal AVD. APK installation, Android UI/reading workflows,
offline startup on a device and authenticated Z-Library workflows remain unverified.

Use the APK checksum and certificate in `artifacts/android/` to identify the
delivered binary. `universal-build-info.json` records its source commit, build
time, size and SHA-256 hash. Back up the keystore and credentials together;
future updates must use that same signing identity.

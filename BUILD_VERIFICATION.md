# Release validation

Prepared on 2026-10-14 from upstream commit `8bf3baa` plus the release and light-reader/network changes.

The release target is LuminaShelf 0.6.0 with application ID
`app.luminashelf.client`, version code 6000, minimum SDK 24 and target SDK 36.
It includes arm64-v8a, armeabi-v7a and x86_64. Release signing uses the locally
generated RSA-3072 certificate kept in `.signing/`.

## Completed checks

- TypeScript checking and production Vite build.
- Rust library test suite: 39 core tests and 3 Tauri bridge tests.
- Rust formatting and core Clippy checks with warnings treated as errors.
- Loopback HTTPS DoH transport tests: full custom port/path/query/authority, raw DNS wire messages, DNS NXDOMAIN inside HTTP 200, and rejection of untrusted certificates.
- Independent A/AAAA query deadlines: an unanswered AAAA query preserves a successful A response.
- Hosts changes reach a second real HTTP listener instead of reusing a connection to the old address.
- Actual public AliDNS and Tencent DoH DNS resolution and Gutendex search, with complete endpoint URLs, no bootstrap IP and system fallback disabled.
- Headless UI checks at 320, 390, 768 and 1280 px: exact custom endpoint save, multi-address Hosts round trip, minimum 48 px reader controls, visible settings, no page overflow, light theme even under a dark OS preference.
- Actual Windows Tauri/WebView2 IPC, using an isolated data directory and a hidden window: native startup, DoH/HTTPS source check, automatic network selection preserving disabled fallback, real search, real 189,249-byte EPUB download, library insertion and chapter reading.
- Native TXT/PDF import without a file picker, reader scroll progress saved to SQLite, actual PDF.js worker rendering and persisted page navigation to page 2.

UI fixture checks are separate from live native checks. Their scripts and evidence
are `scripts/check-ui.mjs`, `scripts/check-native.mjs` and the ignored
`artifacts/testing/v06/` folder. No browser window, native picker, emulator or
authorization prompt is opened by these checks.

Cloudflare's public DoH endpoint was also tried. This host's network reset its
TLS connection (Windows error 10054); it was correctly reported as a failed route,
without silently falling back. Endpoint reachability depends on the user's network.

## Signed APK verification

`npm run android:release` compiles all three ABIs, runs Gradle/R8 release packaging
and Lint Vital, signs with the existing key, checks APK v2/v3 signatures and runs
`zipalign -c -P 16 4`. Final verification also inspects the packaged manifest,
version, ABI list, debug flag, JNI exports and ELF LOAD segment alignment.
`artifacts/android/universal-build-info.json` records the actual artifact's source
commit, dirty state, build time, size and SHA-256 hash. Release builds use two
Cargo jobs and two Gradle workers.

The Android system resolver now defers OS DNS lookups until a request is made,
avoiding an eager JNI query for an active network during application startup.
Host tests cover the native resolver path and offline App Hosts overrides.

## Remaining runtime validation

No physical Android device was connected. This Windows host's previously attempted Android emulator
crashed before reaching an ADB boot, with exit status `0xC0000005`; this happened
with current and archived emulator builds, software rendering, acceleration
disabled and a fresh minimal AVD. APK installation, Android UI/reading workflows,
offline startup on a device and authenticated Z-Library workflows remain unverified.
Windows native QA and protocol tests do not substitute for Android device testing.

Use the APK checksum and certificate in `artifacts/android/` to identify the
delivered binary. `universal-build-info.json` records its source commit, build
time, size and SHA-256 hash. Back up the keystore and credentials together;
future updates must use that same signing identity.

# Release validation

## 0.7.0 acceptance record — 2026-10-15

The 0.7 release target is application ID `app.luminashelf.client`, version code
7000, minimum SDK 24, target SDK 36, and the existing RSA-3072 signing certificate.

Locally completed checks:

- 44 core tests and 10 Tauri bridge tests, including real HTTPS DoH wire requests,
  persistent named DNS/favorites, JSON download descriptor validation, redirect
  handling, Chinese TXT/EPUB round trips and mirror-table exclusion rules.
- Production TypeScript/Vite build and headless UI workflows at 320/390/768/1280 px:
  DNS add/edit/select/delete/reload, optional multiple bootstrap IPs, independent
  favorites, selected download format, long-press file management, 48 px reader
  controls, mirror testing and an honest “no published release” update state.
- A hidden real Windows Tauri instance completed DNS application with system
  fallback disabled, automatic selection followed by real search, a 189,249-byte
  EPUB download and native chapter read, TXT/PDF import, PDF rendering/page/zoom
  progress, TXT→EPUB→TXT conversion and scoped deletion of a QA-managed file.
- Review found and reproduced an export-to-original data-loss bug and a profile
  mix-up when two saved DNS configurations share an endpoint but differ in proxy,
  Hosts, fallback or timeout. Both regressions failed before the fixes and passed
  afterward. Export checks opened-file identity before truncation, including hard
  links; DNS selection compares the full normalized policy. The UI regression runs
  at all four widths and checks editing/saving/deleting the correct profile.
- A separate regression reproduces and fixes announcement cache collisions between
  the same host's different HTTPS ports. Cached feeds are keyed by complete origin
  and checked against the current source before normal display or error fallback,
  keeping another site's activity/donation links out of the selected site's feed.
- Live directory and GitHub community-table retrieval produced 22 unique mirror
  candidates; `zlib.bz` passed current EAPI checks. Selecting it returned 100 actual
  popularity cards, real details and the actual available format. These are
  time-specific reachability results, not a guarantee for every network.
- Announcement checks returned no current source campaign; source HTML returned
  HTTP 503 and was reported. The project had no published GitHub Release and was
  reported as such. No login, email, cloud save, donation or external application
  was invoked by the QA scripts.

The scripts write fresh evidence under `artifacts/testing/v07/`. UI fixtures are
separate from live native checks. GitHub Actions defines signed build and Android
launch verification; its actual run conclusion and uploaded artifacts are the
authoritative CI result. APK identity/source/hash data is generated in
`artifacts/android/*-build-info.json` by the release script.
`scripts/verify-apk.ps1` independently checks the signed APK manifest, three ABIs,
v2/v3 signatures, certificate continuity, 16 KB ZIP/ELF alignment and Kotlin/JNI
exports. CI uploads its report with the signed package.

Real-device document providers, installed external reader/share applications and
authenticated vendor conversion/email/Kindle/cloud actions remain manual checks
listed in `docs/ANDROID_ACCEPTANCE.md`. Website actions use explicit website
entry points; local text conversion and Android system sharing are separate features.

## 0.6 historical validation

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

# Android 1.0.0 validation — 2026-09-18

Delivery target: Android arm64 APK, application ID `app.luminashelf.client`,
versionName `1.0.0`, versionCode `6000`, minSdk 24, targetSdk 36.

Delivered file: `artifacts/android/LuminaShelf_1.0.0_arm64-debug.apk`
(45,160,924 bytes, approximately 43.1 MiB).
SHA-256: `36f4820ba11eefcaed3f6e44ed48387db536c2391f8aff10da5623e39dab02cc`.
APK Signature Scheme v2 verification passed; ZIP and ELF LOAD alignment are 16KB compatible.

Local environment: Android 15 / API 35 AOSP x86_64 emulator on WHPX,
stock Android WebView 124.0.6367.219; NDK r27c; JDK 17; SDK 36.
The arm64 and emulator APKs are built from the same source and web assets.

## Device workflows

`npm run test:android` drives the actual Android DocumentsUI and embedded
WebView, with real Tauri IPC and Rust commands:

- Fresh startup, including system DNS initialization and secure session bootstrap.
- Import TXT, EPUB and PDF using Android SAF content URIs into app-private storage.
- TXT reading and Android Back returning to the library.
- EPUB chapter loading and navigation.
- PDF range loading/rendering using the PDF.js legacy build on WebView 124.
- Force-stop/relaunch preserves library, EPUB chapter and PDF page.
- A separate non-secret Keystore test entry survives process restart, and its
deletion survives a second restart. The account's session is never used by this test.
- Android settings expose managed storage rather than desktop paths.

Evidence: `artifacts/android/smoke/result.json` and adjacent device screenshots.

## Native core and frontend

All 37 native Rust core tests pass on the Android emulator, including localhost
HTTP sequential/segmented resume, rejection of incorrect Content-Range,
removal of invalid download content, no-overwrite publication, real SQLite
reopening and failed import cleanup. They use the Android filesystem and libc.
`scripts/run-android-test.ps1` is the Cargo runner used locally.

Five frontend interaction tests pass, including Android-only import surfaces and
Back handling. Workspace formatting and Clippy run with warnings treated as errors.
Android CI now builds both arm64 and x86_64 and runs the emulator workflow.
The updated CI definition has not been executed on GitHub in this local session.

## Bugs found through Android execution

- System resolver setup tried to read a desktop DNS configuration file, causing
  immediate Android startup abort. Android now uses Bionic's system lookup.
- Modern PDF.js used Promise.try, unavailable in WebView 124. Main runtime and
  worker now use the matching legacy build.
- Hard-link publication was denied by Android policy. Android now uses
  renameat2 with RENAME_NOREPLACE, preserving conflicting existing books.

Code review also led to holding transfer ownership until all disk writes and
manifest updates finish; rejecting unsafe legacy-file adoption; removing poisoned
resume data; checking secure-storage commits; and MIME fallback for SAF names.
The small Android keyring fix is vendored with both upstream licenses.

Review resolution: the Standards review reported transfer finalization, conflicting
file preservation, secure-storage commit handling and SAF MIME fallback issues.
The Spec review reported the overlapping transfer, conflicting file and poisoned
retry cases. Each reported issue was addressed; the filesystem and device tests
above validate the resulting behavior.

## Remaining external validation

No physical arm64 handset was connected. The actual arm64 APK is built and its
signature, ABI, manifest and 16KB ZIP/ELF alignment are checked locally; runtime
flows were exercised on the companion x86_64 emulator APK.

Gutendex requests timed out from this environment, so a live public
search/download roundtrip is not claimed. Authenticated Z-Library requests require
a real account and available endpoint; no user credentials were supplied.
The download engine is covered by deterministic HTTP integration tests.

The release candidate is an installable signed R8 build. Store publication still
requires the owner's release-channel signing policy and store metadata. Android
foreground-service downloads are implemented with the system dataSync service;
Android 15's six-hour service window remains an operating-system limit. Full
EPUB visual layout is intentionally a safe structural renderer, not a browser.

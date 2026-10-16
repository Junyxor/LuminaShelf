# Release validation

## 0.8.0 acceptance — 2026-10-16

Target: same application ID/certificate as 0.7, version code 8000, min SDK24,
target SDK36. Builds provide x86, x86_64 and universal APKs; universal includes
arm64-v8a/armeabi-v7a/x86/x86_64. Each artifact verifies its exact ABI set, ELF
machine/class, JNI exports,16KB alignment, signature and matching source metadata.

Download regressions use real local HTTP sockets: body disconnect and offset
retry, actual simultaneous ranges without a HEAD declaration, range ignored
initially or after probing, mismatched/416 responses, segment pause with no writes
after return, new signed URL and connection limit, ETag change, full legacy-prefix
comparison and cross-origin credential removal. A stalled-header test exercises
the actual 25-second response deadline. Manager tests reopen a disk SQLite file,
recover an active task as paused, resume its saved bytes, renew an expired link,
keep partial bytes after cancellation, enforce the queue limit, deduplicate books,
reattach completed books removed from the shelf, and preserve the library when a
task record is removed. A ZIP book-bundle regression preserves its exact contents.

DoH fixtures use real TLS and DNS wire messages, with custom endpoint/port/query,
strict trust checks, eight simultaneous callers, TTL0/expiry, cache invalidation,
stalled AAAA and both delayed address families. Fast answers must arrive within
350–450ms while late answers must merge into the cache without another query.

Headless UI checks at320/390/768/1280px use explicit fixtures, including the user's
5.5/34MB failed-task pattern. CDP injects actual touch start/end and verifies that
long-press release opens management without activating a destructive action.
Pause/progress/settings survive reload; visible task buttons are at least48px.
Hidden native and signed-Android smoke results are recorded separately under
`artifacts/testing/v08/` and the Actions artifacts; they are not fixture claims.

The [signed package job](https://github.com/Junyxor/LuminaShelf/actions/runs/38858699264/job/116837069260)
built all three APKs from `86e21d7fb287e8f6a830de4c0a495a64262e1699` on
2026-10-16. The checks job passed66core and10bridge tests, Clippy and all four UI
widths. Downloads under `artifacts/android/ci-v08-release/` were independently
verified with `scripts/verify-apk.ps1` against that exact source revision:

- Universal,64,877,565bytes: `9388103d5ec267598a8c0aadcfb1380314c96d871b7fa1501ab5514e6dd31454`.
- x86,20,013,820bytes: `2ee272ccfe361adacf3e4f3cf9cdbf9fae4094012e1f1d8a6afbd55241cecbf3`.
- x86_64,20,165,375bytes: `9040586018cc3bd773f503146eb0e804192cd2bbc1d8a2946687c696afd53334`.

Application inputs and Android build/signing scripts are unchanged in subsequent
CI/test/documentation commits. The package job's success does not assert that
the older acceptance script in that same run passed. Runtime acceptance uses
the same signed artifacts and updated, independently reviewed test facilities.

Android Kotlin and manifest compilation passed locally. The final signed x86_64
APK passed [Android35 acceptance](https://github.com/Junyxor/LuminaShelf/actions/runs/38892027698):
actual public HTTPS download,32,000B/s shared tunnel limit, foreground service,
background byte progress, visible pause at218,112rounded bytes, force-stop and
recovery of that progress, actual750ms Android long touch, resume to completion,
and reading real Moby Dick EPUB text after its cover. Screenshots, XML, service
state and JSON under `artifacts/testing/v08/android-x64-accepted/` were inspected.
The tunnel had238,871limited ciphertext bytes at pause; these include TLS overhead
and are not expected to equal book bytes. No APK instrumentation or TLS bypass
was used. Physical-device and private-account acceptance remain separate.

The final signed universal APK passed [the same Android35 flow](https://github.com/Junyxor/LuminaShelf/actions/runs/38892023366)
on its second attempt. The first attempt was blocked before the download test by
the emulator's system Quickstep ANR, with the app's shelf visible behind the
system dialog. The successful attempt paused at221,184rounded book bytes after
8.49seconds; the same progress survived process death, real long touch opened
management, resume completed and the reader displayed actual EPUB paragraphs.
The relay recorded238,877limited ciphertext bytes at pause. Evidence under
`artifacts/testing/v08/android-universal-accepted/` was downloaded and inspected.
The x86 APK has static signature/ELF/JNI/alignment verification only; no32-bit
Android runtime was used. The latest PR checks also passed independently.

Review outcomes and fixes are recorded in [0.8 review](docs/release-v08-review.md).
Private account, cloud/email delivery and external reading applications are not
exercised automatically. No merge or GitHub Release publication is automatic.

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

# LuminaShelf · 星书

LuminaShelf is a high-performance cross-platform ebook client built around a Rust core and a thin Tauri + React UI.

## Current state

The main desktop workflow is now connected end to end:

- search through pluggable Rust `BookProvider` implementations
- authenticated Z-Library EAPI account, profile, history, search, details and acquisition
- Project Gutenberg / Gutendex public provider
- app-local System DNS / custom DNS / DoH / DoT resolver and App Hosts rules
- native resumable segmented downloader with bounded HTTP Range concurrency
- persisted local library and reading progress in SQLite
- automatic library insertion after successful downloads
- EPUB 2 / EPUB 3 navigation parsing
- metadata-first EPUB / TXT reading with chapter-on-demand loading and bounded caches
- in-chapter text search and adjacent chapter prefetch
- built-in PDF reader with Rust IPC range loading and lazy-loaded PDF.js runtime
- Android arm64 debug APK CI artifact
- signed Android release APK build with persistent local release keys
- Windows NSIS installer packaging workflow

## Architecture

```text
Tauri 2 + React UI
        │
        ▼
Rust command bridge
        │
        ├── BookProvider registry
        │      ├── Gutendex
        │      └── Z-Library EAPI
        │
        ├── AppResolver (Hosts / DNS / DoH / DoT)
        ├── Native segmented downloader
        ├── SQLite library / reading state
        └── EPUB / TXT / PDF reader bridge
```

## Boundaries

- Z-Library integration uses the EAPI provider path; no HTML/JS anti-bot challenge solver is included.
- Remote hosts lists only become candidate address hints; they are never silently written to OS Hosts.
- TLS hostname validation stays enabled even when an address hint is used.
- Account passwords are not stored in generic SQLite metadata.

## Development

Requires a current Rust stable toolchain and Node.js 24+.

```bash
npm ci
npm run build
cargo test -p lumina-core
cargo clippy -p lumina-core --all-targets -- -D warnings
cargo test -p lumina-shelf-desktop --lib
```

Run the desktop app locally with:

```bash
npm run tauri -- dev
```

## Packaging

A Windows installer can be built with:

```bash
npm run tauri -- build --bundles nsis
```

The `Desktop Package` GitHub Actions workflow can also be started manually. Pushing a `v*` tag builds the Windows installer and publishes it to the matching GitHub Release.

Android CI currently produces an arm64 debug APK artifact for real-device testing and can optionally publish a prerelease from `workflow_dispatch`.

For a signed Android release, install JDK 17, Android SDK Platform 36, Build Tools 36.0.0,
NDK 27.2.12479018 and the Rust Android targets, then run:

```bash
npm run android:release
```

The default release includes arm64-v8a, armeabi-v7a and x86_64. The signed APK,
SHA-256 checksum and public certificate are written to `artifacts/android/`.
The first run generates a release keystore and credentials in `.signing/`; subsequent
builds reuse that identity. Both directories are ignored by Git. See
[Android release instructions](ANDROID_RELEASE.md) for environment setup and key backups.

## Status

LuminaShelf is in the integration and release-hardening stage. The core search → details → download → local library → built-in reader path is implemented; remaining work is focused on real-device testing, packaging polish and targeted reliability fixes rather than another architecture rewrite.

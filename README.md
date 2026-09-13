# LuminaShelf · 星书

LuminaShelf is a high-performance cross-platform ebook client built around a Rust core and a thin Tauri 2 + React shell, with Android as the primary mobile target.

## Current repository state

The project now has a usable end-to-end application path rather than only a core prototype.

Implemented:

- app-local System DNS / custom DNS / DoH / DoT resolver
- exact + wildcard App Hosts
- reqwest resolver adapter so provider and download traffic share the same network policy
- mirror registry, probing, ranking and runtime origin coordination
- split DNS / TCP / TLS / TTFB / throughput probes and Happy-Eyeballs address racing
- provider registry and `BookProvider` ABI
- Project Gutenberg / Gutendex reference provider
- authenticated Z-Library EAPI search, profile, quota, history and acquisition flow
- native resumable downloader with bounded segmented HTTP Range concurrency and retries
- SQLite-backed persistent download queue
- pause / resume / retry / cancel / task cleanup and restart recovery
- local ebook file inspection and bounded folder scanning
- React search, book details, downloads, library, account and settings workspaces
- Android/Material-oriented mobile UI with bottom navigation, safe-area handling and bottom-sheet details
- Android download notifications driven by Rust task state
- arm64 Android debug APK GitHub Actions build and artifact upload

## Architecture

```text
React / Tauri 2 UI
        │
        ├── Search / details / account / settings
        ├── Persistent download task controls
        └── Android-native mobile presentation
        │
        ▼
Tauri command + event bridge
        │
        ├── SQLite state / download tasks
        ├── Android notification surface
        └── Provider orchestration
        │
        ▼
Rust Core
   ├─ BookProvider registry
   │   ├─ Gutendex
   │   └─ Z-Library EAPI
   ├─ Mirror Registry / scoring
   ├─ AppResolver (Hosts / DNS / DoH / DoT)
   ├─ Segmented resumable downloader
   └─ Local library inspection
```

### Download path

Downloads intentionally stay inside LuminaShelf rather than being delegated to Android DownloadManager:

```text
Book -> enqueue_download -> Rust DownloadManager -> SegmentedDownloader
                                  │
                                  ├─ Provider session + fresh acquisition URL
                                  ├─ App DNS / DoH / Hosts policy
                                  ├─ HTTP Range segmentation + retry
                                  ├─ resume manifest
                                  ├─ SQLite task state
                                  └─ UI events / Android notification
```

Provider acquisition URLs are refreshed when a task starts or resumes instead of persisting temporary signed URLs in SQLite. Interrupted active tasks are restored as paused on the next app start and can resume through the existing partial file / segment manifest.

## Boundaries

- Z-Library integration uses the EAPI provider path; no HTML/JS anti-bot challenge solver is part of the core.
- Remote hosts lists only become candidate address hints; they are never silently written to OS Hosts.
- TLS hostname validation stays enabled even when an address hint is used.
- Provider secrets must not be stored in generic SQLite metadata; secure credential persistence remains a dedicated layer.
- Android notifications expose download state but are not yet a full Foreground Service. Strong background execution while the app is suspended or killed is a separate Android-native milestone.

## Build and checks

Requires Rust 1.88+ and Node.js for the Tauri shell.

```bash
cargo fmt --all -- --check
cargo test -p lumina-core
cargo clippy -p lumina-core --all-targets -- -D warnings
npm install
npm run build
cargo check -p lumina-shelf-desktop
```

Android arm64 debug APK:

```bash
npm install
npm run tauri -- android init --ci
npm run tauri -- android build --debug --apk --target aarch64
```

The repository also runs these checks in GitHub Actions and uploads the Android debug APK as a workflow artifact.

## Near-term work

1. Android Foreground Service integration for stronger long-running background downloads.
2. Android Storage Access Framework file/folder picker instead of manual path entry.
3. Secure Android credential vault for provider secrets.
4. EPUB/PDF reader and persisted reading progress integration.
5. Cover proxy/cache, production icon, signed Android release pipeline and broader device testing.

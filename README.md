# LuminaShelf · 星书

LuminaShelf is a high-performance cross-platform ebook client built around a Rust core and a thin Tauri 2 + React shell. The current development target is an Android-first daily-driver experience while keeping the desktop shell usable.

## Current state

The repository has moved beyond the original core-only milestone. The following end-to-end paths are already implemented:

- Rust-backed book search and details
- authenticated Z-Library EAPI provider
- automatic/manual Z-Library EAPI origin selection
- account profile, quota and download-history workspace
- native resumable downloader with bounded segmented HTTP Range concurrency
- provider-authenticated download headers
- download progress events bridged into the UI
- deterministic per-book download targets so failed downloads can resume the same partial file
- local ebook inspection and folder scanning
- app-local System DNS / App Hosts / custom DNS / DoT / DoH resolver policy
- live resolver preferences and runtime resolution tests
- mirror registry, probing, ranking and best-origin selection primitives
- desktop sidebar plus Android-oriented top utility actions and bottom navigation
- Android arm64 CI build and prerelease workflow

## Product direction

LuminaShelf intentionally avoids turning the Android client into a miniature desktop dashboard.

The mobile UI direction is:

- Android-native / Material-style surfaces instead of liquid-glass decoration
- opaque surfaces with clear hierarchy and low visual noise
- account and settings as utility destinations rather than primary tabs
- search, downloads and local library as the main everyday flow
- system light/dark preference support
- safe-area-aware top and bottom navigation

The legacy `.glass` class is kept temporarily as a markup compatibility hook, but the active Material override removes blur/transparency effects.

## Architecture

```text
Tauri 2 + React shell
        │
        ▼
BookProvider registry
   ├─ Gutendex
   └─ Z-Library EAPI
        │
        ├── Mirror runtime / scoring
        ├── AppResolver (Hosts / DNS / DoH / DoT)
        ├── Native resumable segmented downloader
        └── Local library inspection
```

## Z-Library boundary

- Integration uses the EAPI provider path.
- No HTML/JS anti-bot challenge solver is part of the client core.
- Passwords are not persisted by the current implementation.
- The current Z-Library session remains in Rust process memory, so reopening the app requires signing in again.
- EAPI origin selection is persisted because it is non-secret configuration.
- TLS hostname validation remains enabled.

## Download model

Downloads are performed inside LuminaShelf by the Rust downloader rather than being handed to a browser.

For large files, the downloader can use bounded concurrent HTTP Range segments and stores a sidecar resume manifest next to the destination. The Tauri bridge now derives a stable destination from provider + book identity, so retrying the same book reuses the same destination and resume manifest instead of silently creating `Book (2).epub` and starting again.

The current UI download list is still process-local. Persisting the task queue, explicit pause/cancel controls and background transfer lifecycle are the next download-manager milestone.

## Build checks

Requires Rust 1.88+ and the Node/Tauri toolchain used by CI.

```bash
cargo test -p lumina-core
cargo clippy -p lumina-core --all-targets -- -D warnings
npm install
npm run build
```

Android builds are also validated by `.github/workflows/android-ci.yml`.

## Next milestones

1. Persist download tasks and restore interrupted transfers after app restart.
2. Add pause / resume / cancel / retry controls around the Rust transfer engine.
3. Add Android secure credential/session storage so users do not have to log in after every restart.
4. Replace free-form mobile filesystem path inputs with platform-native directory/file selection where possible.
5. Add EPUB reading first, then PDF reading/open-with integration.
6. Continue breaking the large React shell into focused mobile-friendly screens/components.

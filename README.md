# LuminaShelf · 星书

LuminaShelf is a high-performance cross-platform ebook client built around a Rust core and a thin Tauri UI shell.

## Current repository state

This first visible GitHub milestone lands the **Rust core** before the UI shell so `main` starts from a coherent slice instead of an empty repository.

Implemented in this commit:

- app-local System DNS / custom DNS / DoH / DoT resolver
- exact + wildcard App Hosts
- reqwest resolver adapter so provider/download traffic shares the same network policy
- mirror registry with provenance merging and address hints
- bounded mirror-source subscriptions
- split DNS / TCP / TLS / TTFB / throughput probes
- Happy-Eyeballs address racing
- mirror scoring and health state
- native resumable downloader with bounded segmented HTTP Range concurrency
- local ebook file inspection and bounded folder scanning
- provider registry and `BookProvider` ABI
- Project Gutenberg / Gutendex reference provider
- Z-Library EAPI provider for authenticated search/profile/history/acquisition

## Architecture

```text
UI / Tauri shell                 (next repository slice)
        │
        ▼
BookProvider registry
   ├─ Gutendex
   └─ Z-Library EAPI
        │
        ├── Mirror Registry / scoring
        ├── AppResolver (Hosts / DNS / DoH / DoT)
        ├── Native segmented downloader
        └── Local library inspection
```

The Tauri shell, SQLite account/favorites/reading state, secure credential vault, cover proxy/cache, mobile UI and Android APK workflow are the next commits. Those pieces already exist in the local v0.5 working snapshot and are being brought into this repository in reviewable slices.

## Boundaries

- Z-Library integration uses the EAPI provider path; no HTML/JS anti-bot challenge solver is part of the core.
- Remote hosts lists only become candidate address hints; they are never silently written to OS Hosts.
- TLS hostname validation stays enabled even when an address hint is used.
- Provider secret persistence belongs to the secure credential layer, not generic SQLite metadata.

## Build the core

Requires Rust 1.88+.

```bash
cargo test -p lumina-core
cargo clippy -p lumina-core --all-targets -- -D warnings
```

## Near-term repository commits

1. SQLite WAL state layer + accounts/favorites/reading progress.
2. Tauri 2 command bridge and React workspaces.
3. Android-native credential vault and mobile navigation.
4. arm64 debug APK GitHub Actions workflow.
5. EPUB/PDF reader work and the final production icon.

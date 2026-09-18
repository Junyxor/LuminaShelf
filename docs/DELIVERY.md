# 0.6.0 delivery scope

This work continues the request to make the existing LuminaShelf repository usable.
The baseline for review is 90b4eb4. The daily-driver branch is integrated while the
newer EPUB/TXT/PDF readers on main remain available. The user explicitly confirmed
Android is the delivery target; desktop checks are supporting checks only. No remote release or PR merge
is part of this local delivery.

Acceptance criteria:

- Download metadata survives restart; interrupted work becomes paused.
- Pause/cancel also work during URL acquisition. A new attempt cannot start until
  the old attempt has stopped and persisted its outcome.
- Retry keeps the stored destination and asks the provider for fresh credentials.
- Partial downloads do not appear as finished ebooks in folder scans.
- File publication preserves a conflicting existing book. Error pages cannot
  become EPUB/PDF library entries.
- Android SAF file selection imports durable app-private copies, including MIME
  fallback for extensionless documents. Failed or
  unsupported imports leave no partial ebook and preserve earlier imports.
- Scanning another directory does not remove previous books from the UI.
- EPUB/TXT/PDF reading and persisted reading position remain functional.
- A late result from a previous search provider cannot replace the current results.
- Produce a signed arm64 Android APK; install the companion x86_64 APK on an
  Android emulator and exercise the actual SAF picker and EPUB/TXT/PDF readers.
- Android startup must work without a desktop DNS configuration file.
- Android Back closes reading/details before leaving the application.
- Android Keystore persistence and deletion survive force-stop/relaunch.
- Frontend workflow tests and HTTP/SQLite integration tests run in CI with locked
  dependencies.

Limits: no Android foreground-service transfers. Emulator validation does not
replace testing an arm64 physical handset. Desktop sessions remain in memory.
Network-provider uptime and authenticated Z-Library operations require a live
endpoint/account. No credentials are included in the smoke tests.

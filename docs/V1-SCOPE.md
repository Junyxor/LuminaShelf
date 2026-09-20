# Android 1.0 completion target

The user has explicitly asked for autonomous, continuous implementation toward a
complete Android application. The 1.0 implementation is the first complete Android daily-driver baseline.
Continue implementing and verifying without repeatedly asking to proceed.

1. Foreground download service, lock-screen/background transfer, notification pause,
   durable bounded queue, fresh acquisition on retry and safe file publication.
2. EPUB/TXT/PDF reading, position inside chapters, bookmarks, reader preferences,
   Android Back, offline restoration; formatted EPUB content and images where supported.
3. Native SAF import, filtering/sorting, metadata editing, remove-from-library and
   explicit managed-file deletion, export/share, portable backup and restore.
4. Working provider UI with format choice, clear network/account errors, useful
   retries and cached search history. Persist Android account sessions securely.
5. Signed release APK with stable local signing material preserved outside Git,
   Android permissions limited to implemented features, documented install/upgrade.
6. Automated core/frontend tests and actual Android emulator end-to-end tests,
   including background download and backup restoration; resolve discovered defects.

Do not present a debug APK as the final version. Physical-handset testing and
external account availability must be reported honestly, but do not stop the other
authorized implementation work because those resources are unavailable.

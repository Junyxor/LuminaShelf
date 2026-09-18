# Local patch

Upstream: android-native-keyring-store 1.0.0 (MIT OR Apache-2.0).
The shipped licenses and source are preserved.

The by-store credential implementation now checks SharedPreferences.Editor.commit()'s
boolean result for save and delete. Android may return false without throwing a JNI
exception when persistence fails. Such failures must reach the caller so LuminaShelf
does not acknowledge session persistence or logout before the disk operation succeeds.

Remove this override when adopting an upstream version with equivalent handling.

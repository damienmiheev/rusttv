# SYNTHETIC fixtures: `rusttv update`

**None of these files were captured from a device.** They are hand-written
stand-ins so `rusttv update check` / `update launcher` can be tested against
`FakeAdb` and `tests/fixtures/fake-adb`. Replace each with a real capture
once the launcher has been checked on a device.

| File | Command | Stands for | Based on |
|---|---|---|---|
| `dumpsys-package-rusttv.txt` | `dumpsys package ai.rusttv.launcher` | RustTV launcher 0.1.0 installed; trimmed (a real dump has many more sections), ids and paths made up | AOSP `PackageManagerService` dump format (Android 14) |
| `dumpsys-package-missing.txt` | same | Launcher not installed: no `Packages:` section, so no `versionName=` | Same; the exact lines Android prints for a missing package are not verified |

The fake releases (`release.json`, `SHA256SUMS`, `SHA256SUMS.minisig`) are
not files here: tests build them with a throwaway key
(`rusttv_core::test_release`, feature `test-util`).

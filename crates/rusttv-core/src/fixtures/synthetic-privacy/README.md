# SYNTHETIC fixtures: privacy profiles

**None of these files were captured from a device.** They are hand-written
stand-ins so the privacy plan (`crates/rusttv-core/src/privacy.rs`) and its
protected-package checks can be tested against `FakeAdb`. Replace each with a
real capture, and update this table, when one is taken (OPTIMIZER.md § Privacy
profiles).

They are used together with the real Philips 65OLED818 captures in
`../philips-65oled818-api34/` (`getprop.txt`, `home-activity.txt`,
`home-candidates*.txt`), so the bundled profile
`compat/devices/tpv-ph1m-ww-9972.toml` matches.

| File | Command | Stands for | Based on |
|---|---|---|---|
| `packages-f.txt` | `pm list packages -f` | The twelve packages tested on the Philips installed, three other catalog packages: `org.droidtv.amazonalexa` (voice, tested there in a second round), `com.google.android.apps.mediashell` (casting, untested) and `org.droidtv.usagelogger` (telemetry, untested; not on the Philips, made up so the tests have an untested default package), a few protected system packages and the RustTV launcher; APK paths are made up | Format of `../emulator-pixel3a-api34/packages-f.txt`. On 2026-10-23 all but `usagelogger` were seen present and enabled on the Philips (not committed: the full list shows which apps the owner uses) |
| `ime-list.txt` | `ime list -a -s` | One input method (Gboard) | Output format of `ime list -s` (one `package/class` id per line), from AOSP `InputMethodManagerService`; not captured |
| `accessibility-none.txt` | `settings get secure enabled_accessibility_services` | No accessibility service enabled | `settings get` prints `null` for an unset key (OPTIMIZER.md § 1) |

The tests build variants inline (an extra input method, an enabled
accessibility service, an extra Home candidate, packages disabled or missing).
The engine never parses `pm disable-user` / `pm enable` output; it verifies by
reading `pm list packages -d` again.

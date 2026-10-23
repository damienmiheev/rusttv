# SYNTHETIC fixtures: optimizer profiles

**None of these files were captured from a device.** They are hand-written
stand-ins so the optimizer plan (`crates/rusttv-core/src/optimizer.rs`) and
`Check::SettingIs` can be tested against `FakeAdb` and the CLI's fake adb.
Replace each with a real capture, and update this table, when one is taken
(OPTIMIZER.md § 6, step 4). Nothing was read from the owner's Philips.

They are used together with the real Philips 65OLED818 captures in
`../philips-65oled818-api34/` (`getprop.txt`, `home-activity.txt`), so the
bundled profile `compat/devices/tpv-ph1m-ww-9972.toml` matches.

| File | Command | Stands for | Based on |
|---|---|---|---|
| `settings-get-null.txt` | `settings get global <key>` | The key is unset (Android uses the default, 1.0) | `settings get` prints the literal `null` for an unset key (`SettingsService.java`, Android 14; OPTIMIZER.md § 1) |
| `settings-get-1.0.txt` | `settings get global <key>` | The key stored as `1.0` (as Developer options writes it) | Format only: the stored string followed by a newline |
| `settings-get-0.5.txt` | `settings get global <key>` | The key at the profile's value | As above |
| `settings-get-invalid.txt` | `settings get global <key>` | A value RustTV can't validate (`fast`), e.g. written by a vendor tool | Made up; the plan must refuse it rather than journal it as an original |

The CLI's fake adb (`crates/rusttv-cli/tests/fixtures/fake-adb`) answers the
Philips scenario's `settings get global` calls with these files. Writes
(`settings put` / `delete`) print nothing; the engine verifies by reading the
key again.

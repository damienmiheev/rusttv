# SYNTHETIC fixtures: Home takeover

**None of these files were captured from a device.** (A takeover and restore have since run on the Philips 65OLED818, 2026-10-23, and behaved as these files assume; they were not re-captured.) They are hand-written
stand-ins for states that no device in this project has been put in yet
(no takeover has ever run). They let the safety engine and Home takeover be
tested against `FakeAdb`. Replace each with a real capture, and update this
table, once a takeover/restore has been recorded on a device
(SECURITY.md § Home takeover, rule 2).

They continue the real Google TV emulator captures in
`../google-tv-emulator-api34/` (`home-activity.txt`, `home-candidates.txt`).

| File | Command | Stands for | Based on |
|---|---|---|---|
| `getprop.txt` | `getprop` | Six properties only (`ro.product.cpu.abilist` is `arm64-v8a`, as on the arm64 emulator); the fingerprint is made up, and no compat profile matches it | Format of `../emulator-pixel3a-api34/getprop.txt` |
| `packages-3.txt` | `pm list packages -3` | RustTV launcher and spike installed | Format of `../emulator-pixel3a-api34/packages-3.txt` |
| `packages-d-none.txt` | `pm list packages -d` | No package disabled (empty output) | Assumption |
| `packages-d-launcher-disabled.txt` | `pm list packages -d` | Stock launcher disabled for user 0 | `pm` prints `package:<name>` lines |
| `pm-disable-user.txt` | `pm disable-user --user 0 com.google.android.apps.tv.launcherx` | Success | Message format from AOSP `PackageManagerShellCommand`; not captured |
| `pm-enable.txt` | `pm enable --user 0 com.google.android.apps.tv.launcherx` | Success | Same |
| `home-activity-rusttv.txt` | `cmd package resolve-activity --brief ... HOME` | Home resolves to RustTV after the stock launcher is disabled | Format of `../google-tv-emulator-api34/home-activity.txt` |
| `home-activity-chooser.txt` | same | Two launchers with equal priority, so Android shows its chooser (`ResolverActivity`) instead of RustTV | Expected Android behaviour; not captured |

| `input-keyevent-home.txt` | `input keyevent KEYCODE_HOME` | Success (empty output) | Assumption |
| `activities-rusttv-front.txt` | `dumpsys activity activities` | RustTV resumed after HOME; only the lines the parser reads, the real dump is far longer | Line formats as of Android 14 (`topResumedActivity=`, `ResumedActivity:`, `mResumedActivity:`); not captured |
| `activities-stock-front.txt` | same | Stock launcher still in front | Same |
| `ps-rusttv-running.txt` | `ps -A -o PID,RSS,NAME` | RustTV process alive | Format of `../emulator-pixel3a-api34/ps.txt`, trimmed |
| `ps-rusttv-not-running.txt` | same | RustTV process gone (crashed) | Same |

The engine never parses the `pm disable-user` / `pm enable` output; it
verifies by reading state again, so a wrong guess there can't hide a failure.

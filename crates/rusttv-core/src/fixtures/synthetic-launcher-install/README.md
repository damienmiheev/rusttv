# SYNTHETIC fixtures: launcher install / uninstall

**None of these files were captured from a device.** They are hand-written
stand-ins so `rusttv launcher install` / `uninstall` can be tested against
`FakeAdb` and `tests/fixtures/fake-adb`; nothing has been installed with
RustTV on a device yet. Replace each with a real capture once that happens.

They continue the real Google TV emulator captures in
`../google-tv-emulator-api34/` and the synthetic takeover states in
`../synthetic-home-takeover/` (whose `getprop.txt` is reused).

| File | Command | Stands for | Based on |
|---|---|---|---|
| `packages-f-rusttv.txt` | `pm list packages -f` | Stock launcher, TV settings, RustTV launcher and spike; trimmed, real lists are much longer; `/data/app` directory names made up | Format of `../emulator-pixel3a-api34/packages-f.txt` |
| `packages-f-no-rusttv.txt` | same | Same without the RustTV launcher | Same |
| `packages-3-no-rusttv.txt` | `pm list packages -3` | Only the spike is a third-party app | Format of `../synthetic-home-takeover/packages-3.txt` |
| `home-candidates-no-rusttv.txt` | `cmd package query-activities --brief ... HOME` | RustTV not installed | `../google-tv-emulator-api34/home-candidates.txt` minus RustTV |
| `home-candidates-stock-disabled.txt` | same | Stock launcher disabled (Home takeover done, possibly from another computer) | Same, minus the stock launcher |
| `adb-install.txt` | `adb install --pkg ai.rusttv.launcher <apk>` | Success | adb's streamed-install messages; not captured |
| `pm-uninstall.txt` | `pm uninstall ai.rusttv.launcher` | Success | AOSP `PackageManagerShellCommand`; not captured |

The engine never parses the install/uninstall output; it verifies by reading
the package list and Home candidates again.

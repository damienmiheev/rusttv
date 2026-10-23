# Emulator capture: Pixel 3a, Android 14 (API 34)

Raw `adb` output captured on 2026-10-23 from the stock `Pixel_3a_API_34` AVD
(arm64 system image) with adb 1.0.41. Package lists were added the same day
after a reboot; the three third-party apps are device-info utilities installed
on that AVD.

| File | Command |
|---|---|
| `devices-l.txt` | `adb devices -l` |
| `getprop.txt` | `adb shell getprop` |
| `meminfo.txt` | `adb shell cat /proc/meminfo` |
| `df-data.txt` | `adb shell df -k /data` |
| `packages-f.txt` | `adb shell pm list packages -f` |
| `packages-3.txt` | `adb shell pm list packages -3` |
| `packages-d.txt` | `adb shell pm list packages -d` |
| `proc-stat-1.txt`, `proc-stat-2.txt` | `adb shell cat /proc/stat`, about 1 s apart |
| `ps.txt` | `adb shell ps -A -o PID,RSS,NAME` |
| `home-activity.txt` | `adb shell cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME` |
| `meminfo-launcher.txt` | `adb shell dumpsys meminfo com.google.android.apps.nexuslauncher` |
| `meminfo-not-running.txt` | `adb shell dumpsys meminfo com.android.nfc` (app not running) |

The `/proc/stat` samples were taken seconds after boot, while the emulator was
fully busy: the idle counter does not move between them (100% busy). A full `dumpsys meminfo` (no package) timed out after
10 s at that point; on the idle emulator later it took ~1 s. Snapshots use
`ps` instead because it stays fast under load.

Redactions: `persist.sys.timezone` and `vendor.qemu.timezone` set to `Etc/UTC`
(the emulator copies the host's timezone). Nothing else was changed.

This is a phone image, not Android TV. Add a TV capture next to it rather than
replacing it.

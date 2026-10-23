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

Redactions: `persist.sys.timezone` and `vendor.qemu.timezone` set to `Etc/UTC`
(the emulator copies the host's timezone). Nothing else was changed.

This is a phone image, not Android TV. Add a TV capture next to it rather than
replacing it.

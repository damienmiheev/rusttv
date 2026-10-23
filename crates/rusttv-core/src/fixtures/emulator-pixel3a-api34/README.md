# Emulator capture: Pixel 3a, Android 14 (API 34)

Raw `adb` output captured on 2026-10-23 from the stock `Pixel_3a_API_34` AVD
(arm64 system image) with adb 1.0.41:

| File | Command |
|---|---|
| `devices-l.txt` | `adb devices -l` |
| `getprop.txt` | `adb shell getprop` |
| `meminfo.txt` | `adb shell cat /proc/meminfo` |
| `df-data.txt` | `adb shell df -k /data` |

Redactions: `persist.sys.timezone` and `vendor.qemu.timezone` set to `Etc/UTC`
(the emulator copies the host's timezone). Nothing else was changed.

This is a phone image, not Android TV. Add a TV capture next to it rather than
replacing it.

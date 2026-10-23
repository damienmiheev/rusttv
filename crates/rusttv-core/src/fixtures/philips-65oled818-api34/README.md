# Philips 65OLED818/12 (Google TV), Android 14 (API 34)

Raw `adb` output captured on 2026-10-23 from the owner's Philips 65OLED818/12
(reports manufacturer `TPV`, model `Philips Google TV TA1`, device
`PH1M_WW_9972`, MediaTek MT9972, firmware `UKN2.241117.001/3000518`) over
wireless debugging with adb 1.0.41, using `scripts/tv-capture.sh`. The RustTV
launcher was not installed yet.

| File | Command |
|---|---|
| `getprop.txt` | `adb shell getprop`, **filtered** to `ro.product.*`, `ro.build.*`, `ro.board.platform`, `ro.hardware`, `ro.soc.*`, `ro.sf.lcd_density` |
| `meminfo.txt` | `adb shell cat /proc/meminfo` |
| `df-data.txt` | `adb shell df -k /data` |
| `proc-stat-1.txt`, `proc-stat-2.txt` | `adb shell cat /proc/stat`, about 1 s apart |
| `home-activity.txt` | `adb shell cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME` |
| `home-candidates.txt` | `adb shell cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.HOME` |
| `dumpsys-power-head.txt` | first 12 lines of `adb shell dumpsys power`, screen awake (the rest lists wake locks per app) |
| `home-candidates-rusttv-installed.txt` | same as `home-candidates.txt`, with the RustTV launcher installed |

Privacy: only firmware-level data is kept, identical on every unit with this
firmware. The full `getprop` (unit state, settings, pairing flags), package,
process and app lists (they show which apps the owner uses), and the
`adb devices -l` line (local IP) were captured but deliberately not committed.

Notable: the userspace is 32-bit only (`ro.product.cpu.abilist` =
`armeabi-v7a,armeabi`). Home candidates include Google's
`setupwraith/.RecoveryActivity` at priority 1, between the stock launcher (2)
and any third-party launcher (0): disabling only the stock launcher would hand
Home to it, not to RustTV (see compat/devices/tpv-ph1m-ww-9972.toml).

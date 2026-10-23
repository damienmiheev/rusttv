# Emulator capture: Google TV, Android 14 (API 34)

Raw `adb` output captured on 2026-10-23 from a Google TV AVD
(`system-images;android-34;google-tv;arm64-v8a`, device `tv_1080p`) with
adb 1.0.41. The RustTV spike launcher (`ai.rusttv.spike`) was installed.

| File | Command and situation |
|---|---|
| `am-start-cold.txt` | `am start -W -n ai.rusttv.spike/.LauncherActivity` after `am force-stop` (COLD) |
| `am-start-hot.txt` | the same while the app was in the background (HOT) |
| `am-start-already-top.txt` | the same while the app was already in front (UNKNOWN, TotalTime 0) |
| `am-start-error.txt` | `am start -W -n ai.rusttv.spike/.Nope`: "Error type 3", **exit status 0** |
| `ps-time.txt` | `ps -A -o PID,TIME+,NAME` (cumulative CPU time per process) |
| `home-activity.txt` | `cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME` (RustTV launcher and spike installed) |
| `home-candidates.txt` | `cmd package query-activities --brief -a android.intent.action.MAIN -c android.intent.category.HOME`, same state |

No redactions were needed.

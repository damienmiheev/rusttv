# Benchmark Plan

Goal: produce numbers that a skeptical reader can reproduce. Performance budgets are set **after** the baseline exists, not before.

## What we compare

| Subject | Why |
|---|---|
| Stock launcher (Google TV home on the Philips) | What users have today |
| RustTV Launcher | Our subject |
| CleanYourTV launcher (Java, minimal) | Floor for a lightweight native launcher |
| One popular Kotlin launcher (e.g. Projectivy or FLauncher) | What enthusiasts use |

Launcher improvements and system-wide improvements (from optimizer changes) are measured and reported **separately**.

## Metrics and methods

| Metric | Method (ADB, rootless) | Unit |
|---|---|---|
| Cold start | `am force-stop <pkg>`; drop to idle; `am start -W -n <home component>` → `TotalTime`; cross-check with `ActivityTaskManager: Displayed` in logcat | ms |
| Warm start | Launcher in background (open another app), then `input keyevent KEYCODE_HOME`; measure via logcat `Displayed` / `reportFullyDrawn` | ms |
| Idle memory | 60 s after reaching home, `dumpsys meminfo <pkg>`: PSS total, RSS, native/graphics breakdown | MB |
| Idle CPU | `/proc/<pid>/stat` utime+stime deltas over 5 min while idle (or `top -b -d 1`) | % of one core |
| Input-to-frame latency | Perfetto trace (API 28+): input event → corresponding frame presented; synthetic D-pad presses via `input keyevent` | ms |
| Frame stability while scrolling | `dumpsys gfxinfo <pkg> framestats` during scripted D-pad scrolling | jank %, p90 frame time |
| App launch from launcher | Time from DPAD_CENTER keyevent to target app `Displayed` | ms |
| Boot-to-usable-home | `adb reboot`; poll `sys.boot_completed`; time to launcher `Displayed` | s |
| APK / install size | APK bytes; `dumpsys package` code size | MB |
| Stability | Crashes/ANRs in logcat over a 24 h idle + scripted-navigation soak | count |

Caveat: force-stopping the *current default* home may make the system relaunch it immediately. Cold-start runs must check (via `pidof`) that the process is really gone before starting the timer; if the system races us, measure with the launcher *not* set as default and launch it explicitly.

Optional later: end-to-end latency with a high-speed camera (remote press → pixel change) to validate the Perfetto numbers.

## Protocol

1. **Record environment**: device model, firmware fingerprint, Android version, ABI, RAM, display mode, network state, RustTV commit, launcher versions. Stored with every result.
2. **Prepare**: reboot; wait 5 minutes for boot jobs to settle; same network; no USB devices except as needed; screen on; same display resolution.
3. **Warm-up** runs are discarded (3 per metric).
4. **Repeat**: ≥ 30 runs for start-up metrics, ≥ 10 for others, interleaving launchers (A, B, C, A, B, C…) to spread drift and thermal effects.
5. **Report**: median, p10/p90, IQR, min/max, n. Never a single number.
6. **Raw data**: every run saved as JSON lines under `benchmarks/raw/<device>/<date>/`, committed with the report.
7. **Claim rule**: "X is faster than Y" only if the median difference exceeds run-to-run noise (IQRs don't overlap) on that device; always state the device. No cross-device generalization without data from that device.
8. **Regressions**: a CI-or-manual run showing a regression beyond noise blocks the merge until explained.

`rusttv performance benchmark --app <component>... --json` implements steps 3–5 and keeps every raw run in its output (save it under `benchmarks/raw/<device>/<date>/` for step 6). How it measures:

- **Cold start**: `am force-stop`, a check that the process is really gone (runs where it isn't are discarded), then `am start -W`; only runs Android reports as `COLD` count.
- **Resume**: bringing an app that's still in memory back to the front (`HOT`), which is what pressing Home feels like. Apps take turns being in front, so this needs two or more apps.
- **Fresh idle memory** (`idle_pss_kib_fresh`): before any series, each app is force-stopped, started, and left for `--fresh-settle-ms`; then PSS from `dumpsys meminfo <package>`. Samples whose start Android doesn't report as `COLD` are dropped. The settle is longer than `--settle-ms` because a cold start peaks while it loads (RustTV on the Philips: ~110 MB in the first ~10 s, then ~65 MB).
- **Idle after the series** (`idle_pss_kib_after_resumes`, `idle_cpu_percent`): after the cold-start and resume series, each app is brought to the front and left for `--settle-ms`; then PSS as above and CPU from `ps` time deltas over `--cpu-window-ms`, summed over the app's processes.
- **Why both**: memory an app keeps per pause/resume piles up over the resume series. On the Philips, RustTV kept ~1.85 MB of native heap per cycle, so its only idle number (taken after 30+ resumes) read 123 MB against ~65 MB after a fresh start, and nothing showed why. `idle_pss_growth_kib` (median after resumes minus median fresh) makes that visible: near zero is healthy; a large positive value means the series left the app bigger and needs a look before quoting idle memory. Quote fresh idle memory as "idle memory"; the after-resumes number describes a launcher that has been used for a while.
- Defaults: 30 runs + 3 warm-up, 2 s pause, 20 s fresh settle, 10 s settle, 5 s CPU window, 3 idle samples (fresh and after the series each).
- **Screensaver**: nobody touches the remote, so the TV's screensaver can start mid-run. Before every start the tool reads `mWakefulness` from `dumpsys power` and, if it isn't `Awake`, wakes the TV with `input keyevent KEYCODE_WAKEUP` (what any key press does). An idle sample is dropped if the screensaver is on at its end (after the settle for fresh samples, after the CPU window otherwise). Each `am start -W` is stopped after 30 s and fails the run with `COMMAND_FAILED` instead of waiting forever.

The quick check `rusttv performance snapshot` measures CPU (two `/proc/stat` samples), memory, launcher PSS/RSS and top processes by RSS (`ps`), stamped with host time and build fingerprint. It avoids a full `dumpsys meminfo`, which timed out (10 s) on a busy emulator.

## Emulator vs real hardware

- Emulator: functional checks and harness development only. **No published performance numbers from the emulator.**
- Physical device: all published numbers.

## Device test strategy

| Stage | Devices | What runs |
|---|---|---|
| Now | Android TV emulator (API 30 & 34, x86_64) | CLI integration tests against a real `adb`; launcher functional tests |
| Now | Owner's Philips Google TV | Read-only CLI, launcher install/usage, benchmarks |
| Later | Low-memory box (1–1.5 GB, 32-bit userspace), a classic Android TV (pre-Google TV UI) | Compatibility + budgets |
| Community | Contributor devices | `rusttv device inspect --json` + benchmark report attached to a compat PR |

- Unit tests use captured fixtures, never a device.
- Device tests are opt-in (`--features device-tests` or an env var naming the device serial) and run read-only unless the human explicitly approves a modifying test for that session.
- A failure on a physical device is an incident to investigate, not a flaky test to retry.

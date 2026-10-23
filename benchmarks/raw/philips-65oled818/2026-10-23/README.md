# Launcher benchmark: Philips 65OLED818/12, 2026-10-23

`rusttv performance benchmark` (BENCHMARK_PLAN.md) comparing the stock Google
TV launcher (`com.google.android.apps.tv.launcherx/.home.HomeActivity`) with
the RustTV launcher 0.0.0 (`ai.rusttv.launcher/.LauncherActivity`, armeabi-v7a
APK). Profile: `compat/devices/tpv-ph1m-ww-9972.toml`.

Protocol: TV restarted, 5 min settle (uptime ≥ 300 s), connected over
wireless debugging, nobody touching the remote. Defaults: 30 counted runs + 3
warm-ups per app, interleaved, 2 s pause, idle memory/CPU 10 s after start
over 5 s, 3 samples. Ran 16:41–16:48 local time.

| | Stock launcher | RustTV |
|---|---|---|
| Cold start, first frame (median, p10–p90) | 410 ms (393–438), n 27 | 564 ms (550–663), n 30 |
| Resume (median, p10–p90) | 94 ms (84–104) | 37 ms (34–48) |
| Idle PSS (median) | 87 MB | 138 MB |
| Idle CPU (median) | 0.2 % | 0 % |

Reading it (claim rule: IQRs must not overlap, this device only):
- Resume: RustTV is faster on this TV (37 vs 94 ms).
- Cold start measures the first frame (`am start -W` TotalTime). The stock
  launcher's first frame shows placeholders and its content arrives about a
  second later (owner's observation, not yet measured); RustTV's first frame
  already shows the full screen. A "visually complete" measurement is needed
  before comparing time to a usable screen.
- Memory: RustTV uses about 50 MB more PSS on this TV.

Redaction: the adb serial (the TV's local IP and port) was replaced with `tv`.
`launchers.json` is the full `--json` output; `launchers.progress.txt` the
per-run progress lines.

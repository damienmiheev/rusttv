# History build vs main: Philips 65OLED818/12, 2026-10-24

Does the history (HISTORY.md: the Settings page, the history thread, the
door, gix) slow Home down or grow its memory? A/B/A with
`rusttv performance benchmark` (BENCHMARK_PLAN.md: 30 counted runs per app
and measurement, 3 warm-ups, RustTV interleaved with the stock launcher), the
stock launcher re-enabled for the runs (`rusttv launcher restore`, a cold
start of the Home app can't be measured while Android relaunches it).
armeabi-v7a `android-release` builds, installed with `rusttv launcher
upgrade`. 16:04–16:36 UTC, nobody touching the remote; no run discarded (the
tool dropped one idle sample per series when the screensaver came on).

- **A** (`A1.json`, `A2.json`): main's launcher (`4afad17` + `f08d424`).
- **B** (`B1.json`): branch `history` at `664b31f`.

| RustTV | A | B | A again |
|---|---|---|---|
| Cold start, median (IQR, p10–p90) | 366 ms (19, 352–383) | 365 ms (19, 350–392) | 368 ms (18, 341–380) |
| Resume, median (IQR, p10–p90) | 33 ms (7, 30–42) | 36 ms (7, 29–44) | 34 ms (7, 30–41) |
| Idle PSS 20 s after a fresh cold start, median (p10–p90) | 63.2 MB (63.1–63.4) | 64.6 MB (64.5–64.7) | 63.5 MB (63.0–63.6) |
| Idle PSS after the resume series, median (n 2) | 65.4 MB | 66.4 MB | 67.5 MB |

Reading it (claim rule: IQRs must not overlap, this device only):
- **Cold start and resume: no change** (the IQRs overlap completely).
- **Idle memory after a fresh start: +1.0–1.4 MB** with the history build
  (its range doesn't overlap A's), inside HISTORY.md § 9's +1.5 MB budget.
  The history store opens on first use, so this is code and Settings UI,
  not history data.
- After the resume series the three values move within their own spread
  (n 2): no claim.

APK (armeabi-v7a): 10.23 MB → 11.83 MB; the native library +1.52 MB, of
which gix and its pack writer measured +1.17 MB alone (spike, § Spike
results).

Also checked on this TV with the history build: the door answers
`rusttv history sync` with RustTV stopped (1.5 s for the whole sync) and
without starting the home screen.

No identifying data: the adb serial is replaced by the model name.

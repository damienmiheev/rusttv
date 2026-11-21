# Panels cold start: main vs panels with lazy built-in glows, Philips 65OLED818, 2026-11-21

`rusttv performance benchmark` (BENCHMARK_PLAN.md defaults: 30 runs, 3
warm-up, first-frame `am start -W` TotalTime), RustTV interleaved with the
stock launcher (`com.google.android.apps.tv.launcherx/.home.HomeActivity`).
armeabi-v7a release APKs, same signature, installed with
`adb install -r --pkg ai.rusttv.launcher` (data kept). The two runs went back
to back, main first, with nobody touching the remote. The stock launcher was
Home, and the Android screensaver was off.

- `main.json`: `origin/main` (0b008fe) APK.
- `panels-fixed.json`: `panels` (be4d988) plus this PR, where the built-in
  tiles' hero glows are made on first use instead of before the first frame.
- `*.progress.txt`: the tool's stderr.

| RustTV | main | panels + fix |
|---|---|---|
| Cold start, median (p10–p90, IQR) | 577 ms (563–592, IQR 12) | 404 ms (390–440, IQR 20) |
| Resume, median (p10–p90) | 36 ms (29–41) | 36 ms (29–41) |
| Fresh idle PSS, median | 61.0 MB | 58.6 MB |
| Idle PSS after resumes, median | 65.8 MB | 65.3 MB |
| Idle CPU, median | 0 % | 0 % |

n = 30 cold and 30 resume runs each; no runs discarded.

Earlier the same day, with the same command: main cold 579 ms (561–602) and
`panels` without the fix 622 ms (609–666).

What the traces showed: a Perfetto CPU callstack-sampling trace of one
`panels` cold start (profileable, unstripped build). In `android_main`, about
a third of the samples before the first frame were in
`Builtins::new` → `BuiltinView::new` → `hero_glow`, which is
material-colors quantization. `Builtins::new` made the hero glow of every
built-in tile eagerly. Main makes four glows (About, Settings, Remote,
Hidden), and Panels added a fifth (History), which accounts for the +43 ms.
The fix defers all five glows to their first use, after the first frame,
which is also why the fixed build is faster than main.

Redaction: the adb serial (the TV's local IP and port) was replaced with the
model name, `65OLED818`.

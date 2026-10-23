# Launcher benchmark re-run: Philips 65OLED818/12, 2026-10-23

Same method and defaults as the first run (`../README.md`), after RustTV
started loading banners after its first frame (commit a62c31b) and hid the
on-screen keyboard at start (80c4aca). RustTV 0.0.0, armeabi-v7a release APK
built from 36d48f2. TV restarted, uptime ≥ 300 s before the start, nobody
touching the remote. Ran 17:49–18:02 local time.

| | Stock launcher | RustTV |
|---|---|---|
| Cold start, first frame (median, p10–p90, IQR) | 462 ms (432–525, IQR 40), n 28 | 365 ms (352–416, IQR 29), n 30 |
| Resume (median, p10–p90, IQR) | 100 ms (92–118, IQR 11) | 35 ms (32–47, IQR 6) |
| Idle PSS (median) — see caveat | 87 MB | 127 MB |
| Idle CPU (median) — see caveat | 0.2 % | 0 % |

Two stock runs were discarded by the tool (process still running after
force-stop; launch state WARM instead of COLD).

Reading it (claim rule: IQRs must not overlap, this device only):
- Cold start: RustTV's first frame is now faster than stock on this TV
  (365 vs 462 ms; first run 564 vs 410 ms). The IQRs do not overlap.
- Resume: RustTV is faster on this TV (35 vs 100 ms), as in the first run.
- First-frame timing still says nothing about content: see `../visual/` for
  visually complete.

Caveat, idle numbers: the TV's screensaver started during the idle sampling,
so some samples were taken with the screensaver in front of the launcher, and
the last RustTV `am start -W` blocked until the remote's Back key closed it.
Do not use this run's idle PSS/CPU; `../memory-profile.md` has the steady
memory measurement. The tool needs a start timeout and a screensaver check
before idle sampling.

Redaction: the adb serial (the TV's local IP and port) was replaced with `tv`.

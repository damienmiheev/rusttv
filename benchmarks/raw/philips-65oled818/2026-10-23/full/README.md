# Full launcher benchmark: Philips 65OLED818/12, 2026-10-23 (after the screensaver fix)

Same method and defaults as `../README.md`, run with the benchmark tool from
bf8d9aa (wakes the TV from its screensaver before every start, drops idle
samples taken under it, stops `am start` after 30 s). RustTV 0.0.0,
armeabi-v7a release APK built from 36d48f2. TV restarted, uptime ≥ 300 s,
nobody touching the remote. Ran 18:42–18:50 local time. No run or idle sample
was discarded, and the screensaver did not start.

| | Stock launcher | RustTV |
|---|---|---|
| Cold start, first frame (median, p10–p90, IQR) | 492 ms (456–550, IQR 46), n 30 | 358 ms (336–391, IQR 22), n 30 |
| Resume (median, p10–p90, IQR) | 101 ms (83–120, IQR 21) | 37 ms (30–46, IQR 8) |
| Idle PSS (median of 3) — see below | 92 MB | 123 MB |
| Idle CPU (median of 3) | 0.2 % | 0 % |

Reading it (claim rule: IQRs must not overlap, this device only):
- Cold start: RustTV's first frame is faster than stock on this TV (358 vs
  492 ms), as in `../rerun/` (365 vs 462 ms).
- Resume: RustTV is faster on this TV (37 vs 101 ms).

Idle memory is not understood yet, so make no claim from it. This run and
`../rerun/` give RustTV ~123–127 MB after the 30-run cold/resume series, but
two short runs that day (2 runs each) gave 68–70 MB, `../memory-profile.md`
measured ~63 MB steady, and one reading right after a single cold start gave
115 MB. The screensaver is ruled out (it did not start in this run). Under
investigation.

Redaction: the adb serial (the TV's local IP and port) was replaced with `tv`.

# Launcher benchmark after the memory fixes: Philips 65OLED818/12, 2026-10-23

`rusttv performance benchmark` (schema `rusttv.v3`: idle memory measured both
after a fresh cold start and after the resume series), RustTV built from
e45aa68: GL context reuse (`third_party/slint-gl-context-reuse.patch`) and
banners kept as GPU textures (`third_party/slint-texture-upload.patch`).
The stock launcher was re-enabled for the run (`rusttv launcher restore`) and
RustTV made Home again afterwards. Uptime ~80 min (no restart before this
run, unlike the earlier ones), nobody touching the remote; the tool woke the
TV from its screensaver once. Ran 19:55–20:06 local time. No run or sample
was discarded.

| | Stock launcher | RustTV |
|---|---|---|
| Cold start, first frame (median, p10–p90, IQR) | 496 ms (452–551, IQR 45), n 30 | 349 ms (326–371, IQR 25), n 30 |
| Resume (median, p10–p90, IQR) | 93 ms (83–111, IQR 11) | 35 ms (29–45, IQR 8) |
| Idle PSS after the resume series (median of 3, p10–p90) | 98 MB (92–98) | 71 MB (64–74) |
| Idle PSS 20 s after a fresh cold start (median of 3) — see below | 94 MB | 112 MB |
| Idle CPU (median) | 0 % | 0 % |

Reading it (claim rule: IQRs must not overlap, this device only):
- Cold start and resume: RustTV is faster on this TV, as in `../full/`.
- Memory after the resume series: RustTV uses less than stock on this TV
  (71 vs 98 MB; the ranges don't overlap). Before the leak fix the same
  measurement gave 123 MB (`../full/`, `../resume-leak.md`).
- Memory 20 s after a fresh cold start: RustTV still holds its start-up peak
  here (~110 MB). Manual readings that day settled at ~66 MB by 15 s; the
  likely difference is when Android's garbage collector frees the banner
  images decoded during start-up. Being fixed; no claim from this number.

Redaction: the adb serial (the TV's local IP and port) was replaced with `tv`.

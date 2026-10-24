# Skia GPU cache limit: Philips 65OLED818/12, 2026-10-24

Why RustTV's native heap grew by ~20 MB after opening About and the long-press
menu a few times, and what a 24 MB Skia resource cache limit does to it.

**Method.** armeabi-v7a `android-release` builds, each installed with
`rusttv launcher upgrade`, then `steps.sh` (force-stop, Home, 100 s idle,
About, the menu, PSS and native heap from `dumpsys meminfo` every 10 s) and
`repeat.sh` (About and the menu twice more, read 20 s after each). One run
per build, the same apps on Home, Ukrainian UI. Raw samples in
`<build>.txt` (KB).

- `old`: commit 521d3c8, before i18n and the menu marquee.
- `current`: commit b50b4f0.
- `cache24`: b50b4f0 plus `gr_context.set_resource_cache_limit(24 MB)`
  (`third_party/slint-gpu-cache-limit.patch`).

**Native heap, MB, settled:**

| Build | Fresh | After About | After menu | Round 2 | Round 3 | PSS at end |
|---|---|---|---|---|---|---|
| old | 29.6 | 37.0 | 41.5 | 43.4 | 46.3 | 96 |
| current | 29.5 | 36.6 | 42.7 | 45.9 | 49.4 | 101 |
| cache24 | 29.6 | 36.1 | 36.6 | 37.0 | 37.4 | 86 |

**Where it is.** An earlier heapprofd run on `current` (same steps) put 4.75
of 5.2 MB of a menu-and-About round in `libGLES_mali` and 0.21 MB in our
library: GPU resources that Skia keeps, since its cache limit defaults to
256 MB. The growth is in the old build too, so i18n didn't cause it.

**Result.** With the limit the menu adds 0.5 MB instead of 4.5–6 MB, and later
rounds stay flat. The first About still adds ~6.5 MB in every build (not
repeated; not investigated). About, the menu and the edge fades rendered
normally with the limit (checked on screenshots, since deleted).

**Not measured:** frame timing. A smaller cache can mean more texture
uploads; there is no BENCHMARK_PLAN.md frame-time data for this build yet.

No identifying data: measurements of RustTV only.

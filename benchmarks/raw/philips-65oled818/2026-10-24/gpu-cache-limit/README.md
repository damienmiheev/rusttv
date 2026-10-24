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
rounds stay flat. About, the menu and the edge fades rendered normally with
the limit (checked on screenshots, since deleted).

**The first ~6.5 MB is the first focus move, not About.** `steps.sh` presses
Right before opening About. Profiled separately on the limited build (fresh
start, heapprofd as above): one focus move added 5.8 MB of native heap, 6.1 MB
of it in `libGLES_mali` (one 4.45 MB block, the size of a 1080×1080 RGBA
surface); a second move added 1.2 MB. Opening and closing About on its own
added 1.1 MB (0.61 MB driver, 0.42 MB ours). These are GPU resources Skia keeps
for reuse, inside the 24 MB limit; the driver has no symbols, so which surface
the 4.45 MB block backs is not known.

## Frames

`frames/`: does the smaller cache drop frames? RustTV draws without
Choreographer, so neither `gfxinfo` nor Perfetto's frame timeline sees its
frames on this TV. Instead `frames/block.sh` reads the present times of
RustTV's layer (`dumpsys SurfaceFlinger --latency`, 127 frames per read)
every 1.5 s through 6 rounds of a fixed navigation (4 rows down, 5 right,
back, long-press menu), after 1 warm-up round, and `frames/frames.py` merges
the reads and counts gaps over 1.5 vsync (16.7 ms) as drops. Blocks A1, B1,
A2, B2 (A = 24 MB limit at f08d424, B = the same without the limit),
installed with `rusttv launcher upgrade`; raw reads in
`frames/<block>-latency.txt.gz`, results in `frames/results.jsonl`.

| Block | Frames | Drops (gaps > 25 ms) | Drop % | p99 gap | Max gap |
|---|---|---|---|---|---|
| A1 limit | 8,739 | 6 | 0.07 | 16.8 ms | 83 ms |
| B1 none | 8,098 | 4 | 0.05 | 16.8 ms | 100 ms |
| A2 limit | 8,762 | 2 | 0.02 | 16.8 ms | 83 ms |
| B2 none | 7,926 | 3 | 0.04 | 16.8 ms | 100 ms |

No difference beyond run-to-run noise: both builds drop under 0.1% of
frames, with p99 at one vsync. One run of 6 rounds per block, not the
≥ 10 runs BENCHMARK_PLAN.md asks for, so this shows no regression rather
than proving equality.

No identifying data: measurements of RustTV only.

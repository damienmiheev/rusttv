# RustTV memory after leaving and returning: Philips 65OLED818/12, 2026-10-23

Why RustTV's idle PSS read anywhere from 63 to 127 MB on this TV.

**Steady state is ~65 MB.** Left alone on screen after a cold start, RustTV
peaks at ~112 MB about 5 s in (native heap ~51 MB while banners load and are
decoded), drops to 65 MB by 15 s, and stays flat for at least 2 minutes
(native heap 30 MB). A reading taken within ~10 s of a start is the peak, not
idle.

**Each time RustTV leaves the screen and comes back, ~1.6–1.9 MB of native
memory is never freed.** Ten cycles of "start the stock launcher, then RustTV
again" (`am start -W` both ways) raised a settled RustTV from 65 to 92 MB
(native heap 30 → 49 MB); five cycles earlier in the day added 1–3 MB each.
The benchmark's idle samples come after its 30-resume series, which explains
123–127 MB there (≈ 65 + 30 × ~2), while 2-resume short runs gave 68–70 MB.

**Where it is.** Perfetto heapprofd (4 KiB sampling, dumps every 10 s, one
leave/return cycle every 10 s, symbolized build): memory allocated by the
Mali GL driver (`libGLES_mali`) grew by ~1.6 MB per cycle (9.8 → 28.9 MB over
12 cycles) and stopped growing when the cycles stopped. Banner pixel buffers
and Skia's pixel copies did not grow (replaced, not accumulated, including
across three app-list reloads).

Slint 1.18.1's Android backend creates a new OpenGL surface, context and Skia
`DirectContext` on every `InitWindow` (`i-slint-renderer-skia`
`set_window_handle` → `OpenGLSurface::new_with_config`) and ignores
`TerminateWindow`. A local experiment that released the surface on
`TerminateWindow` (while the window is still valid) reduced the growth only
slightly (+16 MB instead of +18.5 MB over 10 cycles), so the leak is in the
per-context GL state that is created again each time, not only in the late
teardown.

**Fixed** by reusing one GL context and Skia `DirectContext` across windows
(patched Slint 1.18.1 crates in `third_party/`, see its README): after the
fix, 20 more cycles left native heap at 31.1 → 31.3 MB and PSS at
66.8 → 67.1 MB; the launcher draws normally after each return, and a cold
start still settles at ~65.8 MB by 15 s.

Impact as Home (before the fix): every return to Home from an app is one
cycle, so the launcher grew ~1.6 MB per app exit until Android's low-memory
killer stopped it (it then restarts fresh, which costs a cold start).

No identifying data: measurements of RustTV only.

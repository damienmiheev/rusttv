# Panel Studio step 1 spike: the TV renderer

Throwaway (PANEL_STUDIO.md § 15 step 1). Nothing here ships or merges into
the launcher. Measured on the Philips 65OLED818 (Android 14, armv7) on
2026-11-24, then uninstalled.

The APK has its own applicationId (`ai.rusttv.spike`), the debug key and no
HOME or launcher filter. `spike.sh` is the only way it is driven: fixed
subcommands, nothing free-form reaches adb.

## What it compares

- **Generic**: one `Generic` component fed a flat node list parsed from JSON
  at runtime (rect, text, image; position, colour, radius, border, shadow).
- **Compiled**: the same About-like tile written by hand in Slint.

The stage moves every frame (worst case: everything redrawn). Render time is
CPU time from Slint's BeforeRendering to AfterRendering, 600 frames after a
60-frame warm-up.

## Results

**2026-11-27, `interp-home`** (build with `spike.sh build --interp`): the
same Home as `compiled-home`, built from `ui/spike.slint` at run time by
`slint-interpreter` 1.18.1. Render p50/p95 9.4/10.2 ms vs 6.2/7.1 ms
compiled, PSS about 51 MB for both, source compiled in 30 ms, library
21 MB. See PANEL_STUDIO.md § 6.7.

**2026-11-27, `keys`** (`spike.sh keys 90`, the Philips remote): a held
Back arrives as one key-down (U+F748), then key-down repeats about every
50 ms from about 0.4 s, then one key-up (a 7.6 s hold logged all of
them). Red, green, yellow and blue never reached the app: the TV opened
its manual. Hence the safe look's "hold Back for 5 s" (SECURITY.md).

| nodes | generic render p50 / p95 | compiled render p50 / p95 | PSS generic / compiled |
|---|---|---|---|
| 180 (Home: 18 tiles) | 7.4 / 8.3 ms | 6.3 / 7.2 ms | 41.5 / 41.1 MB |
| 500 | 17.5 / 19.3 ms | 14.8 / 16.4 ms | 43.3 / 42.2 MB |
| 1000 | 34.4 / 36.9 ms | 28.7 / 31.1 ms | 45.9 / 43.5 MB |

- Generic costs about 18–20% more per frame; about 0.031 ms per node vs
  0.026 ms compiled. Cold start (`am start -W`): 212–246 ms vs 208–219 ms.
- JSON parse and flatten: 0.5 ms for 180 nodes, 4 ms for 2000.
- PNG decode (`png` crate, on the UI thread): 180×180 logo 3–4 ms; a
  1920×1080 photo-like PNG (3.9 MB) 285–318 ms, which stalls a frame by
  ~300 ms. The decoder adds about 16 KB to the library.
- `Window::take_snapshot` of 1920×1080: 33–45 ms.
- Accessibility: `accessible-label` does not reach Android's tree
  (uiautomator shows empty labels), so TalkBack can't read these nodes.
- Staging over `adb exec-in content write`: 1 MiB and 4 MiB arrive byte
  exact in about 2 s; the 4 MiB cap stops a 5 MB file. The shell closes its
  end without a clean status, so "complete" must come from a size and hash
  sent first, not from the pipe.
- File picker: the Philips has none (no DocumentsUI; nothing handles
  `OPEN_DOCUMENT` or `GET_CONTENT`).

A first run had the generic tiles half off screen (the component had no
size, so Slint centred it) and made generic look faster; the owner spotted
it on the TV. The table above is after the fix, checked by screenshot.

## Run

```sh
SERIAL=<adb serial> ./spike.sh build
SERIAL=<adb serial> ./spike.sh install
SERIAL=<adb serial> ./spike.sh run generic-home 25
SERIAL=<adb serial> ./spike.sh uninstall
```

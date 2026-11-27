# Patched third-party crates

## Slint 1.18.1: reuse the GL context across Android windows

`i-slint-renderer-skia-1.18.1/` and `i-slint-backend-android-activity-1.18.1/`
are the crates.io sources of Slint 1.18.1, unchanged except for the
`.patch` files described below (applied). Their own licences apply
(`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`;
see each crate's `LICENSES/`). The root `Cargo.toml` uses them through
`[patch.crates-io]`.

**Why.** Android destroys an app's window whenever another app covers it and
gives it a new one when it comes back. Slint 1.18.1 creates a new EGL context
and Skia `DirectContext` for every new window. On the Philips 65OLED818 (Mali
GPU) each one leaked ~1.6 MB of driver memory, so the launcher grew by ~1.85 MB
of native heap per return to Home
(`benchmarks/raw/philips-65oled818/2026-10-23/resume-leak.md`).

**What the patch does.**
- `i-slint-renderer-skia` `opengl_surface.rs`: when an `OpenGLSurface` is
  dropped, its GL context and Skia context are made not current and parked in
  a thread-local slot instead of being destroyed; the next `OpenGLSurface` on
  that thread reuses them with a new EGL window surface (Skia's GL state is
  reset). If the context can't be made current, it is abandoned as before.
- `i-slint-backend-android-activity` `androidwindowadapter.rs`: on
  `TerminateWindow`, while the window is still valid, the renderer releases its
  surface (`suspend()`), instead of keeping a surface for a dead window until
  the next `InitWindow`.

**Measured on the Philips** (`am start` stock launcher, then RustTV, repeated):
native heap +18.5 MB over 10 cycles before; no growth over 20 cycles after
(31.1 → 31.3 MB native heap, 66.8 → 67.1 MB PSS), and the launcher draws
normally after each return.

**Upstream:** reported as https://github.com/slint-ui/slint/issues/14397 (2026-10-23).

## Slint 1.18.1: keep unkeyed images as GPU textures

`slint-texture-upload.patch` (applied to `i-slint-renderer-skia-1.18.1/cached_image.rs`).
Skia got its own copy of every `Image::from_rgba8` buffer (`Data::new_copy`),
on top of Slint's buffer and the GPU texture Skia uploads from it. For images
without a cache key, which Slint caches only per item (that cache is cleared
together with the surface), the renderer now uploads the image to a texture
once and keeps the texture, so the extra pixel copy is freed. Keyed images and
the software renderer are unchanged.

Measured on the Philips with ~31 banners: native heap after a cold start
30.4 → 27.6 MB (−2.9 MB; one run, raw data not kept, so not a claim); the
owner checked the banners on the TV.
Reported upstream as https://github.com/slint-ui/slint/issues/14398.

## Slint 1.18.1: limit Skia's GPU resource cache to 24 MB

`slint-gpu-cache-limit.patch` (applied to `i-slint-renderer-skia-1.18.1/opengl_surface.rs`).
Skia's `DirectContext` keeps GPU resources (textures, glyph atlases) up to
256 MB by default and frees them only above that, so each new screen (About,
the long-press menu) left a few MB of Mali driver memory behind. The renderer
now sets a 24 MB limit when it creates the context.

Measured on the Philips (About and the menu three times): native heap
29.5 → 49.4 MB before, 29.6 → 37.4 MB after; PSS at the end 101 → 86 MB
(`benchmarks/raw/philips-65oled818/2026-10-24/gpu-cache-limit/`). Dropped
frames during scripted navigation: under 0.1% with and without the limit
(A/B/A/B, same place). Not reported upstream: Skia's default is a choice,
not a bug, and the right limit depends on the device.

## Slint 1.18.1: a Done key for single-line text fields

`slint-ime-done.patch` (applied to
`i-slint-backend-android-activity-1.18.1/java/SlintAndroidJavaHelper.java`).
Slint's input view asked the on-screen keyboard for no action, so on the
Google TV keyboard Enter committed a line break, which a single-line
`TextInput` turns into a space: renaming a folder could not be confirmed
from the keyboard. A field without `TYPE_TEXT_FLAG_MULTI_LINE` now asks for
`IME_ACTION_DONE`; the keyboard shows a ✓ key and sends Enter as a key
event, which reaches Slint as Return (`accepted`). Multi-line fields are
unchanged. Checked on the Google TV emulator (API 34); not yet on the
Philips. Not reported upstream yet.

**Removing it.** Once a Slint release fixes the leak, delete
this directory and the `[patch.crates-io]` section, and re-measure.

## Slint 1.18.1: less work per frame (Linux and the TV)

`slint-frame-time.patch` (applied to `i-slint-renderer-skia-1.18.1/itemrenderer.rs`
and `lib.rs`), from PR #78 (profiled by its author on an ARM projector,
Cortex-A53 + Mali-G31):
- an element faded out completely is not drawn (no offscreen layer for
  it), and an opacity that rounds to full needs no layer;
- a dirty region of several rectangles is repainted as its bounding box,
  a scissor instead of a stencil clip;
- after a frame whose changes cover 70% of the window, the next 3 frames
  repaint everything without the dirty-region pass, and the frame after
  them repaints everything too (the pass's item caches miss where items
  were drawn while it was skipped).

Checked on the Philips 2026-11-27, one quick round each of 24 D-pad
presses along the Home row (main, patched, main): launcher CPU 11.68,
11.31 and 11.75 s; no janky or late display frames in any round (one run
each, raw traces not kept, so not a claim). Not reported upstream.

## reis 0.7.1: accept a libei client without a name

`reis-0.7.1/` is the crates.io source of reis 0.7.1 (MIT, `LICENSE`),
unchanged except for `reis-null-name.patch` (applied to
`src/eiproto_eis.rs`). The Linux portal (`rusttv-portal`,
`crates/rusttv-launcher/src/linux/portal.rs`) serves libei with it.

**Why.** libei sends `ei_handshake.name` with a NULL string when the client
never called `ei_configure_name`; libei's own server takes that. KDE Connect
26.08 sets no name, so reis ended its handshake with "NULL value for
non-nullable argument" and the phone's touchpad and keyboard did nothing.

**What the patch does.** The handshake's name is read as optional; a missing
name becomes the empty string. Nothing else changes.


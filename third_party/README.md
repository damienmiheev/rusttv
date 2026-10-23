# Patched third-party crates

## Slint 1.18.1: reuse the GL context across Android windows

`i-slint-renderer-skia-1.18.1/` and `i-slint-backend-android-activity-1.18.1/`
are the crates.io sources of Slint 1.18.1, unchanged except for
`slint-gl-context-reuse.patch` (applied). Their own licences apply
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

**Removing it.** Once a Slint release fixes the leak, delete
this directory and the `[patch.crates-io]` section, and re-measure.

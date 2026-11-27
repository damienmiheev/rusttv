# RustTV Programmable Skin Design Guide (`themes/AGENTS.md`)

This document gives any AI agent or skin author **everything needed** to design, implement, test, and deploy a standalone Level 3 programmable skin (`.slint` + `.rhai`) for RustTV — without reading the rest of the Rust codebase.

---

## 1. Philosophy & Hard Rules

- **Zero shared UI with the built-in shell:** A Level 3 skin is a complete, from-scratch `Window` (`component SkinHome inherits Window`) compiled at runtime via `slint-interpreter` and scripted via sandboxed `rhai`, rendered natively on Skia OpenGL ES at 60 fps.
- **No third-party brand or project names:** Never use company, product, or third-party launcher names in folder names, `skin.toml`, `title`, comments, or docs. Use original descriptive names (e.g. `horizon-frame-pro`, `spotlight-pro`, `classic-home-pro`).
- **Strictly sandboxed:** All imports, fonts, images, and `load_image(...)` calls must stay inside the skin's own directory. Path traversal (`..`) is rejected. Rhai scripts are capped at `50,000` ops/hook, `16` call depth, `4 KiB` strings, `128` image handles, and `eval` is disabled.

---

## 2. Skin Directory Structure

Place every skin in `themes/<skin-id>/` (installed to `/usr/share/rusttv/themes/<skin-id>/` or `~/.config/rusttv/themes/<skin-id>/`):

```text
themes/my-skin-pro/
├── skin.toml        # Required manifest (schema_version = 2)
├── skin.slint       # Required UI entry exporting `SkinHome inherits Window`
├── skin.rhai        # Optional sandboxed Rhai script
├── MyFont.ttf       # Optional bundled OFL font (`import "MyFont.ttf";`)
├── wallpaper.jpg    # Optional bundled assets (`@image-url("wallpaper.jpg")`)
└── icons/           # Optional per-app monochrome overrides (`<safe-id>.svg` / `.png`)
```

### `skin.toml` (exact allowed fields — `deny_unknown_fields`)

```toml
schema_version = 2
name = "My Skin Pro"
author = "RustTV"
description = "Short one-line description shown in Settings → Theme"
entry = "skin.slint"
script = "skin.rhai"
```

---

## 3. `@rusttv/skin.slint` Host API

Every `skin.slint` imports `{ RustTV, SkinTile, SkinAction } from "@rusttv/skin.slint";` and exports `component SkinHome inherits Window`.

### `SkinTile` struct
| Field | Type | Description |
|---|---|---|
| `id` | `string` | App / tile ID (e.g. `com.moonlight_stream.Moonlight`) |
| `label` | `string` | Human-readable app name |
| `image` | `image` | Full-colour launcher artwork / icon |
| `mono-image` | `image` | Skin-transformed icon (`set_tile_image` override, `<skin>/icons/<id>.svg`, or warm-white extracted glyph fallback) |
| `kind` | `int` | `0` = banner (16:9), `1` = square/transparent icon, `2` = no artwork, `4` = loading |
| `running` | `bool` | `true` when the app is currently open in the background |
| `tint` | `color` | Dominant accent colour extracted from the app's artwork |

### `RustTV` global properties
| Property | Type | Meaning |
|---|---|---|
| `tiles` | `[SkinTile]` | Live app/tile list (cached; only rebuilt when apps/icons/running change) |
| `focused-index` | `int` | Currently focused tile index (`0..tiles.length`) |
| `clock-text` | `string` | Formatted local time (e.g. `"21:33"`) |
| `greeting-text` | `string` | Time-of-day greeting (e.g. `"GOOD EVENING"`) |
| `date-text` | `string` | Formatted uppercase date (e.g. `"WEDNESDAY 25 NOVEMBER"`) |
| `hero-title` | `string` | Label of the currently focused tile |
| `hero-color` | `color` | Accent colour of the currently focused tile |
| `hero-image` | `image` | Artwork of the currently focused tile |
| `reduced-effects` | `bool` | **`true` when Settings → Effects is Reduced** (must disable ambient glows & heavy animations!) |
| `rtl` | `bool` | `true` for right-to-left locales |
| `toast` | `string` | Short status message |
| `menu-open` | `bool` | **`true` when the user long-presses an app** (render your in-skin context menu!) |
| `menu-title` | `string` | Title of the long-pressed app |
| `menu-items` | `[string]` | Context menu actions (`Open`, `Move`, `Hide`, `Force stop`, etc.) |
| `menu-selected` | `int` | Currently highlighted index in `menu-items` |
| `quick-actions` | `[SkinAction]` | RustTV's top-right menu entries, in order. `code`: `0` Search, `1` Modify, `2` Audio, `3` Network, `4` KDE Connect, `5` Settings, `6` Power, `7` RustTV, `8` video input (`label` = its name, e.g. `"HDMI"`); `label` is empty for the others, so the skin names and draws them |
| `quick-selected` | `int` | Focused entry of `quick-actions`, or `-1` while focus is on the tiles |
| `mode` | `int` | What Home is doing: `0` browse, `1` moving a tile, `2` launching the focused app (draw your launch feedback), `4` long-press menu, `11` top-right menu |
| `s0..s7` | `string` | Reactive string slots controlled by `skin.rhai` |
| `i0..i7` | `int` | Reactive integer slots controlled by `skin.rhai` |
| `f0..f3` | `float` | Reactive float slots controlled by `skin.rhai` |
| `img0..img3` | `image` | Reactive image slots populated by `set_img_slot(slot, handle)` in `skin.rhai` |

### `RustTV` callbacks
- `RustTV.key(event.text, pressed, repeat) -> bool` — **must** be wired inside a root `FocusScope` with `init => { self.focus(); }`.
- `RustTV.tile-clicked(index, long_press)` — wire on every tile's `TouchArea` for mouse/touch support (`false` = launch, `true` = context menu).
- `RustTV.action(name, arg)` — invokes `on_action(name, arg)` in `skin.rhai`.
- `RustTV.menu-clicked(index)` — wire on every item in your `if RustTV.menu-open:` overlay for mouse/touch support.
- `RustTV.quick-clicked(index)` / `RustTV.quick-hovered(index)` — wire on each `quick-actions` button: a click runs the entry (its panel, Settings, Power…), a pointer move focuses it.

### Window title
Set `title: "RustTV";` exactly. The compositor's Home key raises the window with that title (labwc `rc.xml`), so any other title leaves the user stuck in an app with no way back.

---

## 4. Sandboxed Rhai Scripting & Image Pipeline API (`skin.rhai`)

### Lifecycle hooks (all optional)
- `fn on_init()` — runs once on load; restore saved prefs via `load_pref(...)` and initialize `i0..i7` / `s0..s7`.
- `fn on_tiles_loaded()` — runs whenever the tile list or async icons finish loading; ideal place to run your skin's icon pipeline (`img_extract_glyph`, `img_tint`, `set_tile_image`).
- `fn on_focus(index, id, label)` — runs whenever `focused-index` changes; update scroll offset slots and hero subtitles here.
- `fn on_key(key, pressed) -> bool` — return `true` to consume the key, `false` to fall through to RustTV.
  - Normalized key names passed to `on_key`: `"Up"`, `"Down"`, `"Left"`, `"Right"`, `"Ok"`, `"Back"`, `"Menu"`, plus literal keyboard characters (`"q"`, `"m"`, etc.).
  - **Important:** When `RustTV.menu-open` is `true`, while focus is in the top-right menu (`quick-selected >= 0`), and while an app launches, RustTV handles keys itself (bypassing `on_key`). Return `false` for `Up` from your first row to let RustTV move focus into its top-right menu.
  - **Important:** When handling `"Ok"` inside a top bar / custom zone, return `true` for **both** `pressed == true` and `pressed == false` so RustTV never receives a stray `OkReleased`.
- `fn on_action(name, arg)` — handles `RustTV.action(name, arg)` from `TouchArea` clicks.

### Host functions
- State slots: `set_str(slot, val)`, `get_str(slot)`, `set_int(slot, val)`, `get_int(slot)`, `set_float(slot, val)`, `get_float(slot)`
- Persistence: `save_pref(key, val)`, `load_pref(key, default)` (saved to `~/.local/state/rusttv/skin-prefs/<skin-id>.prefs`, keys `[a-zA-Z0-9_-]`)
- Launcher control: `tile_count()`, `tile_id(i)`, `tile_label(i)`, `focus(i)`, `launch(i)`, `open_settings()`, `log(msg)`
- **Composable image pipeline** (handle-based, `0` = empty/invalid, native-speed Rust):
  - `tile_image(index) -> handle`
  - `load_image(rel_path) -> handle` (strictly inside skin directory)
  - `img_extract_glyph(handle, backing_alpha, fg_hex) -> handle` (perimeter-sampled foreground/backing separation)
  - `img_tint(handle, hex, strength) -> handle`
  - `img_desaturate(handle, amount) -> handle`
  - `img_blur(handle, radius) -> handle` (3-pass separable box blur, `radius` `1..=24`)
  - `img_crop_fade(handle, x, y, w, h, fade_l, fade_r) -> handle` (1920×1080 coords + horizontal edge alpha ramp)
  - `set_tile_image(index, handle)` (sets `RustTV.tiles[index].mono-image`)
  - `set_img_slot(slot, handle)` (sets `RustTV.img0..img3`)

---

## 5. Mandatory Acceptance Checklist (Every Skin Must Pass)

1. **D-pad / TV remote:**
   - Every visual layout geometry (1-row carousel, N-column grid, vertical list) must handle `Left`/`Right`/`Up`/`Down` in `on_key` matching its exact visual rows/columns and scroll offsets (`focus(next)`).
   - Never rely on the built-in 4-column grid if your skin is a 1-row dock or 6-column grid.
2. **Mouse & Touch:**
   - Every app tile has `TouchArea { clicked => { RustTV.tile-clicked(i, false); } }`.
   - Every top-bar button/tab has `TouchArea { clicked => { RustTV.action("...", "..."); } }`.
   - Every long-press menu item has `TouchArea { clicked => { RustTV.menu-clicked(idx); } }`.
3. **Keyboard & Game Controller:**
   - Root `FocusScope` with `init => { self.focus(); }` forwards both `key-pressed` and `key-released` to `RustTV.key(...)`.
   - Gamepad D-pad / A / B / Start map to `"Up"`, `"Down"`, `"Left"`, `"Right"`, `"Ok"`, `"Back"`, `"Menu"` automatically via RustTV's input bridge.
4. **Settings Access:**
   - Must provide an obvious, D-pad-reachable **and** clickable Settings button calling `open_settings()` so the user is never trapped in a skin.
5. **Long-Press Context Menu (`RustTV.menu-open`):**
   - Must render an `if RustTV.menu-open:` overlay displaying `RustTV.menu-title` and `RustTV.menu-items` (highlighting `idx == RustTV.menu-selected`) so long-pressing an app stays 100% inside the skin.
6. **Reduced vs. Full Effects (`RustTV.reduced-effects`):**
   - When `RustTV.reduced-effects` is `true`, disable ambient colour glows and set animation durations to `0ms` (`duration: RustTV.reduced-effects ? 0ms : 180ms;`).
7. **Resolution Independence:**
   - Scale all coordinates from `property <length> u: min(root.width / 1920, root.height / 1080);` so the skin looks crisp at both `1280×720` and `1920×1080`.

---

## 6. 60 fps Performance & Slint Pitfalls

- **Always set `width: self.preferred-width;`** on right-aligned `HorizontalLayout`s (`x: parent.width - self.preferred-width - margin;`). Without explicit `width`, Slint stretches the layout to `parent.width` and pushes right-aligned children off-screen!
- **Never use `parent.parent`** in Slint — give ancestor elements an explicit `id := Rectangle { ... }` and reference `id.prop`.
- **Never declare `property <int> col` or `property <int> row`** on a `Rectangle` — `col`/`row` are reserved GridLayout properties in Slint. Use `gcol`/`grow` or `grid-col`/`grid-row`.
- **GPU texture caching:** Wrap static wallpapers, scrims, and pre-baked frosted dock mats in a `Rectangle { cache-rendering-hint: true; ... }` so low-power TV/embedded GPUs cache them once and Skia partial rendering only repaints the moving focus rect.
- **No per-frame blur:** Always pre-bake blurred backdrops or generate them once in `on_init()` / `on_tiles_loaded()` via `img_blur` + `set_img_slot`.

---

## 7. Testing & Installing a New Skin

1. **Compile & smoke-test locally:**
   ```bash
   cargo test -p rusttv-launcher --features linux skin_runtime
   ```
   Verify your skin compiles with zero Slint/Rhai errors and passes `on_init`, `on_tiles_loaded`, `on_focus`, `on_key`, and `on_action`.
2. **Install on a target device (no Rust rebuild required):**
   Copy your skin folder into either the user themes directory (`~/.config/rusttv/themes/<skin-id>/`) or the system themes directory (`/usr/share/rusttv/themes/<skin-id>/`), ensuring world-readable permissions (`chmod -R a+rX`).
3. **Activate & verify:**
   Select the skin in **Settings → Theme** (or write `skin:<skin-id>` to `~/.local/state/rusttv/theme` and restart RustTV), and check the launcher logs for any `WARN: loading programmable skin ... failed:` diagnostics.

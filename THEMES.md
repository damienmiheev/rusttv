# User Themes for RustTV

RustTV supports built-in and user-authored UI skins defined as small, declarative TOML files (`schema_version = 1`). Themes are data only (never executable code) and can transform the **entire shell** — Home layout, tile chrome, focus choreography, typography, motion cadence, directional wallpaper scrims, vector SVG or PNG backdrops, and every overlay panel (Settings, Quick Panel, Folders, Context Menu, and Power Menu) — live from **Settings → Theme** with zero per-frame blur or shader overhead.

## Built-in Presets

- **`default`** — Classic dark shelf with full hero backdrop.
- **`oled`** — True `#000000` background with softened hero glow for OLED TVs and projectors.
- **`cinema`** — Compact hero showing two rows of apps at once, warm amber focus ring, sway disabled.
- **`nordic`** — Cool slate palette with translucent tinted panels, accent bar selection, and subtle vertical gradient.
- **`grid-fast`** — Full-screen app grid with no hero, glow, or sway and snappy `0.5x` motion (fastest on weak GPUs).

## Reference Skins (`themes/`)

Thirteen ready-to-use TOML skins and three standalone Level 3 programmable skins are included in [`themes/`](themes/):

- **`coastal-glass.toml`** (*Coastal Glass*) — Full-bleed coastal wallpaper (`coastal-cliff.svg`), top-left clean clock, borderless inline-label top-right controls (`inline-labels`), optical pre-baked frosted-glass floating bottom dock (`dock_blur = true`), unframed dock icons with labels inside (`dock-icon`), and frosted glass-pill focus (`glass-pill`).
- **`coastal-minimal.toml`** (*Coastal Minimal*) — Same full-bleed coastal wallpaper and optical frosted-glass bottom dock, paired with borderless monochrome top-right icons (`bare-icons`).
- **`cinema-shelf.toml`** (*Cinema Shelf*) — Full-bleed warm cinematic backdrop (`mars-horizon.svg`), bold top-left display title (`hero_title_position = "top-start"`), auto-hiding top bar, floating frosted-glass 5-card top-shelf dock with borderless focus lift, and row 2 peeking at the bottom edge.
- **`spotlight-circles.toml`** (*Spotlight Circles*) — Top-left pill navigation strip with solid white `Apps` pill, compact circular top-right buttons + `RustTV` wordmark, cinematic hero fade (`cinema-hero.svg`), `"Your apps"` section heading, and true circular app icons (`tile_aspect = "circle"`) with crisp white focus ring.
- **`ambient-pills.toml`** (*Ambient Pills*) — Deep near-black canvas lit by vivid app-colour ambient glow, top-left greeting + clock + vertical divider + uppercase date (`clock_position = "greeting-header"`), top-right stadium glass pills (`top_bar_style = "glass-pills"`), focused app title directly above a wide 4-column framed dock (`hero_title_position = "above-dock"`), and row 2 peeking at the bottom.
- **`horizon-zen.toml`** (*Horizon Zen*) — Wallpaper-first minimalist zen screen with a vector SVG landscape backdrop (`horizon-backdrop.svg`), an ultra-light 116px centered hero clock, frosted floating bottom dock, unfocused tile dimming, underline focus, and translucent tinted overlays.
- **`obsidian-blade.toml`** (*Obsidian Blade*) — Full-height edge-to-edge media-center navigation blade with electric cyan fill-highlights, wide live showcase preview, and consistent cyan accent selection across Settings and Quick Panels.
- **`glass-dock.toml`** (*Glass Dock*) — Floating frosted bottom dock with 5 large rounded cards, borderless focus lift, top-start clock, subtle translucent glass chrome, and clean tinted overlays.
- **`classic-refined.toml`** (*Classic*) — Standard 6-column shelf layout with subtle hue tinting on icon cards.
- **`theatre-rail.toml`** (*Theatre Rail*) — Vertical sidebar rail on the start side with an accent bar highlight, top header strip, and a large live showcase preview on the end side.
- **`cinema-ribbon.toml`** (*Cinema Ribbon*) — Single horizontal coverflow ribbon centered on the focused tile under a centered showcase, unfocused dimming, and vivid ambient glow.
- **`arcade-grid.toml`** (*Arcade Grid*) — Full-screen 5-column square-tile grid with bold outline framing, top header strip, always-visible labels, and crisp fast transitions.
- **`fast-wall.toml`** (*Fast Wall*) — Distraction-free 6-column wall of apps with zero hero, glow, or motion overhead (`motion_scale = 0.0`).

Copy any `.toml` file (and its referenced `.svg` / `.png` wallpaper, if any) into `~/.config/rusttv/themes/` or `/usr/share/rusttv/themes/` and open **Settings → Theme**.

## Where to Place Custom Themes

Drop any `<name>.toml` file into one of these directories (checked in order; earlier directories take precedence if two files share the same stem):

1. `<state_dir>/themes/` — next to `order.toml` (works on both Linux and Android).
2. `$XDG_CONFIG_HOME/rusttv/themes/` (or `~/.config/rusttv/themes/`).
3. `/usr/share/rusttv/themes/` (system-wide themes on Linux).

Opening **Settings → Theme** re-scans these folders automatically, so you can edit or add skins without restarting RustTV. You can also override the active theme for a session with `RUSTTV_THEME` (e.g. `RUSTTV_THEME=user:horizon-zen`).

## Theme Format (`schema_version = 1`)

All sections and fields except `schema_version` and `name` are optional and inherit smart defaults for the chosen `layout.style`. Unknown keys are rejected so typos are caught early.

```toml
schema_version = 1
name = "Obsidian Blade"
author = "Your Name"
description = "Full-height edge-to-edge media blade with cyan highlights"

[colors]
bg = "#070B12"               # Page background (#RGB, #RRGGBB, or #RRGGBBAA)
surface = "#111A29"          # Card, panel, and menu surface
dock_bg = "#0C1422EE"        # Floating dock, sidebar/blade rail, or top header strip backdrop
text = "#F2F6FC"             # Primary text
text_dim = "#8C9DB5"         # Secondary text and hints
accent = "#22B8CF"           # Accent dot, sliders, bar/fill highlights, running-app pill
accent_text = "#061018"      # Text on solid accent fills (auto-contrasted if omitted)
focus_ring = "#22B8CF"       # Tile focus border / glow / underline colour (defaults to `text`)
icon_card_tint = 0.14        # 0.0..=0.5 tint on Linux icon-only cards toward app hue

[layout]
style = "blade"              # "shelf" | "dock" | "sidebar" | "blade" | "grid" | "carousel" | "zen"
columns = 1                  # 1..=8 tiles per row (ignored in "carousel" style)
tile_aspect = "row"          # "wide" (16:9) | "square" (1:1) | "poster" (4:3) | "row" (list) | "circle" (1:1 round)
tile_style = "glass"         # "card" | "glass" | "outline" | "poster" | "minimal" | "dock-icon"
tile_border_width = 1.0      # 0..=4 rpx subtle resting hairline on unfocused tiles
tile_border_color = "#FFFFFF22" # Colour of resting tile border
tile_dim_unfocused = 0.0     # 0.0..=0.55 darken unfocused tiles to spotlight selection
tile_gap = 10                # 12..=72 rpx gap between tiles
grid_margin_x = 28           # 20..=240 rpx horizontal outer margin
hero = "full"                # "full" | "compact" | "off"
hero_align = "end"           # "start" | "center" | "end"
hero_artwork = true          # Show focused app preview card in hero
hero_title_size = 64         # 40..=96 rpx hero title font size
hero_title_position = "default" # "default" | "above-dock" | "top-start"
focus_style = "fill"         # "ring" | "glow" | "bar" | "lift" | "fill" | "underline" | "glass-pill"
dock_surface = true          # Draw container behind dock shelf or sidebar/blade rail
dock_blur = false            # Pre-bake optical frosted-glass blur behind the dock
dock_radius = 0              # 0..=48 rpx dock corner radius override (0 = inherit panel_radius)
section_label = ""           # Optional section heading above row 0 (e.g. "Your apps")
top_bar_style = "minimal"    # "floating" | "strip" | "minimal" | "inline-labels" | "bare-icons" | "glass-pills" | "google-tv"
clock_position = "top-start" # "top-end" | "top-start" | "top-center" | "bottom-end" | "center-hero" | "greeting-header" | "hidden"
tile_radius = 8              # 0..=32 rpx
panel_radius = 12            # 0..=36 rpx (Settings, Quick Panel, Folders, Context & Power menus)
focus_scale = 1.01           # 1.0..=1.25 focused tile zoom
focus_ring_width = 0         # 0..=8 rpx
show_labels = "always"       # "focused" | "always" | "never"
sway = false                 # Idle sway animation on focused tile
glow_strength = 0.8          # 0.0..=1.5 hero colour glow intensity (0 disables glow baking)

[surfaces]
panel_style = "tinted"       # "solid" | "tinted" | "outline" | "elevated"
panel_opacity = 0.90         # 0.55..=1.0 alpha on Settings, Quick Panel, Folders & menus
panel_border_width = 1.5     # 0..=4 rpx border on overlay panels
panel_border_color = "#22B8CF38" # Border colour for overlay panels
scrim_opacity = 0.68         # 0.30..=0.95 modal backdrop dim behind overlays
selection_style = "accent"   # "invert" | "accent" | "bar" (focused row style in menus/Settings)

[typography]
font_family = ""             # Optional installed system font family name (empty = default sans)
clock_size = 44              # 24..=140 rpx clock font size (108+ works great with "center-hero")
clock_weight = 300           # 200..=700 clock font weight
hero_title_weight = 700      # 300..=800 hero title font weight
motion_scale = 0.8           # 0.0..=1.5 animation speed multiplier (0.0 = instant snaps)

[background]
gradient_top = "#0E1A2B"
gradient_bottom = "#05080F"
gradient_angle = 135         # 0..=360 degrees (180 = top-to-bottom)
scrim = "left"               # "none" | "bottom" | "left" | "vignette"
scrim_strength = 0.60        # 0.0..=0.90 directional contrast scrim over wallpaper
# Optional PNG or vector SVG wallpaper in the SAME themes directory:
# image = "horizon-backdrop.svg"
# image_opacity = 0.85       # 0.0..=1.0
```

## Performance & Safety Limits

- **Zero per-frame overhead:** Every TOML skinning option maps directly to GPU-cached Skia primitives, alpha rectangles, and a single decoded wallpaper bitmap (SVG wallpapers are rasterized once on theme load at 1280×720 via `resvg`, and optical frosted-glass docks use a pre-baked Gaussian-blurred copy). Setting `motion_scale = 0.0` and `glow_strength = 0.0` eliminates animations and glow baking completely.
- Theme `.toml` files are capped at **64 KiB**.
- Background images must be a single `.png` or `.svg` filename inside the theme's own directory (no `..` or subdirectories), capped at **4 MiB** (and **1920×1080** for PNGs).
- Numeric ranges are clamped to valid bounds and control / bidirectional override characters are stripped from text fields.
- Invalid theme files are logged and skipped so RustTV always starts cleanly.

## Level 3 Programmable Skins (`.slint` + Sandboxed `.rhai`)

When you want **arbitrary shapes, custom layouts, custom vector `Path`s, and custom scripted behavior** (HTML + CSS + JS parity on native 60 fps Skia, enabled via the `skins` / `linux` feature), drop a folder into any `themes/` directory:

```text
themes/spotlight-pro/
├── skin.toml      # schema_version = 2, name, author, description, entry = "skin.slint", script = "skin.rhai"
├── skin.slint     # arbitrary Slint UI importing { RustTV } from "@rusttv/skin.slint"
└── skin.rhai      # optional sandboxed Rhai script (on_init, on_focus, on_key, on_action, on_tick)
```

- **UI (`skin.slint`)**: Compiled at runtime by `slint-interpreter` onto the exact same Skia OpenGL ES renderer. Imports `{ RustTV, SkinTile, SkinAction } from "@rusttv/skin.slint"` for live tiles, focus, clock/greeting/date, hero data, reactive script slots (`s0..s7`, `i0..i7`, `f0..f3`), and callbacks (`key`, `tile-clicked`, `action`).
- **Scripting (`skin.rhai`)**: Runs in a strict sandbox (50,000 op budget per hook, 16-deep call stack, 4 KiB string cap, zero filesystem/network access) with host functions `set_str`/`get_str`, `set_int`/`get_int`, `set_float`/`get_float`, `tile_count`, `tile_id`, `tile_label`, `launch`, `focus`, `open_settings`, and `log`.
- **Examples**:
  - [`themes/spotlight-pro/`](themes/spotlight-pro/) — standalone interactive pill-tab launcher with live Rhai tabs, spotlight subtitle, and circular app icons.
  - [`themes/coastal-dock-pro/`](themes/coastal-dock-pro/) — standalone frosted wallpaper dock with unframed icons, glass-pill focus, and Rhai-togglable top bar.
  - [`themes/ambient-pills-pro/`](themes/ambient-pills-pro/) — standalone radial-glow canvas with greeting + date header, stadium glass pills, and wide 4-column framed dock.

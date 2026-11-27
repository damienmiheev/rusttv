//! User-generated and built-in UI themes (ROADMAP.md § Customisable Home screen).
//!
//! A theme is a small, versioned TOML file (`schema_version = 1`) describing
//! colours, tile/hero layout, and an optional background gradient or PNG.
//! Themes are untrusted data only — never code — with strict size, range,
//! and path checks so a broken or hostile theme can at worst look odd.
//!
//! Discovery order (later directories never hide earlier ones with the same
//! file stem):
//! - `<state_dir>/themes/*.toml` (next to `order.toml`; works on Android & Linux)
//! - `$XDG_CONFIG_HOME/rusttv/themes/*.toml` (or `~/.config/rusttv/themes`)
//! - `/usr/share/rusttv/themes/*.toml` (system / image themes on Linux)

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::platform::Bitmap;

/// File storing the active theme's ID next to `order.toml`.
pub(crate) const THEME_FILE: &str = "theme";

/// Directory name for user theme files under a state or config root.
pub(crate) const THEMES_DIR: &str = "themes";

/// Current theme schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Maximum size of a `.toml` theme file (64 KiB).
const MAX_THEME_BYTES: u64 = 64 * 1024;

/// Maximum size of a theme background PNG file (4 MiB).
#[cfg_attr(
    not(all(target_os = "linux", feature = "linux")),
    allow(dead_code, reason = "PNG decoding uses the optional `png` crate")
)]
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

/// Maximum width and height of a decoded theme background PNG.
#[cfg_attr(
    not(all(target_os = "linux", feature = "linux")),
    allow(dead_code, reason = "PNG decoding uses the optional `png` crate")
)]
const MAX_IMAGE_SIZE: (u32, u32) = (1920, 1080);

/// Maximum character lengths for user-supplied metadata strings.
const MAX_NAME_LEN: usize = 40;
const MAX_AUTHOR_LEN: usize = 60;
const MAX_DESC_LEN: usize = 120;

/// RGBA colour, 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgb_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    pub fn to_slint(self) -> slint::Color {
        slint::Color::from_argb_u8(self.a, self.r, self.g, self.b)
    }

    /// Parses `#RGB`, `#RRGGBB`, or `#RRGGBBAA`.
    pub fn parse_hex(text: &str) -> Result<Self, String> {
        let hex = text
            .trim()
            .strip_prefix('#')
            .ok_or_else(|| format!("colour {text:?} must start with '#'"))?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("colour {text:?} has non-hex characters"));
        }
        let byte = |range: std::ops::Range<usize>| {
            u8::from_str_radix(&hex[range], 16).map_err(|e| e.to_string())
        };
        match hex.len() {
            3 => {
                let nibble = |i: usize| {
                    let v = u8::from_str_radix(&hex[i..i + 1], 16).map_err(|e| e.to_string())?;
                    Ok::<u8, String>(v * 17)
                };
                Ok(Self::rgb(nibble(0)?, nibble(1)?, nibble(2)?))
            }
            6 => Ok(Self::rgb(byte(0..2)?, byte(2..4)?, byte(4..6)?)),
            8 => Ok(Self {
                r: byte(0..2)?,
                g: byte(2..4)?,
                b: byte(4..6)?,
                a: byte(6..8)?,
            }),
            _ => Err(format!(
                "colour {text:?} must be #RGB, #RRGGBB, or #RRGGBBAA"
            )),
        }
    }
}

/// Overall Home screen layout archetype.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutStyle {
    /// Classic top hero + bottom app shelf that scrolls into a full grid.
    #[default]
    Shelf,
    /// Spacious showcase with a floating rounded dock shelf at the bottom.
    Dock,
    /// Vertical scrollable rail on the start side + live showcase on the end side.
    Sidebar,
    /// Full-screen app grid starting near the top.
    Grid,
    /// Single horizontal coverflow ribbon centered on the focused tile.
    Carousel,
    /// Full-height edge-to-edge left navigation blade + hero & showcase canvas.
    Blade,
    /// Wallpaper-first minimalist canvas with oversized clock and bottom launcher bar.
    Zen,
}

impl LayoutStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Shelf => 0,
            Self::Dock => 1,
            Self::Sidebar => 2,
            Self::Grid => 3,
            Self::Carousel => 4,
            Self::Blade => 5,
            Self::Zen => 6,
        }
    }
}

/// Visual card skin for Home tiles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileStyle {
    /// Standard solid/artwork card (default).
    #[default]
    Card,
    /// Translucent tinted card with subtle hairline frame.
    Glass,
    /// Transparent resting card with crisp border; fills on focus.
    Outline,
    /// Poster card with a bottom gradient scrim and overlaid title inside.
    Poster,
    /// Borderless floating artwork with clean typography underneath.
    Minimal,
    /// Unframed dock icon with label centered inside the bottom of the tile
    /// (Projectivy frosted-dock look).
    #[serde(rename = "dock-icon")]
    DockIcon,
}

impl TileStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Card => 0,
            Self::Glass => 1,
            Self::Outline => 2,
            Self::Poster => 3,
            Self::Minimal => 4,
            Self::DockIcon => 5,
        }
    }
}

/// Global surface look for Settings, Quick Panels, Folders, and Context Menus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelStyle {
    /// Opaque solid surface panels (default).
    #[default]
    Solid,
    /// Translucent tinted panels letting the wallpaper/gradient show through.
    Tinted,
    /// Flat crisp hairline-bordered panels.
    Outline,
    /// Raised surface with subtle accent-tinted frame.
    Elevated,
}

impl PanelStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Solid => 0,
            Self::Tinted => 1,
            Self::Outline => 2,
            Self::Elevated => 3,
        }
    }
}

/// How focused rows highlight in Settings, Quick Panels, and Context Menus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SelectionStyle {
    /// Inverted light pill with dark text (default RustTV look).
    #[default]
    Invert,
    /// Solid `accent` fill with `accent_text` (classic media-center skin).
    Accent,
    /// Subtle surface tint with a vertical `accent` edge bar.
    Bar,
}

impl SelectionStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Invert => 0,
            Self::Accent => 1,
            Self::Bar => 2,
        }
    }
}

/// Directional legibility scrim rendered over a wallpaper or gradient.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BgScrim {
    #[default]
    None,
    /// Dark gradient rising from the bottom edge (ideal for Dock / Zen / Shelf).
    Bottom,
    /// Dark gradient sweeping from the start edge (ideal for Sidebar / Blade).
    Left,
    /// Cinematic top + bottom vignette scrim.
    Vignette,
}

impl BgScrim {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Bottom => 1,
            Self::Left => 2,
            Self::Vignette => 3,
        }
    }
}

/// Shape / aspect ratio of Home tiles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileAspect {
    /// 16:9 widescreen banner cards (default).
    #[default]
    Wide,
    /// 1:1 square icon cards.
    Square,
    /// 4:3 media poster cards.
    Poster,
    /// Compact horizontal list row (icon + label side by side, ideal for Sidebar).
    Row,
    /// Circular 1:1 app icons (Google TV style).
    Circle,
}

impl TileAspect {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Wide => 0,
            Self::Square => 1,
            Self::Poster => 2,
            Self::Row => 3,
            Self::Circle => 4,
        }
    }
}

/// Horizontal alignment of hero title and artwork.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeroAlign {
    /// Aligned to the reading-direction start edge (left in LTR).
    #[default]
    Start,
    /// Centered across the screen.
    Center,
    /// Aligned to the reading-direction end edge (right showcase in Sidebar).
    End,
}

impl HeroAlign {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Start => 0,
            Self::Center => 1,
            Self::End => 2,
        }
    }
}

/// Visual treatment of the focused tile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FocusStyle {
    /// Crisp border ring around the card (default).
    #[default]
    Ring,
    /// Soft accent glow border.
    Glow,
    /// Accent edge indicator bar + highlighted fill (classic list/media rail look).
    Bar,
    /// Borderless elevated float with subtle zoom and shadow.
    Lift,
    /// Solid accent fill inversion with contrasting `accent_text`.
    Fill,
    /// Minimalist accent underline below the focused tile.
    Underline,
    /// Frosted translucent glass card highlight enclosing icon + label
    /// (Projectivy frosted-dock focus style).
    #[serde(rename = "glass-pill")]
    GlassPill,
}

impl FocusStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Ring => 0,
            Self::Glow => 1,
            Self::Bar => 2,
            Self::Lift => 3,
            Self::Fill => 4,
            Self::Underline => 5,
            Self::GlassPill => 6,
        }
    }
}

/// Top quick-action bar presentation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TopBarStyle {
    /// Floating pills in the top-end corner (default).
    #[default]
    Floating,
    /// Translucent full-width top header strip.
    Strip,
    /// Subtle low-opacity floating pills until focused.
    Minimal,
    /// Borderless monochrome icon + text label inline in the top-end corner.
    InlineLabels,
    /// Clean borderless monochrome icons in the top-end corner.
    BareIcons,
    /// Stadium frosted glass pills with hairline borders and icon + label.
    GlassPills,
    /// Google TV top bar: left pill navigation tabs, right circular icon buttons.
    GoogleTv,
}

impl TopBarStyle {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Floating => 0,
            Self::Strip => 1,
            Self::Minimal => 2,
            Self::InlineLabels => 3,
            Self::BareIcons => 4,
            Self::GlassPills => 5,
            Self::GoogleTv => 6,
        }
    }
}

/// Where the focused app's hero title is placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeroTitlePosition {
    /// Standard hero position for the active layout style.
    #[default]
    Default,
    /// Directly above the dock/shelf, aligned to the grid start edge.
    AboveDock,
    /// Bold display title in the top-start corner of the hero canvas (tvOS style).
    TopStart,
}

impl HeroTitlePosition {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Default => 0,
            Self::AboveDock => 1,
            Self::TopStart => 2,
        }
    }
}

/// Where the Home clock / status corner is placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockPosition {
    /// Top-end corner beside the quick bar (default RustTV look).
    #[default]
    TopEnd,
    /// Top-start corner (classic TV header clock).
    TopStart,
    /// Top-center header.
    TopCenter,
    /// Bottom-end corner.
    BottomEnd,
    /// Hidden on Home (idle screen still works).
    Hidden,
    /// Large centerpiece clock on the Home canvas (ideal for `zen` wallpaper skins).
    CenterHero,
    /// Greeting line above large clock + vertical divider + uppercase date in top-start.
    GreetingHeader,
}

impl ClockPosition {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::TopEnd => 0,
            Self::TopStart => 1,
            Self::TopCenter => 2,
            Self::BottomEnd => 3,
            Self::Hidden => 4,
            Self::CenterHero => 5,
            Self::GreetingHeader => 6,
        }
    }
}

/// How tall the hero area above the app grid is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeroMode {
    /// Standard 780 rpx hero above the first row of tiles.
    #[default]
    Full,
    /// Compact 460 rpx hero, showing two rows of tiles immediately.
    Compact,
    /// No hero area: the app grid starts at the top (3+ rows at once).
    Off,
}

impl HeroMode {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Full => 0,
            Self::Compact => 1,
            Self::Off => 2,
        }
    }
}

/// When banner tiles show their name label below the card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LabelMode {
    /// Only under the focused tile (default RustTV look).
    #[default]
    Focused,
    /// Under every tile at all times.
    Always,
    /// Never show tile labels below cards (hero still shows the name).
    Never,
}

impl LabelMode {
    pub const fn as_i32(self) -> i32 {
        match self {
            Self::Focused => 0,
            Self::Always => 1,
            Self::Never => 2,
        }
    }
}

/// Colours of a theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeColors {
    pub bg: Rgba,
    pub surface: Rgba,
    /// Backdrop colour for floating dock, sidebar rail, or top header strip.
    pub dock_bg: Rgba,
    pub text: Rgba,
    pub text_dim: Rgba,
    pub accent: Rgba,
    /// Contrasting text colour drawn on top of `accent` fills (e.g. Kodi-style
    /// accent focus bars/rows). Auto-derived from `accent` luminance if omitted.
    pub accent_text: Rgba,
    /// Focus border colour on tiles; defaults to `text`.
    pub focus_ring: Rgba,
    /// How strongly Linux icon-only cards tint toward the app's dominant hue (0..=0.5).
    pub icon_card_tint: f32,
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            bg: Rgba::rgb(0x0E, 0x10, 0x14),
            surface: Rgba::rgb(0x1C, 0x1F, 0x26),
            dock_bg: Rgba {
                r: 0x1C,
                g: 0x1F,
                b: 0x26,
                a: 0xD8,
            },
            text: Rgba::rgb(0xF2, 0xF3, 0xF5),
            text_dim: Rgba::rgb(0xA4, 0xA9, 0xB4),
            accent: Rgba::rgb(0xF5, 0xA5, 0x24),
            accent_text: Rgba::rgb(0x0E, 0x10, 0x14),
            focus_ring: Rgba::rgb(0xF2, 0xF3, 0xF5),
            icon_card_tint: 0.0,
        }
    }
}

/// Global overlay / Settings / menu skin surfaces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeSurfaces {
    pub panel_style: PanelStyle,
    /// Opacity of Settings, Quick Panels, Folders, and Context Menus (`0.55..=1.0`).
    pub panel_opacity: f32,
    /// Border thickness around overlay panels in reference pixels (`0..=4`).
    pub panel_border_width: f32,
    pub panel_border_color: Rgba,
    /// Backdrop scrim opacity behind open folders/menus/Settings (`0.30..=0.95`).
    pub scrim_opacity: f32,
    /// How focused rows highlight in Settings, Quick Panels, and Menus.
    pub selection_style: SelectionStyle,
}

impl Default for ThemeSurfaces {
    fn default() -> Self {
        Self {
            panel_style: PanelStyle::Solid,
            panel_opacity: 1.0,
            panel_border_width: 0.0,
            panel_border_color: Rgba {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
                a: 0x22,
            },
            scrim_opacity: 0.78,
            selection_style: SelectionStyle::Invert,
        }
    }
}

/// Typography and motion tuning for a skin.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeTypography {
    /// Optional installed system font family name (empty = platform default).
    pub font_family: String,
    /// Home clock font size in reference pixels (`24..=140`).
    pub clock_size: f32,
    /// Home clock font weight (`200..=700`).
    pub clock_weight: i32,
    /// Hero title font weight (`300..=800`).
    pub hero_title_weight: i32,
    /// Animation speed multiplier (`0.0` = instant, `1.0` = standard, `1.5` = silky).
    pub motion_scale: f32,
}

impl Default for ThemeTypography {
    fn default() -> Self {
        Self::defaults_for(LayoutStyle::Shelf)
    }
}

impl ThemeTypography {
    pub fn defaults_for(style: LayoutStyle) -> Self {
        let base = Self {
            font_family: String::new(),
            clock_size: 34.0,
            clock_weight: 400,
            hero_title_weight: 600,
            motion_scale: 1.0,
        };
        match style {
            LayoutStyle::Zen => Self {
                clock_size: 112.0,
                clock_weight: 300,
                ..base
            },
            LayoutStyle::Blade => Self {
                clock_size: 40.0,
                clock_weight: 300,
                hero_title_weight: 700,
                ..base
            },
            _ => base,
        }
    }
}

/// Layout and motion settings of a theme.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeLayout {
    pub style: LayoutStyle,
    /// Tiles per row on Home (`1..=8`; ignored in `Carousel` mode).
    pub columns: usize,
    pub tile_aspect: TileAspect,
    pub tile_style: TileStyle,
    /// Optional resting border thickness around tiles (`0..=4`).
    pub tile_border_width: f32,
    pub tile_border_color: Rgba,
    /// How much unfocused tiles dim (`0.0..=0.55`) so focus pops.
    pub tile_dim_unfocused: f32,
    /// Gap between adjacent tiles in reference pixels (`12..=72`).
    pub tile_gap: f32,
    /// Horizontal outer margin of the grid in reference pixels (`40..=180`).
    pub grid_margin_x: f32,
    pub hero: HeroMode,
    pub hero_align: HeroAlign,
    /// Whether the hero displays the focused app's artwork preview card.
    pub hero_artwork: bool,
    /// Hero title font size in reference pixels (`40..=96`).
    pub hero_title_size: f32,
    /// Where the hero title sits (`default`, `above-dock`, `top-start`).
    pub hero_title_position: HeroTitlePosition,
    pub focus_style: FocusStyle,
    /// Whether to draw a rounded container behind the dock shelf or sidebar rail.
    pub dock_surface: bool,
    /// Pre-bake a blurred wallpaper copy for optical frosted-glass dock containers.
    pub dock_blur: bool,
    /// Optional corner radius override for the floating dock container (`0..=48`;
    /// `0` inherits `panel_radius`).
    pub dock_radius: f32,
    /// Optional section label above row 0 (e.g. `"Your apps"`).
    pub section_label: String,
    pub top_bar_style: TopBarStyle,
    pub clock_position: ClockPosition,
    /// Tile corner radius in reference pixels (0..=32).
    pub tile_radius: f32,
    /// Panel / menu corner radius in reference pixels (0..=36).
    pub panel_radius: f32,
    /// Focused tile zoom multiplier (1.0..=1.25).
    pub focus_scale: f32,
    /// Focused tile border width in reference pixels (0..=8).
    pub focus_ring_width: f32,
    pub show_labels: LabelMode,
    /// Whether the focused tile gently sways while idle.
    pub sway: bool,
    /// Multiplier for the hero colour glow intensity (0.0..=1.5).
    pub glow_strength: f32,
}

impl Default for ThemeLayout {
    fn default() -> Self {
        Self::defaults_for(LayoutStyle::Shelf)
    }
}

impl ThemeLayout {
    /// Smart layout defaults tailored for each [`LayoutStyle`] archetype.
    pub fn defaults_for(style: LayoutStyle) -> Self {
        let base = Self {
            style,
            columns: 6,
            tile_aspect: TileAspect::Wide,
            tile_style: TileStyle::Card,
            tile_border_width: 0.0,
            tile_border_color: Rgba {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
                a: 0x24,
            },
            tile_dim_unfocused: 0.0,
            tile_gap: 48.0,
            grid_margin_x: 90.0,
            hero: HeroMode::Full,
            hero_align: HeroAlign::Start,
            hero_artwork: true,
            hero_title_size: 68.0,
            hero_title_position: HeroTitlePosition::Default,
            focus_style: FocusStyle::Ring,
            dock_surface: false,
            dock_blur: false,
            dock_radius: 0.0,
            section_label: String::new(),
            top_bar_style: TopBarStyle::Floating,
            clock_position: ClockPosition::TopEnd,
            tile_radius: 12.0,
            panel_radius: 16.0,
            focus_scale: 1.15,
            focus_ring_width: 4.0,
            show_labels: LabelMode::Focused,
            sway: true,
            glow_strength: 1.0,
        };
        match style {
            LayoutStyle::Shelf => base,
            LayoutStyle::Dock => Self {
                columns: 5,
                tile_gap: 40.0,
                grid_margin_x: 110.0,
                focus_style: FocusStyle::Lift,
                dock_surface: true,
                clock_position: ClockPosition::TopStart,
                tile_radius: 20.0,
                panel_radius: 28.0,
                focus_scale: 1.16,
                ..base
            },
            LayoutStyle::Sidebar => Self {
                columns: 1,
                tile_aspect: TileAspect::Row,
                tile_gap: 14.0,
                grid_margin_x: 56.0,
                hero: HeroMode::Full,
                hero_align: HeroAlign::End,
                focus_style: FocusStyle::Bar,
                dock_surface: true,
                top_bar_style: TopBarStyle::Strip,
                clock_position: ClockPosition::TopStart,
                tile_radius: 10.0,
                panel_radius: 16.0,
                focus_scale: 1.02,
                show_labels: LabelMode::Always,
                sway: false,
                ..base
            },
            LayoutStyle::Grid => Self {
                hero: HeroMode::Off,
                tile_gap: 36.0,
                grid_margin_x: 90.0,
                show_labels: LabelMode::Always,
                sway: false,
                glow_strength: 0.0,
                ..base
            },
            LayoutStyle::Carousel => Self {
                columns: 6,
                tile_gap: 44.0,
                hero: HeroMode::Full,
                hero_align: HeroAlign::Center,
                focus_style: FocusStyle::Glow,
                dock_surface: false,
                tile_radius: 18.0,
                focus_scale: 1.18,
                show_labels: LabelMode::Focused,
                ..base
            },
            LayoutStyle::Blade => Self {
                columns: 1,
                tile_aspect: TileAspect::Row,
                tile_style: TileStyle::Glass,
                tile_gap: 10.0,
                grid_margin_x: 28.0,
                hero: HeroMode::Full,
                hero_align: HeroAlign::End,
                focus_style: FocusStyle::Fill,
                dock_surface: true,
                top_bar_style: TopBarStyle::Minimal,
                clock_position: ClockPosition::TopStart,
                tile_radius: 8.0,
                panel_radius: 12.0,
                focus_scale: 1.01,
                show_labels: LabelMode::Always,
                sway: false,
                ..base
            },
            LayoutStyle::Zen => Self {
                columns: 6,
                tile_aspect: TileAspect::Square,
                tile_style: TileStyle::Glass,
                tile_dim_unfocused: 0.22,
                tile_gap: 32.0,
                grid_margin_x: 150.0,
                hero: HeroMode::Off,
                hero_artwork: false,
                focus_style: FocusStyle::Underline,
                dock_surface: true,
                top_bar_style: TopBarStyle::Minimal,
                clock_position: ClockPosition::CenterHero,
                tile_radius: 22.0,
                panel_radius: 24.0,
                focus_scale: 1.12,
                show_labels: LabelMode::Focused,
                sway: false,
                glow_strength: 0.35,
                ..base
            },
        }
    }

    /// Navigation parameters `(columns, focus_slots, carousel)` for [`crate::Home::set_layout`].
    pub const fn home_nav(&self) -> (usize, usize, bool) {
        match self.style {
            LayoutStyle::Carousel => (self.columns, 3, true),
            LayoutStyle::Sidebar | LayoutStyle::Blade => {
                let slots = match self.tile_aspect {
                    TileAspect::Row => 8,
                    _ => 4,
                };
                (self.columns, slots, false)
            }
            LayoutStyle::Grid => {
                let slots = match self.tile_aspect {
                    TileAspect::Square => 2,
                    TileAspect::Poster => 3,
                    _ => 3,
                };
                (self.columns, slots, false)
            }
            LayoutStyle::Shelf | LayoutStyle::Dock | LayoutStyle::Zen => (self.columns, 3, false),
        }
    }
}

/// Optional static background gradient, scrim, or wallpaper image (PNG or SVG).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeBackground {
    pub gradient: Option<(Rgba, Rgba)>,
    /// Linear gradient angle in degrees (`0..=360`, default `180` = top-to-bottom).
    pub gradient_angle: f32,
    pub scrim: BgScrim,
    /// Scrim strength (`0.0..=0.9`, default `0.55`).
    pub scrim_strength: f32,
    /// Validated path to a PNG or SVG inside the theme's own directory.
    pub image_path: Option<PathBuf>,
    pub image_opacity: f32,
}

impl Default for ThemeBackground {
    fn default() -> Self {
        Self {
            gradient: None,
            gradient_angle: 180.0,
            scrim: BgScrim::None,
            scrim_strength: 0.55,
            image_path: None,
            image_opacity: 0.35,
        }
    }
}

/// A validated, ready-to-apply UI theme (built-in or loaded from TOML).
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeSpec {
    /// Stable identifier (e.g. `"default"`, `"oled"`, `"user:sunset"`).
    pub id: String,
    /// User-visible name shown in Settings → Theme.
    pub name: String,
    /// Optional author or subtitle shown as detail in Settings → Theme.
    pub detail: String,
    /// True for themes loaded from a user `.toml` file.
    pub user: bool,
    pub colors: ThemeColors,
    pub layout: ThemeLayout,
    pub surfaces: ThemeSurfaces,
    pub typography: ThemeTypography,
    pub background: ThemeBackground,
}

impl Default for ThemeSpec {
    fn default() -> Self {
        Self {
            id: "default".to_owned(),
            name: "Default".to_owned(),
            detail: "Classic dark look with hero backdrop".to_owned(),
            user: false,
            colors: ThemeColors::default(),
            layout: ThemeLayout::default(),
            surfaces: ThemeSurfaces::default(),
            typography: ThemeTypography::default(),
            background: ThemeBackground::default(),
        }
    }
}

/// Raw TOML file format for a user theme.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    schema_version: u32,
    name: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    colors: RawColors,
    #[serde(default)]
    layout: RawLayout,
    #[serde(default)]
    surfaces: RawSurfaces,
    #[serde(default)]
    typography: RawTypography,
    #[serde(default)]
    background: RawBackground,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawColors {
    bg: Option<String>,
    surface: Option<String>,
    dock_bg: Option<String>,
    text: Option<String>,
    text_dim: Option<String>,
    accent: Option<String>,
    accent_text: Option<String>,
    focus_ring: Option<String>,
    icon_card_tint: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayout {
    style: Option<LayoutStyle>,
    columns: Option<usize>,
    tile_aspect: Option<TileAspect>,
    tile_style: Option<TileStyle>,
    tile_border_width: Option<f32>,
    tile_border_color: Option<String>,
    tile_dim_unfocused: Option<f32>,
    tile_gap: Option<f32>,
    grid_margin_x: Option<f32>,
    hero: Option<HeroMode>,
    hero_align: Option<HeroAlign>,
    hero_artwork: Option<bool>,
    hero_title_size: Option<f32>,
    hero_title_position: Option<HeroTitlePosition>,
    focus_style: Option<FocusStyle>,
    dock_surface: Option<bool>,
    dock_blur: Option<bool>,
    dock_radius: Option<f32>,
    section_label: Option<String>,
    top_bar_style: Option<TopBarStyle>,
    clock_position: Option<ClockPosition>,
    tile_radius: Option<f32>,
    panel_radius: Option<f32>,
    focus_scale: Option<f32>,
    focus_ring_width: Option<f32>,
    show_labels: Option<LabelMode>,
    sway: Option<bool>,
    glow_strength: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSurfaces {
    panel_style: Option<PanelStyle>,
    panel_opacity: Option<f32>,
    panel_border_width: Option<f32>,
    panel_border_color: Option<String>,
    scrim_opacity: Option<f32>,
    selection_style: Option<SelectionStyle>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTypography {
    font_family: Option<String>,
    clock_size: Option<f32>,
    clock_weight: Option<i32>,
    hero_title_weight: Option<i32>,
    motion_scale: Option<f32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBackground {
    gradient_top: Option<String>,
    gradient_bottom: Option<String>,
    gradient_angle: Option<f32>,
    scrim: Option<BgScrim>,
    scrim_strength: Option<f32>,
    image: Option<String>,
    image_opacity: Option<f32>,
}

/// Built-in themes shipped with RustTV. The first entry is always `"default"`.
pub fn builtin_themes() -> Vec<ThemeSpec> {
    let default = ThemeSpec::default();
    let oled = ThemeSpec {
        id: "oled".to_owned(),
        name: "OLED Pure".to_owned(),
        detail: "True black background for OLED TVs and projectors".to_owned(),
        user: false,
        colors: ThemeColors {
            bg: Rgba::rgb(0x00, 0x00, 0x00),
            surface: Rgba::rgb(0x12, 0x14, 0x1A),
            dock_bg: Rgba {
                r: 0x10,
                g: 0x12,
                b: 0x18,
                a: 0xD8,
            },
            text: Rgba::rgb(0xF2, 0xF3, 0xF5),
            text_dim: Rgba::rgb(0x98, 0x9E, 0xAA),
            accent: Rgba::rgb(0xF5, 0xA5, 0x24),
            accent_text: Rgba::rgb(0x0E, 0x10, 0x14),
            focus_ring: Rgba::rgb(0xE8, 0xEC, 0xF2),
            icon_card_tint: 0.16,
        },
        layout: ThemeLayout {
            glow_strength: 0.75,
            ..ThemeLayout::default()
        },
        surfaces: ThemeSurfaces::default(),
        typography: ThemeTypography::default(),
        background: ThemeBackground::default(),
    };
    let cinema = ThemeSpec {
        id: "cinema".to_owned(),
        name: "Cinema Compact".to_owned(),
        detail: "Compact hero showing two rows of apps at once".to_owned(),
        user: false,
        colors: ThemeColors {
            bg: Rgba::rgb(0x0B, 0x0D, 0x12),
            surface: Rgba::rgb(0x18, 0x1C, 0x24),
            dock_bg: Rgba {
                r: 0x18,
                g: 0x1C,
                b: 0x24,
                a: 0xD8,
            },
            text: Rgba::rgb(0xF5, 0xF4, 0xF0),
            text_dim: Rgba::rgb(0xA6, 0xA2, 0x99),
            accent: Rgba::rgb(0xF5, 0xA5, 0x24),
            accent_text: Rgba::rgb(0x0B, 0x0D, 0x12),
            focus_ring: Rgba::rgb(0xF5, 0xA5, 0x24),
            icon_card_tint: 0.22,
        },
        layout: ThemeLayout {
            hero: HeroMode::Compact,
            tile_radius: 14.0,
            focus_scale: 1.12,
            sway: false,
            glow_strength: 0.85,
            ..ThemeLayout::default()
        },
        surfaces: ThemeSurfaces::default(),
        typography: ThemeTypography::default(),
        background: ThemeBackground::default(),
    };
    let nordic = ThemeSpec {
        id: "nordic".to_owned(),
        name: "Nordic".to_owned(),
        detail: "Cool slate palette with translucent panels".to_owned(),
        user: false,
        colors: ThemeColors {
            bg: Rgba::rgb(0x19, 0x1F, 0x28),
            surface: Rgba::rgb(0x25, 0x2D, 0x3A),
            dock_bg: Rgba {
                r: 0x25,
                g: 0x2D,
                b: 0x3A,
                a: 0xD8,
            },
            text: Rgba::rgb(0xEC, 0xEF, 0xF4),
            text_dim: Rgba::rgb(0x9A, 0xA6, 0xB8),
            accent: Rgba::rgb(0x88, 0xC0, 0xD0),
            accent_text: Rgba::rgb(0x14, 0x19, 0x20),
            focus_ring: Rgba::rgb(0x88, 0xC0, 0xD0),
            icon_card_tint: 0.18,
        },
        layout: ThemeLayout {
            tile_radius: 16.0,
            panel_radius: 20.0,
            ..ThemeLayout::default()
        },
        surfaces: ThemeSurfaces {
            panel_style: PanelStyle::Tinted,
            panel_opacity: 0.90,
            panel_border_width: 1.5,
            panel_border_color: Rgba {
                r: 0x88,
                g: 0xC0,
                b: 0xD0,
                a: 0x38,
            },
            selection_style: SelectionStyle::Bar,
            ..ThemeSurfaces::default()
        },
        typography: ThemeTypography::default(),
        background: ThemeBackground {
            gradient: Some((Rgba::rgb(0x21, 0x29, 0x36), Rgba::rgb(0x14, 0x19, 0x20))),
            ..ThemeBackground::default()
        },
    };
    let grid_fast = ThemeSpec {
        id: "grid-fast".to_owned(),
        name: "Grid Fast".to_owned(),
        detail: "No hero or sway, three rows at once: fastest on weak GPUs".to_owned(),
        user: false,
        colors: ThemeColors {
            icon_card_tint: 0.20,
            ..ThemeColors::default()
        },
        layout: ThemeLayout {
            focus_scale: 1.08,
            ..ThemeLayout::defaults_for(LayoutStyle::Grid)
        },
        surfaces: ThemeSurfaces::default(),
        typography: ThemeTypography {
            motion_scale: 0.5,
            ..ThemeTypography::default()
        },
        background: ThemeBackground::default(),
    };
    vec![default, oled, cinema, nordic, grid_fast]
}

/// Directories searched for user `.toml` themes, in priority order.
pub fn theme_dirs(state_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = state_dir {
        dirs.push(dir.join(THEMES_DIR));
    }
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(config) = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".config")))
    {
        let candidate = config.join("rusttv").join(THEMES_DIR);
        if !dirs.contains(&candidate) {
            dirs.push(candidate);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let system = PathBuf::from("/usr/share/rusttv/themes");
        if !dirs.contains(&system) {
            dirs.push(system);
        }
    }
    dirs
}

/// Loads all built-in themes followed by valid user `.toml` themes found in `dirs`.
pub fn load_all(dirs: &[PathBuf]) -> Vec<ThemeSpec> {
    let mut themes = builtin_themes();
    let mut seen_ids: HashSet<String> = themes.iter().map(|t| t.id.clone()).collect();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let id = format!("user:{stem}");
            if !seen_ids.insert(id.clone()) {
                continue;
            }
            match load_file(&path, id) {
                Ok(theme) => themes.push(theme),
                Err(error) => log::warn!("ignoring theme {}: {error}", path.display()),
            }
        }
    }
    themes
}

/// Parses and validates a single `.toml` theme file.
pub fn load_file(path: &Path, id: String) -> Result<ThemeSpec, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_THEME_BYTES {
        return Err(format!(
            "file is {} bytes (limit {MAX_THEME_BYTES})",
            meta.len()
        ));
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_theme(&text, id, path.parent())
}

/// Parses and validates theme TOML text; `base_dir` is used to resolve an
/// optional background PNG inside the theme's directory.
pub fn parse_theme(text: &str, id: String, base_dir: Option<&Path>) -> Result<ThemeSpec, String> {
    let raw: ThemeFile = toml::from_str(text).map_err(|e| e.to_string())?;
    if raw.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema_version {} (expected {SCHEMA_VERSION})",
            raw.schema_version
        ));
    }
    let name = clean_text(&raw.name, MAX_NAME_LEN);
    if name.is_empty() {
        return Err("theme name cannot be empty".to_owned());
    }
    let desc = raw
        .description
        .as_deref()
        .map(|s| clean_text(s, MAX_DESC_LEN))
        .unwrap_or_default();
    let author = raw
        .author
        .as_deref()
        .map(|s| clean_text(s, MAX_AUTHOR_LEN))
        .unwrap_or_default();
    let detail = match (desc.is_empty(), author.is_empty()) {
        (false, false) => format!("{desc} · by {author}"),
        (false, true) => desc,
        (true, false) => format!("By {author}"),
        (true, true) => "Custom theme".to_owned(),
    };

    let def_colors = ThemeColors::default();
    let parse_opt = |opt: Option<String>, fallback: Rgba| -> Result<Rgba, String> {
        match opt {
            Some(s) => Rgba::parse_hex(&s),
            None => Ok(fallback),
        }
    };
    let bg = parse_opt(raw.colors.bg, def_colors.bg)?;
    let surface = parse_opt(raw.colors.surface, def_colors.surface)?;
    let default_dock = Rgba {
        r: surface.r,
        g: surface.g,
        b: surface.b,
        a: 0xD8,
    };
    let dock_bg = parse_opt(raw.colors.dock_bg, default_dock)?;
    let text_col = parse_opt(raw.colors.text, def_colors.text)?;
    let text_dim = parse_opt(raw.colors.text_dim, def_colors.text_dim)?;
    let accent = parse_opt(raw.colors.accent, def_colors.accent)?;
    let accent_text = parse_opt(raw.colors.accent_text, contrasting_text(accent))?;
    let focus_ring = parse_opt(raw.colors.focus_ring, text_col)?;
    let icon_card_tint = clamp_finite(
        raw.colors.icon_card_tint,
        def_colors.icon_card_tint,
        0.0,
        0.5,
    );

    let style = raw.layout.style.unwrap_or(LayoutStyle::Shelf);
    let def_layout = ThemeLayout::defaults_for(style);
    let columns = raw
        .layout
        .columns
        .map_or(def_layout.columns, |c| c.clamp(1, 8));
    let tile_border_color = parse_opt(raw.layout.tile_border_color, def_layout.tile_border_color)?;
    let layout = ThemeLayout {
        style,
        columns,
        tile_aspect: raw.layout.tile_aspect.unwrap_or(def_layout.tile_aspect),
        tile_style: raw.layout.tile_style.unwrap_or(def_layout.tile_style),
        tile_border_width: clamp_finite(
            raw.layout.tile_border_width,
            def_layout.tile_border_width,
            0.0,
            4.0,
        ),
        tile_border_color,
        tile_dim_unfocused: clamp_finite(
            raw.layout.tile_dim_unfocused,
            def_layout.tile_dim_unfocused,
            0.0,
            0.55,
        ),
        tile_gap: clamp_finite(raw.layout.tile_gap, def_layout.tile_gap, 12.0, 72.0),
        grid_margin_x: clamp_finite(
            raw.layout.grid_margin_x,
            def_layout.grid_margin_x,
            20.0,
            240.0,
        ),
        hero: raw.layout.hero.unwrap_or(def_layout.hero),
        hero_align: raw.layout.hero_align.unwrap_or(def_layout.hero_align),
        hero_artwork: raw.layout.hero_artwork.unwrap_or(def_layout.hero_artwork),
        hero_title_size: clamp_finite(
            raw.layout.hero_title_size,
            def_layout.hero_title_size,
            40.0,
            96.0,
        ),
        hero_title_position: raw
            .layout
            .hero_title_position
            .unwrap_or(def_layout.hero_title_position),
        focus_style: raw.layout.focus_style.unwrap_or(def_layout.focus_style),
        dock_surface: raw.layout.dock_surface.unwrap_or(def_layout.dock_surface),
        dock_blur: raw.layout.dock_blur.unwrap_or(def_layout.dock_blur),
        dock_radius: clamp_finite(raw.layout.dock_radius, def_layout.dock_radius, 0.0, 48.0),
        section_label: raw
            .layout
            .section_label
            .as_deref()
            .map(|s| clean_text(s, 40))
            .unwrap_or(def_layout.section_label),
        top_bar_style: raw.layout.top_bar_style.unwrap_or(def_layout.top_bar_style),
        clock_position: raw
            .layout
            .clock_position
            .unwrap_or(def_layout.clock_position),
        tile_radius: clamp_finite(raw.layout.tile_radius, def_layout.tile_radius, 0.0, 32.0),
        panel_radius: clamp_finite(raw.layout.panel_radius, def_layout.panel_radius, 0.0, 36.0),
        focus_scale: clamp_finite(raw.layout.focus_scale, def_layout.focus_scale, 1.0, 1.25),
        focus_ring_width: clamp_finite(
            raw.layout.focus_ring_width,
            def_layout.focus_ring_width,
            0.0,
            8.0,
        ),
        show_labels: raw.layout.show_labels.unwrap_or(def_layout.show_labels),
        sway: raw.layout.sway.unwrap_or(def_layout.sway),
        glow_strength: clamp_finite(raw.layout.glow_strength, def_layout.glow_strength, 0.0, 1.5),
    };

    let def_surfaces = ThemeSurfaces::default();
    let panel_style = raw.surfaces.panel_style.unwrap_or(def_surfaces.panel_style);
    let default_panel_opacity = match panel_style {
        PanelStyle::Tinted => 0.86,
        _ => def_surfaces.panel_opacity,
    };
    let default_panel_border = match panel_style {
        PanelStyle::Outline | PanelStyle::Elevated | PanelStyle::Tinted => 1.5,
        PanelStyle::Solid => 0.0,
    };
    let panel_border_color = parse_opt(
        raw.surfaces.panel_border_color,
        def_surfaces.panel_border_color,
    )?;
    let surfaces = ThemeSurfaces {
        panel_style,
        panel_opacity: clamp_finite(raw.surfaces.panel_opacity, default_panel_opacity, 0.55, 1.0),
        panel_border_width: clamp_finite(
            raw.surfaces.panel_border_width,
            default_panel_border,
            0.0,
            4.0,
        ),
        panel_border_color,
        scrim_opacity: clamp_finite(
            raw.surfaces.scrim_opacity,
            def_surfaces.scrim_opacity,
            0.30,
            0.95,
        ),
        selection_style: raw
            .surfaces
            .selection_style
            .unwrap_or(def_surfaces.selection_style),
    };

    let def_typo = ThemeTypography::defaults_for(style);
    let font_family = raw
        .typography
        .font_family
        .as_deref()
        .map(|s| clean_text(s, 60))
        .unwrap_or(def_typo.font_family);
    let typography = ThemeTypography {
        font_family,
        clock_size: clamp_finite(raw.typography.clock_size, def_typo.clock_size, 24.0, 140.0),
        clock_weight: raw
            .typography
            .clock_weight
            .unwrap_or(def_typo.clock_weight)
            .clamp(200, 700),
        hero_title_weight: raw
            .typography
            .hero_title_weight
            .unwrap_or(def_typo.hero_title_weight)
            .clamp(300, 800),
        motion_scale: clamp_finite(raw.typography.motion_scale, def_typo.motion_scale, 0.0, 1.5),
    };

    let gradient = match (raw.background.gradient_top, raw.background.gradient_bottom) {
        (Some(top), Some(bottom)) => Some((Rgba::parse_hex(&top)?, Rgba::parse_hex(&bottom)?)),
        (Some(top), None) => Some((Rgba::parse_hex(&top)?, bg)),
        (None, Some(bottom)) => Some((bg, Rgba::parse_hex(&bottom)?)),
        (None, None) => None,
    };
    let gradient_angle = clamp_finite(raw.background.gradient_angle, 180.0, 0.0, 360.0);
    let scrim = raw.background.scrim.unwrap_or(BgScrim::None);
    let scrim_strength = clamp_finite(raw.background.scrim_strength, 0.55, 0.0, 0.9);
    let image_path = match raw.background.image {
        Some(filename) => Some(resolve_theme_image(&filename, base_dir)?),
        None => None,
    };
    let image_opacity = clamp_finite(raw.background.image_opacity, 0.35, 0.0, 1.0);

    Ok(ThemeSpec {
        id,
        name,
        detail,
        user: true,
        colors: ThemeColors {
            bg,
            surface,
            dock_bg,
            text: text_col,
            text_dim,
            accent,
            accent_text,
            focus_ring,
            icon_card_tint,
        },
        layout,
        surfaces,
        typography,
        background: ThemeBackground {
            gradient,
            gradient_angle,
            scrim,
            scrim_strength,
            image_path,
            image_opacity,
        },
    })
}

/// Picks dark or light text with good contrast against `bg`.
fn contrasting_text(bg: Rgba) -> Rgba {
    let luma = 0.299 * f32::from(bg.r) + 0.587 * f32::from(bg.g) + 0.114 * f32::from(bg.b);
    if luma >= 145.0 {
        Rgba::rgb(0x0E, 0x10, 0x14)
    } else {
        Rgba::rgb(0xF5, 0xF6, 0xF8)
    }
}

/// Validates that `filename` is a plain single-segment PNG or SVG filename
/// inside `base_dir` (never `..`, absolute paths, or subdirectories).
fn resolve_theme_image(filename: &str, base_dir: Option<&Path>) -> Result<PathBuf, String> {
    let rel = Path::new(filename);
    let mut components = rel.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => {
            if !Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png") || e.eq_ignore_ascii_case("svg"))
            {
                return Err(format!(
                    "background image {filename:?} must be a .png or .svg file"
                ));
            }
            let dir =
                base_dir.ok_or_else(|| "no theme directory for background image".to_owned())?;
            Ok(dir.join(name))
        }
        _ => Err(format!(
            "background image {filename:?} must be a filename in the themes directory"
        )),
    }
}

/// Loads a theme's optional PNG or SVG background wallpaper with size and
/// dimension caps (cached once on theme load; zero per-frame decoding cost).
#[cfg(all(target_os = "linux", feature = "linux"))]
pub fn load_background_image(path: &Path) -> Result<Bitmap, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "background image is {} bytes (limit {MAX_IMAGE_BYTES})",
            meta.len()
        ));
    }
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        return load_background_svg(path);
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let info = reader.info();
    if info.width == 0
        || info.height == 0
        || info.width > MAX_IMAGE_SIZE.0
        || info.height > MAX_IMAGE_SIZE.1
    {
        return Err(format!(
            "background image {}x{} exceeds {}x{}",
            info.width, info.height, MAX_IMAGE_SIZE.0, MAX_IMAGE_SIZE.1
        ));
    }
    let size = reader.output_buffer_size().ok_or("image too large")?;
    let mut pixels = vec![0; size];
    let frame = reader.next_frame(&mut pixels).map_err(|e| e.to_string())?;
    pixels.truncate(frame.buffer_size());
    let rgba = match frame.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colours left after expansion".into()),
    };
    Bitmap::new(frame.width, frame.height, rgba).ok_or_else(|| "wrong pixel count".to_owned())
}

/// Renders a vector SVG wallpaper once at 1280×720 for crisp GPU-cached scaling.
#[cfg(all(target_os = "linux", feature = "linux"))]
fn load_background_svg(path: &Path) -> Result<Bitmap, String> {
    use resvg::{tiny_skia, usvg};

    let data = fs::read(path).map_err(|e| e.to_string())?;
    let tree =
        usvg::Tree::from_data(&data, &usvg::Options::default()).map_err(|e| e.to_string())?;
    let size = tree.size();
    if size.width() <= 0.0 || size.height() <= 0.0 {
        return Err("empty SVG wallpaper".to_owned());
    }
    let (target_w, target_h) = (1280u32, 720u32);
    let sx = target_w as f32 / size.width();
    let sy = target_h as f32 / size.height();
    let mut pixmap = tiny_skia::Pixmap::new(target_w, target_h).ok_or("empty pixmap")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let c = pixel.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Bitmap::new(target_w, target_h, rgba).ok_or_else(|| "wrong pixel count".to_owned())
}

/// Pre-composited opaque RGB theme backdrop (solid `bg` + optional angled
/// gradient + optional PNG/SVG wallpaper + optional directional scrim).
///
/// Baking all static background layers once on theme load replaces up to four
/// full-screen alpha-blended shader passes per frame with a single opaque GPU
/// texture blit — essential for locked 60 fps on single-core Mali-G31 GPUs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakedBackdrop {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    /// Optional downscaled Gaussian-blurred copy for optical frosted-glass
    /// dock surfaces (`dock_blur = true`), computed once on theme load.
    pub blurred: Option<(u32, u32, Vec<u8>)>,
}

/// Composites a theme's static background (gradient, wallpaper image, and
/// directional scrim) into one opaque RGB bitmap. Returns `None` when the theme
/// uses only the flat `colors.bg` colour.
pub fn bake_backdrop(spec: &ThemeSpec, right_to_left: bool) -> Option<BakedBackdrop> {
    let wallpaper =
        spec.background
            .image_path
            .as_deref()
            .and_then(|path| match load_background_image(path) {
                Ok(bmp) => Some(bmp),
                Err(error) => {
                    log::warn!("theme background {} skipped: {error}", path.display());
                    None
                }
            });
    if wallpaper.is_none()
        && spec.background.gradient.is_none()
        && spec.background.scrim == BgScrim::None
    {
        return None;
    }

    let (width, height) = if wallpaper.is_some() {
        (1280u32, 720u32)
    } else {
        (320u32, 180u32)
    };
    let w_f = width as f32;
    let h_f = height as f32;
    let bg = [
        f32::from(spec.colors.bg.r),
        f32::from(spec.colors.bg.g),
        f32::from(spec.colors.bg.b),
    ];
    let grad = spec.background.gradient.map(|(top, bottom)| {
        let rad = spec.background.gradient_angle.to_radians();
        let dx = rad.sin();
        let dy = -rad.cos();
        let span = (dx.abs() + dy.abs()).max(1e-4);
        (
            [f32::from(top.r), f32::from(top.g), f32::from(top.b)],
            [
                f32::from(bottom.r),
                f32::from(bottom.g),
                f32::from(bottom.b),
            ],
            dx,
            dy,
            span,
        )
    });

    // Cover-crop mapping if a wallpaper bitmap is present.
    let wp_map = wallpaper.as_ref().map(|bmp| {
        let sw = bmp.width() as f32;
        let sh = bmp.height() as f32;
        let scale = (w_f / sw).max(h_f / sh);
        let vis_w = w_f / scale;
        let vis_h = h_f / scale;
        let ox = (sw - vis_w) * 0.5;
        let oy = (sh - vis_h) * 0.5;
        (bmp, ox, oy, vis_w, vis_h, spec.background.image_opacity)
    });

    let scrim = spec.background.scrim;
    let scrim_strength = spec.background.scrim_strength;

    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let yn = (y as f32 + 0.5) / h_f;
        let v = yn - 0.5;
        for x in 0..width {
            let xn = (x as f32 + 0.5) / w_f;
            let u = xn - 0.5;

            let mut c = if let Some((top, bottom, dx, dy, span)) = grad {
                let t = ((u * dx + v * dy) / span + 0.5).clamp(0.0, 1.0);
                [
                    top[0] + (bottom[0] - top[0]) * t,
                    top[1] + (bottom[1] - top[1]) * t,
                    top[2] + (bottom[2] - top[2]) * t,
                ]
            } else {
                bg
            };

            if let Some((bmp, ox, oy, vis_w, vis_h, opacity)) = wp_map {
                let sx = (ox + xn * vis_w).clamp(0.0, (bmp.width() - 1) as f32);
                let sy = (oy + yn * vis_h).clamp(0.0, (bmp.height() - 1) as f32);
                let ix = sx as u32;
                let iy = sy as u32;
                let idx = ((iy * bmp.width() + ix) * 4) as usize;
                let px = bmp.rgba();
                if idx + 3 < px.len() {
                    let a = (f32::from(px[idx + 3]) / 255.0) * opacity;
                    c[0] += (f32::from(px[idx]) - c[0]) * a;
                    c[1] += (f32::from(px[idx + 1]) - c[1]) * a;
                    c[2] += (f32::from(px[idx + 2]) - c[2]) * a;
                }
            }

            let scrim_a = match scrim {
                BgScrim::None => 0.0,
                BgScrim::Bottom => {
                    if yn >= 0.35 {
                        scrim_strength * ((yn - 0.35) / 0.65).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                }
                BgScrim::Left => {
                    let xs = if right_to_left { 1.0 - xn } else { xn };
                    if xs <= 0.62 {
                        scrim_strength * (1.0 - xs / 0.62).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                }
                BgScrim::Vignette => {
                    if yn <= 0.35 {
                        scrim_strength * 0.8 * (1.0 - yn / 0.35)
                    } else if yn >= 0.60 {
                        scrim_strength * ((yn - 0.60) / 0.40)
                    } else {
                        0.0
                    }
                }
            };
            if scrim_a > 0.0 {
                c[0] += (bg[0] - c[0]) * scrim_a;
                c[1] += (bg[1] - c[1]) * scrim_a;
                c[2] += (bg[2] - c[2]) * scrim_a;
            }

            rgb.push(c[0].round().clamp(0.0, 255.0) as u8);
            rgb.push(c[1].round().clamp(0.0, 255.0) as u8);
            rgb.push(c[2].round().clamp(0.0, 255.0) as u8);
        }
    }
    let blurred = spec
        .layout
        .dock_blur
        .then(|| blur_backdrop(&rgb, width, height));
    Some(BakedBackdrop {
        width,
        height,
        rgb,
        blurred,
    })
}

/// Downsamples an RGB backdrop to 320×180 and applies a 3-pass separable box
/// blur (approximating a ~24px Gaussian blur at 1080p) for zero-cost frosted
/// glass docks.
fn blur_backdrop(src: &[u8], w: u32, h: u32) -> (u32, u32, Vec<u8>) {
    let bw = 320usize;
    let bh = 180usize;
    let sw = w as usize;
    let sh = h as usize;
    let mut cur = vec![0u8; bw * bh * 3];
    for y in 0..bh {
        let sy0 = y * sh / bh;
        let sy1 = ((y + 1) * sh / bh).max(sy0 + 1).min(sh);
        for x in 0..bw {
            let sx0 = x * sw / bw;
            let sx1 = ((x + 1) * sw / bw).max(sx0 + 1).min(sw);
            let mut sum = [0u32; 3];
            let mut count = 0u32;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let i = (sy * sw + sx) * 3;
                    sum[0] += u32::from(src[i]);
                    sum[1] += u32::from(src[i + 1]);
                    sum[2] += u32::from(src[i + 2]);
                    count += 1;
                }
            }
            let dst = (y * bw + x) * 3;
            cur[dst] = (sum[0] / count) as u8;
            cur[dst + 1] = (sum[1] / count) as u8;
            cur[dst + 2] = (sum[2] / count) as u8;
        }
    }

    let radius = 9usize;
    let mut tmp = vec![0u8; bw * bh * 3];
    for _ in 0..3 {
        // Horizontal pass
        for y in 0..bh {
            let row = y * bw;
            for x in 0..bw {
                let x0 = x.saturating_sub(radius);
                let x1 = (x + radius + 1).min(bw);
                let n = (x1 - x0) as u32;
                let mut sum = [0u32; 3];
                for sx in x0..x1 {
                    let i = (row + sx) * 3;
                    sum[0] += u32::from(cur[i]);
                    sum[1] += u32::from(cur[i + 1]);
                    sum[2] += u32::from(cur[i + 2]);
                }
                let d = (row + x) * 3;
                tmp[d] = (sum[0] / n) as u8;
                tmp[d + 1] = (sum[1] / n) as u8;
                tmp[d + 2] = (sum[2] / n) as u8;
            }
        }
        // Vertical pass
        for y in 0..bh {
            let y0 = y.saturating_sub(radius);
            let y1 = (y + radius + 1).min(bh);
            let n = (y1 - y0) as u32;
            for x in 0..bw {
                let mut sum = [0u32; 3];
                for sy in y0..y1 {
                    let i = (sy * bw + x) * 3;
                    sum[0] += u32::from(tmp[i]);
                    sum[1] += u32::from(tmp[i + 1]);
                    sum[2] += u32::from(tmp[i + 2]);
                }
                let d = (y * bw + x) * 3;
                cur[d] = (sum[0] / n) as u8;
                cur[d + 1] = (sum[1] / n) as u8;
                cur[d + 2] = (sum[2] / n) as u8;
            }
        }
    }
    (bw as u32, bh as u32, cur)
}

#[cfg(not(all(target_os = "linux", feature = "linux")))]
pub fn load_background_image(_path: &Path) -> Result<Bitmap, String> {
    Err("background PNG decoding is enabled on Linux builds".to_owned())
}

/// Reads the saved theme ID from `dir/theme`, or `"default"`.
/// `RUSTTV_THEME` overrides the saved ID when set and non-empty.
pub fn load_selected(dir: Option<&Path>) -> String {
    if let Ok(env_id) = std::env::var("RUSTTV_THEME") {
        let trimmed = env_id.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    let Some(dir) = dir else {
        return "default".to_owned();
    };
    fs::read_to_string(dir.join(THEME_FILE))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_owned())
}

/// Persists the active theme ID atomically to `dir/theme`.
pub fn save_selected(dir: &Path, id: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(THEME_FILE);
    let tmp = dir.join(format!("{THEME_FILE}.new"));
    {
        let mut file = fs::File::create(&tmp)?;
        writeln!(file, "{id}")?;
        file.sync_all()?;
    }
    fs::rename(&tmp, &path)
}

/// Mixes `base` with `tint` by `amount` (0..=1) for subtly tinted icon cards.
pub fn mix_rgb(base: Rgba, tint: [u8; 3], amount: f32) -> Rgba {
    let a = amount.clamp(0.0, 1.0);
    let blend = |b: u8, t: u8| -> u8 {
        (f32::from(b) * (1.0 - a) + f32::from(t) * a)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba {
        r: blend(base.r, tint[0]),
        g: blend(base.g, tint[1]),
        b: blend(base.b, tint[2]),
        a: base.a,
    }
}

fn clamp_finite(value: Option<f32>, default: f32, min: f32, max: f32) -> f32 {
    value
        .filter(|v| v.is_finite())
        .map_or(default, |v| v.clamp(min, max))
}

fn clean_text(text: &str, max_chars: usize) -> String {
    text.trim()
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{061C}'
                        | '\u{200E}'
                        | '\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2066}'..='\u{2069}'
                )
        })
        .take(max_chars)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colours() {
        assert_eq!(Rgba::parse_hex("#fff").unwrap(), Rgba::rgb(255, 255, 255));
        assert_eq!(
            Rgba::parse_hex("#0E1014").unwrap(),
            Rgba::rgb(0x0E, 0x10, 0x14)
        );
        assert_eq!(
            Rgba::parse_hex("#11223380").unwrap(),
            Rgba {
                r: 0x11,
                g: 0x22,
                b: 0x33,
                a: 0x80
            }
        );
        assert!(Rgba::parse_hex("0E1014").is_err());
        assert!(Rgba::parse_hex("#12345").is_err());
        assert!(Rgba::parse_hex("#GGGGGG").is_err());
    }

    #[test]
    fn parses_valid_user_theme_and_clamps_ranges() {
        let toml = r##"
schema_version = 1
name = "Sunset\u202E"
author = "Alex"
description = "Warm projector theme"

[colors]
bg = "#120E14"
surface = "#241B28"
accent = "#FF6B4A"
icon_card_tint = 9.0

[layout]
hero = "compact"
tile_radius = 999.0
focus_scale = 2.5
show_labels = "always"
sway = false
glow_strength = -5.0

[background]
gradient_top = "#26162A"
gradient_bottom = "#0C090E"
"##;
        let theme = parse_theme(toml, "user:sunset".to_owned(), None).unwrap();
        assert_eq!(theme.name, "Sunset");
        assert_eq!(theme.detail, "Warm projector theme · by Alex");
        assert_eq!(theme.colors.bg, Rgba::rgb(0x12, 0x0E, 0x14));
        assert_eq!(theme.colors.accent, Rgba::rgb(0xFF, 0x6B, 0x4A));
        assert_eq!(theme.colors.focus_ring, theme.colors.text);
        assert!((theme.colors.icon_card_tint - 0.5).abs() < f32::EPSILON);
        assert_eq!(theme.layout.hero, HeroMode::Compact);
        assert!((theme.layout.tile_radius - 32.0).abs() < f32::EPSILON);
        assert!((theme.layout.focus_scale - 1.25).abs() < f32::EPSILON);
        assert_eq!(theme.layout.show_labels, LabelMode::Always);
        assert!(!theme.layout.sway);
        assert!((theme.layout.glow_strength - 0.0).abs() < f32::EPSILON);
        assert_eq!(
            theme.background.gradient,
            Some((Rgba::rgb(0x26, 0x16, 0x2A), Rgba::rgb(0x0C, 0x09, 0x0E)))
        );
    }

    #[test]
    fn rejects_bad_schema_unknown_fields_and_path_traversal() {
        assert!(parse_theme("schema_version = 2\nname = \"X\"", "u".into(), None).is_err());
        assert!(
            parse_theme(
                "schema_version = 1\nname = \"X\"\nextra = true",
                "u".into(),
                None
            )
            .is_err()
        );
        let traversal = "schema_version = 1\nname = \"X\"\n[background]\nimage = \"../secret.png\"";
        assert!(parse_theme(traversal, "u".into(), Some(Path::new("/tmp"))).is_err());
        let non_png = "schema_version = 1\nname = \"X\"\n[background]\nimage = \"bg.jpg\"";
        assert!(parse_theme(non_png, "u".into(), Some(Path::new("/tmp"))).is_err());
    }

    #[test]
    fn discovers_valid_user_themes_and_skips_broken_ones() {
        let dir = std::env::temp_dir().join(format!("rusttv-themes-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("ocean.toml"),
            "schema_version = 1\nname = \"Ocean\"\n[colors]\nbg = \"#081420\"\n",
        )
        .unwrap();
        fs::write(dir.join("broken.toml"), "not valid toml = = =").unwrap();

        let all = load_all(std::slice::from_ref(&dir));
        assert!(all.iter().any(|t| t.id == "default"));
        let ocean = all.iter().find(|t| t.id == "user:ocean").expect("ocean");
        assert_eq!(ocean.name, "Ocean");
        assert!(ocean.user);
        assert!(!all.iter().any(|t| t.id == "user:broken"));

        save_selected(&dir, "user:ocean").unwrap();
        assert_eq!(load_selected(Some(&dir)), "user:ocean");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_deep_skin_sections_and_defaults() {
        let toml = r##"
schema_version = 1
name = "Deep Skin"

[colors]
accent = "#183058"

[layout]
style = "blade"
tile_style = "outline"
tile_border_width = 2.0
tile_border_color = "#FFFFFF33"
tile_dim_unfocused = 0.25
focus_style = "fill"

[surfaces]
panel_style = "tinted"
panel_opacity = 0.82
panel_border_width = 1.5
panel_border_color = "#54B8F044"
scrim_opacity = 0.72
selection_style = "accent"

[typography]
font_family = "DejaVu Sans"
clock_size = 96.0
clock_weight = 200
hero_title_weight = 600
motion_scale = 0.75

[background]
gradient_top = "#122033"
gradient_bottom = "#05070B"
gradient_angle = 135.0
scrim = "left"
scrim_strength = 0.65
image = "backdrop.svg"
image_opacity = 0.50
"##;
        let theme = parse_theme(toml, "user:deep".into(), Some(Path::new("/tmp"))).unwrap();
        // Dark accent auto-selects light contrasting text.
        assert_eq!(theme.colors.accent_text, Rgba::rgb(0xF5, 0xF6, 0xF8));
        assert_eq!(theme.layout.style, LayoutStyle::Blade);
        assert_eq!(theme.layout.tile_style, TileStyle::Outline);
        assert_eq!(theme.layout.focus_style, FocusStyle::Fill);
        assert!((theme.layout.tile_dim_unfocused - 0.25).abs() < f32::EPSILON);
        assert_eq!(theme.surfaces.panel_style, PanelStyle::Tinted);
        assert!((theme.surfaces.panel_opacity - 0.82).abs() < f32::EPSILON);
        assert_eq!(theme.surfaces.selection_style, SelectionStyle::Accent);
        assert_eq!(theme.typography.font_family, "DejaVu Sans");
        assert!((theme.typography.clock_size - 96.0).abs() < f32::EPSILON);
        assert_eq!(theme.typography.clock_weight, 200);
        assert!((theme.typography.motion_scale - 0.75).abs() < f32::EPSILON);
        assert!((theme.background.gradient_angle - 135.0).abs() < f32::EPSILON);
        assert_eq!(theme.background.scrim, BgScrim::Left);
        assert_eq!(
            theme.background.image_path.as_deref(),
            Some(Path::new("/tmp/backdrop.svg"))
        );

        let zen = ThemeLayout::defaults_for(LayoutStyle::Zen);
        assert_eq!(zen.clock_position, ClockPosition::CenterHero);
        assert_eq!(zen.focus_style, FocusStyle::Underline);
        assert_eq!(zen.hero, HeroMode::Off);
        assert_eq!(
            contrasting_text(Rgba::rgb(0xFF, 0xE0, 0x80)),
            Rgba::rgb(0x0E, 0x10, 0x14)
        );
    }

    #[test]
    fn all_shipped_reference_themes_parse_cleanly() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../themes");
        let entries: Vec<PathBuf> = fs::read_dir(&dir)
            .expect("themes/ directory")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        assert!(
            entries.len() >= 8,
            "expected at least 8 reference themes in {}",
            dir.display()
        );
        for path in entries {
            let stem = path.file_stem().and_then(|s| s.to_str()).expect("stem");
            let loaded = load_file(&path, format!("user:{stem}"))
                .unwrap_or_else(|err| panic!("{} failed: {err}", path.display()));
            assert!(!loaded.name.is_empty());
            #[cfg(all(target_os = "linux", feature = "linux"))]
            if let Some(img) = loaded.background.image_path.as_deref() {
                let bmp = load_background_image(img)
                    .unwrap_or_else(|err| panic!("{} image failed: {err}", img.display()));
                assert!(bmp.width() > 0 && bmp.height() > 0);
                let baked = bake_backdrop(&loaded, false).expect("backdrop should bake");
                assert_eq!(baked.width, 1280);
                assert_eq!(baked.height, 720);
                assert_eq!(baked.rgb.len(), (1280 * 720 * 3) as usize);
            }
            if loaded.background.gradient.is_some() && loaded.background.image_path.is_none() {
                let baked = bake_backdrop(&loaded, false).expect("gradient backdrop should bake");
                assert_eq!(baked.width, 320);
                assert_eq!(baked.height, 180);
                assert_eq!(baked.rgb.len(), (320 * 180 * 3) as usize);
            }
        }
        assert!(bake_backdrop(&ThemeSpec::default(), false).is_none());
    }
}

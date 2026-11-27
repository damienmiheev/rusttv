//! Level 3 programmable skin runtime: runtime `.slint` UI (`slint-interpreter`)
//! paired with sandboxed Rhai scripting (`rhai`).
//!
//! Gives skin authors HTML + CSS + JS-level power — arbitrary vector shapes,
//! custom layouts, animations, focus choreography, state machines, custom
//! keybindings, and scripted widgets — rendered natively on Skia OpenGL ES at
//! 60 fps with strict memory, path, and instruction-budget sandboxing.
//!
//! A programmable skin lives in a directory under `themes/<name>/`:
//! - `skin.toml` (`schema_version = 2`, metadata + `entry` + optional `script`)
//! - `skin.slint` (exports `component SkinHome inherits Window` and imports
//!   `{ RustTV, SkinTile, SkinAction } from "@rusttv/skin.slint"`)
//! - `skin.rhai` (optional sandboxed script with `on_init`, `on_focus`,
//!   `on_key`, `on_action`, `on_tick` hooks)
//! - optional `.svg` / `.png` assets inside the same directory.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

use serde::Deserialize;
use slint::{ComponentHandle as _, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use slint_interpreter::{
    Compiler, ComponentInstance, DiagnosticLevel, Struct as InterpStruct, Value as InterpValue,
};

/// Current programmable skin manifest schema version.
pub const SKIN_SCHEMA_VERSION: u32 = 2;

/// Maximum size of `skin.toml`, `.slint`, or `.rhai` source files (256 KiB).
const MAX_SOURCE_BYTES: u64 = 256 * 1024;

/// Maximum Rhai operations per hook invocation (prevents infinite loops).
const MAX_SCRIPT_OPS: u64 = 50_000;

/// Maximum Rhai call stack depth.
const MAX_SCRIPT_CALL_DEPTH: usize = 16;

/// Maximum Rhai string length in bytes.
const MAX_SCRIPT_STRING: usize = 4096;

/// Maximum Rhai array/map entries.
const MAX_SCRIPT_COLLECTION: usize = 256;

/// Number of string / int / float / image reactive state slots shared between Rhai and Slint.
pub const STR_SLOTS: usize = 8;
pub const INT_SLOTS: usize = 8;
pub const FLOAT_SLOTS: usize = 4;
pub const IMG_SLOTS: usize = 4;

/// Maximum number of in-memory image handles a skin script may hold at once.
const MAX_SCRIPT_IMAGES: usize = 128;

/// Virtual `@rusttv/skin.slint` module imported by programmable skins.
pub const SKIN_API_SLINT: &str = r#"
export struct SkinTile {
    id: string,
    label: string,
    image: image,
    mono-image: image,
    kind: int,
    running: bool,
    tint: color,
}

export struct SkinAction {
    code: int,
    label: string,
}

export global RustTV {
    // Live launcher state pushed from Rust.
    in-out property <[SkinTile]> tiles;
    in-out property <int> focused-index: 0;
    in-out property <string> clock-text: "";
    in-out property <string> greeting-text: "";
    in-out property <string> date-text: "";
    in-out property <string> hero-title: "";
    in-out property <color> hero-color: #1C1F26;
    in-out property <image> hero-image;
    in-out property <[SkinAction]> quick-actions;
    in-out property <int> quick-selected: -1;
    in-out property <int> mode: 0;
    in-out property <bool> rtl: false;
    in-out property <bool> reduced-effects: false;
    in-out property <string> toast: "";
    // Native in-skin long-press / context menu state.
    in-out property <bool> menu-open: false;
    in-out property <string> menu-title: "";
    in-out property <[string]> menu-items;
    in-out property <int> menu-selected: 0;

    // Reactive state slots controlled by optional skin.rhai scripts.
    in-out property <string> s0: "";
    in-out property <string> s1: "";
    in-out property <string> s2: "";
    in-out property <string> s3: "";
    in-out property <string> s4: "";
    in-out property <string> s5: "";
    in-out property <string> s6: "";
    in-out property <string> s7: "";
    in-out property <int> i0: 0;
    in-out property <int> i1: 0;
    in-out property <int> i2: 0;
    in-out property <int> i3: 0;
    in-out property <int> i4: 0;
    in-out property <int> i5: 0;
    in-out property <int> i6: 0;
    in-out property <int> i7: 0;
    in-out property <float> f0: 0.0;
    in-out property <float> f1: 0.0;
    in-out property <float> f2: 0.0;
    in-out property <float> f3: 0.0;
    in-out property <image> img0;
    in-out property <image> img1;
    in-out property <image> img2;
    in-out property <image> img3;

    // Callbacks from skin UI -> Rhai script and RustTV host.
    callback key(string, bool, bool) -> bool;
    callback tile-clicked(int, bool);
    callback tile-hovered(int);
    callback quick-clicked(int);
    callback quick-hovered(int);
    callback scrolled(length);
    callback action(string, string);
    callback menu-clicked(int);
}
"#;

/// Raw `skin.toml` manifest format.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkinManifestFile {
    schema_version: u32,
    name: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default = "default_entry")]
    entry: String,
    #[serde(default)]
    script: Option<String>,
}

fn default_entry() -> String {
    "skin.slint".to_owned()
}

/// Validated metadata for a programmable `.slint` (+ optional `.rhai`) skin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinManifest {
    /// Theme/skin identifier: `"skin:<dir-stem>"`.
    pub id: String,
    pub name: String,
    pub detail: String,
    pub dir: PathBuf,
    pub entry_path: PathBuf,
    pub script_path: Option<PathBuf>,
}

/// Tile data pushed into the `@rusttv/skin.slint` `RustTV.tiles` model.
#[derive(Debug, Clone)]
pub struct SkinTileData {
    pub id: String,
    pub label: String,
    pub image: slint::Image,
    pub kind: i32,
    pub running: bool,
    pub tint: slint::Color,
}

/// Commands queued by a skin's Rhai script for the RustTV host to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostCommand {
    Launch(i32),
    Focus(i32),
    OpenSettings,
}

/// Shared mutable state accessed by sandboxed Rhai host functions.
#[derive(Debug, Default)]
struct ScriptBridge {
    strings: [String; STR_SLOTS],
    ints: [i32; INT_SLOTS],
    floats: [f32; FLOAT_SLOTS],
    img_slots: [slint::Image; IMG_SLOTS],
    dirty_str: [bool; STR_SLOTS],
    dirty_int: [bool; INT_SLOTS],
    dirty_float: [bool; FLOAT_SLOTS],
    dirty_img: [bool; IMG_SLOTS],
    tile_ids: Vec<String>,
    tile_labels: Vec<String>,
    tile_images: Vec<slint::Image>,
    tile_overrides: HashMap<usize, slint::Image>,
    image_pool: Vec<slint::Image>,
    skin_dir: PathBuf,
    commands: Vec<HostCommand>,
    prefs: BTreeMap<String, String>,
    prefs_path: Option<PathBuf>,
}

fn parse_hex_rgb(hex: &str) -> (u8, u8, u8) {
    let s = hex.trim().trim_start_matches('#');
    if s.len() >= 6
        && let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&s[0..2], 16),
            u8::from_str_radix(&s[2..4], 16),
            u8::from_str_radix(&s[4..6], 16),
        )
    {
        return (r, g, b);
    }
    (244, 244, 241)
}

fn tint_image(src: &slint::Image, rgb: (u8, u8, u8), strength: f32) -> slint::Image {
    let Some(rgba) = src.to_rgba8() else {
        return src.clone();
    };
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return src.clone();
    }
    let t = strength.clamp(0.0, 1.0);
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    for (s, d) in rgba.as_slice().iter().zip(out.make_mut_slice().iter_mut()) {
        *d = Rgba8Pixel {
            r: (f32::from(s.r) * (1.0 - t) + f32::from(rgb.0) * t).round() as u8,
            g: (f32::from(s.g) * (1.0 - t) + f32::from(rgb.1) * t).round() as u8,
            b: (f32::from(s.b) * (1.0 - t) + f32::from(rgb.2) * t).round() as u8,
            a: s.a,
        };
    }
    slint::Image::from_rgba8(out)
}

fn desaturate_image(src: &slint::Image, amount: f32) -> slint::Image {
    let Some(rgba) = src.to_rgba8() else {
        return src.clone();
    };
    let (w, h) = (rgba.width(), rgba.height());
    if w == 0 || h == 0 {
        return src.clone();
    }
    let t = amount.clamp(0.0, 1.0);
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w, h);
    for (s, d) in rgba.as_slice().iter().zip(out.make_mut_slice().iter_mut()) {
        let luma = f32::from(s.r) * 0.299 + f32::from(s.g) * 0.587 + f32::from(s.b) * 0.114;
        *d = Rgba8Pixel {
            r: (f32::from(s.r) * (1.0 - t) + luma * t).round() as u8,
            g: (f32::from(s.g) * (1.0 - t) + luma * t).round() as u8,
            b: (f32::from(s.b) * (1.0 - t) + luma * t).round() as u8,
            a: s.a,
        };
    }
    slint::Image::from_rgba8(out)
}

fn blur_image(src: &slint::Image, radius: i64) -> slint::Image {
    let Some(rgba) = src.to_rgba8() else {
        return src.clone();
    };
    let (sw, sh) = (rgba.width() as usize, rgba.height() as usize);
    if sw == 0 || sh == 0 {
        return src.clone();
    }
    // Downscale wide backdrops to <= 480px so 3-pass blur is instant on Mali-G31.
    let (w, h) = if sw > 480 {
        (480, (sh * 480 / sw).max(1))
    } else {
        (sw, sh)
    };
    let sp = rgba.as_slice();
    let mut buf = vec![[0u8; 4]; w * h];
    for y in 0..h {
        let sy = (y * sh / h).min(sh - 1);
        for x in 0..w {
            let sx = (x * sw / w).min(sw - 1);
            let p = sp[sy * sw + sx];
            buf[y * w + x] = [p.r, p.g, p.b, p.a];
        }
    }
    let r = radius.clamp(1, 24) as usize;
    let mut tmp = vec![[0u8; 4]; w * h];
    for _ in 0..3 {
        // Horizontal pass
        for y in 0..h {
            for x in 0..w {
                let x0 = x.saturating_sub(r);
                let x1 = (x + r).min(w - 1);
                let count = (x1 - x0 + 1) as u32;
                let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
                for xi in x0..=x1 {
                    let p = buf[y * w + xi];
                    sr += u32::from(p[0]);
                    sg += u32::from(p[1]);
                    sb += u32::from(p[2]);
                    sa += u32::from(p[3]);
                }
                tmp[y * w + x] = [
                    (sr / count) as u8,
                    (sg / count) as u8,
                    (sb / count) as u8,
                    (sa / count) as u8,
                ];
            }
        }
        // Vertical pass
        for x in 0..w {
            for y in 0..h {
                let y0 = y.saturating_sub(r);
                let y1 = (y + r).min(h - 1);
                let count = (y1 - y0 + 1) as u32;
                let (mut sr, mut sg, mut sb, mut sa) = (0u32, 0u32, 0u32, 0u32);
                for yi in y0..=y1 {
                    let p = tmp[yi * w + x];
                    sr += u32::from(p[0]);
                    sg += u32::from(p[1]);
                    sb += u32::from(p[2]);
                    sa += u32::from(p[3]);
                }
                buf[y * w + x] = [
                    (sr / count) as u8,
                    (sg / count) as u8,
                    (sb / count) as u8,
                    (sa / count) as u8,
                ];
            }
        }
    }
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
    for (s, d) in buf.iter().zip(out.make_mut_slice().iter_mut()) {
        *d = Rgba8Pixel {
            r: s[0],
            g: s[1],
            b: s[2],
            a: s[3],
        };
    }
    slint::Image::from_rgba8(out)
}

/// Crops `(x, y, w, h)` in 1920x1080 coordinates and applies horizontal left/right
/// alpha edge-fades (`fade_l`, `fade_r` in 1920px units, `0.0` = no fade).
fn crop_fade_image(
    src: &slint::Image,
    nx: f32,
    ny: f32,
    nw: f32,
    nh: f32,
    fade_l: f32,
    fade_r: f32,
) -> slint::Image {
    let Some(rgba) = src.to_rgba8() else {
        return src.clone();
    };
    let (sw, sh) = (rgba.width() as usize, rgba.height() as usize);
    if sw == 0 || sh == 0 {
        return src.clone();
    }
    let x0 = ((nx / 1920.0).clamp(0.0, 1.0) * sw as f32).round() as usize;
    let y0 = ((ny / 1080.0).clamp(0.0, 1.0) * sh as f32).round() as usize;
    let cw = (((nw / 1920.0).clamp(0.0, 1.0) * sw as f32).round() as usize)
        .max(1)
        .min(sw.saturating_sub(x0).max(1));
    let ch = (((nh / 1080.0).clamp(0.0, 1.0) * sh as f32).round() as usize)
        .max(1)
        .min(sh.saturating_sub(y0).max(1));
    let fl_px = ((fade_l / 1920.0).max(0.0) * sw as f32).max(0.0);
    let fr_px = ((fade_r / 1920.0).max(0.0) * sw as f32).max(0.0);
    let sp = rgba.as_slice();
    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(cw as u32, ch as u32);
    let dst = out.make_mut_slice();
    for y in 0..ch {
        let sy = (y0 + y).min(sh - 1);
        for x in 0..cw {
            let sx = (x0 + x).min(sw - 1);
            let p = sp[sy * sw + sx];
            let mut mult = 1.0_f32;
            if fl_px > 0.5 && (x as f32) < fl_px {
                mult *= (1.0 - (x as f32 / fl_px)).powf(1.25) * 0.85;
            }
            if fr_px > 0.5 && ((cw - 1 - x) as f32) < fr_px {
                mult *= (1.0 - ((cw - 1 - x) as f32 / fr_px)).powf(1.25) * 0.85;
            }
            dst[y * cw + x] = Rgba8Pixel {
                r: p.r,
                g: p.g,
                b: p.b,
                a: (f32::from(p.a) * mult).round().clamp(0.0, 255.0) as u8,
            };
        }
    }
    slint::Image::from_rgba8(out)
}

const MAX_PREFS_ENTRIES: usize = 64;
const MAX_PREF_VALUE_LEN: usize = 256;

fn is_safe_pref_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn load_prefs_file(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return map;
    };
    for line in text.lines().take(MAX_PREFS_ENTRIES) {
        if let Some((k, v)) = line.split_once('=')
            && is_safe_pref_key(k)
        {
            map.insert(k.to_owned(), v.chars().take(MAX_PREF_VALUE_LEN).collect());
        }
    }
    map
}

fn save_prefs_file(path: &Path, prefs: &BTreeMap<String, String>) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for (k, v) in prefs.iter().take(MAX_PREFS_ENTRIES) {
        out.push_str(k);
        out.push('=');
        out.push_str(&v.replace(['\r', '\n'], " "));
        out.push('\n');
    }
    let _ = fs::write(path, out);
}

/// Converts a launcher icon bitmap into a crisp warm-white (`#F4F4F1`) monochrome
/// mark using adaptive foreground/backing separation:
///
/// 1. Samples the outer perimeter of the icon's non-transparent bounding box to
///    find the dominant backing color (e.g. YouTube red, Spotify green, or a white
///    square card).
/// 2. If the icon has a distinct inner foreground glyph (`dist` from backing):
///    - White/near-white square cards (`bg_luma > 215`) knock out the card to 0%
///      alpha so only the crisp white inner logo remains.
///    - Coloured/dark badges render the inner glyph at 100% crisp warm white and
///      the badge backing as a soft 22% translucent frosted-glass silhouette so
///      recognizable shapes (rounded rects, circles, diamonds) stay legible.
/// 3. Simple transparent glyphs with no distinct backing keep their exact alpha
///    at 100% warm white.
fn make_mono_image(src: &slint::Image) -> slint::Image {
    extract_glyph_image(src, 0.22, (244, 244, 241))
}

fn extract_glyph_image(
    src: &slint::Image,
    badge_backing_alpha: f32,
    fg: (u8, u8, u8),
) -> slint::Image {
    let Some(rgba) = src.to_rgba8() else {
        return src.clone();
    };
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    if w == 0 || h == 0 {
        return src.clone();
    }
    let pixels = rgba.as_slice();

    // 1. Find bounding box of visible pixels (a >= 32).
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0usize, 0usize);
    let mut visible_count = 0usize;
    for y in 0..h {
        for x in 0..w {
            if pixels[y * w + x].a >= 32 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                visible_count += 1;
            }
        }
    }
    if visible_count == 0 {
        return src.clone();
    }

    // 2. Sample the outer ~12% perimeter ring of the bounding box to estimate
    //    the dominant backing colour (histogram quantised to 16-level buckets).
    let bw = (max_x - min_x + 1).max(1);
    let bh = (max_y - min_y + 1).max(1);
    let ring_x = (bw / 8).max(2);
    let ring_y = (bh / 8).max(2);
    type ColorBucket = (u64, u64, u64, u64);
    let mut buckets: HashMap<(u8, u8, u8), ColorBucket> = HashMap::new();
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let on_ring = x < min_x + ring_x
                || x + ring_x > max_x
                || y < min_y + ring_y
                || y + ring_y > max_y;
            if !on_ring {
                continue;
            }
            let p = pixels[y * w + x];
            if p.a < 180 {
                continue;
            }
            let key = (p.r >> 4, p.g >> 4, p.b >> 4);
            let entry = buckets.entry(key).or_insert((0, 0, 0, 0));
            entry.0 += 1;
            entry.1 += u64::from(p.r);
            entry.2 += u64::from(p.g);
            entry.3 += u64::from(p.b);
        }
    }

    let bg = buckets
        .into_values()
        .max_by_key(|v| v.0)
        .filter(|v| v.0 > 0)
        .map(|(n, r, g, b)| ((r / n) as i32, (g / n) as i32, (b / n) as i32));

    // Measure whether there is a real backing + distinct inner foreground glyph.
    let mut is_badge = false;
    let mut bg_is_light_card = false;
    if let Some((br, bg_g, bb)) = bg {
        let mut near_bg = 0usize;
        let mut far_fg = 0usize;
        for p in pixels {
            if p.a < 128 {
                continue;
            }
            let dr = i32::from(p.r) - br;
            let dg = i32::from(p.g) - bg_g;
            let db = i32::from(p.b) - bb;
            let dist = ((dr * dr + dg * dg + db * db) as f32).sqrt();
            if dist < 48.0 {
                near_bg += 1;
            } else if dist > 72.0 {
                far_fg += 1;
            }
        }
        // Badge icon if >= 25% matches backing and >= 4% is a distinct inner glyph.
        is_badge = near_bg * 4 >= visible_count && far_fg * 25 >= visible_count;
        let bg_luma = (br * 77 + bg_g * 150 + bb * 29) >> 8;
        // Check if corners of the bounding box are opaque + very light (white card).
        let corners_opaque = [
            pixels[min_y * w + min_x].a,
            pixels[min_y * w + max_x].a,
            pixels[max_y * w + min_x].a,
            pixels[max_y * w + max_x].a,
        ]
        .iter()
        .filter(|&&a| a >= 200)
        .count()
            >= 3;
        bg_is_light_card = bg_luma > 210 && corners_opaque;
    }

    let mut out = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);
    let dst = out.make_mut_slice();

    if let (true, Some((br, bg_g, bb))) = (is_badge, bg) {
        // Soft frosted-glass backing alpha (0.0 for white square cards, skin-chosen for coloured badges).
        let backing_alpha = if bg_is_light_card {
            0.0_f32
        } else {
            badge_backing_alpha.clamp(0.0, 1.0)
        };
        for (s, d) in pixels.iter().zip(dst.iter_mut()) {
            if s.a == 0 {
                *d = Rgba8Pixel {
                    r: fg.0,
                    g: fg.1,
                    b: fg.2,
                    a: 0,
                };
                continue;
            }
            let dr = i32::from(s.r) - br;
            let dg = i32::from(s.g) - bg_g;
            let db = i32::from(s.b) - bb;
            let dist = ((dr * dr + dg * dg + db * db) as f32).sqrt();
            // Smooth anti-aliased transition from backing (dist <= 34) to crisp glyph (dist >= 92).
            let fg_t = ((dist - 34.0) / 58.0).clamp(0.0, 1.0);
            let weight = backing_alpha + (1.0 - backing_alpha) * fg_t;
            let a = (f32::from(s.a) * weight).round().clamp(0.0, 255.0) as u8;
            *d = Rgba8Pixel {
                r: fg.0,
                g: fg.1,
                b: fg.2,
                a,
            };
        }
    } else {
        // Pure transparent-background glyph (or uniform icon): crisp foreground colour.
        for (s, d) in pixels.iter().zip(dst.iter_mut()) {
            *d = Rgba8Pixel {
                r: fg.0,
                g: fg.1,
                b: fg.2,
                a: s.a,
            };
        }
    }
    slint::Image::from_rgba8(out)
}

/// Sandboxed Rhai script state for a programmable skin.
struct SkinScript {
    engine: rhai::Engine,
    ast: rhai::AST,
    scope: rhai::Scope<'static>,
    bridge: Rc<RefCell<ScriptBridge>>,
}

impl SkinScript {
    fn compile(
        source: &str,
        skin_dir: PathBuf,
        prefs_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let mut engine = rhai::Engine::new();
        engine.set_max_operations(MAX_SCRIPT_OPS);
        engine.set_max_call_levels(MAX_SCRIPT_CALL_DEPTH);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_string_size(MAX_SCRIPT_STRING);
        engine.set_max_array_size(MAX_SCRIPT_COLLECTION);
        engine.set_max_map_size(MAX_SCRIPT_COLLECTION);
        engine.disable_symbol("eval");

        let initial_prefs = prefs_path
            .as_deref()
            .map(load_prefs_file)
            .unwrap_or_default();
        let bridge = Rc::new(RefCell::new(ScriptBridge {
            skin_dir,
            prefs: initial_prefs,
            prefs_path,
            ..ScriptBridge::default()
        }));

        {
            let b = bridge.clone();
            engine.register_fn("set_str", move |slot: i64, val: &str| {
                if let Ok(idx) = usize::try_from(slot)
                    && idx < STR_SLOTS
                {
                    let mut st = b.borrow_mut();
                    let cleaned: String = val.chars().take(MAX_SCRIPT_STRING).collect();
                    if st.strings[idx] != cleaned {
                        st.strings[idx] = cleaned;
                        st.dirty_str[idx] = true;
                    }
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("get_str", move |slot: i64| -> String {
                usize::try_from(slot)
                    .ok()
                    .and_then(|idx| b.borrow().strings.get(idx).cloned())
                    .unwrap_or_default()
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("set_int", move |slot: i64, val: i64| {
                if let Ok(idx) = usize::try_from(slot)
                    && idx < INT_SLOTS
                {
                    let v = i32::try_from(val.clamp(i64::from(i32::MIN), i64::from(i32::MAX)))
                        .unwrap_or(0);
                    let mut st = b.borrow_mut();
                    if st.ints[idx] != v {
                        st.ints[idx] = v;
                        st.dirty_int[idx] = true;
                    }
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("get_int", move |slot: i64| -> i64 {
                usize::try_from(slot)
                    .ok()
                    .and_then(|idx| b.borrow().ints.get(idx).copied())
                    .map(i64::from)
                    .unwrap_or(0)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("set_float", move |slot: i64, val: f32| {
                if let Ok(idx) = usize::try_from(slot)
                    && idx < FLOAT_SLOTS
                    && val.is_finite()
                {
                    let mut st = b.borrow_mut();
                    if (st.floats[idx] - val).abs() > f32::EPSILON {
                        st.floats[idx] = val;
                        st.dirty_float[idx] = true;
                    }
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("get_float", move |slot: i64| -> f32 {
                usize::try_from(slot)
                    .ok()
                    .and_then(|idx| b.borrow().floats.get(idx).copied())
                    .unwrap_or(0.0)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("tile_count", move || -> i64 {
                i64::try_from(b.borrow().tile_ids.len()).unwrap_or(0)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("tile_id", move |index: i64| -> String {
                usize::try_from(index)
                    .ok()
                    .and_then(|i| b.borrow().tile_ids.get(i).cloned())
                    .unwrap_or_default()
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("tile_label", move |index: i64| -> String {
                usize::try_from(index)
                    .ok()
                    .and_then(|i| b.borrow().tile_labels.get(i).cloned())
                    .unwrap_or_default()
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("launch", move |index: i64| {
                if let Ok(i) = i32::try_from(index) {
                    b.borrow_mut().commands.push(HostCommand::Launch(i));
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("focus", move |index: i64| {
                if let Ok(i) = i32::try_from(index) {
                    b.borrow_mut().commands.push(HostCommand::Focus(i));
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("open_settings", move || {
                b.borrow_mut().commands.push(HostCommand::OpenSettings);
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("save_pref", move |key: &str, val: &str| {
                if !is_safe_pref_key(key) {
                    return;
                }
                let mut st = b.borrow_mut();
                if st.prefs.len() >= MAX_PREFS_ENTRIES && !st.prefs.contains_key(key) {
                    return;
                }
                let cleaned: String = val
                    .replace(['\r', '\n'], " ")
                    .chars()
                    .take(MAX_PREF_VALUE_LEN)
                    .collect();
                if st.prefs.get(key).map(String::as_str) != Some(cleaned.as_str()) {
                    st.prefs.insert(key.to_owned(), cleaned);
                    if let Some(path) = st.prefs_path.clone() {
                        save_prefs_file(&path, &st.prefs);
                    }
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("load_pref", move |key: &str, fallback: &str| -> String {
                b.borrow()
                    .prefs
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| fallback.to_owned())
            });
        }
        // Composable skin-owned image pipeline primitives (native-speed, handle-based).
        fn push_image(st: &mut ScriptBridge, img: slint::Image) -> i64 {
            if st.image_pool.len() >= MAX_SCRIPT_IMAGES {
                return 0;
            }
            st.image_pool.push(img);
            i64::try_from(st.image_pool.len()).unwrap_or(0)
        }
        fn get_image(st: &ScriptBridge, handle: i64) -> Option<slint::Image> {
            let idx = usize::try_from(handle).ok()?.checked_sub(1)?;
            st.image_pool.get(idx).cloned()
        }
        {
            let b = bridge.clone();
            engine.register_fn("tile_image", move |index: i64| -> i64 {
                let mut st = b.borrow_mut();
                let Some(img) = usize::try_from(index)
                    .ok()
                    .and_then(|i| st.tile_images.get(i).cloned())
                else {
                    return 0;
                };
                if img.size().width == 0 {
                    return 0;
                }
                push_image(&mut st, img)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("load_image", move |rel_path: &str| -> i64 {
                let mut st = b.borrow_mut();
                let Ok(root) = fs::canonicalize(&st.skin_dir) else {
                    return 0;
                };
                let Ok(file) = fs::canonicalize(st.skin_dir.join(rel_path)) else {
                    return 0;
                };
                if !file.starts_with(&root) || !file.is_file() {
                    return 0;
                }
                let Ok(img) = slint::Image::load_from_path(&file) else {
                    return 0;
                };
                push_image(&mut st, img)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn(
                "img_extract_glyph",
                move |handle: i64, backing_alpha: f64, fg_hex: &str| -> i64 {
                    let mut st = b.borrow_mut();
                    let Some(src) = get_image(&st, handle) else {
                        return 0;
                    };
                    let out =
                        extract_glyph_image(&src, backing_alpha as f32, parse_hex_rgb(fg_hex));
                    push_image(&mut st, out)
                },
            );
        }
        {
            let b = bridge.clone();
            engine.register_fn(
                "img_tint",
                move |handle: i64, hex: &str, strength: f64| -> i64 {
                    let mut st = b.borrow_mut();
                    let Some(src) = get_image(&st, handle) else {
                        return 0;
                    };
                    let out = tint_image(&src, parse_hex_rgb(hex), strength as f32);
                    push_image(&mut st, out)
                },
            );
        }
        {
            let b = bridge.clone();
            engine.register_fn("img_desaturate", move |handle: i64, amount: f64| -> i64 {
                let mut st = b.borrow_mut();
                let Some(src) = get_image(&st, handle) else {
                    return 0;
                };
                let out = desaturate_image(&src, amount as f32);
                push_image(&mut st, out)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("img_blur", move |handle: i64, radius: i64| -> i64 {
                let mut st = b.borrow_mut();
                let Some(src) = get_image(&st, handle) else {
                    return 0;
                };
                let out = blur_image(&src, radius);
                push_image(&mut st, out)
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn(
                "img_crop_fade",
                move |handle: i64,
                      x: f64,
                      y: f64,
                      w: f64,
                      h: f64,
                      fade_l: f64,
                      fade_r: f64|
                      -> i64 {
                    let mut st = b.borrow_mut();
                    let Some(src) = get_image(&st, handle) else {
                        return 0;
                    };
                    let out = crop_fade_image(
                        &src,
                        x as f32,
                        y as f32,
                        w as f32,
                        h as f32,
                        fade_l as f32,
                        fade_r as f32,
                    );
                    push_image(&mut st, out)
                },
            );
        }
        {
            let b = bridge.clone();
            engine.register_fn("set_tile_image", move |index: i64, handle: i64| {
                let mut st = b.borrow_mut();
                if let (Ok(idx), Some(img)) = (usize::try_from(index), get_image(&st, handle)) {
                    st.tile_overrides.insert(idx, img);
                }
            });
        }
        {
            let b = bridge.clone();
            engine.register_fn("set_img_slot", move |slot: i64, handle: i64| {
                let mut st = b.borrow_mut();
                if let (Ok(idx), Some(img)) = (usize::try_from(slot), get_image(&st, handle))
                    && idx < IMG_SLOTS
                {
                    st.img_slots[idx] = img;
                    st.dirty_img[idx] = true;
                }
            });
        }
        engine.register_fn("log", |msg: &str| {
            log::info!("skin script: {msg}");
        });

        let ast = engine
            .compile(source)
            .map_err(|e| format!("Rhai compile error: {e}"))?;
        let mut scope = rhai::Scope::new();
        // Run top-level declarations once.
        let _ = engine
            .eval_ast_with_scope::<rhai::Dynamic>(&mut scope, &ast)
            .map_err(|e| format!("Rhai init error: {e}"))?;

        Ok(Self {
            engine,
            ast,
            scope,
            bridge,
        })
    }

    fn call_void(&mut self, name: &str, args: impl rhai::FuncArgs) {
        if let Err(err) =
            self.engine
                .call_fn::<rhai::Dynamic>(&mut self.scope, &self.ast, name, args)
            && !matches!(*err, rhai::EvalAltResult::ErrorFunctionNotFound(_, _))
        {
            log::warn!("skin script {name}() failed: {err}");
        }
    }

    fn call_bool(&mut self, name: &str, args: impl rhai::FuncArgs) -> bool {
        match self
            .engine
            .call_fn::<bool>(&mut self.scope, &self.ast, name, args)
        {
            Ok(val) => val,
            Err(err) => {
                if !matches!(*err, rhai::EvalAltResult::ErrorFunctionNotFound(_, _)) {
                    log::warn!("skin script {name}() failed: {err}");
                }
                false
            }
        }
    }

    fn take_commands(&self) -> Vec<HostCommand> {
        std::mem::take(&mut self.bridge.borrow_mut().commands)
    }
}

/// Live compiled `.slint` skin instance (+ optional sandboxed Rhai script).
pub struct SkinInstance {
    pub manifest: SkinManifest,
    instance: ComponentInstance,
    script: Option<SkinScript>,
    mono_cache: RefCell<HashMap<String, slint::Image>>,
    /// The `quick-actions` last pushed (code, label), to skip identical updates.
    quick_sig: RefCell<Vec<(i32, String)>>,
}

impl SkinInstance {
    /// Compiles and instantiates a programmable skin from its validated manifest.
    #[cfg(test)]
    pub fn load(manifest: &SkinManifest) -> Result<Self, String> {
        Self::load_with_prefs(manifest, None)
    }

    /// Compiles and instantiates a programmable skin with an optional directory
    /// for persistent `save_pref`/`load_pref` storage.
    #[allow(dead_code, reason = "part of the public programmable skin host API")]
    pub fn load_with_prefs(
        manifest: &SkinManifest,
        prefs_dir: Option<&Path>,
    ) -> Result<Self, String> {
        Self::load_into_window(manifest, prefs_dir, None)
    }

    /// [`Self::load_with_prefs`], drawing into `window` (the host's) instead
    /// of a window of its own. The skin's tree becomes the window's at once;
    /// the host swaps between its own tree and the skin's
    /// (`ui::one_window`), so no window is created or destroyed when built-in
    /// pages open over the skin.
    pub fn load_into_window(
        manifest: &SkinManifest,
        prefs_dir: Option<&Path>,
        window: Option<&slint::Window>,
    ) -> Result<Self, String> {
        let entry_name = manifest
            .entry_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "invalid skin entry filename".to_owned())?
            .to_owned();
        // Wrap the user's skin.slint so `@rusttv/skin.slint`'s `RustTV` global
        // is always exported to the Rust host even if the skin only imports it.
        let wrapper_src = format!(
            "import {{ RustTV, SkinTile, SkinAction }} from \"@rusttv/skin.slint\";\n\
             import {{ SkinHome }} from \"./{entry_name}\";\n\
             export {{ RustTV, SkinTile, SkinAction, SkinHome }}\n"
        );
        let mut compiler = Compiler::new();
        let skin_dir = manifest.dir.clone();
        compiler.set_file_loader(move |path| {
            let p = path.to_string_lossy();
            if p == "@rusttv/skin.slint" || p.ends_with("@rusttv/skin.slint") {
                return Box::pin(async { Some(Ok(SKIN_API_SLINT.to_owned())) });
            }
            // Allow relative .slint imports strictly inside the skin's own directory.
            let candidate = if path.is_relative() {
                skin_dir.join(path)
            } else {
                path.to_path_buf()
            };
            let Ok(canonical_root) = fs::canonicalize(&skin_dir) else {
                return Box::pin(async { None });
            };
            let Ok(canonical_file) = fs::canonicalize(&candidate) else {
                return Box::pin(async { None });
            };
            if !canonical_file.starts_with(&canonical_root) {
                return Box::pin(async {
                    Some(Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "skin import escapes skin directory",
                    )))
                });
            }
            Box::pin(async move {
                Some(read_capped(&canonical_file, MAX_SOURCE_BYTES).map_err(std::io::Error::other))
            })
        });

        let result = poll_sync(
            compiler
                .build_from_source(wrapper_src, manifest.dir.join("__rusttv_skin_root__.slint")),
        );
        let errors: Vec<String> = result
            .diagnostics()
            .filter(|d| d.level() == DiagnosticLevel::Error)
            .map(|d| d.to_string())
            .collect();
        if !errors.is_empty() {
            return Err(format!("Slint compile error(s):\n{}", errors.join("\n")));
        }

        let definition = result.component("SkinHome").ok_or_else(|| {
            "skin.slint must export `component SkinHome inherits Window`".to_owned()
        })?;
        let instance = match window {
            Some(window) => definition.create_with_existing_window(window),
            None => definition.create(),
        }
        .map_err(|e| format!("failed to create SkinHome: {e}"))?;

        let prefs_path = prefs_dir.map(|dir| {
            let stem = manifest.id.strip_prefix("skin:").unwrap_or(&manifest.id);
            dir.join(format!("{stem}.prefs"))
        });
        let mut script = match &manifest.script_path {
            Some(path) => {
                let src = read_capped(path, MAX_SOURCE_BYTES)?;
                Some(SkinScript::compile(&src, manifest.dir.clone(), prefs_path)?)
            }
            None => None,
        };

        if let Some(s) = script.as_mut() {
            s.call_void("on_init", ());
        }

        let skin = Self {
            manifest: manifest.clone(),
            instance,
            script,
            mono_cache: RefCell::new(HashMap::new()),
            quick_sig: RefCell::new(Vec::new()),
        };
        skin.sync_script_slots();
        Ok(skin)
    }

    /// Underlying Slint window for rendering, showing, or headless snapshots.
    #[allow(dead_code, reason = "part of the public programmable skin host API")]
    pub fn window(&self) -> &slint::Window {
        self.instance.window()
    }

    /// Shows the skin window fullscreen.
    #[allow(dead_code, reason = "part of the public programmable skin host API")]
    pub fn show(&self) -> Result<(), String> {
        self.instance.window().set_fullscreen(true);
        self.instance.show().map_err(|e| e.to_string())
    }

    /// Hides the skin window.
    #[allow(dead_code, reason = "part of the public programmable skin host API")]
    pub fn hide(&self) -> Result<(), String> {
        self.instance.hide().map_err(|e| e.to_string())
    }

    /// Resolves a monochrome icon for `tile`: first checks `<skin>/icons/<safe-id>.svg`
    /// (or `.png`), then falls back to a cached warm-white silhouette of `tile.image`.
    fn mono_image_for(&self, tile: &SkinTileData) -> slint::Image {
        if let Some(cached) = self.mono_cache.borrow().get(&tile.id) {
            return cached.clone();
        }
        let safe: String = tile
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let icons_dir = self.manifest.dir.join("icons");
        if let Some(custom) = ["svg", "png"].into_iter().find_map(|ext| {
            let path = icons_dir.join(format!("{safe}.{ext}"));
            path.is_file()
                .then(|| slint::Image::load_from_path(&path).ok())
                .flatten()
        }) {
            self.mono_cache
                .borrow_mut()
                .insert(tile.id.clone(), custom.clone());
            return custom;
        }
        // Don't cache empty placeholders while the async icon loader is still running.
        if tile.image.size().width == 0 {
            return tile.image.clone();
        }
        let resolved = make_mono_image(&tile.image);
        self.mono_cache
            .borrow_mut()
            .insert(tile.id.clone(), resolved.clone());
        resolved
    }

    /// Pushes the live tile list into `RustTV.tiles` and invokes `on_tiles_loaded()`
    /// in Rhai so the skin can run its own image transforms (`img_extract_glyph`,
    /// `img_tint`, `set_tile_image`, etc.).
    pub fn set_tiles(&mut self, tiles: &[SkinTileData]) {
        let mut overrides = HashMap::new();
        if let Some(script) = self.script.as_mut() {
            {
                let mut b = script.bridge.borrow_mut();
                b.tile_ids = tiles.iter().map(|t| t.id.clone()).collect();
                b.tile_labels = tiles.iter().map(|t| t.label.clone()).collect();
                b.tile_images = tiles.iter().map(|t| t.image.clone()).collect();
                b.tile_overrides.clear();
                b.image_pool.clear();
            }
            script.call_void("on_tiles_loaded", ());
            overrides = std::mem::take(&mut script.bridge.borrow_mut().tile_overrides);
        }
        let rows: Vec<InterpValue> = tiles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mono = overrides
                    .remove(&i)
                    .unwrap_or_else(|| self.mono_image_for(t));
                let mut s = InterpStruct::default();
                s.set_field("id".into(), InterpValue::String(t.id.as_str().into()));
                s.set_field("label".into(), InterpValue::String(t.label.as_str().into()));
                s.set_field("image".into(), InterpValue::Image(t.image.clone()));
                s.set_field("mono-image".into(), InterpValue::Image(mono));
                s.set_field("kind".into(), InterpValue::Number(f64::from(t.kind)));
                s.set_field("running".into(), InterpValue::Bool(t.running));
                s.set_field(
                    "tint".into(),
                    InterpValue::Brush(slint::Brush::SolidColor(t.tint)),
                );
                InterpValue::Struct(s)
            })
            .collect();
        let model = ModelRc::from(Rc::new(VecModel::from(rows)));
        let _ = self
            .instance
            .set_global_property("RustTV", "tiles", InterpValue::Model(model));
        self.sync_script_slots();
    }

    /// Updates focused tile index and invokes `on_focus(index, id, label)` in Rhai.
    pub fn set_focused(&mut self, index: i32, id: &str, label: &str) -> Vec<HostCommand> {
        let _ = self.instance.set_global_property(
            "RustTV",
            "focused-index",
            InterpValue::Number(f64::from(index)),
        );
        if let Some(script) = self.script.as_mut() {
            script.call_void(
                "on_focus",
                (i64::from(index), id.to_owned(), label.to_owned()),
            );
        }
        self.sync_script_slots();
        self.take_commands()
    }

    /// Updates `RustTV.reduced-effects` from Settings → Effects.
    pub fn set_reduced_effects(&self, reduced: bool) {
        let _ = self.instance.set_global_property(
            "RustTV",
            "reduced-effects",
            InterpValue::Bool(reduced),
        );
    }

    /// Updates the in-skin long-press context menu state on `RustTV`.
    pub fn set_menu(&self, open: bool, title: &str, items: &[String], selected: i32) {
        if !open
            && matches!(
                self.instance.get_global_property("RustTV", "menu-open"),
                Ok(InterpValue::Bool(false))
            )
        {
            return;
        }
        let _ = self
            .instance
            .set_global_property("RustTV", "menu-open", InterpValue::Bool(open));
        let _ = self.instance.set_global_property(
            "RustTV",
            "menu-title",
            InterpValue::String(SharedString::from(title)),
        );
        let rows: Vec<InterpValue> = items
            .iter()
            .map(|s| InterpValue::String(SharedString::from(s.as_str())))
            .collect();
        let model = ModelRc::from(Rc::new(VecModel::from(rows)));
        let _ =
            self.instance
                .set_global_property("RustTV", "menu-items", InterpValue::Model(model));
        let _ = self.instance.set_global_property(
            "RustTV",
            "menu-selected",
            InterpValue::Number(f64::from(selected)),
        );
    }

    /// Registers a mouse/touch click handler for `RustTV.menu-clicked(index)`.
    pub fn set_menu_callback<F>(&self, on_menu_click: F)
    where
        F: Fn(i32) + 'static,
    {
        let _ = self
            .instance
            .set_global_callback("RustTV", "menu-clicked", move |args| {
                let idx = match args.first() {
                    Some(InterpValue::Number(n)) => *n as i32,
                    _ => -1,
                };
                on_menu_click(idx);
                InterpValue::Void
            });
    }

    /// Updates clock, greeting, and date strings on `RustTV`.
    pub fn set_clock(&self, clock: &str, greeting: &str, date: &str) {
        let _ = self.instance.set_global_property(
            "RustTV",
            "clock-text",
            InterpValue::String(SharedString::from(clock)),
        );
        let _ = self.instance.set_global_property(
            "RustTV",
            "greeting-text",
            InterpValue::String(SharedString::from(greeting)),
        );
        let _ = self.instance.set_global_property(
            "RustTV",
            "date-text",
            InterpValue::String(SharedString::from(date)),
        );
    }

    /// Updates hero title, accent/glow colour, and hero image on `RustTV`.
    pub fn set_hero(&self, title: &str, color: slint::Color, image: slint::Image) {
        let _ = self.instance.set_global_property(
            "RustTV",
            "hero-title",
            InterpValue::String(SharedString::from(title)),
        );
        let _ = self.instance.set_global_property(
            "RustTV",
            "hero-color",
            InterpValue::Brush(slint::Brush::SolidColor(color)),
        );
        let _ =
            self.instance
                .set_global_property("RustTV", "hero-image", InterpValue::Image(image));
    }

    /// Updates quick-bar actions and selected index on `RustTV`. The action
    /// list is replaced only when it changed, so the skin's buttons aren't
    /// rebuilt on every render.
    pub fn set_quick_bar(&self, actions: &[(i32, &str)], selected: i32) {
        let _ = self.instance.set_global_property(
            "RustTV",
            "quick-selected",
            InterpValue::Number(f64::from(selected)),
        );
        let sig: Vec<(i32, String)> = actions
            .iter()
            .map(|(code, label)| (*code, (*label).to_owned()))
            .collect();
        if *self.quick_sig.borrow() == sig {
            return;
        }
        *self.quick_sig.borrow_mut() = sig;
        let rows: Vec<InterpValue> = actions
            .iter()
            .map(|(code, label)| {
                let mut s = InterpStruct::default();
                s.set_field("code".into(), InterpValue::Number(f64::from(*code)));
                s.set_field("label".into(), InterpValue::String((*label).into()));
                InterpValue::Struct(s)
            })
            .collect();
        let model = ModelRc::from(Rc::new(VecModel::from(rows)));
        let _ =
            self.instance
                .set_global_property("RustTV", "quick-actions", InterpValue::Model(model));
    }

    /// Updates `RustTV.mode` (home.slint `Mode` values: 0 browse, 1 moving,
    /// 2 launching, 4 menu, 11 top-right menu, …).
    pub fn set_mode(&self, mode: i32) {
        let current = self.instance.get_global_property("RustTV", "mode");
        if !matches!(current, Ok(InterpValue::Number(n)) if n as i32 == mode) {
            let _ = self.instance.set_global_property(
                "RustTV",
                "mode",
                InterpValue::Number(f64::from(mode)),
            );
        }
    }

    /// Registers handlers for `RustTV.quick-clicked(index)` and
    /// `RustTV.quick-hovered(index)` (entries of `quick-actions`).
    pub fn set_quick_callbacks<FClick, FHover>(&self, on_click: FClick, on_hover: FHover)
    where
        FClick: Fn(i32) + 'static,
        FHover: Fn(i32) + 'static,
    {
        let index = |args: &[InterpValue]| match args.first() {
            Some(InterpValue::Number(n)) => *n as i32,
            _ => -1,
        };
        let _ = self
            .instance
            .set_global_callback("RustTV", "quick-clicked", move |args| {
                on_click(index(args));
                InterpValue::Void
            });
        let _ = self
            .instance
            .set_global_callback("RustTV", "quick-hovered", move |args| {
                on_hover(index(args));
                InterpValue::Void
            });
    }

    /// Dispatches a custom UI action (`RustTV.action(name, arg)`) to `on_action` in Rhai.
    pub fn dispatch_action(&mut self, name: &str, arg: &str) -> Vec<HostCommand> {
        if let Some(script) = self.script.as_mut() {
            script.call_void("on_action", (name.to_owned(), arg.to_owned()));
        }
        self.sync_script_slots();
        self.take_commands()
    }

    /// Gives the Rhai script first crack at a key event via `on_key(key, pressed)`.
    /// Returns `(handled, queued_commands)`.
    pub fn dispatch_key(&mut self, key: &str, pressed: bool) -> (bool, Vec<HostCommand>) {
        let handled = if let Some(script) = self.script.as_mut() {
            script.call_bool("on_key", (key.to_owned(), pressed))
        } else {
            false
        };
        self.sync_script_slots();
        (handled, self.take_commands())
    }

    /// Invokes optional `on_tick(now_ms)` in Rhai.
    #[allow(dead_code, reason = "part of the public programmable skin host API")]
    pub fn tick(&mut self, now_ms: i64) -> Vec<HostCommand> {
        if let Some(script) = self.script.as_mut() {
            script.call_void("on_tick", (now_ms,));
        }
        self.sync_script_slots();
        self.take_commands()
    }

    /// Reads a reactive string slot from `RustTV` (useful for tests/verification).
    #[cfg(test)]
    pub fn get_str_slot(&self, slot: usize) -> String {
        let prop = format!("s{slot}");
        match self.instance.get_global_property("RustTV", &prop) {
            Ok(InterpValue::String(s)) => s.to_string(),
            _ => String::new(),
        }
    }

    /// Reads a reactive int slot from `RustTV` (useful for tests/verification).
    #[cfg(test)]
    pub fn get_int_slot(&self, slot: usize) -> i32 {
        let prop = format!("i{slot}");
        match self.instance.get_global_property("RustTV", &prop) {
            Ok(InterpValue::Number(n)) => n as i32,
            _ => 0,
        }
    }

    /// Registers callback handlers on the `RustTV` global.
    pub fn set_callbacks<FKey, FTile, FAction>(
        &self,
        on_key: FKey,
        on_tile: FTile,
        on_action: FAction,
    ) where
        FKey: Fn(String, bool, bool) -> bool + 'static,
        FTile: Fn(i32, bool) + 'static,
        FAction: Fn(String, String) + 'static,
    {
        let _ = self
            .instance
            .set_global_callback("RustTV", "key", move |args| {
                let text = match args.first() {
                    Some(InterpValue::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                let pressed = matches!(args.get(1), Some(InterpValue::Bool(true)));
                let repeat = matches!(args.get(2), Some(InterpValue::Bool(true)));
                InterpValue::Bool(on_key(text, pressed, repeat))
            });
        let _ = self
            .instance
            .set_global_callback("RustTV", "tile-clicked", move |args| {
                let idx = match args.first() {
                    Some(InterpValue::Number(n)) => *n as i32,
                    _ => -1,
                };
                let menu = matches!(args.get(1), Some(InterpValue::Bool(true)));
                on_tile(idx, menu);
                InterpValue::Void
            });
        let _ = self
            .instance
            .set_global_callback("RustTV", "action", move |args| {
                let name = match args.first() {
                    Some(InterpValue::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                let arg = match args.get(1) {
                    Some(InterpValue::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                on_action(name, arg);
                InterpValue::Void
            });
    }

    fn sync_script_slots(&self) {
        let Some(script) = &self.script else {
            return;
        };
        let mut b = script.bridge.borrow_mut();
        for idx in 0..STR_SLOTS {
            if std::mem::take(&mut b.dirty_str[idx]) {
                let prop = format!("s{idx}");
                let _ = self.instance.set_global_property(
                    "RustTV",
                    &prop,
                    InterpValue::String(SharedString::from(b.strings[idx].as_str())),
                );
            }
        }
        for idx in 0..INT_SLOTS {
            if std::mem::take(&mut b.dirty_int[idx]) {
                let prop = format!("i{idx}");
                let _ = self.instance.set_global_property(
                    "RustTV",
                    &prop,
                    InterpValue::Number(f64::from(b.ints[idx])),
                );
            }
        }
        for idx in 0..FLOAT_SLOTS {
            if std::mem::take(&mut b.dirty_float[idx]) {
                let prop = format!("f{idx}");
                let _ = self.instance.set_global_property(
                    "RustTV",
                    &prop,
                    InterpValue::Number(f64::from(b.floats[idx])),
                );
            }
        }
        for idx in 0..IMG_SLOTS {
            if std::mem::take(&mut b.dirty_img[idx]) {
                let prop = format!("img{idx}");
                let _ = self.instance.set_global_property(
                    "RustTV",
                    &prop,
                    InterpValue::Image(b.img_slots[idx].clone()),
                );
            }
        }
    }

    fn take_commands(&self) -> Vec<HostCommand> {
        self.script
            .as_ref()
            .map(SkinScript::take_commands)
            .unwrap_or_default()
    }
}

/// Discovers all valid Level 3 programmable skins (`<dir>/<stem>/skin.toml`).
pub fn discover_skins(dirs: &[PathBuf]) -> Vec<SkinManifest> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("skin.toml").is_file())
            .collect();
        subdirs.sort();
        for skin_dir in subdirs {
            let Some(stem) = skin_dir.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let id = format!("skin:{stem}");
            if !seen.insert(id.clone()) {
                continue;
            }
            match load_manifest(&skin_dir, id) {
                Ok(manifest) => found.push(manifest),
                Err(error) => log::warn!("ignoring skin {}: {error}", skin_dir.display()),
            }
        }
    }
    found
}

/// Parses and validates `<skin_dir>/skin.toml`.
pub fn load_manifest(skin_dir: &Path, id: String) -> Result<SkinManifest, String> {
    let manifest_path = skin_dir.join("skin.toml");
    let text = read_capped(&manifest_path, MAX_SOURCE_BYTES)?;
    let raw: SkinManifestFile = toml::from_str(&text).map_err(|e| e.to_string())?;
    if raw.schema_version != SKIN_SCHEMA_VERSION {
        return Err(format!(
            "unsupported skin schema_version {} (expected {SKIN_SCHEMA_VERSION})",
            raw.schema_version
        ));
    }
    let name: String = raw
        .name
        .chars()
        .filter(|c| !c.is_control())
        .take(40)
        .collect();
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("skin name cannot be empty".to_owned());
    }
    let desc = raw
        .description
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect::<String>();
    let author = raw
        .author
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect::<String>();
    let detail = match (desc.trim().is_empty(), author.trim().is_empty()) {
        (false, false) => format!("{} · by {}", desc.trim(), author.trim()),
        (false, true) => desc.trim().to_owned(),
        (true, false) => format!("Programmable skin by {}", author.trim()),
        (true, true) => "Programmable .slint skin".to_owned(),
    };
    let entry_path = resolve_local_file(skin_dir, &raw.entry, "slint")?;
    let script_path = match raw.script {
        Some(rel) => Some(resolve_local_file(skin_dir, &rel, "rhai")?),
        None => None,
    };
    Ok(SkinManifest {
        id,
        name,
        detail,
        dir: skin_dir.to_path_buf(),
        entry_path,
        script_path,
    })
}

fn resolve_local_file(root: &Path, rel: &str, expected_ext: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel.trim());
    if rel_path.as_os_str().is_empty()
        || rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("invalid skin file path {rel:?}"));
    }
    if rel_path.extension().and_then(|e| e.to_str()) != Some(expected_ext) {
        return Err(format!("expected .{expected_ext} file, got {rel:?}"));
    }
    let full = root.join(rel_path);
    if !full.is_file() {
        return Err(format!("skin file {} does not exist", full.display()));
    }
    Ok(full)
}

/// Polls a synchronous future (our file loader always resolves immediately)
/// to completion without pulling in an async runtime.
fn poll_sync<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut fut = std::pin::pin!(fut);
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    loop {
        if let Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
            return out;
        }
        std::thread::yield_now();
    }
}

fn read_capped(path: &Path, max_bytes: u64) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > max_bytes {
        return Err(format!(
            "{} is {} bytes (limit {max_bytes})",
            path.display(),
            meta.len()
        ));
    }
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_skin(name: &str, slint: &str, rhai: Option<&str>) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusttv-skin-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script_line = if rhai.is_some() {
            "script = \"skin.rhai\"\n"
        } else {
            ""
        };
        fs::write(
            dir.join("skin.toml"),
            format!("schema_version = 2\nname = \"{name}\"\nentry = \"skin.slint\"\n{script_line}"),
        )
        .unwrap();
        fs::write(dir.join("skin.slint"), slint).unwrap();
        if let Some(code) = rhai {
            fs::write(dir.join("skin.rhai"), code).unwrap();
        }
        dir
    }

    #[test]
    fn runtime_slint_and_rhai_skin_compiles_runs_hooks_and_syncs_state() {
        thread_local!(static BACKEND: () = i_slint_backend_testing::init_no_event_loop());
        BACKEND.with(|_| {});

        let dir = temp_skin(
            "basic",
            r#"
import { RustTV } from "@rusttv/skin.slint";
export component SkinHome inherits Window {
    width: 1280px;
    height: 720px;
    background: #0E0F12;
    Text { text: RustTV.s0; }
}
"#,
            Some(
                r#"
fn on_init() {
    set_str(0, "For you");
    set_int(0, 0);
}
fn on_focus(index, id, label) {
    set_str(1, "Selected: " + label);
    set_int(1, index);
}
fn on_action(name, arg) {
    if name == "tab" {
        set_str(0, arg);
        set_int(0, get_int(0) + 1);
    }
}
fn on_key(key, pressed) {
    if pressed && key == "t" {
        set_str(0, "Apps");
        return true;
    }
    false
}
"#,
            ),
        );

        let manifest = load_manifest(&dir, "skin:basic".to_owned()).unwrap();
        let mut skin = SkinInstance::load(&manifest).unwrap();
        assert_eq!(skin.get_str_slot(0), "For you");
        assert_eq!(skin.get_int_slot(0), 0);

        skin.set_focused(2, "net.netflix/.Main", "Netflix");
        assert_eq!(skin.get_str_slot(1), "Selected: Netflix");
        assert_eq!(skin.get_int_slot(1), 2);

        skin.dispatch_action("tab", "Library");
        assert_eq!(skin.get_str_slot(0), "Library");
        assert_eq!(skin.get_int_slot(0), 1);

        let (handled, _) = skin.dispatch_key("t", true);
        assert!(handled);
        assert_eq!(skin.get_str_slot(0), "Apps");

        let (handled_other, _) = skin.dispatch_key("x", true);
        assert!(!handled_other);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn runaway_rhai_loop_is_stopped_by_operation_limit() {
        thread_local!(static BACKEND: () = i_slint_backend_testing::init_no_event_loop());
        BACKEND.with(|_| {});

        let dir = temp_skin(
            "runaway",
            r#"
import { RustTV } from "@rusttv/skin.slint";
export component SkinHome inherits Window { width: 100px; height: 100px; }
"#,
            Some("fn on_action(name, arg) { loop { } }"),
        );
        let manifest = load_manifest(&dir, "skin:runaway".to_owned()).unwrap();
        let mut skin = SkinInstance::load(&manifest).unwrap();
        // Must terminate cleanly via MAX_SCRIPT_OPS instead of hanging!
        let cmds = skin.dispatch_action("hang", "");
        assert!(cmds.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_rejects_path_traversal() {
        let dir = std::env::temp_dir().join(format!("rusttv-skin-trav-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("skin.toml"),
            "schema_version = 2\nname = \"Bad\"\nentry = \"../escape.slint\"\n",
        )
        .unwrap();
        assert!(load_manifest(&dir, "skin:bad".to_owned()).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundled_spotlight_pro_programmable_skin_loads_and_switches_tabs() {
        thread_local!(static BACKEND: () = i_slint_backend_testing::init_no_event_loop());
        BACKEND.with(|_| {});

        let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("themes");
        let skins = discover_skins(std::slice::from_ref(&themes_dir));
        for expected in [
            "skin:spotlight-pro",
            "skin:coastal-dock-pro",
            "skin:ambient-pills-pro",
        ] {
            let manifest = skins
                .iter()
                .find(|s| s.id == expected)
                .unwrap_or_else(|| panic!("missing bundled skin {expected}"));
            let _ = SkinInstance::load(manifest).unwrap_or_else(|e| panic!("{expected}: {e}"));
        }
        let manifest = skins
            .iter()
            .find(|s| s.id == "skin:spotlight-pro")
            .expect("spotlight-pro programmable skin");
        let mut skin = SkinInstance::load(manifest).expect("compile spotlight-pro");
        assert_eq!(skin.get_str_slot(0), "For you");
        assert_eq!(skin.get_str_slot(2), "Your apps");

        skin.dispatch_action("tab", "3");
        assert_eq!(skin.get_str_slot(0), "Apps");
        assert_eq!(skin.get_str_slot(2), "All installed apps");

        let (handled, _) = skin.dispatch_key("e", true);
        assert!(handled);
        assert_eq!(skin.get_str_slot(0), "Library");

        let cmds = skin.dispatch_action("settings", "");
        assert_eq!(cmds, vec![HostCommand::OpenSettings]);
    }

    #[test]
    fn bundled_classic_home_pro_navigates_like_the_default_home() {
        thread_local!(static BACKEND: () = i_slint_backend_testing::init_no_event_loop());
        BACKEND.with(|_| {});

        let themes_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("themes");
        let skins = discover_skins(std::slice::from_ref(&themes_dir));
        let manifest = skins
            .iter()
            .find(|s| s.id == "skin:classic-home-pro")
            .expect("classic-home-pro programmable skin");
        let mut skin = SkinInstance::load(manifest).expect("compile classic-home-pro");
        // 20 tiles: rows 0..=2 full, row 3 holds two.
        let tiles: Vec<SkinTileData> = (0..20)
            .map(|i| SkinTileData {
                id: format!("app.{i}"),
                label: format!("App {i}"),
                image: slint::Image::default(),
                kind: 0,
                running: false,
                tint: slint::Color::from_argb_u8(0, 0, 0, 0),
            })
            .collect();
        skin.set_tiles(&tiles);
        let _ = skin.set_focused(0, "app.0", "App 0");
        assert_eq!(skin.get_int_slot(5), 1, "row 0 is the shelf");

        // Six columns: Down moves by six, Right stops at the row's end.
        let (handled, cmds) = skin.dispatch_key("Down", true);
        assert!(handled);
        assert_eq!(cmds, vec![HostCommand::Focus(6)]);
        let _ = skin.set_focused(5, "app.5", "App 5");
        let (_, cmds) = skin.dispatch_key("Right", true);
        assert!(cmds.is_empty());
        // Down into the short last row lands on the last tile.
        let _ = skin.set_focused(17, "app.17", "App 17");
        let (_, cmds) = skin.dispatch_key("Down", true);
        assert_eq!(cmds, vec![HostCommand::Focus(19)]);
        // Three visible rows while scrolled.
        let _ = skin.set_focused(19, "app.19", "App 19");
        assert_eq!(skin.get_int_slot(5), 0);
        assert_eq!(skin.get_int_slot(4), 1);

        // Up from row 0 falls through to RustTV's top-right menu, which the
        // skin draws from `quick-actions`.
        let _ = skin.set_focused(2, "app.2", "App 2");
        assert_eq!(skin.get_int_slot(4), 0);
        let (handled, _) = skin.dispatch_key("Up", true);
        assert!(!handled);
        skin.set_quick_bar(&[(0, ""), (8, "HDMI"), (5, ""), (6, "")], 1);

        // OK on a tile falls through to RustTV (launch / long press).
        let (handled, _) = skin.dispatch_key("Ok", true);
        assert!(!handled);
    }
}

//! Draws the on-screen keyboard over other apps like `TextPromptView`
//! (quick.slint) draws it in the launcher: the typed text in a field, four
//! rows of ten keys and the bottom row (layer, space, delete, done). CPU
//! drawing with tiny-skia and ab_glyph into a buffer for `wl_shm`.

use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont as _, point};
use resvg::tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

use super::layout::{BOTTOM_KEYS, CHAR_ROWS, COLUMNS, TextPrompt};

/// The design width the sizes below are for (a 1080p screen).
const DESIGN_WIDTH: f32 = 1920.0;
const KEY_W: f32 = 100.0;
const KEY_H: f32 = 72.0;
const GAP: f32 = 10.0;
const RADIUS: f32 = 12.0;
const MARGIN: f32 = 32.0;
const FIELD_H: f32 = 80.0;
/// The bottom row's keys: x and width, as in quick.slint.
const BOTTOM_X: [f32; BOTTOM_KEYS] = [0.0, 228.0, 712.0, 914.0];
const BOTTOM_W: [f32; BOTTOM_KEYS] = [220.0, 476.0, 194.0, 194.0];

const PANEL: [u8; 4] = [0x0E, 0x10, 0x14, 0xF0];
const SURFACE: [u8; 4] = [0x1C, 0x1F, 0x26, 0xFF];
const TEXT: [u8; 4] = [0xF2, 0xF3, 0xF5, 0xFF];
const TEXT_DIM: [u8; 4] = [0xA4, 0xA9, 0xB4, 0xFF];
const DONE: [u8; 4] = [0x5E, 0x45, 0x1D, 0xFF];

/// Where fonts are looked for, first found wins.
const FONTS: [&str; 3] = [
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

/// Loads the first font of [`FONTS`] there is.
pub fn load_font() -> Option<FontVec> {
    FONTS.iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        FontVec::try_from_vec(bytes).ok()
    })
}

/// The keyboard's height for a surface `width` pixels wide.
pub fn height(width: u32) -> u32 {
    let u = width as f32 / DESIGN_WIDTH;
    let design = MARGIN + FIELD_H + MARGIN / 2.0 + 5.0 * KEY_H + 4.0 * GAP + MARGIN;
    (design * u).ceil() as u32
}

/// The key at (`x`, `y`) on a keyboard `width` pixels wide: row
/// (`CHAR_ROWS` is the bottom row) and column.
pub fn key_at(width: u32, x: f64, y: f64) -> Option<(usize, usize)> {
    let u = f64::from(width) / f64::from(DESIGN_WIDTH);
    let (x, y) = ((x / u) as f32, (y / u) as f32);
    let rows_w = COLUMNS as f32 * KEY_W + (COLUMNS - 1) as f32 * GAP;
    let left = (DESIGN_WIDTH - rows_w) / 2.0;
    let top = MARGIN + FIELD_H + MARGIN / 2.0;
    if x < left || x > left + rows_w || y < top {
        return None;
    }
    let row = ((y - top) / (KEY_H + GAP)) as usize;
    if row < CHAR_ROWS {
        let column = ((x - left) / (KEY_W + GAP)) as usize;
        return Some((row, column.min(COLUMNS - 1)));
    }
    if row > CHAR_ROWS {
        return None;
    }
    let column = (0..BOTTOM_KEYS)
        .find(|&key| x - left >= BOTTOM_X[key] && x - left <= BOTTOM_X[key] + BOTTOM_W[key])?;
    Some((CHAR_ROWS, column))
}

/// The keyboard as `wl_shm` ARGB8888 bytes (premultiplied, little endian:
/// B, G, R, A), `width` × `height` pixels.
pub fn draw(prompt: &TextPrompt, font: &FontVec, width: u32, height: u32) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(width, height)?;
    let u = width as f32 / DESIGN_WIDTH;
    pixmap.fill(color(PANEL));
    let rows_w = COLUMNS as f32 * KEY_W + (COLUMNS - 1) as f32 * GAP;
    let left = (DESIGN_WIDTH - rows_w) / 2.0;
    let mut y = MARGIN;

    // The field: what was typed (dots for a password), and a cursor.
    rounded(
        &mut pixmap,
        left * u,
        y * u,
        rows_w * u,
        FIELD_H * u,
        RADIUS * u,
        SURFACE,
    );
    let shown = prompt.shown() + "|";
    let size = 36.0 * u;
    let room = (rows_w - 56.0) * u;
    let tail = fitting_tail(font, &shown, size, room);
    text(
        &mut pixmap,
        font,
        tail,
        (left + 28.0) * u,
        (y + FIELD_H / 2.0) * u,
        size,
        TEXT,
        false,
    );
    if prompt.text().is_empty() {
        let hint = prompt.title();
        let x = (left + 28.0) * u + width_of(font, "|", size) + 8.0 * u;
        text(
            &mut pixmap,
            font,
            hint,
            x,
            (y + FIELD_H / 2.0) * u,
            size,
            TEXT_DIM,
            false,
        );
    }
    y += FIELD_H + MARGIN / 2.0;

    let (focus_row, focus_column) = prompt.focus();
    let labels = prompt.labels();
    for (index, label) in labels.iter().enumerate() {
        let (row, column) = (index / COLUMNS, index % COLUMNS);
        let focused = (row, column) == (focus_row, focus_column);
        let x = left + column as f32 * (KEY_W + GAP);
        let top = y + row as f32 * (KEY_H + GAP);
        let (fill, ink) = if focused {
            (TEXT, PANEL_INK)
        } else {
            (SURFACE, TEXT)
        };
        rounded(
            &mut pixmap,
            x * u,
            top * u,
            KEY_W * u,
            KEY_H * u,
            RADIUS * u,
            fill,
        );
        text(
            &mut pixmap,
            font,
            label,
            (x + KEY_W / 2.0) * u,
            (top + KEY_H / 2.0) * u,
            34.0 * u,
            ink,
            true,
        );
    }
    let top = y + CHAR_ROWS as f32 * (KEY_H + GAP);
    for key in 0..BOTTOM_KEYS {
        let focused = (focus_row, focus_column) == (CHAR_ROWS, key);
        let x = left + BOTTOM_X[key];
        let w = BOTTOM_W[key];
        let fill = if focused {
            TEXT
        } else if key == 3 {
            DONE
        } else {
            SURFACE
        };
        let ink = if focused { PANEL_INK } else { TEXT };
        rounded(
            &mut pixmap,
            x * u,
            top * u,
            w * u,
            KEY_H * u,
            RADIUS * u,
            fill,
        );
        let (cx, cy) = ((x + w / 2.0) * u, (top + KEY_H / 2.0) * u);
        match key {
            0 => {
                let label = if prompt.symbols() {
                    "abc"
                } else if prompt.capitals() {
                    "?123"
                } else {
                    "ABC"
                };
                text(&mut pixmap, font, label, cx, cy, 30.0 * u, ink, true);
            }
            1 => {}
            // Delete: a key with a cross; done: a tick (quick.slint's paths).
            2 => icon(
                &mut pixmap,
                cx,
                cy,
                u,
                ink,
                &[
                    &[
                        (9.0, 5.0),
                        (21.0, 5.0),
                        (21.0, 19.0),
                        (9.0, 19.0),
                        (2.0, 12.0),
                        (9.0, 5.0),
                    ],
                    &[(12.0, 9.0), (17.0, 15.0)],
                    &[(17.0, 9.0), (12.0, 15.0)],
                ],
            ),
            _ => icon(
                &mut pixmap,
                cx,
                cy,
                u,
                ink,
                &[&[(4.0, 12.0), (10.0, 18.0), (20.0, 6.0)]],
            ),
        }
    }
    // tiny-skia is RGBA; wl_shm's ARGB8888 is B, G, R, A in memory.
    let mut bytes = pixmap.take();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(bytes)
}

/// The dark ink on a focused (light) key.
const PANEL_INK: [u8; 4] = [0x0E, 0x10, 0x14, 0xFF];

fn color([red, green, blue, alpha]: [u8; 4]) -> Color {
    Color::from_rgba8(red, green, blue, alpha)
}

fn paint(rgba: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color(rgba));
    paint.anti_alias = true;
    paint
}

fn rounded(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, fill: [u8; 4]) {
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    if let Some(path) = path.finish() {
        pixmap.fill_path(
            &path,
            &paint(fill),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    } else if let Some(rect) = Rect::from_xywh(x, y, w, h) {
        pixmap.fill_rect(rect, &paint(fill), Transform::identity(), None);
    }
}

/// Strokes lines given on a 24 × 24 grid, drawn 40 design units wide
/// around (`cx`, `cy`).
fn icon(pixmap: &mut Pixmap, cx: f32, cy: f32, u: f32, ink: [u8; 4], lines: &[&[(f32, f32)]]) {
    let scale = 40.0 * u / 24.0;
    let mut path = PathBuilder::new();
    for line in lines {
        for (index, &(x, y)) in line.iter().enumerate() {
            let (x, y) = (cx + (x - 12.0) * scale, cy + (y - 12.0) * scale);
            if index == 0 {
                path.move_to(x, y);
            } else {
                path.line_to(x, y);
            }
        }
    }
    let Some(path) = path.finish() else {
        return;
    };
    let stroke = Stroke {
        width: 2.4 * u * 1.6,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint(ink), &stroke, Transform::identity(), None);
}

fn width_of(font: &FontVec, text: &str, size: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum()
}

/// The end of `text` that fits in `room` pixels.
fn fitting_tail<'a>(font: &FontVec, text: &'a str, size: f32, room: f32) -> &'a str {
    let mut tail = text;
    while width_of(font, tail, size) > room {
        let mut chars = tail.chars();
        chars.next();
        tail = chars.as_str();
    }
    tail
}

/// Draws `text` with its left edge (or centre, if `centred`) at `x` and its
/// middle at `mid`.
#[allow(clippy::too_many_arguments)]
fn text(
    pixmap: &mut Pixmap,
    font: &FontVec,
    text: &str,
    x: f32,
    mid: f32,
    size: f32,
    ink: [u8; 4],
    centred: bool,
) {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut pen = if centred {
        x - width_of(font, text, size) / 2.0
    } else {
        x
    };
    // Cap height ~ 0.7 of the size: centre that on `mid`.
    let baseline = mid + size * 0.35;
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    let data = pixmap.data_mut();
    for c in text.chars() {
        let id = font.glyph_id(c);
        let glyph = id.with_scale_and_position(PxScale::from(size), point(pen, baseline));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if px < 0 || py < 0 || px >= width || py >= height {
                return;
            }
            let index = (py as usize * width as usize + px as usize) * 4;
            let alpha = coverage.clamp(0.0, 1.0) * f32::from(ink[3]) / 255.0;
            for channel in 0..3 {
                let src = f32::from(ink[channel]) * alpha;
                let dst = f32::from(data[index + channel]);
                data[index + channel] = (src + dst * (1.0 - alpha)).round() as u8;
            }
            let dst = f32::from(data[index + 3]);
            data[index + 3] = (alpha * 255.0 + dst * (1.0 - alpha)).round() as u8;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_key_under_the_pointer() {
        let left = f64::from((DESIGN_WIDTH - (10.0 * KEY_W + 9.0 * GAP)) / 2.0);
        let top = f64::from(MARGIN + FIELD_H + MARGIN / 2.0);
        assert_eq!(key_at(1920, left + 5.0, top + 5.0), Some((0, 0)));
        assert_eq!(key_at(1920, left + 115.0, top + 90.0), Some((1, 1)));
        // Bottom row: space, then done; above the keys nothing.
        let bottom = top + 4.0 * f64::from(KEY_H + GAP) + 5.0;
        assert_eq!(key_at(1920, left + 300.0, bottom), Some((4, 1)));
        assert_eq!(key_at(1920, left + 1000.0, bottom), Some((4, 3)));
        assert_eq!(key_at(1920, left + 5.0, 10.0), None);
        // On a 960 pixel wide screen everything is half the size.
        assert_eq!(
            key_at(960, (left + 115.0) / 2.0, (top + 90.0) / 2.0),
            Some((1, 1))
        );
    }

    #[test]
    fn draws_a_full_width_keyboard_with_the_focused_key_light() {
        let Some(font) = load_font() else {
            return; // no system font on this machine
        };
        let prompt = TextPrompt::new("Search", false);
        let width = 1920;
        let height = height(width);
        assert!((500..700).contains(&height), "{height}");
        let bytes = draw(&prompt, &font, width, height).unwrap();
        assert_eq!(bytes.len(), (width * height * 4) as usize);
        // The focused key (q, row 1 column 0) is light; check its middle-left.
        let left = (DESIGN_WIDTH - (10.0 * KEY_W + 9.0 * GAP)) / 2.0;
        let y = MARGIN + FIELD_H + MARGIN / 2.0 + (KEY_H + GAP) + 6.0;
        let index = ((y as u32 * width + left as u32 + 6) * 4) as usize;
        assert!(
            bytes[index] > 0xE0 && bytes[index + 2] > 0xE0,
            "{:?}",
            &bytes[index..index + 4]
        );
    }
}

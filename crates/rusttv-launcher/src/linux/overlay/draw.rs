//! Draws the app overlay: the app in front dimmed, and a panel at the right
//! edge with the app's name and a row per setting, the focused row light
//! (as the launcher's menus). CPU drawing into a `wl_shm` buffer, with the
//! on-screen keyboard's helpers.

use ab_glyph::FontVec;
use resvg::tiny_skia::{Pixmap, Rect, Transform};

use super::panel::{Panel, Row};
use crate::linux::keyboard::draw::{
    PANEL, PANEL_INK, SURFACE, TEXT, TEXT_DIM, paint, rounded, text, width_of,
};
use crate::platform::OverlayTexts;

/// The design width the sizes below are for (a 1080p screen).
const DESIGN_WIDTH: f32 = 1920.0;
const PANEL_W: f32 = 640.0;
const PAD: f32 = 48.0;
const TITLE_MID: f32 = 104.0;
const TITLE_SIZE: f32 = 44.0;
const ROWS_TOP: f32 = 184.0;
const ROW_H: f32 = 112.0;
const GAP: f32 = 16.0;
const RADIUS: f32 = 20.0;
const LABEL_SIZE: f32 = 34.0;
const HINT_SIZE: f32 = 26.0;
/// From a row's right edge: the centres of ›, the value and ‹.
const PLUS_X: f32 = 44.0;
const VALUE_X: f32 = 140.0;
const MINUS_X: f32 = 236.0;
/// Half the width a click on ‹ or › counts in.
const ARROW_REACH: f32 = 44.0;

/// The app beside the panel, dimmed.
const DIM: [u8; 4] = [0x00, 0x00, 0x00, 0x70];

/// Where a point is on the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    /// Beside the panel, over the app.
    Outside,
    /// On the panel, not on a row.
    Panel,
    /// On row `index`.
    Row { index: usize, part: Part },
}

/// Which part of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The ‹ (smaller, quieter).
    Minus,
    /// The › (bigger, louder).
    Plus,
    /// Anywhere else.
    Body,
}

/// Pixels per design unit on a surface `width` pixels wide.
fn unit(width: u32) -> f32 {
    width as f32 / DESIGN_WIDTH
}

/// A row's left edge, top and width, in design units.
fn row_box(index: usize) -> (f32, f32, f32) {
    let left = DESIGN_WIDTH - PANEL_W + PAD;
    let top = ROWS_TOP + index as f32 * (ROW_H + GAP);
    (left, top, PANEL_W - 2.0 * PAD)
}

/// What is at (`x`, `y`) on an overlay `width` pixels wide with `rows` rows.
pub fn hit(width: u32, rows: usize, x: f64, y: f64) -> Hit {
    let u = f64::from(unit(width));
    let (x, y) = ((x / u) as f32, (y / u) as f32);
    if x < DESIGN_WIDTH - PANEL_W {
        return Hit::Outside;
    }
    for index in 0..rows {
        let (left, top, w) = row_box(index);
        if x >= left && x <= left + w && y >= top && y <= top + ROW_H {
            let right = left + w;
            let part = if (x - (right - MINUS_X)).abs() <= ARROW_REACH {
                Part::Minus
            } else if (x - (right - PLUS_X)).abs() <= ARROW_REACH {
                Part::Plus
            } else {
                Part::Body
            };
            return Hit::Row { index, part };
        }
    }
    Hit::Panel
}

/// The overlay as `wl_shm` ARGB8888 bytes (premultiplied, little endian:
/// B, G, R, A), `width` × `height` pixels.
pub fn draw(
    panel: &Panel,
    title: &str,
    texts: &OverlayTexts,
    font: &FontVec,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(width, height)?;
    let u = unit(width);
    pixmap.fill(crate::linux::keyboard::draw::color(DIM));
    let panel_left = (DESIGN_WIDTH - PANEL_W) * u;
    if let Some(rect) = Rect::from_xywh(panel_left, 0.0, PANEL_W * u, height as f32) {
        pixmap.fill_rect(rect, &paint(PANEL), Transform::identity(), None);
    }
    let room = PANEL_W - 2.0 * PAD;
    let left = DESIGN_WIDTH - PANEL_W + PAD;
    text(
        &mut pixmap,
        font,
        &fitting(font, title, TITLE_SIZE * u, room * u),
        left * u,
        TITLE_MID * u,
        TITLE_SIZE * u,
        TEXT,
        false,
    );
    let focus = panel.focus();
    for (index, row) in panel.rows().into_iter().enumerate() {
        let (x, top, w) = row_box(index);
        let focused = index == focus;
        let (fill, ink, dim) = if focused {
            (TEXT, PANEL_INK, PANEL_INK)
        } else {
            (SURFACE, TEXT, TEXT_DIM)
        };
        rounded(
            &mut pixmap,
            x * u,
            top * u,
            w * u,
            ROW_H * u,
            RADIUS * u,
            fill,
        );
        let mid = (top + ROW_H / 2.0) * u;
        let (label, value) = match row {
            Row::Size => (
                texts.size.as_str(),
                panel.size.map(|size| format!("{size} %")),
            ),
            Row::Volume => (
                texts.volume.as_str(),
                panel.volume.map(|volume| {
                    if volume.muted {
                        texts.muted.clone()
                    } else {
                        format!("{:.0} %", volume.level * 100.0)
                    }
                }),
            ),
            Row::Close => (texts.close.as_str(), None),
        };
        let label_room = if value.is_some() {
            w - MINUS_X - 56.0
        } else {
            w - 56.0
        };
        text(
            &mut pixmap,
            font,
            &fitting(font, label, LABEL_SIZE * u, label_room * u),
            (x + 28.0) * u,
            mid,
            LABEL_SIZE * u,
            ink,
            false,
        );
        if let Some(value) = value {
            let right = x + w;
            for (glyph, centre) in [("‹", MINUS_X), ("›", PLUS_X)] {
                text(
                    &mut pixmap,
                    font,
                    glyph,
                    (right - centre) * u,
                    mid,
                    TITLE_SIZE * u,
                    dim,
                    true,
                );
            }
            text(
                &mut pixmap,
                font,
                &value,
                (right - VALUE_X) * u,
                mid,
                LABEL_SIZE * u,
                ink,
                true,
            );
        }
    }
    if panel.resized() {
        text(
            &mut pixmap,
            font,
            &fitting(font, &texts.restarts, HINT_SIZE * u, room * u),
            left * u,
            height as f32 - 72.0 * u,
            HINT_SIZE * u,
            TEXT_DIM,
            false,
        );
    }
    // tiny-skia is RGBA; wl_shm's ARGB8888 is B, G, R, A in memory.
    let mut bytes = pixmap.take();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(bytes)
}

/// `text`, cut short with … where it is wider than `room` pixels.
fn fitting(font: &FontVec, text: &str, size: f32, room: f32) -> String {
    if width_of(font, text, size) <= room {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let cut: String = chars.iter().collect::<String>() + "…";
        if width_of(font, &cut, size) <= room {
            return cut;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Volume;

    #[test]
    fn finds_rows_arrows_and_the_app_beside() {
        let (left, top, w) = row_box(1);
        let right = f64::from(left + w);
        let y = f64::from(top + 10.0);
        assert_eq!(hit(1920, 3, 100.0, 500.0), Hit::Outside);
        assert_eq!(hit(1920, 3, 1800.0, 20.0), Hit::Panel);
        assert_eq!(
            hit(1920, 3, f64::from(left) + 20.0, y),
            Hit::Row {
                index: 1,
                part: Part::Body
            }
        );
        assert_eq!(
            hit(1920, 3, right - f64::from(MINUS_X), y),
            Hit::Row {
                index: 1,
                part: Part::Minus
            }
        );
        assert_eq!(
            hit(1920, 3, right - f64::from(PLUS_X), y),
            Hit::Row {
                index: 1,
                part: Part::Plus
            }
        );
        // Only rows there are count; half the size on a 960 pixel screen.
        assert_eq!(hit(1920, 1, f64::from(left) + 20.0, y), Hit::Panel);
        assert_eq!(
            hit(960, 3, (f64::from(left) + 20.0) / 2.0, y / 2.0),
            Hit::Row {
                index: 1,
                part: Part::Body
            }
        );
    }

    #[test]
    fn draws_the_dimmed_app_and_a_light_focused_row() {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            return; // no system font on this machine
        };
        let panel = Panel::new(
            Some(100),
            Some(Volume {
                level: 0.4,
                muted: false,
            }),
        );
        let (width, height) = (1920, 1080);
        let bytes = draw(
            &panel,
            "Dolphin",
            &OverlayTexts::default(),
            &font,
            width,
            height,
        )
        .unwrap();
        assert_eq!(bytes.len(), (width * height * 4) as usize);
        let at = |x: u32, y: u32| {
            let index = ((y * width + x) * 4) as usize;
            [
                bytes[index],
                bytes[index + 1],
                bytes[index + 2],
                bytes[index + 3],
            ]
        };
        // Beside the panel: see-through dark.
        assert_eq!(at(100, 100)[3], DIM[3]);
        // The first row is focused: light.
        let (left, top, _) = row_box(0);
        assert!(at(left as u32 + 8, top as u32 + 50)[0] > 0xE0);
        // The second is not.
        let (left, top, _) = row_box(1);
        assert!(at(left as u32 + 8, top as u32 + 50)[0] < 0x40);
    }

    #[test]
    fn long_titles_end_in_an_ellipsis() {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            return;
        };
        let cut = fitting(&font, "A very long application name indeed", 44.0, 200.0);
        assert!(cut.ends_with('…'), "{cut}");
        assert_eq!(fitting(&font, "Kodi", 44.0, 200.0), "Kodi");
    }
}

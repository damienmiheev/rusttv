//! Access to the TV: listing and launching apps.
//!
//! [`Platform`] is implemented by the Android bridge on a TV and by
//! [`FakePlatform`] for the desktop dev mode and tests.

#[cfg(any(test, feature = "test-util"))]
use std::collections::BTreeSet;

use material_colors::color::Rgb;
use material_colors::dislike::fix_if_disliked;
use material_colors::hct::Hct;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;

use crate::model::App;

/// Lists and launches apps on the device.
pub trait Platform {
    /// Launchable TV apps (Leanback launcher entries), without the launcher itself.
    fn list_apps(&self) -> Result<Vec<AppEntry>, PlatformError>;

    /// Asks the system to start the app with this component name.
    ///
    /// Returns once the request is handed over; the launcher learns that the
    /// app opened when it is paused.
    fn launch(&mut self, id: &str) -> Result<(), PlatformError>;

    /// Whether apps were installed, updated or removed since the last call;
    /// clears the flag. Lets the launcher skip re-reading the list on resume.
    fn take_apps_changed(&mut self) -> Result<bool, PlatformError>;
}

/// An app and the picture shown on its tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Identity and label.
    pub app: App,
    /// Banner, icon or nothing.
    pub artwork: Artwork,
}

/// What an app provides to draw its tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artwork {
    /// A 16:9 TV banner; fills the tile.
    Banner(Bitmap),
    /// Only a square app icon; drawn centred with the app name.
    Icon(Bitmap),
    /// Nothing to draw; the tile shows the name only.
    Missing,
}

/// An RGBA image, 8 bits per channel, rows top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Bitmap {
    /// Wraps RGBA bytes; `None` if their length is not `width * height * 4`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let expected = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        (rgba.len() == expected).then_some(Self {
            width,
            height,
            rgba,
        })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel bytes: R, G, B, A for each pixel.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Scales down to `width` × `height` by averaging the pixels each target pixel covers.
    ///
    /// Used for the hero backdrop. An empty source gives
    /// a transparent result.
    pub(crate) fn shrink(&self, width: u32, height: u32) -> Self {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            let rows = span(y, height, self.height);
            for x in 0..width {
                let columns = span(x, width, self.width);
                let mut sum = [0_u64; 4];
                for source_y in rows.clone() {
                    for source_x in columns.clone() {
                        let start =
                            (source_y as usize * self.width as usize + source_x as usize) * 4;
                        for (total, byte) in sum.iter_mut().zip(&self.rgba[start..start + 4]) {
                            *total += u64::from(*byte);
                        }
                    }
                }
                let count = (rows.len() * columns.len()).max(1) as u64;
                // An average of bytes fits in a byte.
                rgba.extend(sum.map(|total| (total / count) as u8));
            }
        }
        Self {
            width,
            height,
            rgba,
        }
    }
}

impl Bitmap {
    /// Gaussian-like blur: three box-blur passes of `radius` in each direction,
    /// with edge pixels repeated beyond the border.
    ///
    /// Used for the hero backdrop, on a small image, so
    /// the plain per-pixel loops are cheap enough.
    pub(crate) fn blur(&self, radius: u32) -> Self {
        let mut blurred = self.clone();
        for _ in 0..3 {
            blurred = blurred.box_blur(radius, true).box_blur(radius, false);
        }
        blurred
    }

    /// One box-blur pass along rows (`horizontal`) or columns.
    fn box_blur(&self, radius: u32, horizontal: bool) -> Self {
        let (width, height) = (self.width as usize, self.height as usize);
        let (lines, length) = if horizontal {
            (height, width)
        } else {
            (width, height)
        };
        let index = |line: usize, position: usize| {
            if horizontal {
                (line * width + position) * 4
            } else {
                (position * width + line) * 4
            }
        };
        let radius = radius as usize;
        let count = (2 * radius + 1) as u32;
        let mut rgba = vec![0; self.rgba.len()];
        for line in 0..lines {
            for position in 0..length {
                let mut sum = [0_u32; 4];
                for offset in 0..=2 * radius {
                    // Clamp to the edge: (position + offset - radius) within 0..length.
                    let source = (position + offset).saturating_sub(radius).min(length - 1);
                    let start = index(line, source);
                    for (total, byte) in sum.iter_mut().zip(&self.rgba[start..start + 4]) {
                        *total += u32::from(*byte);
                    }
                }
                let start = index(line, position);
                for (target, total) in rgba[start..start + 4].iter_mut().zip(sum) {
                    // An average of bytes fits in a byte.
                    *target = (total / count) as u8;
                }
            }
        }
        Self {
            width: self.width,
            height: self.height,
            rgba,
        }
    }

    /// Turns artwork into a soft glow of its main colour on a dark base.
    ///
    /// The main colour is picked the way Android picks wallpaper colours
    /// (Material's quantizer and `Score`, with olive tones fixed by its
    /// `DislikeAnalyzer`). The glow is that hue at HCT tone `glow_tone` and the
    /// base the same hue, barely tinted, at `base_tone`; HCT keeps every hue
    /// equally dark. The glow sits where pixels of that hue are, blurred with
    /// `radius` (see [`Bitmap::blur`]), at `gain` times their blurred weight.
    /// Artwork without a usable colour gives a flat neutral base.
    pub(crate) fn glow(&self, radius: u32, gain: u32, base_tone: f64, glow_tone: f64) -> Self {
        /// Chroma of the glow; Material's target chroma for source colours.
        const GLOW_CHROMA: f64 = 48.0;
        /// Chroma of the tinted base, like Material's dark surfaces.
        const BASE_CHROMA: f64 = 6.0;
        /// Pixels this close in hue (degrees) and at least this colourful shape the glow.
        const HUE_RANGE: f64 = 30.0;
        const MIN_CHROMA: f64 = 10.0;

        let opaque: Vec<Rgb> = self
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] >= 128)
            .map(|&[red, green, blue, _]| Rgb::new(red, green, blue))
            .collect();
        // Black is never a scored colour (no chroma), so it marks "none".
        let none = Rgb::new(0, 0, 0);
        let quantized = QuantizerCelebi::quantize(&opaque, 128);
        let accent = Score::score(&quantized.color_to_count, Some(1), Some(none), Some(true))
            .first()
            .copied()
            .filter(|&colour| colour != none)
            .map(|colour| fix_if_disliked(Hct::new(colour)));

        let (base, glow) = match &accent {
            Some(accent) => (
                Hct::from(accent.get_hue(), BASE_CHROMA, base_tone),
                Hct::from(
                    accent.get_hue(),
                    accent.get_chroma().min(GLOW_CHROMA),
                    glow_tone,
                ),
            ),
            None => (
                Hct::from(0.0, 0.0, base_tone),
                Hct::from(0.0, 0.0, base_tone),
            ),
        };
        let (base, glow) = (Rgb::from(base), Rgb::from(glow));

        // Where the accent hue is: chroma-weighted, in the alpha channel, blurred.
        let mut mask = Self {
            width: self.width,
            height: self.height,
            rgba: vec![0; self.rgba.len()],
        };
        if let Some(accent) = &accent {
            for (target, &[red, green, blue, alpha]) in mask
                .rgba
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(self.rgba.as_chunks::<4>().0)
            {
                let pixel = Hct::new(Rgb::new(red, green, blue));
                let distance = (pixel.get_hue() - accent.get_hue()).abs();
                let distance = distance.min(360.0 - distance);
                if alpha >= 128 && pixel.get_chroma() >= MIN_CHROMA && distance <= HUE_RANGE {
                    // A share of 255, so a byte.
                    target[3] = ((pixel.get_chroma() / GLOW_CHROMA).min(1.0) * 255.0) as u8;
                }
            }
        }

        let mut rgba = mask.blur(radius).rgba;
        let pairs = [
            (base.red, glow.red),
            (base.green, glow.green),
            (base.blue, glow.blue),
        ];
        for [red, green, blue, alpha] in rgba.as_chunks_mut::<4>().0 {
            let strength = (u32::from(*alpha) * gain).min(255);
            for (channel, (from, to)) in [red, green, blue].into_iter().zip(pairs) {
                // A mix of two bytes is a byte.
                *channel =
                    ((u32::from(from) * (255 - strength) + u32::from(to) * strength) / 255) as u8;
            }
            *alpha = u8::MAX;
        }
        Self {
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

/// Source pixels covered by target pixel `index` of `target` when scaling from `source`.
///
/// At least one pixel wide unless the source is empty.
fn span(index: u32, target: u32, source: u32) -> std::ops::Range<u32> {
    let start = (u64::from(index) * u64::from(source) / u64::from(target)) as u32;
    let end = (u64::from(index + 1) * u64::from(source) / u64::from(target)) as u32;
    start..end.max((start + 1).min(source))
}

impl std::fmt::Debug for Bitmap {
    // Size only: the pixel bytes would flood logs.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}x{})", self.width, self.height)
    }
}

/// Listing or launching apps failed.
///
/// Follows M-ERRORS-CANONICAL-STRUCTS like `rusttv_core::AdbError`, without a
/// `Backtrace`: failures here are runtime conditions (an app was just
/// uninstalled, a Java exception), shown to the user as "couldn't open".
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PlatformError {
    kind: ErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error("app `{0}` has no TV launch activity")]
    NotLaunchable(String),
    #[error("{operation} failed: {message}")]
    Call {
        operation: &'static str,
        message: String,
    },
}

impl PlatformError {
    /// The app is unknown or cannot be started.
    pub fn not_launchable(id: &str) -> Self {
        Self {
            kind: ErrorKind::NotLaunchable(id.to_owned()),
        }
    }

    /// A call into the platform failed, e.g. with a Java exception.
    pub fn call(operation: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Call {
                operation,
                message: message.into(),
            },
        }
    }

    /// Whether the app is unknown or cannot be started.
    pub fn is_not_launchable(&self) -> bool {
        matches!(self.kind, ErrorKind::NotLaunchable(_))
    }
}

/// Size of generated fake banners.
///
/// The Leanback banner size (320×180, 16:9) that the Android bridge also
/// draws at, so the desktop dev mode scales banners the same way a TV does.
#[cfg(any(test, feature = "test-util"))]
const BANNER_SIZE: (u32, u32) = (320, 180);

/// Size of generated fake icons; a common launcher icon size.
#[cfg(any(test, feature = "test-util"))]
const ICON_SIZE: u32 = 96;

/// Colour pairs for generated artwork, picked by a hash of the app id.
#[cfg(any(test, feature = "test-util"))]
const FAKE_COLOURS: [([u8; 3], [u8; 3]); 6] = [
    ([0xE2, 0x4A, 0x3B], [0x7A, 0x1F, 0x4F]),
    ([0x2F, 0x80, 0xED], [0x1B, 0x2A, 0x6B]),
    ([0x27, 0xAE, 0x60], [0x0F, 0x4C, 0x3A]),
    ([0xF2, 0xC9, 0x4C], [0xC0, 0x5A, 0x1A]),
    ([0x9B, 0x51, 0xE0], [0x3B, 0x1E, 0x6E]),
    ([0x56, 0xCC, 0xF2], [0x1D, 0x5F, 0x7A]),
];

/// A [`Platform`] with made-up apps and generated artwork, for dev mode and tests.
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Clone, Default)]
pub struct FakePlatform {
    apps: Vec<AppEntry>,
    launched: Vec<String>,
    failing: BTreeSet<String>,
    apps_changed: bool,
}

#[cfg(any(test, feature = "test-util"))]
impl FakePlatform {
    /// A platform with these apps, each with a generated banner.
    pub fn new(apps: impl IntoIterator<Item = App>) -> Self {
        Self {
            apps: apps
                .into_iter()
                .map(|app| AppEntry {
                    artwork: Artwork::Banner(fake_banner(&app.id)),
                    app,
                })
                .collect(),
            ..Self::default()
        }
    }

    /// Fourteen generic apps (three rows with About); two have only an icon, one has no artwork.
    pub fn sample() -> Self {
        const NAMES: [&str; 14] = [
            "Movies", "Series", "Music", "News", "Weather", "Photos", "Podcasts", "Sports", "Kids",
            "Fitness", "Radio", "Games", "Browser", "Files",
        ];
        let mut platform = Self::new(NAMES.iter().map(|name| App {
            id: format!("ai.rusttv.fake.{}/.Main", name.to_lowercase()),
            label: (*name).to_owned(),
        }));
        for entry in &mut platform.apps {
            match entry.app.label.as_str() {
                "Browser" | "Files" => entry.artwork = Artwork::Icon(fake_icon(&entry.app.id)),
                "Radio" => entry.artwork = Artwork::Missing,
                _ => {}
            }
        }
        platform
    }

    /// Makes every later launch of `id` fail, as a broken app would.
    pub fn fail_launches_of(&mut self, id: &str) {
        self.failing.insert(id.to_owned());
    }

    /// Reports a changed app list on the next [`Platform::take_apps_changed`].
    pub fn mark_apps_changed(&mut self) {
        self.apps_changed = true;
    }

    /// Component names of successful launches, oldest first.
    pub fn launched(&self) -> &[String] {
        &self.launched
    }
}

#[cfg(any(test, feature = "test-util"))]
impl Platform for FakePlatform {
    fn list_apps(&self) -> Result<Vec<AppEntry>, PlatformError> {
        Ok(self.apps.clone())
    }

    fn launch(&mut self, id: &str) -> Result<(), PlatformError> {
        if !self.apps.iter().any(|entry| entry.app.id == id) {
            return Err(PlatformError::not_launchable(id));
        }
        if self.failing.contains(id) {
            return Err(PlatformError::call(
                "launch",
                format!("{id} crashed (fake)"),
            ));
        }
        self.launched.push(id.to_owned());
        Ok(())
    }

    fn take_apps_changed(&mut self) -> Result<bool, PlatformError> {
        Ok(std::mem::take(&mut self.apps_changed))
    }
}

#[cfg(any(test, feature = "test-util"))]
fn fake_banner(id: &str) -> Bitmap {
    gradient(BANNER_SIZE.0, BANNER_SIZE.1, FAKE_COLOURS[colour_index(id)])
}

#[cfg(any(test, feature = "test-util"))]
fn fake_icon(id: &str) -> Bitmap {
    gradient(ICON_SIZE, ICON_SIZE, FAKE_COLOURS[colour_index(id)])
}

/// Stable index into [`FAKE_COLOURS`] for an id (FNV-1a hash).
#[cfg(any(test, feature = "test-util"))]
fn colour_index(id: &str) -> usize {
    let hash = id.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    // The remainder is below FAKE_COLOURS.len(), so it fits in usize.
    (hash % FAKE_COLOURS.len() as u64) as usize
}

/// An opaque diagonal gradient from the top-left colour to the bottom-right one.
#[cfg(any(test, feature = "test-util"))]
fn gradient(width: u32, height: u32, (from, to): ([u8; 3], [u8; 3])) -> Bitmap {
    let span = (width + height).saturating_sub(2).max(1);
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let t = x + y;
            for (start, end) in from.iter().zip(&to) {
                let mixed = (u32::from(*start) * (span - t) + u32::from(*end) * t) / span;
                // A weighted average of two bytes is at most 255.
                rgba.push(mixed as u8);
            }
            rgba.push(u8::MAX);
        }
    }
    Bitmap {
        width,
        height,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Home, Order, TileId};

    fn app(id: &str) -> App {
        App {
            id: id.to_owned(),
            label: id.to_uppercase(),
        }
    }

    fn pixel(bitmap: &Bitmap, x: u32, y: u32) -> &[u8] {
        let start = ((y * bitmap.width() + x) * 4) as usize;
        &bitmap.rgba()[start..start + 4]
    }

    #[test]
    fn bitmaps_need_exactly_four_bytes_per_pixel() {
        assert!(Bitmap::new(2, 3, vec![0; 24]).is_some());
        assert!(Bitmap::new(2, 3, vec![0; 23]).is_none());
        assert!(Bitmap::new(2, 3, vec![0; 25]).is_none());
        assert!(Bitmap::new(0, 0, Vec::new()).is_some());
        assert!(Bitmap::new(u32::MAX, u32::MAX, Vec::new()).is_none());
    }

    #[test]
    fn shrinking_averages_the_covered_pixels() {
        // 4×2: left half black, right half white, all opaque.
        let mut rgba = Vec::new();
        for _ in 0..2 {
            for value in [0, 0, 255, 255] {
                rgba.extend([value, value, value, 255]);
            }
        }
        let bitmap = Bitmap::new(4, 2, rgba).unwrap();

        let halves = bitmap.shrink(2, 1);
        assert_eq!(halves.rgba(), [0, 0, 0, 255, 255, 255, 255, 255]);

        let one = bitmap.shrink(1, 1);
        assert_eq!(one.rgba(), [127, 127, 127, 255]);
    }

    #[test]
    fn shrinking_a_fake_banner_to_the_backdrop_size_keeps_its_colours() {
        let banner = fake_banner("a");
        let backdrop = banner.shrink(32, 18);
        assert_eq!((backdrop.width(), backdrop.height()), (32, 18));
        let (from, to) = FAKE_COLOURS[colour_index("a")];
        // Corners stay close to the gradient's end colours.
        for (got, want) in pixel(&backdrop, 0, 0).iter().zip(from) {
            assert!(got.abs_diff(want) <= 12, "{got} vs {want}");
        }
        for (got, want) in pixel(&backdrop, 31, 17).iter().zip(to) {
            assert!(got.abs_diff(want) <= 12, "{got} vs {want}");
        }
    }

    #[test]
    fn shrinking_an_empty_bitmap_gives_transparent_pixels() {
        let empty = Bitmap::new(0, 0, Vec::new()).unwrap();
        assert_eq!(empty.shrink(2, 1).rgba(), [0; 8]);
    }

    #[test]
    fn blurring_spreads_a_dot_and_keeps_flat_areas() {
        // 9×1 black, with one white pixel in the middle.
        let mut rgba = [0, 0, 0, 255].repeat(9);
        rgba[16..20].copy_from_slice(&[255, 255, 255, 255]);
        let blurred = Bitmap::new(9, 1, rgba).unwrap().blur(1);
        let red: Vec<u8> = blurred.rgba().chunks(4).map(|pixel| pixel[0]).collect();
        // Peak in the middle, falling off symmetrically, no longer a hard dot.
        assert!(red[4] < 255 && red[4] > red[3] && red[3] > red[2]);
        assert_eq!(red[3], red[5]);
        assert!(blurred.rgba().chunks(4).all(|pixel| pixel[3] == 255));

        let flat = Bitmap::new(3, 2, [10, 20, 30, 255].repeat(6)).unwrap();
        assert_eq!(flat.blur(4), flat);
    }

    const BASE_TONE: f64 = 12.0;
    const GLOW_TONE: f64 = 30.0;

    fn is_grey(pixel: &[u8]) -> bool {
        pixel[0] == pixel[1] && pixel[1] == pixel[2]
    }

    #[test]
    fn grey_and_white_artwork_gives_a_flat_neutral_base() {
        // White with grey "text" and slightly tinted antialiasing.
        let rgba = [
            [250, 250, 250, 255],
            [90, 90, 90, 255],
            [200, 196, 192, 255],
        ]
        .repeat(4)
        .concat();
        let glow = Bitmap::new(4, 3, rgba)
            .unwrap()
            .glow(2, 4, BASE_TONE, GLOW_TONE);
        let first = glow.rgba()[..4].to_vec();
        assert!(is_grey(&first) && first[0] < 40, "{first:?}");
        assert!(glow.rgba().chunks(4).all(|pixel| pixel == first));
    }

    #[test]
    fn a_small_colourful_logo_glows_in_its_colour() {
        // 20×1 white, two red pixels in the middle.
        let mut rgba = [255, 255, 255, 255].repeat(20);
        rgba[36..44].copy_from_slice(&[230, 20, 20, 255, 230, 20, 20, 255]);
        let glow = Bitmap::new(20, 1, rgba)
            .unwrap()
            .glow(2, 4, BASE_TONE, GLOW_TONE);
        let pixels: Vec<&[u8]> = glow.rgba().chunks(4).collect();
        // Red at the logo, fading towards the base at the ends; dark, never bright.
        assert!(pixels[9][0] > pixels[9][1] + 40 && pixels[9][0] > pixels[9][2] + 40);
        assert!(pixels[9][0] > pixels[5][0] && pixels[5][0] >= pixels[0][0]);
        assert!(pixels.iter().all(|pixel| pixel[0] < 140 && pixel[3] == 255));
    }

    #[test]
    fn a_multicoloured_logo_glows_in_one_colour() {
        // Mostly green with a little blue: the glow is green, the blue adds nothing.
        let green = [20, 170, 60, 255];
        let blue = [30, 80, 230, 255];
        let pixels = [green; 9].into_iter().chain([blue; 3]).collect::<Vec<_>>();
        let glow = Bitmap::new(12, 1, pixels.concat())
            .unwrap()
            .glow(3, 4, BASE_TONE, GLOW_TONE);
        for pixel in glow.rgba().chunks(4) {
            assert!(pixel[1] > pixel[0] && pixel[1] > pixel[2], "{pixel:?}");
        }
    }

    #[test]
    fn transparent_pixels_carry_no_colour() {
        let glow = Bitmap::new(2, 1, vec![255, 0, 0, 0, 255, 0, 0, 0])
            .unwrap()
            .glow(0, 4, BASE_TONE, GLOW_TONE);
        assert!(glow.rgba().chunks(4).all(is_grey));
    }

    #[test]
    fn bitmap_debug_shows_the_size_not_the_pixels() {
        let bitmap = Bitmap::new(2, 1, vec![7; 8]).unwrap();
        assert_eq!(format!("{bitmap:?}"), "Bitmap(2x1)");
    }

    #[test]
    fn fake_banners_are_opaque_16_by_9_gradients() {
        let platform = FakePlatform::new([app("a")]);
        let apps = platform.list_apps().unwrap();
        let Artwork::Banner(banner) = &apps[0].artwork else {
            panic!("expected a banner, got {:?}", apps[0].artwork);
        };
        assert_eq!((banner.width(), banner.height()), (320, 180));
        assert!(banner.rgba().chunks(4).all(|pixel| pixel[3] == u8::MAX));
        let (from, to) = FAKE_COLOURS[colour_index("a")];
        assert_eq!(pixel(banner, 0, 0)[..3], from);
        assert_eq!(pixel(banner, 319, 179)[..3], to);
    }

    #[test]
    fn fake_artwork_is_stable_per_app() {
        let first = FakePlatform::new([app("a"), app("b")]).list_apps().unwrap();
        let second = FakePlatform::new([app("a"), app("b")]).list_apps().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn listing_keeps_the_given_apps_in_order() {
        let platform = FakePlatform::new([app("b"), app("a")]);
        let ids: Vec<String> = platform
            .list_apps()
            .unwrap()
            .into_iter()
            .map(|entry| entry.app.id)
            .collect();
        assert_eq!(ids, ["b", "a"]);
    }

    #[test]
    fn launches_are_recorded() {
        let mut platform = FakePlatform::new([app("a"), app("b")]);
        platform.launch("b").unwrap();
        platform.launch("a").unwrap();
        assert_eq!(platform.launched(), ["b", "a"]);
    }

    #[test]
    fn launching_an_unknown_app_fails() {
        let mut platform = FakePlatform::new([app("a")]);
        let error = platform.launch("gone").unwrap_err();
        assert!(error.is_not_launchable());
        assert!(error.to_string().contains("`gone`"));
        assert!(platform.launched().is_empty());
    }

    #[test]
    fn a_failing_app_fails_to_launch() {
        let mut platform = FakePlatform::new([app("a")]);
        platform.fail_launches_of("a");
        let error = platform.launch("a").unwrap_err();
        assert!(!error.is_not_launchable());
        assert!(error.to_string().starts_with("launch failed"));
        assert!(platform.launched().is_empty());
    }

    #[test]
    fn a_changed_app_list_is_reported_once() {
        let mut platform = FakePlatform::new([app("a")]);
        assert!(!platform.take_apps_changed().unwrap());
        platform.mark_apps_changed();
        assert!(platform.take_apps_changed().unwrap());
        assert!(!platform.take_apps_changed().unwrap());
    }

    #[test]
    fn the_sample_covers_every_kind_of_artwork() {
        let apps = FakePlatform::sample().list_apps().unwrap();
        let count =
            |wanted: fn(&Artwork) -> bool| apps.iter().filter(|e| wanted(&e.artwork)).count();
        assert_eq!(count(|a| matches!(a, Artwork::Banner(_))), 11);
        assert_eq!(count(|a| matches!(a, Artwork::Icon(_))), 2);
        assert_eq!(count(|a| matches!(a, Artwork::Missing)), 1);
    }

    #[test]
    fn every_sample_app_gets_a_tile_and_launches() {
        let mut platform = FakePlatform::sample();
        let entries = platform.list_apps().unwrap();
        let apps: Vec<App> = entries.iter().map(|entry| entry.app.clone()).collect();
        let mut home = Home::new(Order::default(), false);
        home.set_apps(&apps);

        // 14 apps + About: three rows, the last one short.
        assert_eq!(home.tiles().len(), 15);
        for entry in &entries {
            assert!(home.tiles().contains(&TileId::App(entry.app.id.clone())));
            platform.launch(&entry.app.id).unwrap();
        }
        assert_eq!(platform.launched().len(), 14);
    }
}

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

    /// The artwork of an app that [`Platform::list_apps`] returned as
    /// [`Artwork::Later`]. Called after the first frame, a few apps at a time.
    fn artwork(&self, id: &str) -> Result<Artwork, PlatformError>;

    /// The third-party licence notices shipped with the launcher, as plain text.
    ///
    /// Large (~560 KB on Android), so read only when the About page opens.
    fn notices(&self) -> Result<String, PlatformError>;
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
    /// Not loaded yet: fetch it with [`Platform::artwork`]. Lets the home
    /// screen appear before every banner has been drawn (~230 ms for 31 apps
    /// on the Philips 65OLED818).
    Later,
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

/// The hero backdrop for an app: a soft glow of its main colour on a dark base.
///
/// Drawn by Slint as a radial gradient, so no backdrop bitmap is kept: a
/// full-screen bitmap per app cost up to ~20 MB on the Philips TV.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Glow {
    /// The dark background, the main colour's hue barely tinted.
    pub base: [u8; 3],
    /// The glow's colour; equal to `base` when the artwork has no usable colour.
    pub colour: [u8; 3],
    /// Where the main colour sits in the artwork, 0..1 from the left and top.
    pub x: f32,
    pub y: f32,
    /// How strongly the glow shows, 0..1; 0 when there is no main colour.
    pub strength: f32,
}

impl Bitmap {
    /// The hero backdrop for this artwork (see [`Glow`]).
    ///
    /// The main colour is picked the way Android picks wallpaper colours
    /// (Material's quantizer and `Score`, with olive tones fixed by its
    /// `DislikeAnalyzer`). The glow is that hue at HCT tone `glow_tone` and the
    /// base the same hue, barely tinted, at `base_tone`; HCT keeps every hue
    /// equally dark. The glow is centred where pixels of that hue are, and is
    /// stronger the more of the artwork they cover. Artwork without a usable
    /// colour gives a flat neutral base. Meant for a small copy (see
    /// [`Bitmap::shrink`]): it converts every pixel to HCT.
    pub(crate) fn glow(&self, base_tone: f64, glow_tone: f64) -> Glow {
        /// Chroma of the glow; Material's target chroma for source colours.
        const GLOW_CHROMA: f64 = 48.0;
        /// Chroma of the tinted base, like Material's dark surfaces.
        const BASE_CHROMA: f64 = 6.0;
        /// Pixels this close in hue (degrees) and at least this colourful place the glow.
        const HUE_RANGE: f64 = 30.0;
        const MIN_CHROMA: f64 = 10.0;
        /// Even a small logo glows this strongly; full strength from 1/8 coverage.
        const MIN_STRENGTH: f32 = 0.55;

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
        let rgb = |hct: Hct| {
            let rgb = Rgb::from(hct);
            [rgb.red, rgb.green, rgb.blue]
        };

        let Some(accent) = accent else {
            let base = rgb(Hct::from(0.0, 0.0, base_tone));
            return Glow {
                base,
                colour: base,
                x: 0.5,
                y: 0.5,
                strength: 0.0,
            };
        };

        // Where the accent hue is: a chroma-weighted centroid and coverage.
        let (mut total, mut sum_x, mut sum_y) = (0.0_f64, 0.0_f64, 0.0_f64);
        let width = self.width.max(1) as usize;
        for (index, &[red, green, blue, alpha]) in self.rgba.as_chunks::<4>().0.iter().enumerate() {
            let pixel = Hct::new(Rgb::new(red, green, blue));
            let distance = (pixel.get_hue() - accent.get_hue()).abs();
            let distance = distance.min(360.0 - distance);
            if alpha >= 128 && pixel.get_chroma() >= MIN_CHROMA && distance <= HUE_RANGE {
                let weight = (pixel.get_chroma() / GLOW_CHROMA).min(1.0);
                total += weight;
                sum_x += weight * ((index % width) as f64 + 0.5);
                sum_y += weight * ((index / width) as f64 + 0.5);
            }
        }
        let pixels = (self.width as f64 * self.height as f64).max(1.0);
        let (x, y) = if total > 0.0 {
            (
                sum_x / total / self.width.max(1) as f64,
                sum_y / total / self.height.max(1) as f64,
            )
        } else {
            (0.5, 0.5)
        };
        let coverage = (total / pixels) as f32;
        Glow {
            base: rgb(Hct::from(accent.get_hue(), BASE_CHROMA, base_tone)),
            colour: rgb(Hct::from(
                accent.get_hue(),
                accent.get_chroma().min(GLOW_CHROMA),
                glow_tone,
            )),
            x: x as f32,
            y: y as f32,
            strength: MIN_STRENGTH + (1.0 - MIN_STRENGTH) * (coverage * 8.0).min(1.0),
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

/// Numbered lines in the fake notices: enough to scroll through several screens.
#[cfg(any(test, feature = "test-util"))]
const FAKE_NOTICE_LINES: usize = 200;

/// A [`Platform`] with made-up apps and generated artwork, for dev mode and tests.
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Clone, Default)]
pub struct FakePlatform {
    apps: Vec<AppEntry>,
    launched: Vec<String>,
    failing: BTreeSet<String>,
    apps_changed: bool,
    /// List artwork as [`Artwork::Later`], like the Android bridge.
    lazy: bool,
    artwork_calls: std::cell::Cell<usize>,
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

    /// Lists artwork as [`Artwork::Later`], to be fetched with [`Platform::artwork`].
    #[must_use]
    pub fn with_lazy_artwork(mut self) -> Self {
        self.lazy = true;
        self
    }

    /// How many times [`Platform::artwork`] was called.
    pub fn artwork_calls(&self) -> usize {
        self.artwork_calls.get()
    }

    /// Reports a changed app list on the next [`Platform::take_apps_changed`].
    pub fn mark_apps_changed(&mut self) {
        self.apps_changed = true;
    }

    /// Uninstalls `id` and reports the change, as Android would.
    pub fn remove_app(&mut self, id: &str) {
        self.apps.retain(|entry| entry.app.id != id);
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
        let mut apps = self.apps.clone();
        if self.lazy {
            for entry in &mut apps {
                entry.artwork = Artwork::Later;
            }
        }
        Ok(apps)
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

    fn artwork(&self, id: &str) -> Result<Artwork, PlatformError> {
        self.artwork_calls.set(self.artwork_calls.get() + 1);
        self.apps
            .iter()
            .find(|entry| entry.app.id == id)
            .map(|entry| entry.artwork.clone())
            .ok_or_else(|| PlatformError::not_launchable(id))
    }

    /// A stand-in: the real notices are generated only for the APK.
    fn notices(&self) -> Result<String, PlatformError> {
        let mut text = String::from(
            "THIRD-PARTY SOFTWARE NOTICES (stand-in)\n\n\
             The Android app shows the notices generated by scripts/licenses.sh.\n",
        );
        for line in 1..=FAKE_NOTICE_LINES {
            text.push_str(&format!("\nStand-in notice line {line}."));
        }
        Ok(text)
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

    const BASE_TONE: f64 = 12.0;
    const GLOW_TONE: f64 = 30.0;

    fn is_grey(rgb: [u8; 3]) -> bool {
        rgb[0] == rgb[1] && rgb[1] == rgb[2]
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
        let glow = Bitmap::new(4, 3, rgba).unwrap().glow(BASE_TONE, GLOW_TONE);
        assert!(is_grey(glow.base) && glow.base[0] < 40, "{glow:?}");
        assert_eq!(glow.strength, 0.0);
    }

    #[test]
    fn a_small_colourful_logo_glows_in_its_colour_where_it_is() {
        // 20×1 white, two red pixels near the left.
        let mut rgba = [255, 255, 255, 255].repeat(20);
        rgba[8..16].copy_from_slice(&[230, 20, 20, 255, 230, 20, 20, 255]);
        let glow = Bitmap::new(20, 1, rgba).unwrap().glow(BASE_TONE, GLOW_TONE);
        let [red, green, blue] = glow.colour;
        assert!(red > green + 40 && red > blue + 40, "{glow:?}");
        assert!(red < 140, "dark enough for white text: {glow:?}");
        // Pixels 2 and 3 of 20: centre at 3/20.
        assert!((glow.x - 0.15).abs() < 0.01, "{glow:?}");
        assert!(glow.strength > 0.5 && glow.strength <= 1.0, "{glow:?}");
    }

    #[test]
    fn a_multicoloured_logo_glows_in_one_colour() {
        // Mostly green with a little blue: the glow is green.
        let green = [20, 170, 60, 255];
        let blue = [30, 80, 230, 255];
        let pixels = [green; 9].into_iter().chain([blue; 3]).collect::<Vec<_>>();
        let glow = Bitmap::new(12, 1, pixels.concat())
            .unwrap()
            .glow(BASE_TONE, GLOW_TONE);
        let [red, green, blue] = glow.colour;
        assert!(green > red && green > blue, "{glow:?}");
        // Centred on the green part (pixels 0..9 of 12).
        assert!(glow.x < 0.5, "{glow:?}");
    }

    #[test]
    fn transparent_pixels_carry_no_colour() {
        let glow = Bitmap::new(2, 1, vec![255, 0, 0, 0, 255, 0, 0, 0])
            .unwrap()
            .glow(BASE_TONE, GLOW_TONE);
        assert_eq!(glow.strength, 0.0);
        assert!(is_grey(glow.base));
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
    fn fake_notices_are_a_long_stand_in() {
        let text = FakePlatform::new([]).notices().unwrap();
        assert!(text.starts_with("THIRD-PARTY SOFTWARE NOTICES (stand-in)"));
        assert!(text.ends_with("Stand-in notice line 200."));
        assert!(text.lines().count() > 200);
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

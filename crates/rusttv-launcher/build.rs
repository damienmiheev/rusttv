//! Compiles the Slint UI into Rust code, with its translations bundled in, and
//! decodes the built-in tiles' banners into raw pixels.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

/// The banners of the launcher's own tiles, rendered by scripts/brand/render.sh.
const TILES: [&str; 3] = ["about", "settings", "hidden"];

/// Their size: an Android TV banner (xhdpi), like the apps' banners.
const TILE_SIZE: (u32, u32) = (320, 180);

#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking; Slint prints the diagnostics"
)]
fn main() {
    // `translations/<lang>/LC_MESSAGES/rusttv-launcher.po`, compiled into the
    // library, so the APK ships no separate files. No default context: each
    // text is translated once, whichever component shows it.
    let config = CompilerConfiguration::new()
        .with_bundled_translations("translations")
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/home.slint", config)
        .expect("ui/home.slint should compile");

    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    for name in TILES {
        let source = format!("assets/tiles/{name}.png");
        println!("cargo::rerun-if-changed={source}");
        let rgba = decode(Path::new(&source));
        std::fs::write(Path::new(&out).join(format!("{name}.rgba")), rgba)
            .expect("OUT_DIR should be writable");
    }
}

/// Decodes a `TILE_SIZE` PNG into RGBA bytes.
#[expect(
    clippy::expect_used,
    reason = "a build script reports errors by panicking"
)]
fn decode(path: &Path) -> Vec<u8> {
    let file = File::open(path).expect("tile banner should exist (scripts/brand/render.sh)");
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("tile banner should be a PNG");
    let size = reader
        .output_buffer_size()
        .expect("tile banner should fit in memory");
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .expect("tile banner should decode");
    assert_eq!(
        (info.width, info.height),
        TILE_SIZE,
        "{} should be 320×180",
        path.display()
    );
    pixels.truncate(info.buffer_size());
    match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[red, green, blue]| [red, green, blue, 255])
            .collect(),
        other => panic!("{}: unexpected colour type {other:?}", path.display()),
    }
}

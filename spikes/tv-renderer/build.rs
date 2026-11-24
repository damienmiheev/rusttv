//! Compiles the Slint UI and writes a 1920×1080 PNG fixture for the decode test.
use std::{env, fs::File, io::BufWriter, path::Path};

fn main() {
    slint_build::compile("ui/spike.slint").expect("slint");
    let out = env::var("OUT_DIR").expect("OUT_DIR");
    let file = File::create(Path::new(&out).join("photo.png")).expect("png file");
    let mut enc = png::Encoder::new(BufWriter::new(file), 1920, 1080);
    enc.set_color(png::ColorType::Rgba);
    let mut w = enc.write_header().expect("header");
    // A photo-like gradient with noise, so compression is realistic (not all zeros).
    let mut seed: u32 = 1;
    let mut data = Vec::with_capacity(1920 * 1080 * 4);
    for y in 0..1080u32 {
        for x in 0..1920u32 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (seed >> 27) as u8;
            data.extend_from_slice(&[(x / 8) as u8 ^ n, (y / 5) as u8 ^ n, ((x + y) / 12) as u8, 255]);
        }
    }
    w.write_image_data(&data).expect("data");
}

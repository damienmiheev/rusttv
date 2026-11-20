//! Pass bar 1 and 3 (PANELS.md § 4.1): the generated tile renders exactly
//! like the hand-written one with Slint's software renderer, LTR and RTL,
//! every update state, in English and in Ukrainian (`@tr` through the
//! converter). Writes every picture to target/pixels/ for a look by eye.

use std::cell::RefCell;
use std::rc::Rc;

use panel_templates_spike::{Dir, GenWindow, HandWindow};
use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize};

thread_local! {
    static NEXT: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
}

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        NEXT.with(|next| *next.borrow_mut() = Some(window.clone()));
        Ok(window)
    }
}

/// Renders a freshly created component into RGB pixels.
fn render(make: impl FnOnce()) -> (u32, u32, Vec<u8>) {
    make();
    let window = NEXT
        .with(|next| next.borrow_mut().take())
        .expect("a window was created");
    // 250 × 141 `u` at u = 2px (spike.slint), scale factor 1.
    let (w, h) = (500, 282);
    window.set_size(PhysicalSize::new(w, h));
    let mut buffer = vec![Rgb8Pixel::default(); (w * h) as usize];
    slint::platform::update_timers_and_animations();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, w as usize);
    });
    let bytes = buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    (w, h, bytes)
}

fn save(name: &str, w: u32, h: u32, rgb: &[u8]) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pixels");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let file = std::fs::File::create(dir.join(format!("{name}.png"))).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgb))
        .expect("write png");
}

#[test]
fn generated_tile_matches_the_hand_written_one() {
    slint::platform::set_platform(Box::new(TestPlatform)).expect("first platform");
    // Bundled translations register with the first component.
    let first = HandWindow::new().expect("hand");
    NEXT.with(|next| next.borrow_mut().take());
    let mut compared = 0;
    let mut english = std::collections::HashMap::new();
    let mut english_or_uk_ltr = std::collections::HashMap::new();
    for lang in ["en", "uk"] {
        slint::select_bundled_translation(lang).expect("bundled language");
        for rtl in [false, true] {
            for status in ["", "update", "downloading", "installing"] {
                let setup = |version: &str| (version.to_owned(), status.to_owned());
                let (version, status_s) = setup("0.4.0");
                let hand = render(|| {
                    let window = HandWindow::new().expect("hand");
                    window.global::<Dir>().set_rtl(rtl);
                    window.set_version(version.clone().into());
                    window.set_status(status_s.clone().into());
                    window.show().expect("show");
                    std::mem::forget(window);
                });
                let generated = render(|| {
                    let window = GenWindow::new().expect("gen");
                    window.global::<Dir>().set_rtl(rtl);
                    window.set_version(version.clone().into());
                    window.set_status(status_s.clone().into());
                    window.show().expect("show");
                    std::mem::forget(window);
                });
                let name = format!(
                    "{lang}-{}-{}",
                    if rtl { "rtl" } else { "ltr" },
                    if status.is_empty() { "none" } else { status }
                );
                save(&format!("{name}-hand"), hand.0, hand.1, &hand.2);
                save(
                    &format!("{name}-gen"),
                    generated.0,
                    generated.1,
                    &generated.2,
                );
                assert_eq!((hand.0, hand.1), (generated.0, generated.1), "{name}: size");
                let diff = hand
                    .2
                    .iter()
                    .zip(&generated.2)
                    .filter(|(a, b)| a != b)
                    .count();
                assert_eq!(diff, 0, "{name}: {diff} bytes differ");
                // Something must actually be drawn: not a blank frame.
                assert!(hand.2.iter().any(|&b| b > 0x40), "{name}: blank");
                compared += 1;
                // The translation really applies: a translated caption
                // changes the picture; an empty one or "RustTV {}" does not.
                // RTL really mirrors: never the same picture as LTR.
                if rtl {
                    let ltr = english_or_uk_ltr
                        .get(&name.replace("-rtl-", "-ltr-"))
                        .expect("ltr first");
                    assert_ne!(ltr, &generated.2, "{name}: RTL looks like LTR");
                } else {
                    english_or_uk_ltr.insert(name.clone(), generated.2.clone());
                }
                if lang == "en" {
                    english.insert(name.replacen("en", "", 1), generated.2);
                } else {
                    let same = english[&name.replacen("uk", "", 1)] == generated.2;
                    assert_eq!(same, status.is_empty(), "{name}: translated = {}", !same);
                }
            }
        }
    }
    drop(first);
    assert_eq!(compared, 16);
}

//! Desktop dev mode: the home screen in a window, with made-up apps.
//!
//! Arrow keys move, Return is OK (hold it for a menu to move or hide a tile),
//! Escape is Back.
//!
//! `--order <file>` keeps the tile order in that file between runs.
//!
//! `--demo <dir>` instead plays a fixed key script and writes a BMP snapshot
//! after each step into `<dir>`, then quits. Needs no screen-capture permission.

use std::cell::RefCell;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use rusttv_launcher::{Bitmap, DemoStep, FakePlatform, RunOptions, run, run_demo};

/// Stand-in for the time an app takes to cover the launcher, so the launch
/// animation finishes before the launcher "comes back".
const SIMULATED_APP_START: Duration = Duration::from_millis(700);

/// The sample app whose launches fail in the demo, to show the error message.
const FAILING_APP: &str = "ai.rusttv.fake.games/.Main";

const UP: &str = "\u{F700}";
const DOWN: &str = "\u{F701}";
const LEFT: &str = "\u{F702}";
const RIGHT: &str = "\u{F703}";
const OK: &str = "\n";
const BACK: &str = "\u{1b}";

fn ms(millis: u64) -> DemoStep {
    DemoStep::Wait(Duration::from_millis(millis))
}

/// Walks through every state of the home screen. Sample order: Browser, Files,
/// Fitness, Games, Kids, Movies / Music, News, Photos, Podcasts, Radio, Series /
/// Sports, Weather, About.
fn demo_script() -> Vec<DemoStep> {
    use DemoStep::{Press, Release, Snapshot, Tap};
    vec![
        ms(800),
        Snapshot("01-start"),
        Tap(RIGHT),
        Tap(RIGHT),
        ms(400),
        Snapshot("02-focus-fitness"),
        ms(1500),
        Snapshot("03-idle-sway"),
        Tap(DOWN),
        ms(500),
        Snapshot("04-grid-row-1"),
        Tap(DOWN),
        ms(500),
        Snapshot("05-short-last-row"),
        Tap(OK),
        ms(400),
        Snapshot("06-about-page"),
        Tap(DOWN),
        Tap(DOWN),
        ms(300),
        Snapshot("06b-about-scrolled"),
        Tap(BACK),
        Tap(BACK),
        ms(500),
        Snapshot("07-back-to-first-row"),
        Press(OK),
        ms(900),
        Snapshot("08-menu"),
        Release(OK),
        Tap(OK),
        ms(300),
        Snapshot("08b-move-mode"),
        Tap(RIGHT),
        ms(300),
        Snapshot("09-tile-moved"),
        Tap(OK),
        ms(300),
        Tap(OK),
        ms(250),
        Snapshot("10-launching"),
        ms(1000),
        Tap(LEFT),
        Tap(OK),
        ms(300),
        Snapshot("11-launch-error"),
        Press(OK),
        ms(700),
        Release(OK),
        Tap(DOWN),
        ms(300),
        Snapshot("12-menu-hide"),
        Tap(OK),
        ms(400),
        Snapshot("13-hidden"),
        Tap(DOWN),
        Tap(DOWN),
        Tap(RIGHT),
        Tap(RIGHT),
        Tap(RIGHT),
        ms(400),
        Snapshot("14-hidden-apps-tile"),
        Tap(OK),
        ms(400),
        Snapshot("15-hidden-apps-page"),
        Tap(OK),
        ms(400),
        Snapshot("16-shown-again"),
        Tap(UP),
    ]
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut options = RunOptions {
        right_to_left: false,
        simulated_pause: Some(SIMULATED_APP_START),
        order_file: None,
        lifecycle: Default::default(),
    };
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        return Ok(run(FakePlatform::sample(), options)?);
    };
    let dir = match (flag.as_str(), args.next()) {
        ("--order", Some(file)) => {
            options.order_file = Some(PathBuf::from(file));
            return Ok(run(FakePlatform::sample(), options)?);
        }
        ("--demo", Some(dir)) => PathBuf::from(dir),
        _ => return Err("usage: rusttv-launcher-desktop [--order <file> | --demo <dir>]".into()),
    };
    std::fs::create_dir_all(&dir)?;

    let mut platform = FakePlatform::sample();
    platform.fail_launches_of(FAILING_APP);
    let failures: Rc<RefCell<Vec<String>>> = Rc::default();
    let record = failures.clone();
    run_demo(platform, options, &demo_script(), move |name, picture| {
        let path = dir.join(format!("{name}.bmp"));
        let result = picture
            .map_err(|error| error.to_string())
            .and_then(|bitmap| write_bmp(&path, &bitmap).map_err(|error| error.to_string()));
        if let Err(error) = result {
            record.borrow_mut().push(format!("{name}: {error}"));
        }
    })?;

    let failures = failures.take();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; ").into())
    }
}

/// Writes an uncompressed 32-bit BMP, which macOS `sips` and most viewers read.
fn write_bmp(path: &Path, bitmap: &Bitmap) -> std::io::Result<()> {
    const HEADER_SIZE: u32 = 14 + 40;
    let too_big = || std::io::Error::other("image too large for BMP");
    let width = i32::try_from(bitmap.width()).map_err(|_| too_big())?;
    let height = i32::try_from(bitmap.height()).map_err(|_| too_big())?;
    let pixels = u32::try_from(bitmap.rgba().len()).map_err(|_| too_big())?;

    let mut file = Vec::with_capacity(HEADER_SIZE as usize + pixels as usize);
    file.extend(b"BM");
    file.extend((HEADER_SIZE + pixels).to_le_bytes());
    file.extend([0; 4]);
    file.extend(HEADER_SIZE.to_le_bytes());
    file.extend(40_u32.to_le_bytes());
    file.extend(width.to_le_bytes());
    // Negative height: rows are stored top to bottom.
    file.extend((-height).to_le_bytes());
    file.extend(1_u16.to_le_bytes());
    file.extend(32_u16.to_le_bytes());
    file.extend([0; 4]); // no compression
    file.extend(pixels.to_le_bytes());
    file.extend([0; 16]); // resolution and palette: unused
    for [red, green, blue, alpha] in bitmap.rgba().as_chunks::<4>().0 {
        file.extend([*blue, *green, *red, *alpha]);
    }
    std::fs::write(path, file)
}

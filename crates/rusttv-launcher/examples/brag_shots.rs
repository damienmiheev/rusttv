//! Temporary (untracked): snapshots of Settings → History for the promo video.
#![allow(clippy::unwrap_used, clippy::print_stderr, missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use rusttv_launcher::{DemoStep, FakePlatform, RunOptions, run_demo, start_desktop_history};

const UP: &str = "\u{F700}";
const DOWN: &str = "\u{F701}";
const LEFT: &str = "\u{F702}";
const RIGHT: &str = "\u{F703}";
const OK: &str = "\n";

fn ms(millis: u64) -> DemoStep {
    DemoStep::Wait(Duration::from_millis(millis))
}

fn main() {
    use DemoStep::{Press, Release, Snapshot, Tap};
    let dir = PathBuf::from(std::env::args().nth(1).unwrap());
    let script = vec![
        ms(800),
        // Move the first tile one to the right: a TV change in the history.
        Press(OK),
        ms(900),
        Release(OK),
        Tap(OK),
        ms(300),
        Tap(RIGHT),
        ms(300),
        Tap(OK),
        ms(600),
        Snapshot("h0-moved"),
        Tap(LEFT),
        // Settings: same path as the remote demo.
        Tap(DOWN),
        Tap(DOWN),
        Tap(RIGHT),
        Tap(RIGHT),
        Tap(RIGHT),
        Tap(OK),
        ms(600),
        Snapshot("h1-settings"),
        Tap(DOWN),
        Tap(DOWN),
        ms(400),
        Snapshot("h2-history"),
        Tap(RIGHT),
        ms(400),
        Snapshot("h3-history-list"),
        Tap(DOWN),
        ms(300),
        Snapshot("h4-history-second"),
        Tap(OK),
        ms(600),
        Snapshot("h5-history-open"),
        Tap(DOWN),
        ms(300),
        Snapshot("h6-history-open-down"),
        Tap(UP),
    ];
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state");
    let _ = std::fs::remove_dir_all(&state);
    std::fs::create_dir_all(&state).unwrap();
    let options = RunOptions {
        language: std::env::var("RUSTTV_LANG").ok(),
        right_to_left: false,
        simulated_pause: Some(Duration::from_millis(400)),
        order_file: Some(state.join("order.toml")),
        lifecycle: Default::default(),
        remote_pairing: false,
        remote_port: None,
    };
    start_desktop_history(state);
    let out = dir.clone();
    run_demo(FakePlatform::sample(), options, &script, move |name, picture| {
        let bitmap = picture.unwrap();
        let (w, h) = (bitmap.width(), bitmap.height());
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        for px in bitmap.rgba().chunks_exact(4) {
            ppm.extend(&px[..3]);
        }
        std::fs::write(out.join(format!("{name}.ppm")), ppm).unwrap();
    })
    .unwrap();
}

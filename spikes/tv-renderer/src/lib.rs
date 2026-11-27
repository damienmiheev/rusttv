//! Throwaway Panel Studio step 1 spike (PANEL_STUDIO.md § 15). Not product code.
//!
//! Launched with an intent extra `mode`:
//! - `compiled-home` / `generic-home`: 18 tiles, hand-written vs from JSON;
//! - `generic-N`: N nodes from JSON (per-node cost);
//! - `compiled-N`: N/10 hand-written tiles.
//!
//! Logs to logcat (tag `RustTVSpike`): time to the first frame, render time per
//! frame (CPU, BeforeRendering → AfterRendering) and frame intervals, the PNG
//! decode cost, and a `take_snapshot` (offscreen image of the window) cost.
#![allow(clippy::unwrap_used, clippy::expect_used)] // throwaway spike

use android_activity::AndroidApp;
use jni::objects::{JObject, JString};
use serde_json::Value;
use slint::{Color, ComponentHandle, Image, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

slint::include_modules!();

const TAG: &str = "RustTVSpike";

/// About-like tile, the same 10 nodes as `CompiledTile` in spike.slint.
const TILE: &str = r##"[
 {"k":"rect","x":0,"y":0,"w":250,"h":141,"bg":"#1e2633","r":16,"shadow":12},
 {"k":"rect","x":16,"y":16,"w":48,"h":48,"bg":"#e8572a","r":12},
 {"k":"text","x":76,"y":16,"w":158,"h":26,"text":"RustTV","size":20,"fg":"#ffffff","weight":700},
 {"k":"text","x":76,"y":42,"w":158,"h":22,"text":"0.4.0","size":15,"fg":"#9aa4b2","weight":400},
 {"k":"rect","x":16,"y":96,"w":218,"h":8,"bg":"#2c3644","r":4},
 {"k":"rect","x":16,"y":96,"w":140,"h":8,"bg":"#4caf50","r":4},
 {"k":"text","x":16,"y":110,"w":218,"h":20,"text":"Update available","size":14,"fg":"#c8d0da","weight":400},
 {"k":"rect","x":214,"y":12,"w":24,"h":24,"bg":"#e53935","r":12},
 {"k":"text","x":214,"y":12,"w":24,"h":24,"text":"1","size":13,"fg":"#ffffff","weight":700,"center":true},
 {"k":"rect","x":0,"y":0,"w":250,"h":141,"bg":"#00000000","r":16,"border":2,"bc":"#ffffff40"}
]"##;

fn color(v: &Value) -> Color {
    let s = v.as_str().unwrap_or("#00000000").trim_start_matches('#');
    let n = u32::from_str_radix(s, 16).unwrap_or(0);
    if s.len() == 8 {
        Color::from_argb_u8((n & 0xff) as u8, (n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8)
    } else {
        Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
    }
}

fn f(v: &Value, k: &str) -> f32 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// Parses the tile JSON and places `tiles` copies on a Home-like grid.
fn nodes(tiles: usize, image: &Image) -> Vec<Node> {
    let tile: Vec<Value> = serde_json::from_str(TILE).unwrap();
    let mut out = Vec::with_capacity(tiles * tile.len());
    for i in 0..tiles {
        let (ox, oy) = (100.0 + (i % 6) as f32 * 280.0 + (i / 30) as f32 * 9.0, 200.0 + ((i / 6) % 5) as f32 * 170.0 + (i / 30) as f32 * 9.0);
        for n in &tile {
            let kind = match n["k"].as_str() {
                Some("text") => 1,
                Some("image") => 2,
                _ => 0,
            };
            let text: SharedString = n.get("text").and_then(Value::as_str).unwrap_or("").into();
            out.push(Node {
                kind,
                x: ox + f(n, "x"),
                y: oy + f(n, "y"),
                w: f(n, "w"),
                h: f(n, "h"),
                bg: color(&n["bg"]),
                radius: f(n, "r"),
                border: f(n, "border"),
                border_color: color(&n["bc"]),
                shadow: f(n, "shadow"),
                label: text.clone(),
                text,
                size: f(n, "size"),
                fg: color(&n["fg"]),
                weight: f(n, "weight") as i32,
                center: n.get("center").and_then(Value::as_bool).unwrap_or(false),
                img: image.clone(),
            });
        }
    }
    out
}

fn mode(app: &AndroidApp) -> String {
    // SAFETY: android-activity owns the VM and a global ref to the activity for
    // the app's lifetime; we only borrow them.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.unwrap();
    let mut env = vm.attach_current_thread().unwrap();
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let intent = env
        .call_method(&activity, "getIntent", "()Landroid/content/Intent;", &[])
        .and_then(|v| v.l())
        .unwrap();
    let key = env.new_string("mode").unwrap();
    let value = env
        .call_method(&intent, "getStringExtra", "(Ljava/lang/String;)Ljava/lang/String;", &[(&key).into()])
        .and_then(|v| v.l())
        .unwrap();
    if value.is_null() {
        return "generic-home".into();
    }
    env.get_string(&JString::from(value)).unwrap().into()
}

#[cfg(feature = "png-decode")]
fn decode_test() -> Image {
    for (name, bytes) in [
        ("logo", &include_bytes!("../../panel-templates/assets/rusttv-logo.png")[..]),
        ("photo1080p", &include_bytes!(concat!(env!("OUT_DIR"), "/photo.png"))[..]),
    ] {
        let t = Instant::now();
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
        let mut reader = dec.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        log::info!(
            "PNG {name}: {} bytes -> {}x{} decoded in {:.1} ms",
            bytes.len(),
            info.width,
            info.height,
            t.elapsed().as_secs_f64() * 1e3
        );
    }
    Image::default()
}

#[cfg(not(feature = "png-decode"))]
fn decode_test() -> Image {
    Image::default()
}

#[derive(Default)]
struct Stats {
    start: Option<Instant>,
    before: Option<Instant>,
    last: Option<Instant>,
    render: Vec<f64>,
    interval: Vec<f64>,
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn install_stats(window: &slint::Window, main_start: Instant) {
    let stats = Rc::new(RefCell::new(Stats { start: Some(main_start), ..Default::default() }));
    let s = stats.clone();
    let mut frames = 0u32;
    window
        .set_rendering_notifier(move |state, _| {
            let now = Instant::now();
            let mut st = s.borrow_mut();
            match state {
                slint::RenderingState::BeforeRendering => st.before = Some(now),
                slint::RenderingState::AfterRendering => {
                    frames += 1;
                    if frames == 1 {
                        log::info!(
                            "first frame {:.1} ms after android_main",
                            now.duration_since(st.start.unwrap()).as_secs_f64() * 1e3
                        );
                    }
                    if let Some(b) = st.before {
                        st.render.push(now.duration_since(b).as_secs_f64() * 1e3);
                    }
                    if let Some(l) = st.last {
                        st.interval.push(now.duration_since(l).as_secs_f64() * 1e3);
                    }
                    st.last = Some(now);
                    // Skip warm-up (first 60 frames), then report each 600 frames.
                    if frames == 60 {
                        st.render.clear();
                        st.interval.clear();
                    }
                    if frames > 60 && (frames - 60) % 600 == 0 {
                        let n = st.render.len();
                        let (r50, r95) = (pct(&mut st.render, 0.5), pct(&mut st.render, 0.95));
                        let (i50, i95, imax) = (
                            pct(&mut st.interval, 0.5),
                            pct(&mut st.interval, 0.95),
                            pct(&mut st.interval, 1.0),
                        );
                        log::info!(
                            "STATS frames={n} render p50={r50:.2} p95={r95:.2} ms; interval p50={i50:.1} p95={i95:.1} max={imax:.1} ms"
                        );
                        st.render.clear();
                        st.interval.clear();
                    }
                }
                _ => {}
            }
        })
        .unwrap();

}

/// `interp-home`: the same Home as `compiled-home`, compiled from source at
/// run time by slint-interpreter.
#[cfg(feature = "interp")]
fn interp_home(main_start: Instant) {
    use slint_interpreter::{Compiler, Value as V};
    let t = Instant::now();
    let result = spin_on(Compiler::default().build_from_source(
        include_str!("../ui/spike.slint").into(),
        "spike.slint".into(),
    ));
    for d in result.diagnostics() {
        log::warn!("slint: {d}");
    }
    let def = result.component("Spike").unwrap();
    log::info!("interpreter compile {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let ui = def.create().unwrap();
    ui.set_property("compiled-tiles", V::Number(18.0)).unwrap();
    ui.set_property("mode", V::Number(1.0)).unwrap();
    ui.set_property("status", V::String("spike interp-home".into())).unwrap();
    ui.set_property("animate", V::Bool(true)).unwrap();
    log::info!("interpreter create {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    install_stats(ui.window(), main_start);
    ui.run().unwrap();
}

/// Polls a future that completes without waiting (no file loads).
#[cfg(feature = "interp")]
fn spin_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    loop {
        if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let main_start = Instant::now();
    android_logger::init_once(android_logger::Config::default().with_tag(TAG).with_max_level(log::LevelFilter::Info));
    slint::android::init(app.clone()).unwrap();
    let mode = mode(&app);
    log::info!("mode {mode}");
    let image = Image::default();
    #[cfg(feature = "interp")]
    if mode == "interp-home" {
        return interp_home(main_start);
    }

    let ui = Spike::new().unwrap();
    let t = Instant::now();
    let (generic, count) = match mode.split_once('-') {
        Some(("compiled", "home")) => (false, 18),
        Some(("generic", "home")) => (true, 18),
        Some(("compiled", n)) => (false, n.parse::<usize>().unwrap_or(500) / 10),
        Some((_, n)) => (true, n.parse::<usize>().unwrap_or(500) / 10),
        None => (true, 18),
    };
    if generic {
        let list = nodes(count, &image);
        log::info!("JSON parse + flatten: {} nodes in {:.2} ms", list.len(), t.elapsed().as_secs_f64() * 1e3);
        ui.set_nodes(ModelRc::new(VecModel::from(list)));
        ui.set_mode(0);
    } else {
        ui.set_compiled_tiles(count as i32);
        ui.set_mode(1);
    }
    ui.set_status(format!("spike {mode}").into());
    ui.set_animate(true);

    install_stats(ui.window(), main_start);

    // PNG decode cost, after the first frame so it doesn't count in startup.
    slint::Timer::single_shot(Duration::from_secs(3), || {
        let _ = decode_test();
    });

    // Offscreen image of the window (phone-mirror question), once, after 5 s.
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_secs(5), move || {
        if let Some(ui) = weak.upgrade() {
            let t = Instant::now();
            match ui.window().take_snapshot() {
                Ok(buf) => log::info!(
                    "take_snapshot {}x{} in {:.1} ms",
                    buf.width(),
                    buf.height(),
                    t.elapsed().as_secs_f64() * 1e3
                ),
                Err(e) => log::warn!("take_snapshot failed: {e}"),
            }
        }
    });

    ui.run().unwrap();
}

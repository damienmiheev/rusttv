//! The volume bar over everything: a volume key shows the new volume at
//! the top of the screen for a moment, drawn as the launcher's own bar
//! (`VolumeOsd` in ui/quick.slint: the same size, colours, shadow and
//! fades). A `wlr-layer-shell` overlay surface that takes neither keys nor
//! clicks; one thread keeps it for good.

use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use resvg::tiny_skia::{LineCap, LineJoin, PathBuilder, Pixmap, Stroke, Transform};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_region::WlRegion,
    wl_registry::WlRegistry,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{
        self as layer_surface, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
    },
};

use crate::device::Volume;
use crate::linux::keyboard::draw::{TEXT, TEXT_DIM, paint, rounded, text, width_of};

/// How long the bar stays after the last change (as the launcher's own).
const SHOWN_FOR: Duration = Duration::from_millis(1500);
/// Fading in and out (`animate opacity` in `VolumeOsd`).
const FADE_IN: Duration = Duration::from_millis(120);
const FADE_OUT: Duration = Duration::from_millis(400);
/// Between the frames of a fade.
const FRAME: Duration = Duration::from_millis(16);
/// The bar, in surface units (the screen is 1920 wide: one design unit).
const BAR_W: f32 = 520.0;
const BAR_H: f32 = 88.0;
const TOP: f32 = 60.0;
/// Room around the bar for its shadow.
const PAD: f32 = 40.0;
const WIDTH: u32 = (BAR_W + 2.0 * PAD) as u32;
const HEIGHT: u32 = (BAR_H + 2.0 * PAD) as u32;
/// `Colours` in ui/quick.slint: the surface at 95 %, the accent, the
/// track (#FFFFFF26), and the shadow (#00000080, blurred 32).
const BAR: [u8; 4] = [0x1C, 0x1F, 0x26, 0xF2];
const ACCENT: [u8; 4] = [0xF5, 0xA5, 0x24, 0xFF];
const TRACK: [u8; 4] = [0xFF, 0xFF, 0xFF, 0x26];
const SHADOW_ALPHA: f32 = 0.5;
const SHADOW_BLUR: usize = 10;

/// What the bar shows.
#[derive(Debug, Clone)]
struct Shows {
    volume: Volume,
    /// "Muted", in the current language.
    muted: String,
}

/// Shows the volume over apps; the thread behind it lives as long as this.
#[derive(Debug)]
pub struct VolumeBar {
    volumes: Sender<Shows>,
    wake: OwnedFd,
}

impl VolumeBar {
    /// Starts the bar's thread; `None` without a font, pipe or thread.
    pub fn start() -> Option<Self> {
        let font = crate::linux::keyboard::draw::load_font()?;
        let (wake_read, wake) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )
        .ok()?;
        let (volumes, received) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("rusttv-volume-bar".into())
            .spawn(move || {
                if let Err(error) = run(font, &received, &wake_read) {
                    log::warn!("volume bar over apps: {error}");
                }
            });
        if let Err(error) = spawned {
            log::warn!("no thread for the volume bar over apps: {error}");
            return None;
        }
        Some(Self { volumes, wake })
    }

    /// Shows `volume` for a moment; `muted`: the word for a muted one.
    pub fn show(&self, volume: Volume, muted: &str) {
        let shows = Shows {
            volume,
            muted: muted.to_owned(),
        };
        if self.volumes.send(shows).is_ok() {
            let _ = rustix::io::write(&self.wake, &[1]);
        }
    }
}

fn run(font: FontVec, volumes: &Receiver<Shows>, wake: &OwnedFd) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let bind = |name: &'static str| move |error| format!("no {name}: {error}");
    let mut state = State {
        compositor: globals.bind(&qh, 1..=4, ()).map_err(bind("compositor"))?,
        layer_shell: globals.bind(&qh, 1..=4, ()).map_err(bind("layer shell"))?,
        shm: globals.bind(&qh, 1..=1, ()).map_err(bind("shm"))?,
        font,
        shown: None,
        shows: Shows {
            volume: Volume {
                level: 0.0,
                muted: false,
            },
            muted: String::new(),
        },
        fade: Fade::still(0.0),
    };
    let mut hide_at: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let timeout = if state.fade.moving(now) {
            Some(FRAME)
        } else {
            hide_at.map(|at| at.saturating_duration_since(now))
        };
        if super::wait(&mut queue, &mut state, wake, timeout)? {
            loop {
                match volumes.try_recv() {
                    Ok(shows) => state.shows = shows,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            let now = Instant::now();
            state.fade = state.fade.towards(1.0, FADE_IN, now);
            state.show(&qh);
            hide_at = Some(now + SHOWN_FOR);
            continue;
        }
        let now = Instant::now();
        if hide_at.is_some_and(|at| now >= at) {
            hide_at = None;
            state.fade = state.fade.towards(0.0, FADE_OUT, now);
        }
        if state.fade.at(now) <= 0.0 && !state.fade.moving(now) {
            state.hide();
        } else {
            state.redraw(&qh);
        }
    }
}

/// The bar's opacity: from `from` at `start` to `to` over `length`, eased
/// out (as Slint's `ease-out`).
#[derive(Debug, Clone, Copy)]
struct Fade {
    from: f32,
    to: f32,
    start: Instant,
    length: Duration,
}

impl Fade {
    fn still(opacity: f32) -> Self {
        Self {
            from: opacity,
            to: opacity,
            start: Instant::now(),
            length: Duration::ZERO,
        }
    }

    /// The opacity at `now`.
    fn at(&self, now: Instant) -> f32 {
        if self.length.is_zero() {
            return self.to;
        }
        let t = (now.saturating_duration_since(self.start).as_secs_f32()
            / self.length.as_secs_f32())
        .clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        self.from + (self.to - self.from) * eased
    }

    fn moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < self.length && self.from != self.to
    }

    /// A fade from where this one is at `now` to `to`.
    fn towards(&self, to: f32, length: Duration, now: Instant) -> Self {
        let from = self.at(now);
        if from == to {
            return Self::still(to);
        }
        Self {
            from,
            to,
            start: now,
            length,
        }
    }
}

struct Shown {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    configured: bool,
}

struct State {
    compositor: WlCompositor,
    layer_shell: ZwlrLayerShellV1,
    shm: WlShm,
    font: FontVec,
    shown: Option<Shown>,
    shows: Shows,
    fade: Fade,
}

impl State {
    fn show(&mut self, qh: &QueueHandle<Self>) {
        if self.shown.is_some() {
            self.redraw(qh);
            return;
        }
        let surface = self.compositor.create_surface(qh, ());
        // Clicks go through to the app.
        let region = self.compositor.create_region(qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let layer = self.layer_shell.get_layer_surface(
            &surface,
            None,
            Layer::Overlay,
            "rusttv-volume".into(),
            qh,
            (),
        );
        layer.set_anchor(Anchor::Top);
        layer.set_size(WIDTH, HEIGHT);
        // The bar's top edge at TOP; the shadow's room above it.
        layer.set_margin((TOP - PAD) as i32, 0, 0, 0);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        self.shown = Some(Shown {
            surface,
            layer,
            configured: false,
        });
    }

    fn hide(&mut self) {
        if let Some(shown) = self.shown.take() {
            shown.layer.destroy();
            shown.surface.destroy();
        }
    }

    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        let Some(shown) = self.shown.as_ref().filter(|shown| shown.configured) else {
            return;
        };
        let opacity = self.fade.at(Instant::now());
        let Some(pixels) = draw(&self.shows, &self.font, opacity) else {
            return;
        };
        match super::shm_buffer(&self.shm, &pixels, WIDTH, HEIGHT, qh) {
            Ok(buffer) => {
                shown.surface.attach(Some(&buffer), 0, 0);
                shown.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                shown.surface.commit();
            }
            Err(error) => log::warn!("drawing the volume bar: {error}"),
        }
    }
}

/// The bar as `VolumeOsd` draws it: a pill with a soft shadow, a speaker
/// outline (crossed out when muted), the level as an accent line on a faint
/// track, and the level in percent (or "Muted"), all at `opacity`.
fn draw(shows: &Shows, font: &FontVec, opacity: f32) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT)?;
    shadow(&mut pixmap);
    let (x0, y0) = (PAD, PAD);
    rounded(&mut pixmap, x0, y0, BAR_W, BAR_H, BAR_H / 2.0, BAR);
    let volume = shows.volume;
    speaker(&mut pixmap, x0 + 30.0, y0 + 22.0, 44.0, volume.muted);
    // The level line: 300 × 10 from x 96, centred.
    let (left, width, line) = (x0 + 96.0, 300.0, 10.0);
    let top = y0 + (BAR_H - line) / 2.0;
    rounded(&mut pixmap, left, top, width, line, line / 2.0, TRACK);
    let filled = width * volume.level.clamp(0.0, 1.0);
    if filled > 0.0 {
        let fill = if volume.muted { TEXT_DIM } else { ACCENT };
        let r = (line / 2.0).min(filled / 2.0);
        rounded(&mut pixmap, left, top, filled, line, r, fill);
    }
    // Right-aligned in x 412..492.
    let label = if volume.muted {
        shows.muted.clone()
    } else {
        format!("{:.0}%", volume.level * 100.0)
    };
    let size = 27.0;
    let right = x0 + 492.0;
    let x = right - width_of(font, &label, size);
    text(
        &mut pixmap,
        font,
        &label,
        x,
        y0 + BAR_H / 2.0,
        size,
        TEXT,
        false,
    );
    let mut bytes = pixmap.take();
    let opacity = opacity.clamp(0.0, 1.0);
    for pixel in bytes.as_chunks_mut::<4>().0 {
        // Premultiplied: all four scale with the opacity.
        for channel in pixel.iter_mut() {
            *channel = (f32::from(*channel) * opacity).round() as u8;
        }
        pixel.swap(0, 2);
    }
    Some(bytes)
}

/// The bar's shadow: its shape in black at [`SHADOW_ALPHA`], blurred
/// (three box blurs, about Skia's blur of 32).
fn shadow(pixmap: &mut Pixmap) {
    let mut shape = match Pixmap::new(WIDTH, HEIGHT) {
        Some(shape) => shape,
        None => return,
    };
    let black = [0, 0, 0, (SHADOW_ALPHA * 255.0) as u8];
    rounded(&mut shape, PAD, PAD, BAR_W, BAR_H, BAR_H / 2.0, black);
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut alpha: Vec<f32> = shape
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| f32::from(pixel[3]))
        .collect();
    for _ in 0..3 {
        alpha = blur_rows(&alpha, w, h, SHADOW_BLUR);
        alpha = transpose(&alpha, w, h);
        alpha = blur_rows(&alpha, h, w, SHADOW_BLUR);
        alpha = transpose(&alpha, h, w);
    }
    for (pixel, a) in pixmap
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(alpha)
    {
        *pixel = [0, 0, 0, a.round().clamp(0.0, 255.0) as u8];
    }
}

/// Each row of a `w` × `h` grid averaged over ±`r`.
fn blur_rows(values: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut out = vec![0.0; values.len()];
    let span = (2 * r + 1) as f32;
    for y in 0..h {
        let row = &values[y * w..(y + 1) * w];
        let at = |x: isize| -> f32 {
            usize::try_from(x)
                .ok()
                .and_then(|x| row.get(x))
                .copied()
                .unwrap_or(0.0)
        };
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for x in 0..w {
            out[y * w + x] = sum / span;
            let x = x as isize;
            sum += at(x + r as isize + 1) - at(x - r as isize);
        }
    }
    out
}

fn transpose(values: &[f32], w: usize, h: usize) -> Vec<f32> {
    let mut out = vec![0.0; values.len()];
    for y in 0..h {
        for x in 0..w {
            out[x * h + y] = values[y * w + x];
        }
    }
    out
}

/// The speaker of `VolumeOsd`: on a 24 × 24 grid drawn `size` wide at
/// (`x`, `y`), a box and cone with two sound arcs, or a cross when muted;
/// strokes 2.6 wide.
fn speaker(pixmap: &mut Pixmap, x: f32, y: f32, size: f32, muted: bool) {
    let scale = size / 24.0;
    let at = |gx: f32, gy: f32| (x + gx * scale, y + gy * scale);
    let mut path = PathBuilder::new();
    let cone = [
        (3.0, 9.0),
        (7.0, 9.0),
        (12.0, 5.0),
        (12.0, 19.0),
        (7.0, 15.0),
        (3.0, 15.0),
    ];
    for (index, &(gx, gy)) in cone.iter().enumerate() {
        let (px, py) = at(gx, gy);
        if index == 0 {
            path.move_to(px, py);
        } else {
            path.line_to(px, py);
        }
    }
    path.close();
    if muted {
        for [(ax, ay), (bx, by)] in [[(16.0, 9.0), (22.0, 15.0)], [(22.0, 9.0), (16.0, 15.0)]] {
            let (px, py) = at(ax, ay);
            path.move_to(px, py);
            let (px, py) = at(bx, by);
            path.line_to(px, py);
        }
    } else {
        // "A 4 4 0 0 1" from (15.5, 9) to (15.5, 15), and "A 8 8" from
        // (18, 6) to (18, 18): arcs bulging right, about (cx, 12).
        for (end_x, half, r) in [(15.5_f32, 3.0_f32, 4.0_f32), (18.0, 6.0, 8.0)] {
            let cx = end_x - (r * r - half * half).sqrt();
            let reach = (half / r).asin();
            const STEPS: usize = 16;
            for step in 0..=STEPS {
                let angle = -reach + 2.0 * reach * step as f32 / STEPS as f32;
                let (px, py) = at(cx + r * angle.cos(), 12.0 + r * angle.sin());
                if step == 0 {
                    path.move_to(px, py);
                } else {
                    path.line_to(px, py);
                }
            }
        }
    }
    let Some(path) = path.finish() else {
        return;
    };
    let stroke = Stroke {
        width: 2.6,
        line_cap: LineCap::Butt,
        line_join: LineJoin::Miter,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint(TEXT), &stroke, Transform::identity(), None);
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: layer_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            layer_surface::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
                if let Some(shown) = &mut state.shown {
                    shown.configured = true;
                }
                state.redraw(qh);
            }
            layer_surface::Event::Closed => state.hide(),
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: WlCompositor);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: WlRegion);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(bytes: &[u8], x: f32, y: f32) -> [u8; 4] {
        let index = ((y as usize) * WIDTH as usize + x as usize) * 4;
        // BGRA in the buffer.
        [
            bytes[index + 2],
            bytes[index + 1],
            bytes[index],
            bytes[index + 3],
        ]
    }

    #[test]
    fn draws_the_level_as_an_accent_line_on_a_faint_track() {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            return; // no system font on this machine
        };
        let shows = Shows {
            volume: Volume {
                level: 0.5,
                muted: false,
            },
            muted: "Muted".into(),
        };
        let bytes = draw(&shows, &font, 1.0).unwrap();
        let mid = PAD + BAR_H / 2.0;
        // Filled up to half the line (x 96 + 150), the track after.
        assert_eq!(pixel(&bytes, PAD + 150.0, mid), ACCENT);
        let track = pixel(&bytes, PAD + 330.0, mid);
        assert!(track[0] > BAR[0] && track[0] < 0x60, "{track:?}");
        // The bar itself, and a shadow fading out around it.
        let bar = pixel(&bytes, PAD + 260.0, PAD + 8.0);
        assert!(
            bar[3] > 0xF0 && (i32::from(bar[0]) - 0x1B).abs() <= 2,
            "{bar:?}"
        );
        let near = pixel(&bytes, PAD + 260.0, PAD - 4.0)[3];
        let far = pixel(&bytes, PAD + 260.0, 1.0)[3];
        assert!(near > far && far < 8, "{near} {far}");
    }

    #[test]
    fn a_fading_bar_is_drawn_at_its_opacity() {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            return;
        };
        let shows = Shows {
            volume: Volume {
                level: 0.5,
                muted: true,
            },
            muted: "Muted".into(),
        };
        let full = draw(&shows, &font, 1.0).unwrap();
        let half = draw(&shows, &font, 0.5).unwrap();
        let mid = PAD + BAR_H / 2.0;
        let (full, half) = (
            pixel(&full, PAD + 150.0, mid),
            pixel(&half, PAD + 150.0, mid),
        );
        // Muted: the line is dim, not the accent.
        assert_eq!(full, TEXT_DIM);
        assert_eq!(half[3], 0x80);
    }

    #[test]
    fn fades_ease_out_towards_their_end() {
        let start = Instant::now();
        let fade = Fade::still(0.0).towards(1.0, FADE_IN, start);
        assert!(fade.moving(start));
        let halfway = fade.at(start + FADE_IN / 2);
        assert!(halfway > 0.5 && halfway < 1.0, "{halfway}");
        assert_eq!(fade.at(start + FADE_IN), 1.0);
        assert!(!fade.moving(start + FADE_IN));
        // Turning round midway starts from where it was.
        let back = fade.towards(0.0, FADE_OUT, start + FADE_IN / 2);
        assert_eq!(back.at(start + FADE_IN / 2), halfway);
    }
}

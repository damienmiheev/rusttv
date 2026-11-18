//! The volume bar over apps: a volume key while an app is in front shows
//! the new volume at the top of the screen for a moment, as the launcher's
//! own bar does over the home screen. A `wlr-layer-shell` overlay surface
//! that takes neither keys nor clicks; one thread keeps it for good.

use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use ab_glyph::FontVec;
use resvg::tiny_skia::{FillRule, PathBuilder, Pixmap, Transform};
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
use crate::linux::keyboard::draw::{PANEL, SURFACE, TEXT, TEXT_DIM, paint, rounded, text};

/// How long the bar stays after the last change (as the launcher's own).
const SHOWN_FOR: Duration = Duration::from_millis(1500);
/// The bar's size and distance from the top, in surface units.
const WIDTH: u32 = 560;
const HEIGHT: u32 = 104;
const TOP: i32 = 48;

/// Shows the volume over apps; the thread behind it lives as long as this.
#[derive(Debug)]
pub struct VolumeBar {
    volumes: Sender<Volume>,
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

    /// Shows `volume` for a moment.
    pub fn show(&self, volume: Volume) {
        if self.volumes.send(volume).is_ok() {
            let _ = rustix::io::write(&self.wake, &[1]);
        }
    }
}

fn run(font: FontVec, volumes: &Receiver<Volume>, wake: &OwnedFd) -> Result<(), String> {
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
        volume: Volume {
            level: 0.0,
            muted: false,
        },
    };
    let mut hide_at: Option<Instant> = None;
    loop {
        let timeout = hide_at.map(|at| at.saturating_duration_since(Instant::now()));
        if super::wait(&mut queue, &mut state, wake, timeout)? {
            loop {
                match volumes.try_recv() {
                    Ok(volume) => state.volume = volume,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()),
                }
            }
            state.show(&qh);
            hide_at = Some(Instant::now() + SHOWN_FOR);
        } else if hide_at.is_some_and(|at| Instant::now() >= at) {
            hide_at = None;
            state.hide();
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
    volume: Volume,
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
        layer.set_margin(TOP, 0, 0, 0);
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
        let Some(pixels) = draw(self.volume, &self.font, WIDTH, HEIGHT) else {
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

/// The bar: a speaker (crossed out when muted), the level as a line, and
/// the level in percent.
fn draw(volume: Volume, font: &FontVec, width: u32, height: u32) -> Option<Vec<u8>> {
    let mut pixmap = Pixmap::new(width, height)?;
    let (w, h) = (width as f32, height as f32);
    rounded(&mut pixmap, 0.0, 0.0, w, h, h / 2.0, PANEL);
    let mid = h / 2.0;
    let ink = if volume.muted { TEXT_DIM } else { TEXT };
    // The speaker: a box and a cone.
    let mut speaker = PathBuilder::new();
    speaker.move_to(40.0, mid - 9.0);
    speaker.line_to(52.0, mid - 9.0);
    speaker.line_to(68.0, mid - 22.0);
    speaker.line_to(68.0, mid + 22.0);
    speaker.line_to(52.0, mid + 9.0);
    speaker.line_to(40.0, mid + 9.0);
    speaker.close();
    if let Some(path) = speaker.finish() {
        pixmap.fill_path(
            &path,
            &paint(ink),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    if volume.muted {
        let mut cross = PathBuilder::new();
        cross.move_to(78.0, mid - 10.0);
        cross.line_to(98.0, mid + 10.0);
        cross.move_to(98.0, mid - 10.0);
        cross.line_to(78.0, mid + 10.0);
        if let Some(path) = cross.finish() {
            let stroke = resvg::tiny_skia::Stroke {
                width: 4.0,
                line_cap: resvg::tiny_skia::LineCap::Round,
                ..Default::default()
            };
            pixmap.stroke_path(&path, &paint(ink), &stroke, Transform::identity(), None);
        }
    }
    let (left, right) = (120.0, w - 132.0);
    let level = volume.level.clamp(0.0, 1.0);
    rounded(
        &mut pixmap,
        left,
        mid - 6.0,
        right - left,
        12.0,
        6.0,
        SURFACE,
    );
    if level > 0.0 {
        let filled = ((right - left) * level).max(12.0);
        rounded(&mut pixmap, left, mid - 6.0, filled, 12.0, 6.0, ink);
    }
    let percent = format!("{:.0} %", volume.level * 100.0);
    text(&mut pixmap, font, &percent, w - 66.0, mid, 34.0, ink, true);
    let mut bytes = pixmap.take();
    for pixel in bytes.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some(bytes)
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

    #[test]
    fn draws_the_level_as_a_light_line() {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            return; // no system font on this machine
        };
        let volume = Volume {
            level: 0.5,
            muted: false,
        };
        let bytes = draw(volume, &font, WIDTH, HEIGHT).unwrap();
        let at = |x: u32| {
            let index = (((HEIGHT / 2) * WIDTH + x) * 4) as usize;
            bytes[index]
        };
        // Filled up to half the line, dark after.
        assert!(at(140) > 0xE0);
        assert!(at(WIDTH - 160) < 0x40);
    }
}

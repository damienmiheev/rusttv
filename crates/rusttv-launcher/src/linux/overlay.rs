//! The app overlay: holding Home over an app shows a panel at the right
//! edge, over the app (a `wlr-layer-shell` overlay surface that takes the
//! keyboard), with the app's size, the volume and closing the app. The app
//! stays visible beside it, dimmed. Nothing of the home screen comes up.
//!
//! Every input works in it: Up and Down (remote, keyboard, controller) pick
//! a row, Left and Right step its value, OK runs it, Back closes the
//! overlay. The mouse focuses the row under it and a click is OK (on ‹ or ›
//! a step); the wheel, or a trackpad's two-finger scroll, steps the row
//! under the pointer; a click beside the panel closes it. When it closes,
//! the launcher hears how ([`crate::ui::overlay_closed`]).
//!
//! The volume bar ([`VolumeBar`]) shows over apps the same way, when a
//! volume key is pressed while an app is in front.

mod draw;
mod panel;
mod volume_bar;

use std::fs::File;
use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use ab_glyph::FontVec;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_keyboard::{self, WlKeyboard},
    wl_pointer::{self, WlPointer},
    wl_registry::WlRegistry,
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy as _, QueueHandle, WEnum, delegate_noop,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{
        self as layer_surface, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
    },
};

use crate::device::{DeviceSettings, Volume};
use crate::platform::{AppOverlay, OverlayClosed, OverlayTexts};
use draw::{Hit, Part};
use panel::{Effect, Panel};
pub use volume_bar::VolumeBar;

/// Linux input event codes (`linux/input-event-codes.h`) the overlay reads.
mod code {
    pub const ESC: u32 = 1;
    pub const BACKSPACE: u32 = 14;
    pub const ENTER: u32 = 28;
    pub const KP_ENTER: u32 = 96;
    pub const UP: u32 = 103;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const DOWN: u32 = 108;
    /// `KEY_COMPOSE`, xkb's `Menu` (a controller's top button).
    pub const MENU: u32 = 127;
    pub const BACK: u32 = 158;
    pub const HOMEPAGE: u32 = 172;
    pub const OK: u32 = 352;
    pub const SELECT: u32 = 353;
    pub const BTN_LEFT: u32 = 0x110;
    pub const BTN_RIGHT: u32 = 0x111;
    /// A mouse's back button.
    pub const BTN_SIDE: u32 = 0x113;
}

/// Scroll distance (surface units) per step: a wheel notch is about 15, a
/// trackpad sends many small amounts.
const SCROLL_STEP: f64 = 10.0;

/// Whether the overlay is on screen (game controllers steer it then).
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether the app overlay is on screen.
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// What the launcher tells an open overlay.
enum Command {
    /// Close; `to_home`: Home was pressed.
    Close { to_home: bool },
    /// The volume changed (a volume key).
    Volume(Volume),
}

/// An open app overlay; it runs on a thread of its own until it closes.
#[derive(Debug)]
pub struct Overlay {
    commands: Sender<Command>,
    /// Wakes the overlay's thread for a command.
    wake: OwnedFd,
}

impl Overlay {
    /// Opens the overlay; `None` without a font or a thread for it. Without
    /// a compositor that offers a layer shell it closes again at once.
    pub fn open(overlay: AppOverlay, device: Arc<dyn DeviceSettings>) -> Option<Self> {
        let Some(font) = crate::linux::keyboard::draw::load_font() else {
            log::warn!("no font for the app overlay");
            return None;
        };
        let (wake_read, wake) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )
        .map_err(|error| log::warn!("no pipe for the app overlay: {error}"))
        .ok()?;
        let (commands, received) = mpsc::channel();
        let app = overlay.app.clone();
        let spawned = std::thread::Builder::new()
            .name("rusttv-overlay".into())
            .spawn(move || {
                let closed =
                    run(overlay, device, font, &received, &wake_read).unwrap_or_else(|error| {
                        log::warn!("app overlay: {error}");
                        OverlayClosed {
                            app,
                            size: None,
                            close_app: false,
                            to_home: false,
                        }
                    });
                SHOWN.store(false, Ordering::Relaxed);
                // The app gets the keys back: its text field may ask again.
                crate::linux::keyboard::window_switched();
                crate::ui::overlay_closed(closed);
            });
        if let Err(error) = spawned {
            log::warn!("no thread for the app overlay: {error}");
            return None;
        }
        Some(Self { commands, wake })
    }

    /// Closes the overlay; `to_home`: Home was pressed.
    pub fn close(&self, to_home: bool) {
        self.send(Command::Close { to_home });
    }

    /// Shows a new volume.
    pub fn volume(&self, volume: Volume) {
        self.send(Command::Volume(volume));
    }

    fn send(&self, command: Command) {
        // Fails once the overlay closed; then there is nothing to tell.
        if self.commands.send(command).is_ok() {
            let _ = rustix::io::write(&self.wake, &[1]);
        }
    }
}

/// A `wl_shm` buffer with these ARGB8888 pixels; it destroys itself once
/// the compositor lets go of it.
fn shm_buffer<D>(
    shm: &WlShm,
    pixels: &[u8],
    width: u32,
    height: u32,
    qh: &QueueHandle<D>,
) -> Result<WlBuffer, String>
where
    D: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, ()> + 'static,
{
    let fd: OwnedFd = rustix::fs::memfd_create("rusttv-overlay", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|error| error.to_string())?;
    let mut file = File::from(fd);
    file.write_all(pixels).map_err(|error| error.to_string())?;
    let size = i32::try_from(pixels.len()).map_err(|error| error.to_string())?;
    let width = i32::try_from(width).map_err(|error| error.to_string())?;
    let height = i32::try_from(height).map_err(|error| error.to_string())?;
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(
        0,
        width,
        height,
        width * 4,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

/// Waits for Wayland events, or for the wake pipe (`None`: no limit, else
/// at most this long), and reads what came. `true` if the pipe woke it.
fn wait<D>(
    queue: &mut EventQueue<D>,
    state: &mut D,
    wake: &OwnedFd,
    timeout: Option<std::time::Duration>,
) -> Result<bool, String> {
    use rustix::event::{PollFd, PollFlags, Timespec};

    queue
        .dispatch_pending(state)
        .map_err(|error| error.to_string())?;
    queue.flush().map_err(|error| error.to_string())?;
    let Some(guard) = queue.prepare_read() else {
        return Ok(false);
    };
    let timespec = timeout.and_then(|timeout| Timespec::try_from(timeout).ok());
    let (wayland, woken) = {
        let connection = guard.connection_fd();
        let mut fds = [
            PollFd::new(&connection, PollFlags::IN),
            PollFd::new(wake, PollFlags::IN),
        ];
        match rustix::event::poll(&mut fds, timespec.as_ref()) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.to_string()),
        }
        (
            !fds[0].revents().is_empty(),
            fds[1].revents().contains(PollFlags::IN),
        )
    };
    if wayland {
        match guard.read() {
            Ok(_) => {}
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    if woken {
        let mut bytes = [0_u8; 64];
        while matches!(rustix::io::read(wake, &mut bytes), Ok(read) if read > 0) {}
    }
    queue
        .dispatch_pending(state)
        .map_err(|error| error.to_string())?;
    Ok(woken)
}

fn run(
    overlay: AppOverlay,
    device: Arc<dyn DeviceSettings>,
    font: FontVec,
    commands: &Receiver<Command>,
    wake: &OwnedFd,
) -> Result<OverlayClosed, String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let bind = |name: &'static str| move |error| format!("no {name}: {error}");
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).map_err(bind("compositor"))?;
    let layer_shell: ZwlrLayerShellV1 =
        globals.bind(&qh, 1..=4, ()).map_err(bind("layer shell"))?;
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(bind("shm"))?;
    // Its capabilities bring the keyboard and the pointer.
    let _seat: WlSeat = globals.bind(&qh, 1..=5, ()).map_err(bind("seat"))?;

    let surface = compositor.create_surface(&qh, ());
    let layer = layer_shell.get_layer_surface(
        &surface,
        None,
        Layer::Overlay,
        "rusttv-overlay".into(),
        &qh,
        (),
    );
    layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
    layer.set_size(0, 0);
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    surface.commit();

    let volume = device.volume_now();
    let mut state = State {
        shm,
        surface,
        layer,
        size: None,
        keyboard: None,
        pointer: None,
        font,
        panel: Panel::new(overlay.size, volume),
        title: overlay.title,
        texts: overlay.texts,
        device,
        pointer_at: None,
        scroll: 0.0,
        ok_down: false,
        done: None,
    };
    SHOWN.store(true, Ordering::Relaxed);
    let (close_app, to_home) = loop {
        if let Some(done) = state.done {
            break done;
        }
        if wait(&mut queue, &mut state, wake, None)? {
            while let Ok(command) = commands.try_recv() {
                match command {
                    Command::Close { to_home } => state.done = Some((false, to_home)),
                    Command::Volume(volume) => {
                        if state.panel.volume.is_some() {
                            state.panel.volume = Some(volume);
                            state.redraw(&qh);
                        }
                    }
                }
            }
        }
    };
    if let Some(keyboard) = state.keyboard.take()
        && keyboard.version() >= 3
    {
        keyboard.release();
    }
    if let Some(pointer) = state.pointer.take()
        && pointer.version() >= 3
    {
        pointer.release();
    }
    state.layer.destroy();
    state.surface.destroy();
    let _ = queue.flush();
    Ok(OverlayClosed {
        app: overlay.app,
        size: state.panel.size,
        close_app,
        to_home,
    })
}

struct State {
    shm: WlShm,
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    /// The surface's size, once the compositor said.
    size: Option<(u32, u32)>,
    keyboard: Option<WlKeyboard>,
    /// The seat's pointer (a mouse, trackpad or air mouse).
    pointer: Option<WlPointer>,
    font: FontVec,
    panel: Panel,
    title: String,
    texts: OverlayTexts,
    device: Arc<dyn DeviceSettings>,
    /// Where the pointer is on the overlay, if it is.
    pointer_at: Option<(f64, f64)>,
    /// Scrolled and not stepped yet.
    scroll: f64,
    /// OK went down here (its release runs the row).
    ok_down: bool,
    /// Closing: (close the app, Home was pressed).
    done: Option<(bool, bool)>,
}

impl State {
    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        let Some((width, height)) = self.size else {
            return;
        };
        let Some(pixels) = draw::draw(
            &self.panel,
            &self.title,
            &self.texts,
            &self.font,
            width,
            height,
        ) else {
            return;
        };
        match shm_buffer(&self.shm, &pixels, width, height, qh) {
            Ok(buffer) => {
                self.surface.attach(Some(&buffer), 0, 0);
                self.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                self.surface.commit();
            }
            Err(error) => log::warn!("drawing the app overlay: {error}"),
        }
    }

    fn apply(&mut self, effect: Effect, qh: &QueueHandle<Self>) {
        match effect {
            Effect::None => {}
            Effect::Redraw => self.redraw(qh),
            Effect::Volume(change) => {
                if let Some(volume) = self.device.volume(change) {
                    self.panel.volume = Some(volume);
                }
                self.redraw(qh);
            }
            Effect::CloseApp => self.done = Some((true, false)),
            Effect::Dismiss => self.done = Some((false, false)),
        }
    }

    fn hit(&self) -> Option<Hit> {
        let (x, y) = self.pointer_at?;
        let (width, _) = self.size?;
        Some(draw::hit(width, self.panel.rows().len(), x, y))
    }

    /// The pointer moved: the row under it gets the focus.
    fn hover(&mut self, x: f64, y: f64) -> Effect {
        self.pointer_at = Some((x, y));
        match self.hit() {
            Some(Hit::Row { index, .. }) if self.panel.focus_on(index) => Effect::Redraw,
            _ => Effect::None,
        }
    }

    /// A left click: OK on the row, a step on ‹ or ›; beside the panel it
    /// closes the overlay.
    fn click(&mut self) -> Effect {
        match self.hit() {
            Some(Hit::Outside) => Effect::Dismiss,
            Some(Hit::Row { index, part }) => match part {
                Part::Minus => self.panel.step(index, false),
                Part::Plus => self.panel.step(index, true),
                Part::Body => self.panel.activate(index),
            },
            Some(Hit::Panel) | None => Effect::None,
        }
    }

    /// Scrolled by `amount` (down is positive): each [`SCROLL_STEP`] steps
    /// the value of the row under the pointer (up is more), or moves the
    /// focus where that row has none.
    fn scroll(&mut self, amount: f64) -> Effect {
        if self.scroll.signum() != amount.signum() {
            self.scroll = 0.0;
        }
        self.scroll += amount;
        if self.scroll.abs() < SCROLL_STEP {
            return Effect::None;
        }
        let up = self.scroll < 0.0;
        self.scroll = 0.0;
        let index = match self.hit() {
            Some(Hit::Row { index, .. }) => index,
            _ => self.panel.focus(),
        };
        match self.panel.step(index, up) {
            Effect::None => self.panel.move_focus(if up { -1 } else { 1 }),
            effect => effect,
        }
    }
}

/// A key from the remote, a keyboard or a controller. OK runs the row on
/// its release, and only after its press came here (not the release of a
/// key that opened the overlay); `ok_down` keeps that.
fn key_effect(panel: &mut Panel, ok_down: &mut bool, key: u32, pressed: bool) -> Effect {
    let focus = panel.focus();
    match (key, pressed) {
        (code::UP, true) => panel.move_focus(-1),
        (code::DOWN, true) => panel.move_focus(1),
        (code::LEFT, true) => panel.step(focus, false),
        (code::RIGHT, true) => panel.step(focus, true),
        (code::ENTER | code::KP_ENTER | code::OK | code::SELECT, true) => {
            *ok_down = true;
            Effect::None
        }
        (code::ENTER | code::KP_ENTER | code::OK | code::SELECT, false) if *ok_down => {
            *ok_down = false;
            panel.activate(focus)
        }
        (code::ESC | code::BACK | code::BACKSPACE | code::MENU | code::HOMEPAGE, true) => {
            Effect::Dismiss
        }
        _ => Effect::None,
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: WEnum::Value(key_state),
            ..
        } = event
        {
            let pressed = key_state == wl_keyboard::KeyState::Pressed;
            let effect = key_effect(&mut state.panel, &mut state.ok_down, key, pressed);
            state.apply(effect, qh);
        }
    }
}

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let effect = match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if surface == state.surface {
                    state.hover(surface_x, surface_y)
                } else {
                    state.pointer_at = None;
                    Effect::None
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_at = None;
                Effect::None
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } if state.pointer_at.is_some() => state.hover(surface_x, surface_y),
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Released),
                ..
            } => match button {
                code::BTN_LEFT => state.click(),
                code::BTN_RIGHT | code::BTN_SIDE => Effect::Dismiss,
                _ => Effect::None,
            },
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } => state.scroll(value),
            _ => Effect::None,
        };
        state.apply(effect, qh);
    }
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
            layer_surface::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                if width > 0 && height > 0 {
                    state.size = Some((width, height));
                }
                state.redraw(qh);
            }
            layer_surface::Event::Closed => state.done = Some((false, false)),
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
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
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_walk_the_rows() {
        let mut panel = Panel::new(Some(100), None);
        let mut ok = false;
        let mut key = |panel: &mut Panel, code, pressed| key_effect(panel, &mut ok, code, pressed);
        assert_eq!(key(&mut panel, code::RIGHT, true), Effect::Redraw);
        assert_eq!(panel.size, Some(110));
        // An OK released without its press here does nothing.
        assert_eq!(key(&mut panel, code::ENTER, false), Effect::None);
        assert_eq!(key(&mut panel, code::DOWN, true), Effect::Redraw);
        assert_eq!(key(&mut panel, code::SELECT, true), Effect::None);
        assert_eq!(key(&mut panel, code::SELECT, false), Effect::CloseApp);
        assert_eq!(key(&mut panel, code::BACK, true), Effect::Dismiss);
        assert_eq!(key(&mut panel, code::MENU, true), Effect::Dismiss);
        // Volume up and a letter do nothing.
        assert_eq!(key(&mut panel, 115, true), Effect::None);
        assert_eq!(key(&mut panel, 30, true), Effect::None);
    }
}

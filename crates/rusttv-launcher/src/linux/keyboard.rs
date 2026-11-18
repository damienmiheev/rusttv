//! The on-screen keyboard over other apps: an input method
//! (`input-method-unstable-v2`, labwc and other wlroots compositors).
//!
//! When a text field in an app gets focus, the compositor activates the
//! input method: the keyboard shows at the bottom of the screen (a
//! `wlr-layer-shell` overlay that never takes focus) and takes the remote's
//! keys (`layout.rs`); each key types straight into the app, and the
//! launcher's own text fields (a Wi-Fi password) take it the same way. ✓ closes it and presses Enter (through a virtual keyboard); Back
//! closes it and gives the keys back to the app until a text field gets
//! focus again. Typing on a real keyboard (or a phone's, through the
//! remote desktop portal) closes it and passes the keys on to the app.

pub(super) mod draw;
mod layout;

use std::fs::File;
use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};

use ab_glyph::FontVec;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_keyboard,
    wl_pointer::{self, WlPointer},
    wl_registry::WlRegistry,
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy as _, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_keyboard_grab_v2::{self as grab, ZwpInputMethodKeyboardGrabV2},
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self as input_method, ZwpInputMethodV2},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{
        self as layer_surface, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1,
    },
};

use crate::model::Input;
use layout::{BottomKey, TextPrompt};

/// Linux input event codes (`linux/input-event-codes.h`) the keyboard uses.
mod code {
    pub const ESC: u32 = 1;
    pub const BACKSPACE: u32 = 14;
    pub const ENTER: u32 = 28;
    pub const KP_ENTER: u32 = 96;
    pub const UP: u32 = 103;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const DOWN: u32 = 108;
    pub const BACK: u32 = 158;
    pub const OK: u32 = 352;
    pub const SELECT: u32 = 353;

    /// OK on a remote, Enter on a keyboard.
    pub fn is_ok(key: u32) -> bool {
        matches!(key, ENTER | KP_ENTER | OK | SELECT)
    }

    /// Keys that type or edit text on a real keyboard (letters, digits,
    /// punctuation, space, Tab, the keypad, Home/End, Page Up/Down,
    /// Insert/Delete) rather than steer the on-screen keyboard.
    pub fn types(key: u32) -> bool {
        matches!(
            key,
            2..=13 | 15..=27 | 30..=41 | 43..=53 | 55 | 57 | 71..=83 | 86 | 98 | 102 | 104 | 107
                | 109..=111
        )
    }
}

/// Whether the keyboard is on screen (game controllers steer it then).
static SHOWN: AtomicBool = AtomicBool::new(false);

/// An app window came to the front (or got the keys back from the app
/// overlay), and no text field took the focus since.
static FRESH: AtomicBool = AtomicBool::new(false);

/// An app window came to the front, or the app overlay gave the keys back
/// to the app. A text field that has the focus as the app comes up (Dolphin
/// puts it in its location bar) was put there by the app, not by the
/// person: the keyboard waits for OK on it. Moving to a field brings the
/// keyboard up as ever.
pub fn window_switched() {
    FRESH.store(true, Ordering::Relaxed);
}

/// Whether the on-screen keyboard is showing.
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// `text-input-v3` content purposes the keyboard tells apart.
const PURPOSE_URL: u32 = 5;
const PURPOSE_EMAIL: u32 = 6;
const PURPOSE_PASSWORD: u32 = 8;
const PURPOSE_PIN: u32 = 9;

/// What a key does while the keyboard shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Effect {
    /// Nothing to send; draw again if `true`.
    Redraw(bool),
    /// Type this into the app.
    Type(String),
    /// Delete the character before the cursor in the app.
    Delete,
    /// Close the keyboard and press Enter in the app.
    Enter,
    /// Close the keyboard; the app gets the keys again.
    Hide,
    /// Someone types on a real keyboard (or a phone's, through the remote
    /// desktop portal): close the keyboard and pass this key on to the app.
    Pass,
}

/// Applies a remote key to the keyboard.
fn key(prompt: &mut TextPrompt, key: u32, pressed: bool) -> Effect {
    let input = match (key, pressed) {
        (code::UP, true) => Input::Up,
        (code::DOWN, true) => Input::Down,
        (code::LEFT, true) => Input::Left,
        (code::RIGHT, true) => Input::Right,
        (code::ESC | code::BACK, true) => return Effect::Hide,
        (code::BACKSPACE, true) => {
            prompt.delete();
            return Effect::Delete;
        }
        (code::ENTER | code::KP_ENTER | code::SELECT, false) => Input::OkReleased,
        (key, true) if code::types(key) => return Effect::Pass,
        _ => return Effect::Redraw(false),
    };
    if input != Input::OkReleased {
        prompt.handle(input);
        return Effect::Redraw(true);
    }
    match prompt.bottom_key() {
        Some(BottomKey::Delete) => {
            prompt.delete();
            Effect::Delete
        }
        Some(BottomKey::Done) => Effect::Enter,
        _ => {
            let before = prompt.text().len();
            prompt.handle(input);
            let typed = prompt.text().get(before..).unwrap_or_default();
            if typed.is_empty() {
                Effect::Redraw(true)
            } else {
                Effect::Type(typed.to_owned())
            }
        }
    }
}

/// The keyboard's hint and whether it hides what is typed, by purpose.
fn title(purpose: u32) -> (&'static str, bool) {
    match purpose {
        PURPOSE_PASSWORD | PURPOSE_PIN => ("Password", true),
        PURPOSE_URL => ("Address", false),
        PURPOSE_EMAIL => ("Email", false),
        _ => ("Type", false),
    }
}

/// Starts the keyboard on a thread of its own. Without a compositor that
/// offers an input method (or a system font), apps get no keyboard.
pub fn start() {
    let Some(font) = draw::load_font() else {
        log::warn!("no font for the on-screen keyboard");
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("rusttv-keyboard".into())
        .spawn(move || {
            if let Err(error) = run(font) {
                log::warn!("on-screen keyboard for apps: {error}");
            }
        });
    if let Err(error) = spawned {
        log::warn!("no thread for the on-screen keyboard: {error}");
    }
}

fn bind_error(name: &'static str) -> impl Fn(wayland_client::globals::BindError) -> String {
    move |error| format!("no {name}: {error}")
}

fn run(font: FontVec) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    let seat: WlSeat = globals.bind(&qh, 1..=5, ()).map_err(bind_error("seat"))?;
    let manager: ZwpInputMethodManagerV2 = globals
        .bind(&qh, 1..=1, ())
        .map_err(bind_error("input method"))?;
    let state = State {
        input_method: manager.get_input_method(&seat, &qh, ()),
        compositor: globals
            .bind(&qh, 1..=4, ())
            .map_err(bind_error("compositor"))?,
        shm: globals.bind(&qh, 1..=1, ()).map_err(bind_error("shm"))?,
        layer_shell: globals
            .bind(&qh, 1..=4, ())
            .map_err(bind_error("layer shell"))?,
        virtual_keyboards: globals.bind(&qh, 1..=1, ()).ok(),
        seat,
        font,
        serial: 0,
        pending_active: None,
        pending_purpose: 0,
        active: false,
        purpose: 0,
        grab: None,
        virtual_keyboard: None,
        passthrough: None,
        passing: false,
        waiting: false,
        swallow: None,
        modifiers: [0; 4],
        surface: None,
        prompt: TextPrompt::new("", false),
        pointer_at: None,
        echoes: 0,
        pointer: None,
    };
    log::info!("on-screen keyboard for apps ready");
    let mut state = state;
    loop {
        queue
            .blocking_dispatch(&mut state)
            .map_err(|error| error.to_string())?;
    }
}

struct Surface {
    surface: WlSurface,
    layer: ZwlrLayerSurfaceV1,
    size: Option<(u32, u32)>,
}

struct State {
    input_method: ZwpInputMethodV2,
    compositor: WlCompositor,
    shm: WlShm,
    layer_shell: ZwlrLayerShellV1,
    virtual_keyboards: Option<ZwpVirtualKeyboardManagerV1>,
    seat: WlSeat,
    font: FontVec,
    /// `done` events so far: the serial of `commit`.
    serial: u32,
    pending_active: Option<bool>,
    pending_purpose: u32,
    active: bool,
    purpose: u32,
    grab: Option<ZwpInputMethodKeyboardGrabV2>,
    virtual_keyboard: Option<ZwpVirtualKeyboardV1>,
    /// Passes a real keyboard's keys on to the app, with the keymap of
    /// whichever keyboard sent them last (the grab says which).
    passthrough: Option<ZwpVirtualKeyboardV1>,
    /// A real keyboard is typing: keys still coming through the grab go on
    /// to the app, until a text field asks for the keyboard again.
    passing: bool,
    /// A text field has the focus since its app came up: keys go on to the
    /// app, and OK brings the keyboard up ([`window_switched`]).
    waiting: bool,
    /// The OK that brought the keyboard up: its release is no key press.
    swallow: Option<u32>,
    /// The grab's modifiers: depressed, latched, locked, group.
    modifiers: [u32; 4],
    surface: Option<Surface>,
    prompt: TextPrompt,
    /// Where the pointer is over the keyboard, if it is.
    pointer_at: Option<(f64, f64)>,
    /// Backspaces sent through the virtual keyboard that may still come
    /// back through the grab (compositors that don't pass the input
    /// method's own keys around it); ignored, so they don't delete again.
    echoes: u32,
    /// The seat's pointer (the air mouse: hovering focuses a key, a click
    /// presses it), taken when the seat says it has one: a Bluetooth remote
    /// often connects after the launcher started, and a pointer asked for
    /// before that never gets events.
    pointer: Option<WlPointer>,
}

impl State {
    fn show(&mut self, qh: &QueueHandle<Self>) {
        let (title, secret) = title(self.purpose);
        self.prompt = TextPrompt::new(title, secret);
        self.passing = false;
        self.waiting = false;
        SHOWN.store(true, Ordering::Relaxed);
        if self.grab.is_none() {
            self.grab = Some(self.input_method.grab_keyboard(qh, ()));
        }
        if self.surface.is_none() {
            let surface = self.compositor.create_surface(qh, ());
            let layer = self.layer_shell.get_layer_surface(
                &surface,
                None,
                Layer::Overlay,
                "rusttv-keyboard".into(),
                qh,
                (),
            );
            layer.set_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer.set_size(0, draw::height(1920));
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            surface.commit();
            self.surface = Some(Surface {
                surface,
                layer,
                size: None,
            });
        } else {
            self.redraw(qh);
        }
    }

    /// A text field has the focus, but the keyboard waits for OK on it:
    /// the keys are taken and passed on to the app meanwhile.
    fn wait(&mut self, qh: &QueueHandle<Self>) {
        self.waiting = true;
        self.passing = true;
        if self.grab.is_none() {
            self.grab = Some(self.input_method.grab_keyboard(qh, ()));
        }
    }

    fn hide(&mut self) {
        SHOWN.store(false, Ordering::Relaxed);
        self.waiting = false;
        self.echoes = 0;
        if let Some(grab) = self.grab.take() {
            grab.release();
        }
        if let Some(shown) = self.surface.take() {
            shown.layer.destroy();
            shown.surface.destroy();
        }
    }

    fn redraw(&mut self, qh: &QueueHandle<Self>) {
        let Some(shown) = &self.surface else {
            return;
        };
        let Some((width, height)) = shown.size else {
            return;
        };
        let Some(pixels) = draw::draw(&self.prompt, &self.font, width, height) else {
            return;
        };
        match self.buffer(&pixels, width, height, qh) {
            Ok(buffer) => {
                shown.surface.attach(Some(&buffer), 0, 0);
                shown.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
                shown.surface.commit();
            }
            Err(error) => log::warn!("drawing the on-screen keyboard: {error}"),
        }
    }

    /// A `wl_shm` buffer with these pixels; it destroys itself once the
    /// compositor lets go of it.
    fn buffer(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        qh: &QueueHandle<Self>,
    ) -> Result<WlBuffer, String> {
        let fd: OwnedFd =
            rustix::fs::memfd_create("rusttv-keyboard", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| error.to_string())?;
        let mut file = File::from(fd);
        file.write_all(pixels).map_err(|error| error.to_string())?;
        let size = i32::try_from(pixels.len()).map_err(|error| error.to_string())?;
        let width = i32::try_from(width).map_err(|error| error.to_string())?;
        let height = i32::try_from(height).map_err(|error| error.to_string())?;
        let pool = self.shm.create_pool(file.as_fd(), size, qh, ());
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

    /// Passes a key on to the app through the pass-through keyboard.
    fn pass(&self, time: u32, key: u32, pressed: bool) {
        if let Some(keyboard) = &self.passthrough {
            let [depressed, latched, locked, group] = self.modifiers;
            keyboard.modifiers(depressed, latched, locked, group);
            keyboard.key(time, key, u32::from(pressed));
        }
    }

    fn apply(&mut self, effect: Effect, time: u32, key: u32, qh: &QueueHandle<Self>) {
        match effect {
            Effect::Redraw(false) => {}
            Effect::Redraw(true) => self.redraw(qh),
            Effect::Type(text) => {
                self.input_method.commit_string(text);
                self.input_method.commit(self.serial);
                self.redraw(qh);
            }
            Effect::Delete => {
                // A Backspace key rather than delete_surrounding_text, which
                // some toolkits ignore (winit, so RustTV's own text fields).
                if let Some(keyboard) = &self.virtual_keyboard {
                    self.echoes += 1;
                    keyboard.key(time, code::BACKSPACE, 1);
                    keyboard.key(time.saturating_add(1), code::BACKSPACE, 0);
                } else {
                    self.input_method.delete_surrounding_text(1, 0);
                    self.input_method.commit(self.serial);
                }
                self.redraw(qh);
            }
            Effect::Hide => self.hide(),
            Effect::Pass => {
                // The grab goes first, so the following keys reach the app.
                self.hide();
                self.passing = true;
                self.pass(time, key, true);
            }
            Effect::Enter => {
                // The grab goes first, so the Enter reaches the app.
                self.hide();
                if let Some(keyboard) = &self.virtual_keyboard {
                    let pressed = wl_keyboard::KeyState::Pressed as u32;
                    let released = wl_keyboard::KeyState::Released as u32;
                    keyboard.key(time, code::ENTER, pressed);
                    keyboard.key(time.saturating_add(1), code::ENTER, released);
                }
            }
        }
    }
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: input_method::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            input_method::Event::Activate => {
                state.pending_active = Some(true);
                state.pending_purpose = 0;
            }
            input_method::Event::Deactivate => state.pending_active = Some(false),
            input_method::Event::ContentType {
                purpose: WEnum::Value(purpose),
                ..
            } => state.pending_purpose = u32::from(purpose),
            input_method::Event::Done => {
                state.serial = state.serial.wrapping_add(1);
                state.purpose = state.pending_purpose;
                match state.pending_active.take() {
                    Some(true) => {
                        state.active = true;
                        if FRESH.swap(false, Ordering::Relaxed) {
                            state.wait(qh);
                        } else {
                            state.show(qh);
                        }
                    }
                    Some(false) => {
                        state.active = false;
                        state.hide();
                    }
                    None => {}
                }
            }
            input_method::Event::Unavailable => {
                log::warn!("another input method is running; no on-screen keyboard for apps");
                state.hide();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: grab::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            grab::Event::Keymap { format, fd, size } => {
                // The virtual keyboard (for Enter) uses the seat's keymap.
                if state.virtual_keyboard.is_none()
                    && let (Some(manager), WEnum::Value(format)) =
                        (&state.virtual_keyboards, format)
                {
                    let keyboard = manager.create_virtual_keyboard(&state.seat, qh, ());
                    keyboard.keymap(format as u32, fd.as_fd(), size);
                    state.virtual_keyboard = Some(keyboard);
                }
                // Keys passed on mean what they meant on their keyboard.
                if let (Some(manager), WEnum::Value(format)) = (&state.virtual_keyboards, format) {
                    let keyboard = state.passthrough.get_or_insert_with(|| {
                        manager.create_virtual_keyboard(&state.seat, qh, ())
                    });
                    keyboard.keymap(format as u32, fd.as_fd(), size);
                }
            }
            grab::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                state.modifiers = [mods_depressed, mods_latched, mods_locked, group];
                if state.passing
                    && let Some(keyboard) = &state.passthrough
                {
                    keyboard.modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            grab::Event::Key {
                time,
                key: pressed_key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                if state.waiting && pressed && code::is_ok(pressed_key) {
                    state.swallow = Some(pressed_key);
                    state.show(qh);
                    return;
                }
                if !pressed && state.swallow == Some(pressed_key) {
                    state.swallow = None;
                    return;
                }
                if state.passing {
                    state.pass(time, pressed_key, pressed);
                    return;
                }
                if !state.active || state.surface.is_none() {
                    return;
                }
                if pressed_key == code::BACKSPACE && state.echoes > 0 {
                    if !pressed {
                        state.echoes -= 1;
                    }
                    return;
                }
                let effect = key(&mut state.prompt, pressed_key, pressed);
                state.apply(effect, time, pressed_key, qh);
            }
            _ => {}
        }
    }
}

impl State {
    /// The pointer moved to (`x`, `y`) on the keyboard: focus follows it.
    fn hover(&mut self, x: f64, y: f64, qh: &QueueHandle<Self>) {
        self.pointer_at = Some((x, y));
        let Some((width, _)) = self.surface.as_ref().and_then(|shown| shown.size) else {
            return;
        };
        if let Some((row, column)) = draw::key_at(width, x, y)
            && self.prompt.focus() != (row, column)
        {
            self.prompt.focus_on(row, column);
            self.redraw(qh);
        }
    }

    /// A click: presses the key under the pointer as OK would.
    fn click(&mut self, time: u32, qh: &QueueHandle<Self>) {
        let (Some((x, y)), Some((width, _))) = (
            self.pointer_at,
            self.surface.as_ref().and_then(|shown| shown.size),
        ) else {
            return;
        };
        let Some((row, column)) = draw::key_at(width, x, y) else {
            return;
        };
        self.prompt.focus_on(row, column);
        let effect = key(&mut self.prompt, code::ENTER, false);
        self.apply(effect, time, code::ENTER, qh);
    }
}

/// `BTN_LEFT` in `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;

impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let ours = |surface: &WlSurface, state: &Self| {
            state
                .surface
                .as_ref()
                .is_some_and(|shown| &shown.surface == surface)
        };
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                if ours(&surface, state) {
                    state.hover(surface_x, surface_y, qh);
                } else {
                    state.pointer_at = None;
                }
            }
            wl_pointer::Event::Leave { .. } => state.pointer_at = None,
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if state.pointer_at.is_some() {
                    state.hover(surface_x, surface_y, qh);
                }
            }
            wl_pointer::Event::Button {
                time,
                button: BTN_LEFT,
                state: WEnum::Value(wl_pointer::ButtonState::Released),
                ..
            } => state.click(time, qh),
            _ => {}
        }
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
                let wanted = draw::height(width);
                if let Some(shown) = &mut state.surface {
                    if height != wanted {
                        // Sized for this screen's width.
                        shown.layer.set_size(0, wanted);
                    }
                    shown.size = Some((width, wanted));
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
            let has_pointer = capabilities.contains(wl_seat::Capability::Pointer);
            match (has_pointer, state.pointer.take()) {
                (true, None) => state.pointer = Some(seat.get_pointer(qh, ())),
                (true, Some(pointer)) => state.pointer = Some(pointer),
                (false, Some(pointer)) => {
                    state.pointer_at = None;
                    if pointer.version() >= 3 {
                        pointer.release();
                    }
                }
                (false, None) => {}
            }
        }
    }
}
delegate_noop!(State: WlCompositor);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ZwlrLayerShellV1);
delegate_noop!(State: ZwpInputMethodManagerV2);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    fn tap(prompt: &mut TextPrompt, code: u32) -> Effect {
        key(prompt, code, true);
        key(prompt, code, false)
    }

    #[test]
    fn ok_types_the_focused_key_into_the_app_and_arrows_move() {
        let mut prompt = TextPrompt::new("Type", false);
        assert_eq!(tap(&mut prompt, code::ENTER), Effect::Type("q".into()));
        assert_eq!(key(&mut prompt, code::RIGHT, true), Effect::Redraw(true));
        assert_eq!(tap(&mut prompt, code::SELECT), Effect::Type("w".into()));
        assert_eq!(prompt.text(), "qw");
    }

    #[test]
    fn the_bottom_row_deletes_in_the_app_switches_layers_and_finishes_with_enter() {
        let mut prompt = TextPrompt::new("Type", false);
        for _ in 0..3 {
            key(&mut prompt, code::DOWN, true);
        }
        // Bottom row, under q: the layer key.
        assert_eq!(tap(&mut prompt, code::ENTER), Effect::Redraw(true));
        assert!(prompt.capitals());
        key(&mut prompt, code::RIGHT, true);
        key(&mut prompt, code::RIGHT, true);
        // Delete works even with nothing typed here: the app has text.
        assert_eq!(tap(&mut prompt, code::ENTER), Effect::Delete);
        key(&mut prompt, code::RIGHT, true);
        assert_eq!(tap(&mut prompt, code::ENTER), Effect::Enter);
    }

    #[test]
    fn back_hides_typing_passes_on_and_other_keys_do_nothing() {
        let mut prompt = TextPrompt::new("Type", false);
        assert_eq!(key(&mut prompt, code::BACK, true), Effect::Hide);
        assert_eq!(key(&mut prompt, code::ESC, true), Effect::Hide);
        // Volume up and a lone Shift.
        assert_eq!(key(&mut prompt, 115, true), Effect::Redraw(false));
        assert_eq!(key(&mut prompt, 42, true), Effect::Redraw(false));
        // H, space and Delete from a real keyboard go to the app.
        assert_eq!(key(&mut prompt, 35, true), Effect::Pass);
        assert_eq!(key(&mut prompt, 57, true), Effect::Pass);
        assert_eq!(key(&mut prompt, 111, true), Effect::Pass);
        assert_eq!(key(&mut prompt, 35, false), Effect::Redraw(false));
        assert_eq!(prompt.text(), "");
        assert_eq!(key(&mut prompt, code::BACKSPACE, true), Effect::Delete);
    }

    #[test]
    fn passwords_are_hidden() {
        assert_eq!(title(PURPOSE_PASSWORD), ("Password", true));
        assert_eq!(title(0), ("Type", false));
    }
}

//! The portal's input into the compositor: a virtual pointer
//! (`wlr-virtual-pointer`) and virtual keyboards (`virtual-keyboard`).
//!
//! Keys come with the seat's keymap, so a remote client's key codes mean
//! what they mean on the TV's own keyboard. Text goes through a second
//! virtual keyboard whose keymap holds just the characters to type (as
//! `wtype` does), so any character can be typed whatever the layout.

use std::fs::File;
use std::io::Write as _;
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_keyboard::{self, WlKeyboard},
    wl_output::{self, WlOutput},
    wl_pointer,
    wl_registry::WlRegistry,
    wl_seat::WlSeat,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// `wl_keyboard.keymap_format.xkb_v1`.
const XKB_V1: u32 = 1;

/// Distinct characters per text keymap; longer texts take several.
const TEXT_KEYS: usize = 200;

/// The compositor connection and the virtual devices.
pub(super) struct Output {
    pub(super) connection: Connection,
    pub(super) queue: EventQueue<Devices>,
    pub(super) devices: Devices,
}

/// The virtual devices and what the compositor told about the seat.
pub(super) struct Devices {
    seat: WlSeat,
    keyboards: ZwpVirtualKeyboardManagerV1,
    pointer: ZwlrVirtualPointerV1,
    /// Types keys with the seat's keymap, once the compositor sent it.
    keyboard: Option<ZwpVirtualKeyboardV1>,
    /// Types text with keymaps of its own.
    text: ZwpVirtualKeyboardV1,
    /// The seat's keymap (an xkb keymap file) and its size.
    keymap: Option<(OwnedFd, u32)>,
    /// The output's size in pixels, for absolute pointer motion.
    size: Option<(u32, u32)>,
}

impl Output {
    /// Connects to the compositor (`WAYLAND_DISPLAY`) and creates the
    /// devices.
    pub(super) fn connect() -> Result<Self, String> {
        let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<Devices>(&connection).map_err(|error| error.to_string())?;
        let qh = queue.handle();
        let missing = |name: &'static str| move |error| format!("no {name}: {error}");
        let seat: WlSeat = globals.bind(&qh, 1..=7, ()).map_err(missing("seat"))?;
        let pointers: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .map_err(missing("virtual pointer"))?;
        let keyboards: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(missing("virtual keyboard"))?;
        // The first output's mode, for absolute motion (a TV has one).
        let _output: Option<WlOutput> = globals.bind(&qh, 1..=2, ()).ok();
        // Only for its keymap: the portal never has the keyboard focus.
        let _keyboard = seat.get_keyboard(&qh, ());
        let mut devices = Devices {
            pointer: pointers.create_virtual_pointer(Some(&seat), &qh, ()),
            text: keyboards.create_virtual_keyboard(&seat, &qh, ()),
            keyboards,
            seat,
            keyboard: None,
            keymap: None,
            size: None,
        };
        // The keymap and the mode arrive in reply to the binds.
        for _ in 0..2 {
            queue
                .roundtrip(&mut devices)
                .map_err(|error| error.to_string())?;
        }
        Ok(Self {
            connection,
            queue,
            devices,
        })
    }

    /// Reads and handles what the compositor sent (a new keymap).
    pub(super) fn dispatch(&mut self) -> Result<(), String> {
        if let Some(guard) = self.queue.prepare_read() {
            // Nothing to read is no error: the loop woke for another source.
            let _ = guard.read();
        }
        self.queue
            .dispatch_pending(&mut self.devices)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Sends what the devices did.
    pub(super) fn flush(&self) {
        if let Err(error) = self.connection.flush() {
            log::warn!("compositor: {error}");
        }
    }
}

/// Milliseconds for Wayland from libei's microseconds (both monotonic).
#[expect(
    clippy::cast_possible_truncation,
    reason = "Wayland times are milliseconds that wrap around"
)]
fn millis(micros: u64) -> u32 {
    (micros / 1000) as u32
}

impl Devices {
    /// The seat's keymap for the EI keyboard, if the compositor sent one.
    pub(super) fn keymap(&self) -> Option<(BorrowedFd<'_>, u32)> {
        self.keymap.as_ref().map(|(fd, size)| (fd.as_fd(), *size))
    }

    /// The output's size, if the compositor told it.
    pub(super) fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    pub(super) fn motion(&self, time: u64, dx: f32, dy: f32) {
        self.pointer
            .motion(millis(time), f64::from(dx), f64::from(dy));
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "positions on the output, clamped to it"
    )]
    pub(super) fn motion_absolute(&self, time: u64, x: f32, y: f32) {
        let Some((width, height)) = self.size else {
            return;
        };
        let x = x.clamp(0.0, width.saturating_sub(1) as f32) as u32;
        let y = y.clamp(0.0, height.saturating_sub(1) as f32) as u32;
        self.pointer
            .motion_absolute(millis(time), x, y, width, height);
    }

    pub(super) fn button(&self, time: u64, button: u32, pressed: bool) {
        let state = if pressed {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        self.pointer.button(millis(time), button, state);
    }

    /// Smooth scrolling (a finger on the phone), in pixels.
    pub(super) fn scroll(&self, time: u64, dx: f32, dy: f32) {
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        for (axis, value) in [
            (wl_pointer::Axis::HorizontalScroll, dx),
            (wl_pointer::Axis::VerticalScroll, dy),
        ] {
            if value != 0.0 {
                self.pointer.axis(millis(time), axis, f64::from(value));
            }
        }
    }

    /// The finger left: scrolling stops (kinetic scrolling may follow).
    pub(super) fn scroll_stop(&self, time: u64, x: bool, y: bool) {
        self.pointer.axis_source(wl_pointer::AxisSource::Finger);
        if x {
            self.pointer
                .axis_stop(millis(time), wl_pointer::Axis::HorizontalScroll);
        }
        if y {
            self.pointer
                .axis_stop(millis(time), wl_pointer::Axis::VerticalScroll);
        }
    }

    /// Wheel clicks, in 120ths of a click.
    pub(super) fn scroll_discrete(&self, time: u64, dx: i32, dy: i32) {
        self.pointer.axis_source(wl_pointer::AxisSource::Wheel);
        for (axis, value) in [
            (wl_pointer::Axis::HorizontalScroll, dx),
            (wl_pointer::Axis::VerticalScroll, dy),
        ] {
            if value != 0 {
                // 15 pixels per click, as libinput reports a wheel.
                let pixels = f64::from(value) * 15.0 / 120.0;
                self.pointer
                    .axis_discrete(millis(time), axis, pixels, value / 120);
            }
        }
    }

    /// Ends a group of pointer events.
    pub(super) fn frame(&self) {
        self.pointer.frame();
    }

    /// A key by its Linux input event code, with the seat's keymap.
    pub(super) fn key(&self, time: u64, code: u32, pressed: bool) {
        if let Some(keyboard) = &self.keyboard {
            keyboard.key(millis(time), code, u32::from(pressed));
        }
    }

    /// Types `text` through the text keyboard.
    pub(super) fn type_text(&self, time: u64, text: &str) {
        for chunk in text_keymaps(text) {
            let Some(fd) = keymap_file(&chunk.keymap) else {
                return;
            };
            let size = u32::try_from(chunk.keymap.len() + 1).unwrap_or(u32::MAX);
            self.text.keymap(XKB_V1, fd.as_fd(), size);
            for code in chunk.codes {
                self.text.key(millis(time), code, 1);
                self.text.key(millis(time), code, 0);
            }
        }
    }
}

/// A keymap for some characters, and the key codes that type them in order.
#[derive(Debug, PartialEq, Eq)]
struct TextKeymap {
    keymap: String,
    /// Linux input event codes (the keymap's key codes minus 8).
    codes: Vec<u32>,
}

/// The xkb name of the key symbol that types `character`, if any.
fn keysym_name(character: char) -> Option<String> {
    match character {
        '\n' | '\r' => Some("Return".to_owned()),
        '\t' => Some("Tab".to_owned()),
        '\u{8}' => Some("BackSpace".to_owned()),
        character if character.is_control() => None,
        // xkbcommon reads U+0020..U+00FF as Latin-1 key symbols and
        // anything above as Unicode ones.
        character => Some(format!("U{:04X}", u32::from(character))),
    }
}

/// Keymaps that type `text`, at most [`TEXT_KEYS`] characters each.
fn text_keymaps(text: &str) -> Vec<TextKeymap> {
    let mut keymaps = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut codes = Vec::new();
    for character in text.chars() {
        let Some(name) = keysym_name(character) else {
            continue;
        };
        let index = if let Some(index) = names.iter().position(|known| *known == name) {
            index
        } else {
            if names.len() == TEXT_KEYS {
                keymaps.push(text_keymap(&names, std::mem::take(&mut codes)));
                names.clear();
            }
            names.push(name);
            names.len() - 1
        };
        // Key code 9 + index; input event codes are key codes minus 8.
        codes.push(u32::try_from(index).unwrap_or(0) + 1);
    }
    if !codes.is_empty() {
        keymaps.push(text_keymap(&names, codes));
    }
    keymaps
}

fn text_keymap(names: &[String], codes: Vec<u32>) -> TextKeymap {
    let mut keycodes = String::new();
    let mut symbols = String::new();
    for (index, name) in names.iter().enumerate() {
        let code = index + 9;
        keycodes.push_str(&format!("<K{code}> = {code};\n"));
        symbols.push_str(&format!("key <K{code}> {{ [ {name} ] }};\n"));
    }
    let keymap = format!(
        "xkb_keymap {{\n\
         xkb_keycodes \"rusttv\" {{\nminimum = 8;\nmaximum = {};\n{keycodes}}};\n\
         xkb_types \"rusttv\" {{ include \"complete\" }};\n\
         xkb_compat \"rusttv\" {{ include \"complete\" }};\n\
         xkb_symbols \"rusttv\" {{\n{symbols}}};\n\
         }};\n",
        names.len() + 9
    );
    TextKeymap { keymap, codes }
}

/// A sealed-off memory file holding `keymap` and a closing NUL.
fn keymap_file(keymap: &str) -> Option<OwnedFd> {
    let fd = rustix::fs::memfd_create("rusttv-portal-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|error| log::warn!("no keymap file: {error}"))
        .ok()?;
    let mut file = File::from(fd);
    file.write_all(keymap.as_bytes())
        .and_then(|()| file.write_all(&[0]))
        .map_err(|error| log::warn!("keymap file: {error}"))
        .ok()?;
    Some(OwnedFd::from(file))
}

impl Dispatch<WlKeyboard, ()> for Devices {
    fn event(
        devices: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap {
            format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
            fd,
            size,
        } = event
            // Only the first: the seat's own keymap. Later ones are mostly
            // our virtual keyboards' (the compositor passes on the keymap of
            // the keyboard used last); answering those sent a keymap back and
            // forth until the connection broke.
            && devices.keymap.is_none()
        {
            let keyboard = devices.keyboard.get_or_insert_with(|| {
                devices
                    .keyboards
                    .create_virtual_keyboard(&devices.seat, qh, ())
            });
            keyboard.keymap(XKB_V1, fd.as_fd(), size);
            devices.keymap = Some((fd, size));
        }
    }
}

impl Dispatch<WlOutput, ()> for Devices {
    fn event(
        devices: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode {
            flags: WEnum::Value(flags),
            width,
            height,
            ..
        } = event
            && flags.contains(wl_output::Mode::Current)
            && let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height))
        {
            devices.size = Some((width, height));
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Devices {
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

delegate_noop!(Devices: ignore WlSeat);
delegate_noop!(Devices: ZwlrVirtualPointerManagerV1);
delegate_noop!(Devices: ZwlrVirtualPointerV1);
delegate_noop!(Devices: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Devices: ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_types_each_character_once_per_key() {
        let keymaps = text_keymaps("Hä€a\n");
        assert_eq!(keymaps.len(), 1);
        let keymap = &keymaps[0];
        assert_eq!(keymap.codes, [1, 2, 3, 4, 5]);
        assert!(keymap.keymap.contains("key <K9> { [ U0048 ] };"));
        assert!(keymap.keymap.contains("key <K10> { [ U00E4 ] };"));
        assert!(keymap.keymap.contains("key <K11> { [ U20AC ] };"));
        assert!(keymap.keymap.contains("key <K13> { [ Return ] };"));
        assert!(keymap.keymap.contains("maximum = 14;"));
    }

    #[test]
    fn repeated_characters_share_a_key_and_controls_are_dropped() {
        let keymaps = text_keymaps("aa\u{1}b");
        assert_eq!(keymaps[0].codes, [1, 1, 2]);
        assert!(text_keymaps("\u{1}").is_empty());
    }

    #[test]
    fn long_texts_take_several_keymaps() {
        let text: String = (0..TEXT_KEYS + 10)
            .filter_map(|index| char::from_u32(0x4E00 + u32::try_from(index).unwrap_or(0)))
            .collect();
        let keymaps = text_keymaps(&text);
        assert_eq!(keymaps.len(), 2);
        assert_eq!(keymaps[0].codes.len(), TEXT_KEYS);
        assert_eq!(keymaps[1].codes, (1..=10).collect::<Vec<_>>());
    }
}

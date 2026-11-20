//! Game controllers steer the TV like the remote: the D-pad, the hat and
//! the left stick are the arrow keys, A is OK, B and Select are Back, and
//! the Guide button is Home.
//!
//! Every gamepad under `/dev/input` (found again every few seconds, so a
//! Bluetooth pad that connects later works too) is read on a thread of its
//! own; the keys go out through a virtual keyboard with a keymap of just
//! those keys (the seat may have none when this starts), so they reach whatever takes the remote's keys: the home screen, its
//! text fields, and the on-screen keyboard. While an app is in front only
//! Home goes out, so a game gets the controller to itself.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use evdev::{AbsoluteAxisCode, Device, EventType, KeyCode};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_registry::WlRegistry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};

/// Whether the home screen is in front (set by the app-window tracker):
/// only then do the controller's arrows, OK and Back go out.
pub static HOME_IN_FRONT: AtomicBool = AtomicBool::new(true);

/// Whether the app in front takes the controller as keys: an app the
/// launcher started that reads no controller itself (not a game, not Kodi).
/// Games get no extra keys, so nothing happens twice.
pub static KEYS_TO_APP: AtomicBool = AtomicBool::new(false);

/// How often to look for newly connected controllers.
const RESCAN: Duration = Duration::from_secs(3);

/// A stick counts as pushed past this share of its travel, and as back in
/// the middle below the second (so it doesn't flicker at the edge).
const STICK_PUSHED: f32 = 0.6;
const STICK_CENTRED: f32 = 0.4;

/// Linux input event codes (`linux/input-event-codes.h`) sent on.
mod key {
    pub const ESC: u32 = 1;
    /// `KEY_COMPOSE`, xkb's `Menu`: the tile menu (as a long OK).
    pub const MENU: u32 = 127;
    pub const ENTER: u32 = 28;
    pub const UP: u32 = 103;
    pub const LEFT: u32 = 105;
    pub const RIGHT: u32 = 106;
    pub const DOWN: u32 = 108;
    pub const HOMEPAGE: u32 = 172;
    /// `KEY_PROG1` (`XF86Launch1`): Home held, the app menu.
    pub const PROG1: u32 = 148;
}

/// How long Guide is held before it opens the app overlay (as holding the
/// remote's Home does); let go sooner, it is Home.
const HOLD: Duration = Duration::from_millis(600);

/// Gamepad button codes read.
mod button {
    pub const SOUTH: u16 = 0x130;
    pub const EAST: u16 = 0x131;
    /// The top and left face buttons (Y and X on Xbox pads; drivers differ
    /// in which is which).
    pub const NORTH: u16 = 0x133;
    pub const WEST: u16 = 0x134;
    pub const SELECT: u16 = 0x13a;
    pub const MODE: u16 = 0x13c;
    pub const DPAD_UP: u16 = 0x220;
    pub const DPAD_DOWN: u16 = 0x221;
    pub const DPAD_LEFT: u16 = 0x222;
    pub const DPAD_RIGHT: u16 = 0x223;
}

/// Gamepad axis codes read.
mod axis {
    pub const X: u16 = 0x00;
    pub const Y: u16 = 0x01;
    pub const HAT0X: u16 = 0x10;
    pub const HAT0Y: u16 = 0x11;
}

/// A key going down or up.
type KeyChange = (u32, bool);

/// Turns one controller's events into key changes.
#[derive(Debug, Default)]
struct Mapper {
    /// Axis ranges: minimum and maximum.
    ranges: HashMap<u16, (i32, i32)>,
    /// The key each axis holds down, if any.
    held: HashMap<u16, u32>,
}

impl Mapper {
    fn new(ranges: HashMap<u16, (i32, i32)>) -> Self {
        Self {
            ranges,
            held: HashMap::new(),
        }
    }

    fn button(code: u16, value: i32) -> Vec<KeyChange> {
        let key = match code {
            button::SOUTH => key::ENTER,
            button::EAST | button::SELECT => key::ESC,
            button::MODE => key::HOMEPAGE,
            button::NORTH | button::WEST => key::MENU,
            button::DPAD_UP => key::UP,
            button::DPAD_DOWN => key::DOWN,
            button::DPAD_LEFT => key::LEFT,
            button::DPAD_RIGHT => key::RIGHT,
            _ => return Vec::new(),
        };
        // 2 is the kernel's autorepeat: the app repeats held keys itself.
        match value {
            0 => vec![(key, false)],
            1 => vec![(key, true)],
            _ => Vec::new(),
        }
    }

    /// An axis moved: a hat or stick direction is a held arrow key.
    fn axis(&mut self, code: u16, value: i32) -> Vec<KeyChange> {
        let (negative, positive) = match code {
            axis::X | axis::HAT0X => (key::LEFT, key::RIGHT),
            axis::Y | axis::HAT0Y => (key::UP, key::DOWN),
            _ => return Vec::new(),
        };
        let held = self.held.get(&code).copied();
        let wanted = if matches!(code, axis::HAT0X | axis::HAT0Y) {
            match value.signum() {
                -1 => Some(negative),
                1 => Some(positive),
                _ => None,
            }
        } else {
            let Some(&(minimum, maximum)) = self.ranges.get(&code) else {
                return Vec::new();
            };
            let half = (f64::from(maximum) - f64::from(minimum)) / 2.0;
            if half <= 0.0 {
                return Vec::new();
            }
            let middle = f64::from(minimum) + half;
            #[expect(clippy::cast_possible_truncation, reason = "a share between -1 and 1")]
            let share = ((f64::from(value) - middle) / half) as f32;
            if share.abs() >= STICK_PUSHED {
                Some(if share < 0.0 { negative } else { positive })
            } else if share.abs() > STICK_CENTRED {
                held
            } else {
                None
            }
        };
        if wanted == held {
            return Vec::new();
        }
        let mut changes = Vec::new();
        if let Some(key) = held {
            changes.push((key, false));
            self.held.remove(&code);
        }
        if let Some(key) = wanted {
            changes.push((key, true));
            self.held.insert(code, key);
        }
        changes
    }
}

/// Starts looking for controllers on a thread of its own.
pub fn start() {
    let spawned = std::thread::Builder::new()
        .name("rusttv-gamepads".into())
        .spawn(|| {
            if let Err(error) = run() {
                log::warn!("no game controllers: {error}");
            }
        });
    if let Err(error) = spawned {
        log::warn!("no thread for game controllers: {error}");
    }
}

/// What the controller threads report.
enum Message {
    Keys(Vec<KeyChange>),
    Gone(PathBuf),
}

fn run() -> Result<(), String> {
    let keyboard = VirtualKeys::connect()?;
    let (sender, messages) = mpsc::channel();
    let mut open = HashSet::new();
    let mut pressed = HashSet::new();
    let mut scanned = Instant::now()
        .checked_sub(RESCAN)
        .unwrap_or_else(Instant::now);
    // Since when Guide (Home) is down, while nothing went out for it yet:
    // released before `HOLD` it is Home, held it is the app overlay.
    let mut home_down: Option<Instant> = None;
    loop {
        if scanned.elapsed() >= RESCAN {
            scanned = Instant::now();
            scan(&mut open, &sender);
        }
        let wait = home_down.map_or(RESCAN, |since| HOLD.saturating_sub(since.elapsed()));
        match messages.recv_timeout(wait) {
            Ok(Message::Keys(changes)) => {
                for (key, down) in changes {
                    if key == key::HOMEPAGE {
                        if down {
                            home_down = Some(Instant::now());
                        } else if home_down.take().is_some() {
                            keyboard.key(key::HOMEPAGE, true);
                            keyboard.key(key::HOMEPAGE, false);
                        }
                        continue;
                    }
                    send(&keyboard, &mut pressed, key, down);
                }
                keyboard.flush()?;
            }
            Err(RecvTimeoutError::Timeout)
                if home_down.is_some_and(|since| since.elapsed() >= HOLD) =>
            {
                home_down = None;
                keyboard.key(key::PROG1, true);
                keyboard.key(key::PROG1, false);
                keyboard.flush()?;
            }
            Ok(Message::Gone(path)) => {
                open.remove(&path);
                for key in std::mem::take(&mut pressed) {
                    keyboard.key(key, false);
                }
                keyboard.flush()?;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Sends a key on: Home always, the rest only to the home screen. A key
/// that went down always comes up again.
fn send(keyboard: &VirtualKeys, pressed: &mut HashSet<u32>, key: u32, down: bool) {
    if down {
        let to_home = HOME_IN_FRONT.load(Ordering::Relaxed)
            || KEYS_TO_APP.load(Ordering::Relaxed)
            || super::keyboard::shown()
            || super::overlay::shown();
        if (key == key::HOMEPAGE || key == key::PROG1 || to_home) && pressed.insert(key) {
            keyboard.key(key, true);
        }
    } else if pressed.remove(&key) {
        keyboard.key(key, false);
    }
}

/// Opens controllers not open yet, each read on its own thread.
fn scan(open: &mut HashSet<PathBuf>, sender: &Sender<Message>) {
    for (path, device) in evdev::enumerate() {
        if open.contains(&path) || !is_gamepad(&device) {
            continue;
        }
        let name = device.name().unwrap_or("controller").to_owned();
        let ranges = device
            .get_absinfo()
            .map(|axes| {
                axes.map(|(code, info): (AbsoluteAxisCode, _)| {
                    (code.0, (info.minimum(), info.maximum()))
                })
                .collect()
            })
            .unwrap_or_default();
        let sender = sender.clone();
        let read_path = path.clone();
        let spawned = std::thread::Builder::new()
            .name("rusttv-gamepad".into())
            .spawn(move || read(device, Mapper::new(ranges), &read_path, &sender));
        match spawned {
            Ok(_) => {
                log::info!("game controller {name} ({})", path.display());
                open.insert(path);
            }
            Err(error) => log::warn!("no thread for {name}: {error}"),
        }
    }
}

/// A gamepad has a South (A) button and a stick or hat. Key remappers
/// (some TV remotes' drivers) declare every key code, buttons too, but
/// no axes.
fn is_gamepad(device: &Device) -> bool {
    let has_button = device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::BTN_SOUTH));
    let has_axis = device.supported_absolute_axes().is_some_and(|axes| {
        axes.contains(AbsoluteAxisCode::ABS_X) || axes.contains(AbsoluteAxisCode::ABS_HAT0X)
    });
    has_button && has_axis
}

fn read(mut device: Device, mut mapper: Mapper, path: &Path, sender: &Sender<Message>) {
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(error) => {
                log::info!("game controller {} gone: {error}", path.display());
                let _ = sender.send(Message::Gone(path.to_path_buf()));
                return;
            }
        };
        let mut changes = Vec::new();
        for event in events {
            match event.event_type() {
                EventType::KEY => changes.extend(Mapper::button(event.code(), event.value())),
                EventType::ABSOLUTE => changes.extend(mapper.axis(event.code(), event.value())),
                _ => {}
            }
        }
        if !changes.is_empty() && sender.send(Message::Keys(changes)).is_err() {
            return;
        }
    }
}

/// A virtual keyboard with the seat's keymap.
struct VirtualKeys {
    connection: Connection,
    keyboard: ZwpVirtualKeyboardV1,
    start: Instant,
}

impl VirtualKeys {
    fn connect() -> Result<Self, String> {
        let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
        let (globals, queue) =
            registry_queue_init::<Globals>(&connection).map_err(|error| error.to_string())?;
        let qh = queue.handle();
        let seat: WlSeat = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| format!("no seat: {error}"))?;
        let manager: ZwpVirtualKeyboardManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| format!("no virtual keyboards: {error}"))?;
        let (fd, size) = keymap_file(&keymap()).map_err(|error| format!("keymap: {error}"))?;
        let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());
        keyboard.keymap(wl_keyboard::KeymapFormat::XkbV1 as u32, fd.as_fd(), size);
        connection.flush().map_err(|error| error.to_string())?;
        Ok(Self {
            connection,
            keyboard,
            start: Instant::now(),
        })
    }

    fn key(&self, key: u32, down: bool) {
        let time = u32::try_from(self.start.elapsed().as_millis()).unwrap_or(u32::MAX);
        self.keyboard.key(time, key, u32::from(down));
    }

    fn flush(&self) -> Result<(), String> {
        self.connection.flush().map_err(|error| error.to_string())
    }
}

/// An xkb keymap with the keys a controller sends, under their own codes.
fn keymap() -> String {
    let keys = [
        (key::ESC, "Escape"),
        (key::MENU, "Menu"),
        (key::ENTER, "Return"),
        (key::UP, "Up"),
        (key::LEFT, "Left"),
        (key::RIGHT, "Right"),
        (key::DOWN, "Down"),
        (key::HOMEPAGE, "XF86HomePage"),
        (key::PROG1, "XF86Launch1"),
    ];
    let mut keycodes = String::new();
    let mut symbols = String::new();
    for (code, name) in keys {
        let code = code + 8;
        keycodes.push_str(&format!(
            "<K{code}> = {code};
"
        ));
        symbols.push_str(&format!(
            "key <K{code}> {{ [ {name} ] }};
"
        ));
    }
    format!(
        "xkb_keymap {{
         xkb_keycodes \"rusttv\" {{
minimum = 8;
maximum = 255;
{keycodes}}};
         xkb_types \"rusttv\" {{ include \"complete\" }};
         xkb_compat \"rusttv\" {{ include \"complete\" }};
         xkb_symbols \"rusttv\" {{
{symbols}}};
         }};
"
    )
}

/// A memory file holding `keymap` and a closing NUL, and its size.
fn keymap_file(keymap: &str) -> std::io::Result<(OwnedFd, u32)> {
    let fd = rustix::fs::memfd_create("rusttv-gamepad-keymap", rustix::fs::MemfdFlags::CLOEXEC)?;
    let mut file = File::from(fd);
    file.write_all(keymap.as_bytes())?;
    file.write_all(&[0])?;
    let size = u32::try_from(keymap.len() + 1).map_err(std::io::Error::other)?;
    Ok((OwnedFd::from(file), size))
}

/// Nothing to handle: the connection only sends.
struct Globals;

impl Dispatch<WlRegistry, GlobalListContents> for Globals {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Globals: ignore WlSeat);
delegate_noop!(Globals: ZwpVirtualKeyboardManagerV1);
delegate_noop!(Globals: ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_are_the_remotes_keys_and_repeats_are_left_to_the_app() {
        assert_eq!(Mapper::button(button::SOUTH, 1), vec![(key::ENTER, true)]);
        assert_eq!(Mapper::button(button::SOUTH, 0), vec![(key::ENTER, false)]);
        assert_eq!(Mapper::button(button::EAST, 1), vec![(key::ESC, true)]);
        assert_eq!(Mapper::button(button::MODE, 1), vec![(key::HOMEPAGE, true)]);
        assert_eq!(Mapper::button(button::NORTH, 1), vec![(key::MENU, true)]);
        assert_eq!(
            Mapper::button(button::DPAD_LEFT, 1),
            vec![(key::LEFT, true)]
        );
        assert_eq!(Mapper::button(button::DPAD_LEFT, 2), vec![]);
        assert_eq!(Mapper::button(button::WEST, 1), vec![(key::MENU, true)]);
        // A shoulder button: nothing.
        assert_eq!(Mapper::button(0x136, 1), vec![]);
    }

    #[test]
    fn the_hat_holds_one_arrow_at_a_time() {
        let mut mapper = Mapper::default();
        assert_eq!(mapper.axis(axis::HAT0Y, -1), vec![(key::UP, true)]);
        assert_eq!(
            mapper.axis(axis::HAT0Y, 1),
            vec![(key::UP, false), (key::DOWN, true)]
        );
        assert_eq!(mapper.axis(axis::HAT0Y, 0), vec![(key::DOWN, false)]);
        assert_eq!(mapper.axis(axis::HAT0Y, 0), vec![]);
    }

    #[test]
    fn the_stick_presses_an_arrow_once_pushed_well_out_and_lets_go_near_the_middle() {
        let mut mapper = Mapper::new(HashMap::from([(axis::X, (0, 255))]));
        assert_eq!(mapper.axis(axis::X, 140), vec![]);
        assert_eq!(mapper.axis(axis::X, 250), vec![(key::RIGHT, true)]);
        // Between the two thresholds: still held.
        assert_eq!(mapper.axis(axis::X, 190), vec![]);
        assert_eq!(mapper.axis(axis::X, 130), vec![(key::RIGHT, false)]);
        assert_eq!(mapper.axis(axis::X, 0), vec![(key::LEFT, true)]);
    }

    #[test]
    fn the_keymap_names_every_key_sent() {
        let keymap = keymap();
        assert!(keymap.contains("<K36> = 36;"));
        assert!(keymap.contains("key <K180> { [ XF86HomePage ] };"));
        assert!(keymap.contains("key <K9> { [ Escape ] };"));
    }

    #[test]
    fn a_stick_without_a_known_range_does_nothing() {
        let mut mapper = Mapper::default();
        assert_eq!(mapper.axis(axis::Y, -32_768), vec![]);
    }
}

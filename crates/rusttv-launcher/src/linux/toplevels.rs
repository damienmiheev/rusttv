//! The compositor's app windows, via `wlr-foreign-toplevel-management`
//! (labwc, sway, Wayfire, Hyprland, niri, …): which apps have a window, which
//! one is in front, and bringing one to the front.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, Proxy as _, QueueHandle, event_created_child};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

/// `zwlr_foreign_toplevel_handle_v1.state` value of the window in front.
const ACTIVATED: u32 = 2;

/// Called with a window's app ID when it comes to the front.
pub type OnActivated = Box<dyn Fn(&str) + Send>;

/// The app windows the compositor reports, updated on a thread of their own.
#[derive(Clone)]
pub struct Toplevels {
    shared: Arc<Mutex<Shared>>,
    connection: Connection,
    seat: WlSeat,
}

impl std::fmt::Debug for Toplevels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toplevels").finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Shared {
    windows: Vec<Window>,
    /// Apps left while they were starting (Home): the first window of one
    /// that comes to the front before `until` is minimized instead.
    held: Vec<Held>,
}

/// An app whose window must not cover Home ([`Toplevels::hold_back`]).
struct Held {
    /// App IDs of its window, lower case.
    keys: Vec<String>,
    until: Instant,
}

impl Shared {
    /// Whether a window of `app_id` coming to the front is held back; it is
    /// so only once.
    fn take_held(&mut self, app_id: &str, now: Instant) -> bool {
        self.held.retain(|held| held.until > now);
        let app_id = app_id.to_lowercase();
        let Some(index) = self
            .held
            .iter()
            .position(|held| held.keys.contains(&app_id))
        else {
            return false;
        };
        self.held.remove(index);
        true
    }
}

struct Window {
    handle: ZwlrForeignToplevelHandleV1,
    app_id: String,
    activated: bool,
    /// State received since the last `done`.
    pending_app_id: Option<String>,
    pending_activated: Option<bool>,
}

impl Toplevels {
    /// Connects to the compositor (`WAYLAND_DISPLAY`) and follows its windows.
    /// `None` if there is no compositor or it lacks the protocol.
    pub fn start(on_activated: OnActivated) -> Option<Self> {
        let connection = Connection::connect_to_env()
            .map_err(|error| log::warn!("no Wayland connection for the app windows: {error}"))
            .ok()?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| log::warn!("reading the Wayland globals failed: {error}"))
            .ok()?;
        let qh = queue.handle();
        let _manager: ZwlrForeignToplevelManagerV1 = globals
            .bind(&qh, 1..=3, ())
            .map_err(|error| {
                log::warn!(
                    "the compositor does not list app windows ({error}); apps may start twice"
                );
            })
            .ok()?;
        let seat: WlSeat = globals
            .bind(&qh, 1..=1, ())
            .map_err(|error| log::warn!("no Wayland seat: {error}"))
            .ok()?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut state = State {
            shared: shared.clone(),
            on_activated,
        };
        let spawned = std::thread::Builder::new()
            .name("rusttv-windows".into())
            .spawn(move || {
                loop {
                    if let Err(error) = queue.blocking_dispatch(&mut state) {
                        log::warn!("following the app windows stopped: {error}");
                        return;
                    }
                }
            });
        if let Err(error) = spawned {
            log::warn!("no thread for the app windows: {error}");
            return None;
        }
        Some(Self {
            shared,
            connection,
            seat,
        })
    }

    /// The app ID of the window in front, if it is no window of the
    /// launcher's (which has none).
    pub fn front(&self) -> Option<String> {
        lock(&self.shared)
            .windows
            .iter()
            .find(|window| window.activated && !window.app_id.is_empty())
            .map(|window| window.app_id.clone())
    }

    /// Brings the first window whose app ID matches one of `keys`
    /// (lower case) to the front; whether there was one.
    pub fn activate(&self, keys: &[String]) -> bool {
        let shared = lock(&self.shared);
        let Some(window) = shared
            .windows
            .iter()
            .find(|window| keys.contains(&window.app_id.to_lowercase()))
        else {
            return false;
        };
        window.handle.activate(&self.seat);
        if let Err(error) = self.connection.flush() {
            log::warn!("raising {} failed: {error}", window.app_id);
            return false;
        }
        true
    }
}

impl Toplevels {
    /// Keeps the next window of the app with these `keys` (lower case) from
    /// covering Home: it is minimized when it comes to the front, until
    /// `until` or [`Toplevels::release`].
    pub fn hold_back(&self, keys: Vec<String>, until: Instant) {
        let mut shared = lock(&self.shared);
        shared.held.retain(|held| held.keys != keys);
        shared.held.push(Held { keys, until });
    }

    /// The app with these `keys` was opened again: its window may come to
    /// the front.
    pub fn release(&self, keys: &[String]) {
        lock(&self.shared).held.retain(|held| held.keys != keys);
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

struct State {
    shared: Arc<Mutex<Shared>>,
    on_activated: OnActivated,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            lock(&state.shared).windows.push(Window {
                handle: toplevel,
                app_id: String::new(),
                activated: false,
                pending_app_id: None,
                pending_activated: None,
            });
        }
    }

    event_created_child!(State, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut shared = lock(&state.shared);
        let Some(index) = shared
            .windows
            .iter()
            .position(|window| window.handle.id() == proxy.id())
        else {
            return;
        };
        let mut activated_app = None;
        match event {
            handle::Event::AppId { app_id } => shared.windows[index].pending_app_id = Some(app_id),
            handle::Event::State { state: states } => {
                let activated = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|bytes| u32::from_ne_bytes(*bytes))
                    .any(|value| value == ACTIVATED);
                shared.windows[index].pending_activated = Some(activated);
            }
            handle::Event::Done => {
                let window = &mut shared.windows[index];
                if let Some(app_id) = window.pending_app_id.take() {
                    if window.app_id.is_empty() {
                        log::info!("window of {app_id} opened");
                    }
                    window.app_id = app_id;
                }
                if let Some(activated) = window.pending_activated.take() {
                    let came_up = activated && !window.activated;
                    window.activated = activated;
                    if came_up {
                        let app_id = window.app_id.clone();
                        if shared.take_held(&app_id, Instant::now()) {
                            // Home was pressed while it started: Home stays.
                            log::info!("window of {app_id} kept behind Home");
                            shared.windows[index].handle.set_minimized();
                            if let Err(error) = connection.flush() {
                                log::warn!("minimizing {app_id} failed: {error}");
                            }
                        } else {
                            log::info!("window of {app_id} activated");
                            activated_app = Some(app_id);
                        }
                    }
                }
            }
            handle::Event::Closed => {
                let window = shared.windows.remove(index);
                window.handle.destroy();
            }
            _ => {}
        }
        drop(shared);
        if let Some(app_id) = activated_app {
            (state.on_activated)(&app_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn held(keys: &[&str], until: Instant) -> Held {
        Held {
            keys: keys.iter().map(|key| (*key).to_owned()).collect(),
            until,
        }
    }

    #[test]
    fn a_held_app_is_kept_back_once() {
        let now = Instant::now();
        let mut shared = Shared::default();
        shared.held.push(held(
            &["org.kde.kdeconnect.app", "kdeconnect-app"],
            now + Duration::from_secs(30),
        ));
        assert!(!shared.take_held("firefox", now), "another app comes up");
        assert!(
            shared.take_held("org.kde.kdeconnect.App", now),
            "app IDs in any case"
        );
        assert!(
            !shared.take_held("org.kde.kdeconnect.app", now),
            "only its first window"
        );
    }

    #[test]
    fn a_hold_ends_with_the_launch_timeout() {
        let now = Instant::now();
        let mut shared = Shared::default();
        shared
            .held
            .push(held(&["kodi"], now + Duration::from_secs(30)));
        assert!(!shared.take_held("kodi", now + Duration::from_secs(31)));
        assert!(shared.held.is_empty());
    }
}

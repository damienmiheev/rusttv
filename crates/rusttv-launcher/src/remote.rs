//! The phone remote inside the launcher (REMOTE.md § 3–4): starts the
//! listener when a phone is paired, keeps the phones file, turns the model
//! into the view phones mirror, and hands phone events to the UI thread.
//!
//! The listener runs in its own threads (`rusttv-remote`); a forwarding
//! thread waits on its events and delivers each one to the UI thread, so
//! nothing polls while no phone talks. Phone input then goes through the
//! same model calls as the remote's keys (`ui.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rusttv_remote::protocol::{FolderView, HomeMode, HomeView, TextField, TileView, TvMessage};
use rusttv_remote::{Event, Options, PAGE, PORT, Phones, Remote, RemoteError};

use crate::model::{FOLDER_PAGE_SIZE, Home, State, TileId};
use crate::order_file::name_of;

/// Where the paired phones are kept, under the launcher's files directory.
const PHONES_FILE: &str = "remote/phones.toml";

/// The running listener and what was last sent to the phones.
pub(crate) struct Link {
    remote: Remote,
    phones_file: PathBuf,
    last: Option<TvMessage>,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link")
            .field("phones_file", &self.phones_file)
            .finish_non_exhaustive()
    }
}

/// The OS's secure random source (getrandom: the `getrandom` syscall on
/// Android and Linux, `getentropy` on macOS).
fn random(bytes: &mut [u8]) {
    // Fails only where the OS has no random source at all, which no
    // supported TV or computer lacks; a remote with predictable keys must
    // not run, so stop rather than go on.
    #[expect(clippy::expect_used, reason = "no secure randomness is a broken OS")]
    getrandom::fill(bytes).expect("the OS has a secure random source");
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The paired phones, or none: a damaged file is kept as `.bak` and treated
/// as no phones, so it can never let anyone in.
fn load_phones(path: &Path) -> Phones {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Phones::default(),
        Err(error) => {
            log::warn!("can't read the phones file: {error}");
            return Phones::default();
        }
    };
    Phones::parse(&text).unwrap_or_else(|error| {
        log::warn!("the phones file is damaged ({error}); keeping it as .bak");
        let _ = std::fs::rename(path, path.with_extension("toml.bak"));
        Phones::default()
    })
}

/// Writes the phones file atomically (a new file, then a rename).
fn save_phones(path: &Path, phones: &Phones) -> Result<(), String> {
    let text = phones.to_toml().map_err(|error| error.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let partial = path.with_extension("toml.partial");
    std::fs::write(&partial, text).map_err(|error| error.to_string())?;
    std::fs::rename(&partial, path).map_err(|error| error.to_string())
}

impl Link {
    /// Starts the listener if a phone is paired (or `pairing`, the desktop
    /// dev mode's switch, asks for it). `files` is the launcher's files
    /// directory. `deliver` runs on the forwarding thread for each event and
    /// must hand it to the UI thread.
    pub(crate) fn start(
        files: &Path,
        pairing: bool,
        deliver: impl Fn(Event) + Send + 'static,
    ) -> Option<Self> {
        let phones_file = files.join(PHONES_FILE);
        let phones = load_phones(&phones_file);
        if phones.is_empty() && !pairing {
            return None;
        }
        let mut remote = match Remote::start(
            Options {
                port: PORT,
                page: PAGE,
                random: Arc::new(random),
                clock: Arc::new(now),
            },
            phones,
        ) {
            Ok(remote) => remote,
            Err(RemoteError::PortTaken(port)) => {
                log::warn!("RustTV Remote: port {port} is taken; not listening");
                return None;
            }
            Err(error) => {
                log::warn!("RustTV Remote didn't start: {error}");
                return None;
            }
        };
        let events = remote.take_events();
        let spawned = std::thread::Builder::new()
            .name("rusttv-remote-events".to_owned())
            .spawn(move || {
                // Ends when the listener stops (the sender is dropped).
                for event in events {
                    deliver(event);
                }
            });
        if let Err(error) = spawned {
            log::warn!("RustTV Remote: no event thread: {error}");
            return None;
        }
        if pairing {
            let mut secret = [0; 32];
            random(&mut secret);
            remote.open_pairing(secret);
            let hex: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
            log::info!("RustTV Remote: pair at http://<this computer's address>:{PORT}/#p={hex}");
        }
        log::info!("RustTV Remote listening on port {PORT}");
        Some(Self {
            remote,
            phones_file,
            last: None,
        })
    }

    pub(crate) fn remote(&self) -> &Remote {
        &self.remote
    }

    /// Saves the phones file (after pairing, renaming or removing a phone).
    pub(crate) fn save_phones(&self) {
        if let Err(error) = save_phones(&self.phones_file, &self.remote.phones()) {
            log::warn!("can't save the phones file: {error}");
        }
    }

    /// Sends `message` to the phones if it differs from the last one.
    pub(crate) fn push(&mut self, message: TvMessage) {
        if self.last.as_ref() == Some(&message) {
            return;
        }
        if let Err(error) = self.remote.push(&message) {
            log::warn!("RustTV Remote: view not sent: {error}");
        }
        self.last = Some(message);
    }

    /// A phone connected: it gets the current view again.
    pub(crate) fn resend(&mut self) {
        if let Some(message) = self.last.take() {
            self.push(message);
        }
    }
}

/// What Home looks like, for the phones (REMOTE.md § 4).
///
/// `label` names a tile as the TV shows it; `review` is the approval screen
/// (phones get nothing from it and their input is dropped); `rename` is the
/// folder name being typed.
pub(crate) fn home_view(
    home: &Home,
    label: &dyn Fn(&TileId) -> String,
    review: bool,
    rename: Option<String>,
) -> HomeView {
    let mut tiles: Vec<TileView> = home
        .tiles()
        .iter()
        .map(|id| TileView {
            id: name_of(id),
            label: label(id),
            banner: None,
            folder: None,
        })
        .collect();
    for (number, folder) in home.folders() {
        let folder_name = name_of(&TileId::Folder(*number));
        for app in &folder.apps {
            let id = TileId::App(app.clone());
            tiles.push(TileView {
                id: name_of(&id),
                label: label(&id),
                banner: None,
                folder: Some(folder_name.clone()),
            });
        }
    }
    let open = home.open_folder();
    let focused = match open {
        Some(open) => open
            .focus
            .and_then(|index| home.folders().get(&open.id)?.apps.get(index).cloned())
            .map(|app| name_of(&TileId::App(app))),
        None => home.focused().map(name_of),
    };
    let folder = open.and_then(|open| {
        Some(FolderView {
            id: name_of(&TileId::Folder(open.id)),
            name: home.folders().get(&open.id)?.name.clone(),
            page: u16::try_from(open.focus.unwrap_or(0) / FOLDER_PAGE_SIZE).unwrap_or(0),
        })
    });
    let mode = if review {
        HomeMode::Review
    } else if rename.is_some() {
        HomeMode::Text
    } else {
        match home.state() {
            State::Moving => HomeMode::Moving,
            State::Menu => HomeMode::Menu,
            State::Hidden => HomeMode::Hidden,
            State::Settings | State::About => HomeMode::Settings,
            State::Browse | State::Launching | State::Folder => HomeMode::Browse,
        }
    };
    HomeView {
        tiles,
        focused,
        folder,
        mode,
        text: rename.map(|value| TextField {
            label: "Folder name".to_owned(),
            value,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{App, Category, Input, Order};
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;

    fn home() -> Home {
        let mut home = Home::new(Order::default(), BTreeMap::new(), BTreeSet::new(), false);
        let apps: Vec<App> = ["a00", "a01", "a02"]
            .iter()
            .map(|id| App {
                id: format!("com.example.{id}/.Main"),
                label: id.to_uppercase(),
                category: Category::Other,
            })
            .collect();
        home.set_apps(&apps);
        home
    }

    fn label(id: &TileId) -> String {
        match id {
            TileId::App(component) => component.clone(),
            TileId::Folder(n) => format!("Folder {n}"),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn the_view_lists_tiles_in_order_with_focus() {
        let home = home();
        let view = home_view(&home, &label, false, None);
        assert_eq!(view.tiles[0].id, "com.example.a00/.Main");
        assert_eq!(view.focused.as_deref(), Some("com.example.a00/.Main"));
        assert!(view.tiles.iter().any(|tile| tile.id == "about"));
        assert_eq!(view.mode, HomeMode::Browse);
        assert!(view.folder.is_none() && view.text.is_none());
    }

    #[test]
    fn review_and_renaming_show_as_their_modes() {
        let home = home();
        assert_eq!(home_view(&home, &label, true, None).mode, HomeMode::Review);
        let renaming = home_view(&home, &label, false, Some("Stream".into()));
        assert_eq!(renaming.mode, HomeMode::Text);
        assert_eq!(renaming.text.unwrap().value, "Stream");
    }

    #[test]
    fn folder_apps_are_listed_with_their_folder_and_the_open_folder_is_named() {
        let mut home = home();
        // Long press opens the menu; Move; Right onto a01; wait; drop.
        let ms = Duration::from_millis;
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::Tick, ms(700));
        home.handle(Input::OkReleased, ms(710));
        home.handle(Input::OkPressed, ms(720)); // Move
        home.handle(Input::OkReleased, ms(730));
        home.handle(Input::Right, ms(1000));
        home.handle(Input::Tick, ms(1700));
        home.handle(Input::OkPressed, ms(1800));
        home.handle(Input::OkReleased, ms(1810));
        let view = home_view(&home, &label, false, None);
        let inside: Vec<&str> = view
            .tiles
            .iter()
            .filter(|tile| tile.folder.as_deref() == Some("folder:1"))
            .map(|tile| tile.id.as_str())
            .collect();
        assert_eq!(inside.len(), 2, "{view:?}");
        assert!(
            view.tiles
                .iter()
                .any(|tile| tile.id == "folder:1" && tile.folder.is_none())
        );
        let folder = view.folder.expect("the new folder is open");
        assert_eq!(folder.id, "folder:1");
        assert!(view.focused.is_some());
    }

    #[test]
    fn a_damaged_phones_file_means_no_phones_and_is_kept() {
        let dir = std::env::temp_dir().join(format!("rusttv-remote-phones-{}", std::process::id()));
        let path = dir.join(PHONES_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "format = 1\n[[phone]]\nid = \"nope\"\n").unwrap();
        assert!(load_phones(&path).is_empty());
        assert!(path.with_extension("toml.bak").exists());
        assert!(!path.exists());
        save_phones(&path, &Phones::default()).unwrap();
        assert!(load_phones(&path).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}

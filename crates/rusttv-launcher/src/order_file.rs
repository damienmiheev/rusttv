//! Reads and writes the tile order file, `order.toml`: the tile order, hidden
//! apps and the last focused tile.
//!
//! The launcher's own private state, not a change to the TV, so it is outside
//! the CLI safety engine.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{KEY_STEP, Order, TileId};

/// Name used for the About tile in the file. Component names always contain
/// a `/`, so this cannot clash with an app.
const ABOUT: &str = "about";

/// Name used for the Hidden apps tile in the file (only as `focus`).
const HIDDEN: &str = "hidden-apps";

/// Name used for the Settings tile in the file.
const SETTINGS: &str = "settings";

/// File contents. Fields added later are optional, so older files still load.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderFile {
    /// The tile focused when the launcher last left the screen.
    #[serde(default)]
    focus: Option<String>,
    /// Component names of apps kept off Home.
    #[serde(default)]
    hidden: BTreeSet<String>,
    /// Tile name → order key.
    keys: BTreeMap<String, i64>,
}

/// The result of [`load`].
#[derive(Debug)]
pub(crate) struct Loaded {
    /// The stored order, or an empty one (first run) when there is none.
    pub order: Order,
    /// Component names of apps kept off Home.
    pub hidden: BTreeSet<String>,
    /// The tile to focus on start-up, if one was saved.
    pub focus: Option<TileId>,
    /// Set when the file existed but could not be used; it was kept as `.bak`.
    pub warning: Option<String>,
}

/// Reads the order from `path`.
///
/// A missing file is a first run. An unreadable or invalid file is renamed to
/// `<path>.bak` and the launcher starts from the alphabetical order.
pub(crate) fn load(path: &Path) -> Loaded {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Loaded {
                order: Order::default(),
                hidden: BTreeSet::new(),
                focus: None,
                warning: None,
            };
        }
        Err(error) => return set_aside(path, &error.to_string()),
    };
    match toml::from_str::<OrderFile>(&text) {
        Ok(file) => Loaded {
            order: Order::from_keys(
                file.keys
                    .into_iter()
                    .map(|(name, key)| (tile_for_name(name), key)),
            ),
            hidden: file.hidden,
            focus: file.focus.map(tile_for_name),
            warning: None,
        },
        Err(error) => set_aside(path, &error.to_string()),
    }
}

/// The tile a name in the file stands for. [`TileId::Hidden`] in `keys` is
/// dropped when the order meets the app list.
fn tile_for_name(name: String) -> TileId {
    match name.as_str() {
        ABOUT => TileId::About,
        SETTINGS => TileId::Settings,
        HIDDEN => TileId::Hidden,
        _ => TileId::App(name),
    }
}

fn name_of(id: &TileId) -> &str {
    match id {
        TileId::App(component) => component,
        TileId::About => ABOUT,
        TileId::Settings => SETTINGS,
        TileId::Hidden => HIDDEN,
    }
}

/// A TOML string, which is also a valid quoted key.
fn quoted(name: &str) -> toml::Value {
    toml::Value::String(name.to_owned())
}

fn set_aside(path: &Path, reason: &str) -> Loaded {
    let backup = with_suffix(path, ".bak");
    let kept = match fs::rename(path, &backup) {
        Ok(()) => format!("kept as {}", backup.display()),
        Err(error) => format!("could not keep it as {}: {error}", backup.display()),
    };
    Loaded {
        order: Order::default(),
        hidden: BTreeSet::new(),
        focus: None,
        warning: Some(format!(
            "ignoring tile order file {} ({reason}); {kept}",
            path.display()
        )),
    }
}

/// Writes `order`, `hidden` and `focus` to `path` atomically: a temporary
/// file next to it, then a rename.
pub(crate) fn save(
    path: &Path,
    order: &Order,
    hidden: &BTreeSet<String>,
    focus: Option<&TileId>,
) -> io::Result<()> {
    let mut text = String::from("# RustTV launcher tile order. Lower keys come first.\n\n");
    if let Some(focus) = focus {
        text.push_str(&format!("focus = {}\n", quoted(name_of(focus))));
    }
    if !hidden.is_empty() {
        let names: Vec<String> = hidden.iter().map(|name| quoted(name).to_string()).collect();
        text.push_str(&format!("hidden = [{}]\n", names.join(", ")));
    }
    if focus.is_some() || !hidden.is_empty() {
        text.push('\n');
    }
    text.push_str("[keys]\n");
    for (id, key) in order.keys() {
        text.push_str(&format!("{} = {key}\n", quoted(name_of(id))));
    }

    let temporary = with_suffix(path, ".tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)
}

/// The file whose presence turns the launcher's own update check off
/// (HISTORY.md § 8.3). One flag file, read by Rust and by Kotlin's
/// `Updater`, so both see the same switch from the first resume.
pub(crate) const UPDATE_CHECK_OFF: &str = "update-check-off";

/// The launcher's own state as history records it: tiles in order, hidden
/// apps and the update switch. `dir` holds `order.toml` and the flag file.
pub(crate) fn launcher_state(dir: &Path) -> rusttv_history::Launcher {
    let loaded = load(&dir.join("order.toml"));
    rusttv_history::Launcher {
        order: loaded
            .order
            .keys()
            .iter()
            .map(|(id, _)| name_of(id).to_owned())
            .collect(),
        hidden: loaded.hidden,
        update_check: !dir.join(UPDATE_CHECK_OFF).exists(),
    }
}

/// Writes the launcher's state from history: `order.toml` (one atomic
/// rename, keeping the saved focus) and the update flag file.
pub(crate) fn write_launcher_state(dir: &Path, state: &rusttv_history::Launcher) -> io::Result<()> {
    let path = dir.join("order.toml");
    let focus = load(&path).focus;
    let order = Order::from_keys(
        state
            .order
            .iter()
            .zip(1..)
            .map(|(name, index)| (tile_for_name(name.clone()), index * KEY_STEP)),
    );
    save(&path, &order, &state.hidden, focus.as_ref())?;
    let flag = dir.join(UPDATE_CHECK_OFF);
    if state.update_check {
        match fs::remove_file(&flag) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    } else {
        fs::write(&flag, b"")?;
    }
    Ok(())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh empty directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rusttv-order-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self) -> PathBuf {
            self.0.join("order.toml")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample() -> Order {
        Order::from_keys([
            (TileId::App("com.example.b/.Main".to_owned()), 1000),
            (TileId::App("com.example.a/.Tv \"quoted\"".to_owned()), 1500),
            (TileId::About, 2000),
        ])
    }

    #[test]
    fn a_saved_order_loads_back() {
        let dir = TempDir::new("roundtrip");
        save(&dir.file(), &sample(), &BTreeSet::new(), None).unwrap();
        let loaded = load(&dir.file());
        assert_eq!(loaded.order, sample());
        assert_eq!(loaded.focus, None);
        assert!(loaded.warning.is_none());
        assert!(!with_suffix(&dir.file(), ".tmp").exists());
    }

    #[test]
    fn saving_replaces_the_previous_order() {
        let dir = TempDir::new("replace");
        save(&dir.file(), &sample(), &BTreeSet::new(), None).unwrap();
        let shorter = Order::from_keys([(TileId::About, 1000)]);
        save(&dir.file(), &shorter, &BTreeSet::new(), None).unwrap();
        assert_eq!(load(&dir.file()).order, shorter);
    }

    #[test]
    fn the_focused_tile_is_saved_and_loads_back() {
        let dir = TempDir::new("focus");
        let app = TileId::App("com.example.a/.Tv \"quoted\"".to_owned());
        for focus in [app, TileId::About, TileId::Hidden] {
            save(&dir.file(), &sample(), &BTreeSet::new(), Some(&focus)).unwrap();
            let loaded = load(&dir.file());
            assert_eq!(loaded.focus, Some(focus));
            assert_eq!(loaded.order, sample());
        }
    }

    #[test]
    fn hidden_apps_are_saved_and_load_back() {
        let dir = TempDir::new("hidden");
        let hidden = BTreeSet::from([
            "com.example.c/.Main".to_owned(),
            "com.example.d/.Tv \"quoted\"".to_owned(),
        ]);
        let focus = TileId::App("com.example.b/.Main".to_owned());
        save(&dir.file(), &sample(), &hidden, Some(&focus)).unwrap();
        let loaded = load(&dir.file());
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.hidden, hidden);
        assert_eq!(loaded.focus, Some(focus));
        assert_eq!(loaded.order, sample());

        // Showing them all again leaves no `hidden` line.
        save(&dir.file(), &sample(), &BTreeSet::new(), None).unwrap();
        assert!(!fs::read_to_string(dir.file()).unwrap().contains("hidden"));
        assert!(load(&dir.file()).hidden.is_empty());
    }

    #[test]
    fn an_older_file_without_focus_or_hidden_apps_still_loads() {
        let dir = TempDir::new("old-format");
        fs::write(
            dir.file(),
            "# RustTV launcher tile order. Lower keys come first.\n\n[keys]\n\"a/.B\" = 1000\nabout = 2000\n",
        )
        .unwrap();
        let loaded = load(&dir.file());
        assert!(loaded.warning.is_none());
        assert_eq!(loaded.focus, None);
        assert!(loaded.hidden.is_empty());
        assert_eq!(
            loaded.order,
            Order::from_keys([
                (TileId::App("a/.B".to_owned()), 1000),
                (TileId::About, 2000)
            ])
        );
    }

    #[test]
    fn a_missing_file_is_a_first_run() {
        let dir = TempDir::new("missing");
        let loaded = load(&dir.file());
        assert_eq!(loaded.order, Order::default());
        assert!(loaded.warning.is_none());
    }

    #[test]
    fn a_corrupt_file_is_kept_as_bak_and_ignored() {
        let dir = TempDir::new("corrupt");
        fs::write(dir.file(), "keys = [not toml").unwrap();
        let loaded = load(&dir.file());
        assert_eq!(loaded.order, Order::default());
        assert!(loaded.warning.unwrap().contains("order.toml.bak"));
        assert!(!dir.file().exists());
        assert_eq!(
            fs::read_to_string(with_suffix(&dir.file(), ".bak")).unwrap(),
            "keys = [not toml"
        );
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_rejected() {
        for text in [
            "[keys]\n\"a/.B\" = 1\n[extra]\n",
            "[keys]\n\"a/.B\" = \"one\"\n",
            "hidden = \"a/.B\"\n[keys]\n",
            "",
        ] {
            let dir = TempDir::new("invalid");
            fs::write(dir.file(), text).unwrap();
            assert!(load(&dir.file()).warning.is_some(), "{text:?}");
        }
    }
}

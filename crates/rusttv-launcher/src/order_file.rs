//! Reads and writes the tile order file, `order.toml`.
//!
//! The launcher's own private state, not a change to the TV, so it is outside
//! the CLI safety engine.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::model::{Order, TileId};

/// Name used for the About tile in the file. Component names always contain
/// a `/`, so this cannot clash with an app.
const ABOUT: &str = "about";

/// File contents: tile name → order key.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OrderFile {
    keys: BTreeMap<String, i64>,
}

/// The result of [`load`].
#[derive(Debug)]
pub(crate) struct Loaded {
    /// The stored order, or an empty one (first run) when there is none.
    pub order: Order,
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
                warning: None,
            };
        }
        Err(error) => return set_aside(path, &error.to_string()),
    };
    match toml::from_str::<OrderFile>(&text) {
        Ok(file) => Loaded {
            order: Order::from_keys(file.keys.into_iter().map(|(name, key)| {
                let id = if name == ABOUT {
                    TileId::About
                } else {
                    TileId::App(name)
                };
                (id, key)
            })),
            warning: None,
        },
        Err(error) => set_aside(path, &error.to_string()),
    }
}

fn set_aside(path: &Path, reason: &str) -> Loaded {
    let backup = with_suffix(path, ".bak");
    let kept = match fs::rename(path, &backup) {
        Ok(()) => format!("kept as {}", backup.display()),
        Err(error) => format!("could not keep it as {}: {error}", backup.display()),
    };
    Loaded {
        order: Order::default(),
        warning: Some(format!(
            "ignoring tile order file {} ({reason}); {kept}",
            path.display()
        )),
    }
}

/// Writes `order` to `path` atomically: a temporary file next to it, then a rename.
pub(crate) fn save(path: &Path, order: &Order) -> io::Result<()> {
    let mut text = String::from("# RustTV launcher tile order. Lower keys come first.\n\n[keys]\n");
    for (id, key) in order.keys() {
        let name = match id {
            TileId::App(component) => component.as_str(),
            TileId::About => ABOUT,
        };
        // A TOML string literal is also a valid quoted key.
        let name = toml::Value::String(name.to_owned());
        text.push_str(&format!("{name} = {key}\n"));
    }

    let temporary = with_suffix(path, ".tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)
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
        save(&dir.file(), &sample()).unwrap();
        let loaded = load(&dir.file());
        assert_eq!(loaded.order, sample());
        assert!(loaded.warning.is_none());
        assert!(!with_suffix(&dir.file(), ".tmp").exists());
    }

    #[test]
    fn saving_replaces_the_previous_order() {
        let dir = TempDir::new("replace");
        save(&dir.file(), &sample()).unwrap();
        let shorter = Order::from_keys([(TileId::About, 1000)]);
        save(&dir.file(), &shorter).unwrap();
        assert_eq!(load(&dir.file()).order, shorter);
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
            "",
        ] {
            let dir = TempDir::new("invalid");
            fs::write(dir.file(), text).unwrap();
            assert!(load(&dir.file()).warning.is_some(), "{text:?}");
        }
    }
}

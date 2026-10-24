//! This computer's copies of TV histories (HISTORY.md § 3, § 4.4):
//! `<state dir>/history/<history id>.git`, `devices.toml` mapping each TV's
//! `ro.serialno` to its histories, and the watchlist baselines.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusttv_core::{Adb, DeviceCommand};
use rusttv_history::{HistoryId, Store};
use serde::{Deserialize, Serialize};

use super::Failure;
use crate::CliError;

/// `<state dir>/history`.
pub(crate) fn dir() -> Result<PathBuf, CliError> {
    Ok(rusttv_core::state_dir()?.join("history"))
}

/// The repository of history `id` (created empty if there is none).
pub(crate) fn open(id: &HistoryId) -> Result<Store, CliError> {
    let path = dir()?.join(format!("{id}.git"));
    fs::create_dir_all(&path).map_err(|error| Failure::storage(&path, &error))?;
    let mut store = Store::open_or_init(&path).map_err(Failure::from)?;
    store.set_id(id).map_err(Failure::from)?;
    Ok(store)
}

/// The repository of history `id` if this computer has one.
pub(crate) fn existing(id: &HistoryId) -> Result<Option<Store>, CliError> {
    let path = dir()?.join(format!("{id}.git"));
    if !path.join("HEAD").is_file() {
        return Ok(None);
    }
    open(id).map(Some)
}

/// `devices.toml`: each TV's `ro.serialno` and its histories, oldest first.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Devices {
    #[serde(default)]
    serials: BTreeMap<String, Vec<String>>,
}

impl Devices {
    fn path() -> Result<PathBuf, CliError> {
        Ok(dir()?.join("devices.toml"))
    }

    /// Reads `devices.toml`; none yet is empty.
    pub(crate) fn load() -> Result<Self, CliError> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|error| {
                Failure::new("HISTORY_STORAGE", format!("{}: {error}", path.display())).into()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(Failure::storage(&path, &error).into()),
        }
    }

    /// The histories of the TV with `serialno`, oldest first; ids that
    /// don't parse are left out.
    pub(crate) fn histories(&self, serialno: &str) -> Vec<HistoryId> {
        self.serials
            .get(serialno)
            .into_iter()
            .flatten()
            .filter_map(|id| HistoryId::parse(id).ok())
            .collect()
    }

    /// Records that the TV with `serialno` has history `id` now (moved to the
    /// end: the newest), and saves the file.
    pub(crate) fn remember(serialno: &str, id: &HistoryId) -> Result<(), CliError> {
        let mut devices = Self::load()?;
        let ids = devices.serials.entry(serialno.to_owned()).or_default();
        ids.retain(|known| known != id.as_str());
        ids.push(id.as_str().to_owned());
        let text = toml::to_string(&devices)
            .map_err(|error| Failure::new("HISTORY_STORAGE", error.to_string()))?;
        write_atomic(&Self::path()?, text.as_bytes())
    }
}

/// The TV's `ro.serialno`, or `None` when it is empty, `unknown` or not a
/// plain serial: then nothing is mapped and `--history` is needed.
pub(crate) fn serialno(adb: &dyn Adb, serial: &str) -> Result<Option<String>, CliError> {
    let output = adb.run(serial, &DeviceCommand::SerialNo)?;
    let value = output.trim();
    let plain = !value.is_empty()
        && value.len() <= 64
        && value != "unknown"
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    Ok(plain.then(|| value.to_owned()))
}

/// The watchlist baseline of history `id`: what `history sync` saw last.
pub(crate) fn watchlist_path(id: &HistoryId) -> Result<PathBuf, CliError> {
    Ok(dir()?.join(format!("{id}.watchlist.toml")))
}

/// Writes a file atomically: a sibling file, then a rename.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let fail = |error: std::io::Error| Failure::storage(path, &error);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let partial = path.with_extension("partial");
    let mut file = fs::File::create(&partial).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&partial, path).map_err(fail)?;
    Ok(())
}

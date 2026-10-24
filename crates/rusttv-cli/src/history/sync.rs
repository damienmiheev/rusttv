//! `history sync` (HISTORY.md § 4.4): fetch the TV's new entries into this
//! computer's copy, and the sensitive-settings watchlist (§ 7).

use std::collections::BTreeMap;
use std::fs;

use rusttv_core::{Adb, DeviceCommand, PackageName, WatchKey};
use rusttv_history::door::ExportHeader;
use rusttv_history::{EntryId, HistoryId, Limits, Store};
use serde::{Deserialize, Serialize};

use super::door::{self, ExportReply};
use super::{Failure, mac};
use crate::CliError;

/// What a sync found and did.
#[derive(Debug)]
pub(crate) struct Synced {
    /// The TV's `ro.serialno`, if it is a plain serial.
    pub(crate) serialno: Option<String>,
    /// The TV's history; `None` while it is empty.
    pub(crate) history: Option<HistoryId>,
    /// Its newest entry, as this computer has it now.
    pub(crate) head: Option<EntryId>,
    /// Entries fetched now.
    pub(crate) new_entries: usize,
    /// Histories this computer holds for this TV (by `ro.serialno`), with
    /// at least one entry, oldest first.
    pub(crate) mac_histories: Vec<HistoryId>,
}

impl Synced {
    /// The TV's history is empty while this computer holds one for it: the
    /// CLI sends nothing and offers `history restore-tv` (HISTORY.md § 4.5).
    pub(crate) fn restore_tv_offered(&self) -> bool {
        self.history.is_none() && !self.mac_histories.is_empty()
    }
}

/// Fetches the TV's history into this computer's copy: the entries after
/// this computer's newest (all of them for a history it doesn't have), in
/// parts until the TV says there are no more, each verified on import
/// (`rusttv_history::Store::import`). With `forced`, the TV's history must
/// be that one. R0 on the device.
pub(crate) fn sync(
    adb: &dyn Adb,
    serial: &str,
    forced: Option<&HistoryId>,
) -> Result<Synced, CliError> {
    let serialno = mac::serialno(adb, serial)?;
    let mapped = match &serialno {
        Some(serialno) => mac::Devices::load()?.histories(serialno),
        None => Vec::new(),
    };
    let mut mac_histories = Vec::new();
    for id in &mapped {
        if let Some(store) = mac::existing(id)?
            && store.head().map_err(Failure::from)?.is_some()
        {
            mac_histories.push(id.clone());
        }
    }
    let candidate = match forced {
        Some(id) => Some(id.clone()),
        None => mac_histories.last().cloned(),
    };
    let candidate_head = match &candidate {
        Some(id) => match mac::existing(id)? {
            Some(store) => store.head().map_err(Failure::from)?,
            None => None,
        },
        None => None,
    };

    let mut since = candidate_head.clone();
    let mut reply = door::export(adb, serial, since.as_ref())?;
    if let ExportReply::Refused(code) = &reply
        && since.is_some()
        && code == "HISTORY_NOT_FOUND"
    {
        // The TV doesn't hold that entry: its history is empty or another one.
        since = None;
        reply = door::export(adb, serial, None)?;
    }
    let header = match &reply {
        ExportReply::Refused(code) => {
            return Err(Failure::refused(code, "reading the TV's history").into());
        }
        ExportReply::Export { header, .. } => header.clone(),
    };
    let Some(id) = header.history.clone() else {
        return Ok(Synced {
            serialno,
            history: None,
            head: None,
            new_entries: 0,
            mac_histories,
        });
    };
    if let Some(forced) = forced
        && &id != forced
    {
        return Err(Failure::new(
            "HISTORY_NOT_FOUND",
            format!("the TV's history is {id}, not {forced}"),
        )
        .into());
    }
    let mut store = mac::open(&id)?;
    let before = store.head().map_err(Failure::from)?;
    if since != before {
        // Exported from somewhere else than this copy's newest entry.
        reply = door::export(adb, serial, before.as_ref())?;
    }
    loop {
        let header = match &reply {
            ExportReply::Export { header, .. } => header.clone(),
            ExportReply::Refused(code) => {
                return Err(Failure::refused(
                    code,
                    "reading the TV's history after this computer's newest entry",
                )
                .into());
            }
        };
        check_part(&header, &id)?;
        if let Some(head) = &header.head
            && store.head().map_err(Failure::from)?.as_ref() != Some(head)
        {
            store
                .import(reply.pack(), head, Limits::default())
                .map_err(Failure::from)?;
        }
        if !header.more {
            break;
        }
        reply = door::export(adb, serial, header.head.as_ref())?;
    }
    let head = store.head().map_err(Failure::from)?;
    let new_entries = count_after(&store, before.as_ref())?;
    if let Some(serialno) = &serialno {
        mac::Devices::remember(serialno, &id)?;
        if !mac_histories.contains(&id) {
            mac_histories.push(id.clone());
        }
    }
    Ok(Synced {
        serialno,
        history: Some(id),
        head,
        new_entries,
        mac_histories,
    })
}

/// A part must belong to the history it continues.
fn check_part(header: &ExportHeader, id: &HistoryId) -> Result<(), CliError> {
    match &header.history {
        Some(part) if part == id => Ok(()),
        _ => Err(Failure::invalid(format!(
            "a part of the TV's export doesn't belong to history {id}"
        ))
        .into()),
    }
}

/// Entries newer than `before` (all for `None`).
fn count_after(store: &Store, before: Option<&EntryId>) -> Result<usize, CliError> {
    let log = store.log(usize::MAX).map_err(Failure::from)?;
    Ok(log
        .iter()
        .take_while(|entry| Some(&entry.id) != before)
        .count())
}

/// One watchlist value that changed since the last sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct WatchChange {
    /// `<namespace> <key>`, or `write_secure_settings` for the launcher's grant.
    pub(crate) key: String,
    /// At the last sync; null = unset.
    pub(crate) before: Option<String>,
    /// Now; null = unset.
    pub(crate) now: Option<String>,
}

/// The watchlist baseline file.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Baseline {
    #[serde(default)]
    values: BTreeMap<String, String>,
}

/// The grant's key in the watchlist.
const GRANT: &str = "write_secure_settings";

/// Reads the watchlist (R0) and compares it with the baseline of history
/// `id` on this computer, which it then replaces. The first time there is
/// nothing to compare. Values over 512 characters are cut.
pub(crate) fn watchlist(
    adb: &dyn Adb,
    serial: &str,
    id: &HistoryId,
) -> Result<Vec<WatchChange>, CliError> {
    let mut now = BTreeMap::new();
    for key in WatchKey::ALL {
        let output = adb.run(serial, &DeviceCommand::WatchedSetting(key))?;
        let value: String = output.trim().chars().take(512).collect();
        now.insert(key.to_string(), value);
    }
    let info = adb.run(
        serial,
        &DeviceCommand::PackageInfo(
            PackageName::new(rusttv_core::RUSTTV_LAUNCHER)
                .map_err(|error| Failure::invalid(error.to_string()))?,
        ),
    )?;
    let grant = match rusttv_core::launcher_grant(&info) {
        None => "launcher not installed",
        Some(true) => "granted",
        Some(false) => "not granted",
    };
    now.insert(GRANT.to_owned(), grant.to_owned());

    let path = mac::watchlist_path(id)?;
    let before = match fs::read_to_string(&path) {
        Ok(text) => Some(toml::from_str::<Baseline>(&text).map_err(|error| {
            Failure::new("HISTORY_STORAGE", format!("{}: {error}", path.display()))
        })?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(Failure::storage(&path, &error).into()),
    };
    let unset = |value: Option<&String>| value.filter(|value| *value != "null").cloned();
    let changes = match &before {
        None => Vec::new(),
        Some(before) => now
            .iter()
            .filter(|(key, value)| before.values.get(*key) != Some(value))
            .map(|(key, value)| WatchChange {
                key: key.clone(),
                before: unset(before.values.get(key)),
                now: unset(Some(value)),
            })
            .collect(),
    };
    let text = toml::to_string(&Baseline { values: now })
        .map_err(|error| Failure::new("HISTORY_STORAGE", error.to_string()))?;
    mac::write_atomic(&path, text.as_bytes())?;
    Ok(changes)
}

//! What plan-applying commands tell the TV's history (HISTORY.md § 4.2): an
//! observation before the plan, a note after it, each through the door
//! (launcher-private, RL). Never from `inspect`, `sync`, `bench`, `perf` or
//! MCP. A failing door is a warning, never an error: the next observation
//! records the difference.

use std::collections::BTreeMap;
use std::path::Path;

use rusttv_core::{
    Adb, ApprovedBy, DeviceCommand, HOME_TAKEOVER, JournalStatus, PackageName, SettingKey,
    SettingState,
};
use rusttv_history::door::{Message, Note, NoteApproval, Observation, Outcome};
use rusttv_history::{Device, PackageState};

use super::{door, sync, words};
use crate::{CliError, output};

/// Observes the device and tells the TV's history around one plan.
pub(crate) struct Recorder<'a> {
    adb: &'a dyn Adb,
    serial: String,
    /// Whether the door may be written, once known: the launcher is
    /// installed and the empty-history rule doesn't hold it back.
    allowed: Option<bool>,
}

impl std::fmt::Debug for Recorder<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recorder")
            .field("serial", &self.serial)
            .field("allowed", &self.allowed)
            .finish_non_exhaustive()
    }
}

impl<'a> Recorder<'a> {
    /// Before a plan: if the launcher is installed, syncs, mentions the
    /// approvals waiting on the TV, and sends an observation. Prints
    /// warnings; never fails.
    pub(crate) fn start(adb: &'a dyn Adb, serial: &str) -> Self {
        let mut recorder = Self {
            adb,
            serial: serial.to_owned(),
            allowed: None,
        };
        match recorder.check() {
            Ok(Some(true)) => {
                remind_pending(adb, serial);
                if let Err(error) = observe(adb, serial).and_then(|observation| {
                    door::send_message(
                        adb,
                        serial,
                        &Message::Observe { observation },
                        "recording the device before the plan",
                    )
                }) {
                    warn("the device couldn't be recorded before the plan", &error);
                }
                recorder.allowed = Some(true);
            }
            Ok(Some(false)) => recorder.allowed = Some(false),
            Ok(None) => {}
            Err(error) => {
                warn("can't reach the TV's history", &error);
                recorder.allowed = Some(false);
            }
        }
        recorder
    }

    /// Whether the door may be written: `None` when the launcher isn't
    /// installed; `Some(false)` (after printing the offer) while the TV's
    /// history is empty and this computer holds one for it.
    fn check(&self) -> Result<Option<bool>, CliError> {
        if !launcher_installed(self.adb, &self.serial)? {
            return Ok(None);
        }
        let synced = sync::sync(self.adb, &self.serial, None)?;
        if synced.restore_tv_offered() {
            offer_restore_tv(synced.mac_histories.len());
            return Ok(Some(false));
        }
        Ok(Some(true))
    }

    /// After a plan: if the launcher is installed, sends a note (the plan,
    /// its journal file, how it was approved, how it ended, and the device
    /// now), then syncs. `consumed` is the TV approval `apply-pending` used.
    /// Prints warnings; never fails.
    pub(crate) fn finish(&self, plan: &str, journal: &Path, restore: bool, consumed: Option<&str>) {
        let allowed = match self.allowed {
            // Installed when the plan started; an uninstall plan removes it.
            Some(allowed) => allowed && launcher_installed(self.adb, &self.serial).unwrap_or(false),
            None => match self.check() {
                Ok(allowed) => allowed == Some(true),
                Err(error) => {
                    warn("can't reach the TV's history", &error);
                    false
                }
            },
        };
        if !allowed {
            return;
        }
        let Ok(recorded) = rusttv_core::load_journal(journal) else {
            // Nothing was applied: no journal, nothing to note.
            return;
        };
        let result = note(
            self.adb,
            &self.serial,
            plan,
            journal,
            &recorded,
            restore,
            consumed,
        )
        .and_then(|note| {
            door::send_message(
                self.adb,
                &self.serial,
                &Message::Note { note },
                "recording the plan",
            )
        })
        .and_then(|_| sync::sync(self.adb, &self.serial, None));
        if let Err(error) = result {
            warn("the plan couldn't be recorded in the TV's history", &error);
        }
    }
}

fn note(
    adb: &dyn Adb,
    serial: &str,
    plan: &str,
    path: &Path,
    journal: &rusttv_core::Journal,
    restore: bool,
    consumed: Option<&str>,
) -> Result<Note, CliError> {
    let approved_by = if restore {
        journal.restore_approved_by.as_ref()
    } else {
        journal.approved_by.as_ref()
    };
    let approved_by = match approved_by {
        Some(ApprovedBy::Terminal) => NoteApproval::Terminal,
        Some(ApprovedBy::Delegation { .. }) => NoteApproval::Delegation,
        Some(ApprovedBy::Upgrade { .. }) => NoteApproval::Upgrade,
        Some(ApprovedBy::Tv { .. }) => NoteApproval::Tv,
        None => NoteApproval::None,
    };
    let outcome = match journal.status {
        JournalStatus::Applied => Outcome::Applied,
        JournalStatus::RolledBack => Outcome::RolledBack,
        JournalStatus::Restored => Outcome::Restored,
        JournalStatus::RestoreFailed => Outcome::RestoreFailed,
        // Still `applying`: the process stopped mid-plan.
        JournalStatus::RollbackFailed | JournalStatus::Applying => Outcome::RollbackFailed,
    };
    Ok(Note {
        plan: plan.to_owned(),
        journal: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        approved_by,
        outcome,
        consumed: consumed.map(str::to_owned),
        observation: observe(adb, serial)?,
    })
}

/// Whether the RustTV launcher is installed (`dumpsys package
/// ai.rusttv.launcher` has its package section). R0.
pub(crate) fn launcher_installed(adb: &dyn Adb, serial: &str) -> Result<bool, CliError> {
    let info = adb.run(serial, &DeviceCommand::PackageInfo(launcher_package()?))?;
    Ok(rusttv_core::launcher_grant(&info).is_some())
}

pub(crate) fn launcher_package() -> Result<PackageName, CliError> {
    PackageName::new(rusttv_core::RUSTTV_LAUNCHER)
        .map_err(|error| super::Failure::invalid(error.to_string()).into())
}

/// What the Mac sees on the device (HISTORY.md § 4.2): the allowlisted
/// settings, the catalog packages' states (never the packages a Home
/// takeover changes), the firmware and whether a takeover is active (a
/// restorable takeover journal on this computer). Never the launcher's own
/// state. R0.
pub(crate) fn observe(adb: &dyn Adb, serial: &str) -> Result<Observation, CliError> {
    let mut settings = BTreeMap::new();
    for key in SettingKey::ALL {
        let output = adb.run(serial, &DeviceCommand::GetSetting(key))?;
        match SettingState::from_output(&output) {
            Ok(SettingState::Value(value)) => {
                settings.insert(key.as_str().to_owned(), value.as_str().to_owned());
            }
            Ok(SettingState::Unset) => {}
            Err(error) => {
                return Err(super::Failure::invalid(format!("{key}: {error}")).into());
            }
        }
    }
    let takeover: Vec<String> = rusttv_core::bundled_profiles()
        .into_iter()
        .filter_map(|profile| profile.launcher)
        .flat_map(|launcher| {
            std::iter::once(launcher.stock_package).chain(launcher.disable_home_packages)
        })
        .collect();
    let installed = adb.run(serial, &DeviceCommand::ListPackages)?;
    let installed =
        rusttv_core::parse_package_list(&installed).map_err(rusttv_core::AdbError::from)?;
    let disabled = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;
    let disabled =
        rusttv_core::parse_package_list(&disabled).map_err(rusttv_core::AdbError::from)?;
    let mut packages = BTreeMap::new();
    for entry in rusttv_core::bundled_catalog() {
        let name = entry.package.as_str();
        if takeover.iter().any(|package| package == name)
            || !installed.iter().any(|(package, _)| *package == name)
        {
            continue;
        }
        let state = if disabled.iter().any(|(package, _)| *package == name) {
            PackageState::Disabled
        } else {
            PackageState::Enabled
        };
        packages.insert(name.to_owned(), state);
    }
    let fingerprint = rusttv_core::device_fingerprint(adb, serial)?;
    let takeover = rusttv_core::find_restorable_journal(
        &rusttv_core::journals_dir()?,
        HOME_TAKEOVER,
        &fingerprint,
    )
    .found
    .is_ok();
    Ok(Observation {
        settings,
        packages,
        device: Device {
            fingerprint: Some(fingerprint),
            takeover: Some(takeover),
        },
    })
}

/// Mentions the approvals waiting on the TV and their time left (only in
/// `history` and plan-applying commands: reading them starts the launcher's
/// process). Prints a warning if they can't be read.
pub(crate) fn remind_pending(adb: &dyn Adb, serial: &str) {
    match door::pending(adb, serial) {
        Ok(list) => {
            for approval in &list.approval {
                output::print_note(&format!(
                    "An approval made on the TV is waiting ({} change{}): run `rusttv history \
                     apply-pending` within {}.",
                    approval.change.len(),
                    if approval.change.len() == 1 { "" } else { "s" },
                    words::duration(approval.seconds_left)
                ));
            }
        }
        Err(error) => warn("can't read the approvals waiting on the TV", &error),
    }
}

/// The `restore-tv` offer (HISTORY.md § 4.5).
pub(crate) fn offer_restore_tv(histories: usize) {
    output::print_note(&format!(
        "The TV's history is empty, and this computer holds {} for it: put it back with `rusttv \
         history restore-tv` before anything is recorded on the TV. Once something is, the old \
         history can't go back on the TV (it stays readable here). Nothing was recorded in the \
         TV's history.",
        if histories == 1 {
            "one".to_owned()
        } else {
            format!("{histories}")
        }
    ));
}

pub(crate) fn warn(what: &str, error: &CliError) {
    output::print_note(&format!(
        "warning: history: {what}: {}",
        output::message_with_causes(error)
    ));
}

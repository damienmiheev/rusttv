//! Journals: the host-side record of a change, written before every step.
//!
//! One TOML file per applied plan, in [`journals_dir`] (SECURITY.md § Change
//! lifecycle). Plain text, so a
//! person can read what was changed and how to undo it by hand.

use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::command::DeviceCommand;
use crate::plan::{Check, PlanError};

/// Version of the journal file format. Bump on any incompatible change.
pub const JOURNAL_SCHEMA_VERSION: u32 = 1;

/// The record of one applied plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    /// [`JOURNAL_SCHEMA_VERSION`] when written.
    pub schema_version: u32,
    /// Kind of plan, e.g. `home-takeover`.
    pub plan: String,
    /// adb serial the plan ran on.
    pub serial: String,
    /// `ro.build.fingerprint` of the device when the plan ran.
    pub fingerprint: String,
    /// When applying started (host clock).
    pub started_at_unix_ms: u64,
    /// Where the change stands.
    pub status: JournalStatus,
    /// Every step of the plan, in order.
    pub steps: Vec<JournalStep>,
}

/// One step of a [`Journal`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalStep {
    /// The exact command, for people. Ignored when restoring.
    pub adb: String,
    /// What ran.
    pub command: DeviceCommand,
    /// The state the device was in before, checked before anything ran.
    pub original_state: Option<Check>,
    /// How far this step got.
    pub status: StepStatus,
}

/// Where a journaled change stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    /// Steps are running, or the process stopped while they were.
    Applying,
    /// Every step ran and was verified.
    Applied,
    /// A step failed and every step that ran was undone.
    RolledBack,
    /// A step failed and undoing failed too: the device may be half-changed.
    RollbackFailed,
    /// Undone later with `restore`.
    Restored,
    /// `restore` failed for at least one step.
    RestoreFailed,
}

impl JournalStatus {
    /// Whether the device may still carry changes from this journal.
    pub fn is_restorable(self) -> bool {
        !matches!(self, Self::RolledBack | Self::Restored)
    }
}

impl fmt::Display for JournalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::RolledBack => "rolled back",
            Self::RollbackFailed => "rollback failed",
            Self::Restored => "restored",
            Self::RestoreFailed => "restore failed",
        })
    }
}

/// How far one step got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Not started.
    Pending,
    /// The command may have run; it was not verified (yet).
    Started,
    /// Ran and verified.
    Done,
    /// Undone and verified.
    Undone,
}

impl StepStatus {
    /// Whether the step may have changed the device, and so needs undoing.
    pub fn may_have_run(self) -> bool {
        matches!(self, Self::Started | Self::Done)
    }
}

/// Where the engine writes a journal. [`FileJournal`] in production.
pub trait JournalStore {
    /// Replaces the stored journal with `journal`, durably.
    fn save(&mut self, journal: &Journal) -> io::Result<()>;
}

/// A journal kept in one TOML file.
#[derive(Debug, Clone)]
pub struct FileJournal {
    path: PathBuf,
}

impl FileJournal {
    /// A journal at `path`; the file and its directory are created on first save.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Where the journal is written.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl JournalStore for FileJournal {
    fn save(&mut self, journal: &Journal) -> io::Result<()> {
        let text = toml::to_string(journal).map_err(io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        // Write a sibling file and rename it over the journal, so a crash
        // leaves either the old or the new journal, never half of one.
        let partial = self.path.with_extension("toml.partial");
        let mut file = fs::File::create(&partial)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&partial, &self.path)
    }
}

/// `<state dir>/journals`, where the CLI writes journals and plans look for
/// an active Home takeover. The state dir is `RUSTTV_STATE_DIR` if set, else
/// `~/Library/Application Support/rusttv` on macOS and
/// `$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`) elsewhere.
pub fn journals_dir() -> Result<PathBuf, PlanError> {
    let state = if let Some(dir) = env::var_os("RUSTTV_STATE_DIR") {
        PathBuf::from(dir)
    } else {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(|| PlanError::journal("HOME is not set; set RUSTTV_STATE_DIR"))?;
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/rusttv")
        } else {
            env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".local/state"))
                .join("rusttv")
        }
    };
    Ok(state.join("journals"))
}

/// Reads a journal file.
pub fn load_journal(path: &Path) -> Result<Journal, PlanError> {
    let text = fs::read_to_string(path)
        .map_err(|error| PlanError::journal(format!("{}: {error}", path.display())))?;
    let journal: Journal = toml::from_str(&text)
        .map_err(|error| PlanError::journal(format!("{}: {error}", path.display())))?;
    if journal.schema_version != JOURNAL_SCHEMA_VERSION {
        return Err(PlanError::journal(format!(
            "{}: unsupported schema_version {}",
            path.display(),
            journal.schema_version
        )));
    }
    Ok(journal)
}

/// File name for a new journal: sorts by time, e.g. `1761220000000-home-takeover.toml`.
pub fn journal_file_name(journal_started_at_unix_ms: u64, plan: &str) -> String {
    format!("{journal_started_at_unix_ms:013}-{plan}.toml")
}

/// The newest journal in `dir` for `plan` on the device with `fingerprint`
/// that may still need restoring. Files that don't parse are skipped.
pub fn find_restorable_journal(
    dir: &Path,
    plan: &str,
    fingerprint: &str,
) -> Result<(PathBuf, Journal), PlanError> {
    let not_found = || {
        PlanError::no_journal(format!(
            "no {plan} journal to restore for firmware `{fingerprint}` in {}; pass `--journal <file>`",
            dir.display()
        ))
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(not_found()),
        Err(error) => return Err(PlanError::journal(format!("{}: {error}", dir.display()))),
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect();
    // Names start with a zero-padded timestamp: newest last.
    paths.sort();
    paths
        .into_iter()
        .rev()
        .filter_map(|path| load_journal(&path).ok().map(|journal| (path, journal)))
        .find(|(_, journal)| {
            journal.plan == plan
                && journal.fingerprint == fingerprint
                && journal.status.is_restorable()
        })
        .ok_or_else(not_found)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::command::PackageName;

    /// Keeps every saved version in memory; optionally fails from the
    /// `fail_from`-th save (0-based) on.
    #[derive(Debug, Default)]
    pub(crate) struct MemoryJournal {
        pub(crate) saves: Vec<Journal>,
        pub(crate) fail_from: Option<usize>,
    }

    impl MemoryJournal {
        pub(crate) fn last(&self) -> &Journal {
            self.saves.last().expect("nothing was saved")
        }
    }

    impl JournalStore for MemoryJournal {
        fn save(&mut self, journal: &Journal) -> io::Result<()> {
            if self.fail_from.is_some_and(|from| self.saves.len() >= from) {
                return Err(io::Error::other("disk full"));
            }
            self.saves.push(journal.clone());
            Ok(())
        }
    }

    fn journal(status: JournalStatus, started_at_unix_ms: u64) -> Journal {
        let package = PackageName::new("com.example.launcher").unwrap();
        Journal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            plan: "home-takeover".to_owned(),
            serial: "192.168.1.20:5555".to_owned(),
            fingerprint: "google/tv/x:14/A/1:user/release-keys".to_owned(),
            started_at_unix_ms,
            status,
            steps: vec![JournalStep {
                adb: "adb -s 192.168.1.20:5555 shell pm disable-user --user 0 com.example.launcher"
                    .to_owned(),
                command: DeviceCommand::DisablePackage(package.clone()),
                original_state: Some(Check::PackageEnabled(package)),
                status: StepStatus::Done,
            }],
        }
    }

    /// A fresh directory under the system temp dir, unique per test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rusttv-journal-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn file_journal_round_trips_as_readable_toml() {
        let dir = temp_dir("round-trip");
        let path = dir
            .join("journals")
            .join(journal_file_name(42, "home-takeover"));
        let original = journal(JournalStatus::Applied, 42);

        FileJournal::new(&path).save(&original).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("status = \"applied\""), "{text}");
        assert!(text.contains("disable_package"), "{text}");
        assert_eq!(load_journal(&path).unwrap(), original);
        assert!(!path.with_extension("toml.partial").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_journal_with_an_unsafe_package_name_does_not_load() {
        let dir = temp_dir("evil");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("evil.toml");
        let text = toml::to_string(&journal(JournalStatus::Applied, 1))
            .unwrap()
            .replace("arg = \"com.example.launcher\"", "arg = \"com.x;reboot\"");
        fs::write(&path, text).unwrap();
        assert_eq!(load_journal(&path).unwrap_err().code(), "JOURNAL_FAILED");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_newest_restorable_journal_for_the_firmware_is_found() {
        let dir = temp_dir("find");
        let fingerprint = journal(JournalStatus::Applied, 0).fingerprint;
        for (time, status) in [
            (1, JournalStatus::Applied),
            (2, JournalStatus::Applying),
            (3, JournalStatus::Restored),
        ] {
            let path = dir.join(journal_file_name(time, "home-takeover"));
            FileJournal::new(path).save(&journal(status, time)).unwrap();
        }
        fs::write(dir.join("9999999999999-garbage.toml"), "not a journal").unwrap();

        let (path, found) = find_restorable_journal(&dir, "home-takeover", &fingerprint).unwrap();
        assert_eq!(found.started_at_unix_ms, 2);
        assert!(path.ends_with("0000000000002-home-takeover.toml"));

        let error = find_restorable_journal(&dir, "home-takeover", "other").unwrap_err();
        assert_eq!(error.code(), "NO_JOURNAL");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_journal_directory_means_no_journal() {
        let error =
            find_restorable_journal(&temp_dir("missing"), "home-takeover", "x").unwrap_err();
        assert_eq!(error.code(), "NO_JOURNAL");
    }
}

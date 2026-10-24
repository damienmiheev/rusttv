//! The TV's side of the history (HISTORY.md § 4–6): the one owner of the
//! store, answering the door, applying approved changes with a journal
//! written first, pending approvals for the Mac, and the code lockout.
//!
//! Platform access goes through [`TvDevice`] and [`TvClock`], so the same
//! code runs in the launcher and in tests.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};

use crate::HistoryError;
use crate::door::{
    self, ApprovedChange, CommitKind, CommitResult, ExportHeader, Message, NoteApproval,
    PendingApproval, PendingList, approval_hash,
};
use crate::entry::{ApprovedBy, Author, EntryId, Meta};
use crate::state::{Launcher, SETTING_KEYS, State};
use crate::store::{HistoryId, Limits, Store};

/// How long a pending approval stays valid, in TV uptime (HISTORY.md § 6.3).
pub const PENDING_VALID_MS: u64 = 2 * 60 * 60 * 1000;

/// Entries an hour from the door (HISTORY.md § 2).
const DOOR_PER_HOUR: usize = 30;

/// Wrong codes in a row that lock approvals.
const WRONG_CODES: u32 = 3;

/// The first lockout; each further one doubles it.
const LOCKOUT_MS: u64 = 60 * 60 * 1000;

const HOUR_MS: u64 = 60 * 60 * 1000;

/// What the launcher reads and writes on the TV.
pub trait TvDevice {
    /// The launcher's own state now.
    fn launcher(&self) -> Result<Launcher, HistoryError>;
    /// Replaces the launcher's state (one atomic file write).
    fn write_launcher(&mut self, launcher: &Launcher) -> Result<(), HistoryError>;
    /// An allowlisted global setting; `None` = unset.
    fn setting(&self, key: &str) -> Result<Option<String>, HistoryError>;
    /// Whether the launcher may write settings (`WRITE_SECURE_SETTINGS`).
    fn can_write_settings(&self) -> bool;
    /// Writes an allowlisted global setting.
    fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError>;
}

/// The TV's clocks.
pub trait TvClock {
    /// Milliseconds since this boot (`SystemClock.elapsedRealtime`).
    fn elapsed_ms(&self) -> u64;
    /// Boots so far (`Settings.Global.BOOT_COUNT`).
    fn boot_count(&self) -> u64;
    /// Wall clock, Unix seconds (may be wrong before NTP).
    fn now_unix(&self) -> i64;
}

/// Why something on the TV was refused; `code` is what the door returns.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Refused {
    /// e.g. `TV_BUSY`, `HISTORY_NOT_FOUND`, `HISTORY_RATE_LIMITED`.
    pub code: &'static str,
    /// For people and logs (no app names).
    pub message: String,
}

impl Refused {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<HistoryError> for Refused {
    fn from(error: HistoryError) -> Self {
        let code = match error {
            HistoryError::FormatNewer(_) => "HISTORY_FORMAT_NEWER",
            HistoryError::NotAContinuation(_) | HistoryError::NoSuchEntry(_) => "HISTORY_NOT_FOUND",
            HistoryError::Invalid(_) => "HISTORY_INVALID",
            HistoryError::Storage(_) => "HISTORY_STORAGE",
        };
        Self::new(code, error.to_string())
    }
}

/// How applying an approved change on the TV ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Everything the TV does was applied and recorded; `pending` changes
    /// wait for the Mac (none if empty).
    Applied {
        /// Changes left for the Mac.
        pending: Vec<ApprovedChange>,
    },
    /// A step failed; everything done was undone.
    RolledBack {
        /// What failed.
        reason: String,
    },
    /// A step failed and undoing failed too: these settings are left as
    /// they are now (key, value now).
    RollbackFailed {
        /// What failed.
        reason: String,
        /// Settings not put back.
        left: Vec<(String, Option<String>)>,
    },
}

/// The TV history: the store and everything around it, under one owner.
#[derive(Debug)]
pub struct TvHistory {
    root: PathBuf,
    store: Store,
}

/// One step of a TV journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum JournalStep {
    Setting {
        key: String,
        original: Option<String>,
        target: String,
    },
    Launcher {
        original: Launcher,
        target: Launcher,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JournalStatus {
    Applying,
    Applied,
}

/// The TV journal, written before the first step (HISTORY.md § 4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    approval: String,
    status: JournalStatus,
    /// Steps started so far (the last may or may not have taken effect).
    started: usize,
    steps: Vec<JournalStep>,
    #[serde(default)]
    pending: Vec<ApprovedChange>,
}

/// A stored pending approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPending {
    boot_count: u64,
    approved_elapsed_ms: u64,
    change: Vec<ApprovedChange>,
}

/// The lockout state (HISTORY.md § 6.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LockoutFile {
    wrong_in_row: u32,
    lockouts: u32,
    boot_count: u64,
    at_elapsed_ms: u64,
    remaining_ms: u64,
}

/// Door entries in the last hour, by (boot, uptime ms).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RateFile {
    #[serde(default)]
    entries: Vec<(u64, u64)>,
}

impl TvHistory {
    /// Opens the history under `root` (the launcher's files dir), creating
    /// it on first use, and finishes a change a crash interrupted.
    pub fn open(
        root: &Path,
        device: &mut dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<Self, HistoryError> {
        fs::create_dir_all(root.join("history")).map_err(HistoryError::storage)?;
        let store = Store::open_or_init(&root.join("history.git"))?;
        let mut history = Self {
            root: root.to_owned(),
            store,
        };
        history.recover(device, clock, new_id)?;
        Ok(history)
    }

    /// The store, for reading.
    pub fn store(&self) -> &Store {
        &self.store
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.join("history").join(name)
    }

    fn journal(&self) -> Result<Option<Journal>, HistoryError> {
        read_toml(&self.file("journal.toml"))
    }

    /// Whether a TV change is in progress (door writes get `TV_BUSY`, the
    /// UI doesn't write the order file).
    pub fn busy(&self) -> Result<bool, HistoryError> {
        Ok(self.journal()?.is_some())
    }

    /// The live state: the head's packages and device facts, with the
    /// launcher's state and the settings as they are now.
    fn live(&self, device: &dyn TvDevice) -> Result<State, HistoryError> {
        let mut state = self.store.head_state()?.unwrap_or_else(State::new);
        state.launcher = device.launcher()?;
        state.settings.global = live_settings(device)?;
        Ok(state)
    }

    fn record(
        &mut self,
        state: &State,
        meta: &Meta,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<Option<EntryId>, HistoryError> {
        if self.store.id()?.is_none() {
            self.store.set_id(&new_id())?;
        }
        self.store.record(state, meta, clock.now_unix())
    }

    /// Records the launcher's own edit (move, hide, unhide): no code needed.
    pub fn record_launcher_edit(
        &mut self,
        device: &dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<Option<EntryId>, HistoryError> {
        let state = self.live(device)?;
        self.record(&state, &Meta::new(Author::Tv), clock, new_id)
    }

    /// Packs if it is time; stops early when `interrupt` is set.
    pub fn pack(&mut self, interrupt: &AtomicBool) -> Result<(), HistoryError> {
        self.store.pack(interrupt).map(|_| ())
    }

    /// The `export/<since>` read: header line and pack.
    pub fn export(&self, since: Option<&str>) -> Result<Vec<u8>, Refused> {
        let since = since.map(EntryId::parse).transpose()?;
        let export = self.store.export(since.as_ref())?;
        let header = ExportHeader {
            history: self.store.id()?,
            head: export.head,
            more: export.more,
        };
        let mut bytes = header.encode().into_bytes();
        bytes.extend_from_slice(&export.pack);
        Ok(bytes)
    }

    /// The `pending` read: valid approvals, with their time left.
    pub fn pending(&self, clock: &dyn TvClock) -> Result<PendingList, HistoryError> {
        let mut list = PendingList::default();
        for (hash, stored) in self.stored_pending()? {
            if let Some(left) = time_left(&stored, clock) {
                list.approval.push(PendingApproval {
                    hash,
                    seconds_left: left / 1000,
                    change: stored.change,
                });
            }
        }
        Ok(list)
    }

    fn stored_pending(&self) -> Result<Vec<(String, StoredPending)>, HistoryError> {
        let dir = self.file("pending");
        let mut found = Vec::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(found);
        };
        for entry in entries {
            let path = entry.map_err(HistoryError::storage)?.path();
            let Some(hash) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !door::is_hash(hash) {
                continue;
            }
            if let Some(stored) = read_toml::<StoredPending>(&path)? {
                found.push((hash.to_owned(), stored));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }

    /// `content call … commit`: processes an inbox file's `bytes`.
    pub fn commit(
        &mut self,
        kind: CommitKind,
        bytes: &[u8],
        device: &dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> CommitResult {
        let result = self.commit_inner(kind, bytes, device, clock, new_id);
        let head = self.store.head().ok().flatten();
        CommitResult {
            error: result.err().map(|refused| refused.code.to_owned()),
            head,
        }
    }

    fn commit_inner(
        &mut self,
        kind: CommitKind,
        bytes: &[u8],
        device: &dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<(), Refused> {
        if self.busy()? {
            return Err(Refused::new("TV_BUSY", "a change on the TV is in progress"));
        }
        match kind {
            CommitKind::Import => self.import(bytes, device, clock),
            CommitKind::Message => match Message::parse(bytes)? {
                Message::Consume { approval } => self.consume(&approval, clock),
                Message::Observe { observation } => {
                    self.rate(clock)?;
                    let head = self.live(device)?;
                    let state = observation.applied_to(&head)?;
                    self.record(&state, &Meta::new(Author::Drift), clock, new_id)?;
                    Ok(())
                }
                Message::Note { note } => {
                    self.rate(clock)?;
                    let approved_by = match note.approved_by {
                        NoteApproval::Terminal => Some(ApprovedBy::Terminal),
                        NoteApproval::Delegation => Some(ApprovedBy::Delegation),
                        NoteApproval::Upgrade => Some(ApprovedBy::Upgrade),
                        NoteApproval::None => None,
                        NoteApproval::Tv => {
                            let hash = note.consumed.as_deref().unwrap_or_default();
                            if !self.file("consumed").join(hash).is_file() {
                                return Err(Refused::new(
                                    "HISTORY_INVALID",
                                    "the note claims a TV approval that wasn't consumed",
                                ));
                            }
                            Some(ApprovedBy::TvCode(hash.to_owned()))
                        }
                    };
                    let head = self.live(device)?;
                    let state = note.observation.applied_to(&head)?;
                    let meta = Meta {
                        author: Author::Mac,
                        journal: Some(note.journal),
                        approved_by,
                    };
                    self.record(&state, &meta, clock, new_id)?;
                    Ok(())
                }
            },
        }
    }

    /// Counts a door entry against the hourly cap.
    fn rate(&self, clock: &dyn TvClock) -> Result<(), Refused> {
        let path = self.file("door-rate.toml");
        let mut rate: RateFile = read_toml(&path)?.unwrap_or_default();
        let (boot, now) = (clock.boot_count(), clock.elapsed_ms());
        rate.entries
            .retain(|(entry_boot, at)| *entry_boot == boot && now.saturating_sub(*at) < HOUR_MS);
        if rate.entries.len() >= DOOR_PER_HOUR {
            return Err(Refused::new(
                "HISTORY_RATE_LIMITED",
                "too many entries from the computer this hour",
            ));
        }
        rate.entries.push((boot, now));
        write_toml(&path, &rate)?;
        Ok(())
    }

    fn consume(&self, hash: &str, clock: &dyn TvClock) -> Result<(), Refused> {
        let from = self.file("pending").join(format!("{hash}.toml"));
        let stored: Option<StoredPending> = read_toml(&from)?;
        match stored.as_ref().and_then(|stored| time_left(stored, clock)) {
            Some(_) => {
                let consumed = self.file("consumed");
                fs::create_dir_all(&consumed).map_err(HistoryError::storage)?;
                // The rename is the claim: it succeeds once.
                fs::rename(&from, consumed.join(hash)).map_err(|_| {
                    Refused::new("PENDING_INVALID", "the approval was already used")
                })?;
                Ok(())
            }
            None => {
                let _ = fs::remove_file(&from);
                Err(Refused::new(
                    "PENDING_INVALID",
                    "no valid approval with that hash",
                ))
            }
        }
    }

    fn import(
        &mut self,
        bytes: &[u8],
        device: &dyn TvDevice,
        clock: &dyn TvClock,
    ) -> Result<(), Refused> {
        if self.store.head()?.is_some() {
            return Err(Refused::new(
                "HISTORY_NOT_EMPTY",
                "the TV already has a history",
            ));
        }
        let (header, pack) = ExportHeader::split(bytes)?;
        let (Some(id), Some(head)) = (header.history, header.head) else {
            return Err(Refused::new("HISTORY_INVALID", "the export is empty"));
        };
        if header.more {
            return Err(Refused::new(
                "HISTORY_INVALID",
                "an import must be one whole history",
            ));
        }
        self.store.import(pack, &head, Limits::default())?;
        self.store.set_id(&id)?;
        let state = self.live(device)?;
        self.store
            .record(&state, &Meta::new(Author::Import), clock.now_unix())?;
        Ok(())
    }

    /// Applies an approved change made on the TV (HISTORY.md § 4.1): the
    /// journal first, then settings, then the launcher's state last, each
    /// verified; packages (and settings without the grant) become a
    /// pending approval for the Mac.
    pub fn apply(
        &mut self,
        changes: &[ApprovedChange],
        launcher: Option<&Launcher>,
        device: &mut dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<ApplyOutcome, Refused> {
        if self.busy()? {
            return Err(Refused::new("TV_BUSY", "a change is in progress"));
        }
        if !changes.is_empty() {
            door::validate_changes(changes)?;
        }
        let mut list = changes.to_vec();
        let launcher_change = launcher.cloned();
        if let Some(target) = &launcher_change {
            // The launcher's state is part of what was approved too.
            let mut state = State::new();
            state.launcher = target.clone();
            state.validate()?;
        }
        let hash = approval_hash(&list)?;
        let mut steps = Vec::new();
        let mut pending = Vec::new();
        for change in list.drain(..) {
            match &change {
                ApprovedChange::Setting { key, old, new } if device.can_write_settings() => {
                    let now = device.setting(key)?;
                    if !same_scale(now.as_deref(), old.as_deref()) {
                        return Err(Refused::new(
                            "CHANGED_MEANWHILE",
                            format!("{key} changed since you approved"),
                        ));
                    }
                    steps.push(JournalStep::Setting {
                        key: key.clone(),
                        original: now,
                        // An app can't unset a setting: unset ≡ "1".
                        target: new.clone().unwrap_or_else(|| "1".to_owned()),
                    });
                }
                _ => pending.push(change),
            }
        }
        if let Some(target) = launcher_change {
            steps.push(JournalStep::Launcher {
                original: device.launcher()?,
                target,
            });
        }
        let mut journal = Journal {
            approval: hash.clone(),
            status: JournalStatus::Applying,
            started: 0,
            steps,
            pending,
        };
        let journal_path = self.file("journal.toml");
        write_toml(&journal_path, &journal)?;
        for index in 0..journal.steps.len() {
            journal.started = index + 1;
            write_toml(&journal_path, &journal)?;
            if let Err(reason) = run_step(&journal.steps[index], device) {
                let left = undo(&journal.steps[..=index], device);
                fs::remove_file(&journal_path).map_err(HistoryError::storage)?;
                return Ok(if left.is_empty() {
                    ApplyOutcome::RolledBack { reason }
                } else {
                    let state = self.live(device)?;
                    self.record(&state, &Meta::new(Author::Tv), clock, new_id)?;
                    ApplyOutcome::RollbackFailed { reason, left }
                });
            }
        }
        journal.status = JournalStatus::Applied;
        write_toml(&journal_path, &journal)?;
        self.finish(&journal, device, clock, new_id)?;
        Ok(ApplyOutcome::Applied {
            pending: journal.pending,
        })
    }

    /// Commits an applied journal and stores its pending part.
    fn finish(
        &mut self,
        journal: &Journal,
        device: &dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<(), HistoryError> {
        if !journal.steps.is_empty() {
            let state = self.live(device)?;
            let meta = Meta {
                author: Author::Tv,
                journal: None,
                approved_by: Some(ApprovedBy::TvCode(journal.approval.clone())),
            };
            self.record(&state, &meta, clock, new_id)?;
        }
        if !journal.pending.is_empty() {
            let stored = StoredPending {
                boot_count: clock.boot_count(),
                approved_elapsed_ms: clock.elapsed_ms(),
                change: journal.pending.clone(),
            };
            let hash = approval_hash(&journal.pending)?;
            fs::create_dir_all(self.file("pending")).map_err(HistoryError::storage)?;
            write_toml(&self.file("pending").join(format!("{hash}.toml")), &stored)?;
        }
        fs::remove_file(self.file("journal.toml")).map_err(HistoryError::storage)
    }

    /// Finishes what a crash interrupted: an applied journal is committed,
    /// an applying one undone (HISTORY.md § 4.1 step 7). Returns whether
    /// the launcher's state changed (the UI then reloads it).
    fn recover(
        &mut self,
        device: &mut dyn TvDevice,
        clock: &dyn TvClock,
        new_id: &dyn Fn() -> HistoryId,
    ) -> Result<bool, HistoryError> {
        let Some(journal) = self.journal()? else {
            return Ok(false);
        };
        let touches_launcher = journal
            .steps
            .iter()
            .any(|step| matches!(step, JournalStep::Launcher { .. }));
        match journal.status {
            JournalStatus::Applied => {
                self.finish(&journal, device, clock, new_id)?;
            }
            JournalStatus::Applying => {
                undo(
                    &journal.steps[..journal.started.min(journal.steps.len())],
                    device,
                );
                fs::remove_file(self.file("journal.toml")).map_err(HistoryError::storage)?;
                if self.store.head()?.is_some() {
                    let state = self.live(device)?;
                    self.store
                        .record(&state, &Meta::new(Author::Tv), clock.now_unix())?;
                }
            }
        }
        Ok(touches_launcher)
    }

    /// Milliseconds approvals stay locked, 0 when they aren't.
    /// Each check saves the countdown, so time run before a reboot counts.
    pub fn locked_ms(&self, clock: &dyn TvClock) -> Result<u64, HistoryError> {
        let path = self.file("lockout.toml");
        let Some(mut lockout) = read_toml::<LockoutFile>(&path)? else {
            return Ok(0);
        };
        lockout.remaining_ms = remaining(&lockout, clock);
        lockout.boot_count = clock.boot_count();
        lockout.at_elapsed_ms = clock.elapsed_ms();
        write_toml(&path, &lockout)?;
        Ok(lockout.remaining_ms)
    }

    /// A wrong code: after [`WRONG_CODES`] in a row, approvals lock for an
    /// hour, doubling with each further lockout. Returns the lock time now.
    pub fn wrong_code(&self, clock: &dyn TvClock) -> Result<u64, HistoryError> {
        let path = self.file("lockout.toml");
        let mut lockout: LockoutFile = read_toml(&path)?.unwrap_or_default();
        lockout.remaining_ms = remaining(&lockout, clock);
        lockout.boot_count = clock.boot_count();
        lockout.at_elapsed_ms = clock.elapsed_ms();
        lockout.wrong_in_row += 1;
        if lockout.wrong_in_row >= WRONG_CODES {
            lockout.wrong_in_row = 0;
            lockout.remaining_ms = LOCKOUT_MS << lockout.lockouts.min(16);
            lockout.lockouts += 1;
        }
        write_toml(&path, &lockout)?;
        Ok(lockout.remaining_ms)
    }

    /// A correct code resets the counts.
    pub fn correct_code(&self) -> Result<(), HistoryError> {
        match fs::remove_file(self.file("lockout.toml")) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(HistoryError::storage(error))
            }
            _ => Ok(()),
        }
    }
}

/// Time a lockout has left now: it runs down only while the TV runs.
fn remaining(lockout: &LockoutFile, clock: &dyn TvClock) -> u64 {
    let ran = if lockout.boot_count == clock.boot_count() {
        clock.elapsed_ms().saturating_sub(lockout.at_elapsed_ms)
    } else {
        clock.elapsed_ms()
    };
    lockout.remaining_ms.saturating_sub(ran)
}

fn time_left(stored: &StoredPending, clock: &dyn TvClock) -> Option<u64> {
    if stored.boot_count != clock.boot_count() {
        return None;
    }
    let age = clock.elapsed_ms().checked_sub(stored.approved_elapsed_ms)?;
    PENDING_VALID_MS.checked_sub(age).filter(|left| *left > 0)
}

fn live_settings(device: &dyn TvDevice) -> Result<BTreeMap<String, String>, HistoryError> {
    let mut settings = BTreeMap::new();
    for key in SETTING_KEYS {
        if let Some(value) = device.setting(key)? {
            settings.insert(key.to_owned(), value);
        }
    }
    Ok(settings)
}

/// Whether two scale values are the same, unset = "1".
fn same_scale(a: Option<&str>, b: Option<&str>) -> bool {
    let number = |value: Option<&str>| value.unwrap_or("1").parse::<f64>().ok();
    match (number(a), number(b)) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        _ => a == b,
    }
}

fn run_step(step: &JournalStep, device: &mut dyn TvDevice) -> Result<(), String> {
    match step {
        JournalStep::Setting { key, target, .. } => {
            device
                .put_setting(key, target)
                .map_err(|error| error.to_string())?;
            let now = device.setting(key).map_err(|error| error.to_string())?;
            if same_scale(now.as_deref(), Some(target)) {
                Ok(())
            } else {
                Err(format!("{key} reads {now:?} after setting {target}"))
            }
        }
        JournalStep::Launcher { target, .. } => {
            device
                .write_launcher(target)
                .map_err(|error| error.to_string())?;
            if device.launcher().map_err(|error| error.to_string())? == *target {
                Ok(())
            } else {
                Err("the launcher's state didn't save".to_owned())
            }
        }
    }
}

/// Undoes `steps`, last first, where the value is still what the step set;
/// returns the settings it couldn't put back.
fn undo(steps: &[JournalStep], device: &mut dyn TvDevice) -> Vec<(String, Option<String>)> {
    let mut left = Vec::new();
    for step in steps.iter().rev() {
        match step {
            JournalStep::Setting {
                key,
                original,
                target,
            } => {
                let now = device.setting(key).ok().flatten();
                if !same_scale(now.as_deref(), Some(target)) {
                    if !same_scale(now.as_deref(), original.as_deref()) {
                        // Changed by someone else since: left as it is.
                        left.push((key.clone(), now));
                    }
                    continue;
                }
                let back = original.clone().unwrap_or_else(|| "1".to_owned());
                let restored = device.put_setting(key, &back).is_ok()
                    && same_scale(device.setting(key).ok().flatten().as_deref(), Some(&back));
                if !restored {
                    left.push((key.clone(), device.setting(key).ok().flatten()));
                }
            }
            JournalStep::Launcher { original, target } => {
                if device.launcher().ok().as_ref() == Some(target) {
                    let _ = device.write_launcher(original);
                }
            }
        }
    }
    left
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, HistoryError> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .map_err(|error| HistoryError::invalid(format!("{}: {error}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(HistoryError::storage(error)),
    }
}

/// Writes atomically: a temp file, then a rename.
fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<(), HistoryError> {
    let text = toml::to_string(value).map_err(|error| HistoryError::invalid(error.to_string()))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(HistoryError::storage)?;
    }
    let temp = path.with_extension("tmp");
    fs::write(&temp, text).map_err(HistoryError::storage)?;
    fs::rename(&temp, path).map_err(HistoryError::storage)
}

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::door::{Note, Observation, Outcome};
use crate::state::PackageState;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "rusttv-tv-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Default)]
struct FakeDevice {
    launcher: Launcher,
    settings: BTreeMap<String, String>,
    grant: bool,
    /// Writes to this key fail.
    broken: Option<String>,
}

impl TvDevice for FakeDevice {
    fn launcher(&self) -> Result<Launcher, HistoryError> {
        Ok(self.launcher.clone())
    }
    fn write_launcher(&mut self, launcher: &Launcher) -> Result<(), HistoryError> {
        self.launcher = launcher.clone();
        Ok(())
    }
    fn setting(&self, key: &str) -> Result<Option<String>, HistoryError> {
        Ok(self.settings.get(key).cloned())
    }
    fn can_write_settings(&self) -> bool {
        self.grant
    }
    fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        if self.broken.as_deref() == Some(key) {
            return Err(HistoryError::storage("permission denied"));
        }
        self.settings.insert(key.to_owned(), value.to_owned());
        Ok(())
    }
}

struct FakeClock {
    boot: Cell<u64>,
    elapsed: Cell<u64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            boot: Cell::new(7),
            elapsed: Cell::new(1_000_000),
        }
    }
    fn advance(&self, ms: u64) {
        self.elapsed.set(self.elapsed.get() + ms);
    }
    fn reboot(&self, elapsed: u64) {
        self.boot.set(self.boot.get() + 1);
        self.elapsed.set(elapsed);
    }
}

impl TvClock for FakeClock {
    fn elapsed_ms(&self) -> u64 {
        self.elapsed.get()
    }
    fn boot_count(&self) -> u64 {
        self.boot.get()
    }
    fn now_unix(&self) -> i64 {
        1_790_000_000 + (self.elapsed.get() / 1000) as i64
    }
}

fn id() -> HistoryId {
    HistoryId::parse("0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69").unwrap()
}

fn setting(new: &str) -> ApprovedChange {
    ApprovedChange::Setting {
        key: "animator_duration_scale".into(),
        old: None,
        new: Some(new.into()),
    }
}

fn package() -> ApprovedChange {
    ApprovedChange::Package {
        name: "com.example.telemetry".into(),
        old: Some(PackageState::Enabled),
        new: PackageState::Disabled,
    }
}

fn message(message: &Message) -> Vec<u8> {
    message.to_toml().unwrap().into_bytes()
}

fn setup() -> (TempDir, FakeDevice, FakeClock, TvHistory) {
    let dir = TempDir::new();
    let mut device = FakeDevice {
        launcher: Launcher::default(),
        grant: true,
        ..FakeDevice::default()
    };
    let clock = FakeClock::new();
    let mut history = TvHistory::open(&dir.0, &mut device, &clock, &id).unwrap();
    device.launcher.order = vec!["about".into()];
    history.record_launcher_edit(&device, &clock, &id).unwrap();
    (dir, device, clock, history)
}

#[test]
fn launcher_edits_are_recorded_with_the_live_settings() {
    let (_dir, mut device, clock, mut history) = setup();
    assert_eq!(history.store().id().unwrap(), Some(id()));
    device.launcher.hidden.insert("com.example.a/.Main".into());
    device
        .settings
        .insert("window_animation_scale".into(), "0.5".into());
    let entry = history
        .record_launcher_edit(&device, &clock, &id)
        .unwrap()
        .unwrap();
    let state = history.store().state(&entry).unwrap();
    assert!(state.launcher.hidden.contains("com.example.a/.Main"));
    assert_eq!(state.settings.global["window_animation_scale"], "0.5");
    assert!(
        history
            .record_launcher_edit(&device, &clock, &id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn an_approved_change_is_journaled_applied_verified_and_recorded() {
    let (_dir, mut device, clock, mut history) = setup();
    let mut launcher = device.launcher.clone();
    launcher.update_check = false;
    let outcome = history
        .apply(
            &[setting("0.5"), package()],
            Some(&launcher),
            &mut device,
            &clock,
            &id,
        )
        .unwrap();
    assert_eq!(
        outcome,
        ApplyOutcome::Applied {
            pending: vec![package()]
        }
    );
    assert_eq!(device.settings["animator_duration_scale"], "0.5");
    assert!(!device.launcher.update_check);
    assert!(!history.busy().unwrap());
    let head = history.store().log(1).unwrap().remove(0);
    assert!(matches!(head.meta.approved_by, Some(ApprovedBy::TvCode(_))));
    let pending = history.pending(&clock).unwrap();
    assert_eq!(pending.approval.len(), 1);
    assert_eq!(pending.approval[0].change, vec![package()]);
}

#[test]
fn without_the_grant_settings_wait_for_the_computer() {
    let (_dir, mut device, clock, mut history) = setup();
    device.grant = false;
    let outcome = history
        .apply(&[setting("0.5")], None, &mut device, &clock, &id)
        .unwrap();
    assert_eq!(
        outcome,
        ApplyOutcome::Applied {
            pending: vec![setting("0.5")]
        }
    );
    assert!(device.settings.is_empty());
    assert_eq!(history.pending(&clock).unwrap().approval.len(), 1);
}

#[test]
fn a_failed_step_is_undone() {
    let (_dir, mut device, clock, mut history) = setup();
    device.broken = Some("window_animation_scale".into());
    let changes = [
        setting("0.5"),
        ApprovedChange::Setting {
            key: "window_animation_scale".into(),
            old: None,
            new: Some("0.5".into()),
        },
    ];
    let before = history.store().head().unwrap();
    let outcome = history
        .apply(&changes, None, &mut device, &clock, &id)
        .unwrap();
    assert!(
        matches!(outcome, ApplyOutcome::RolledBack { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        device
            .settings
            .get("animator_duration_scale")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(history.store().head().unwrap(), before);
    assert!(!history.busy().unwrap());
}

#[test]
fn a_value_changed_since_approval_is_refused() {
    let (_dir, mut device, clock, mut history) = setup();
    device
        .settings
        .insert("animator_duration_scale".into(), "2".into());
    let refused = history
        .apply(&[setting("0.5")], None, &mut device, &clock, &id)
        .unwrap_err();
    assert_eq!(refused.code, "CHANGED_MEANWHILE");
    assert_eq!(device.settings["animator_duration_scale"], "2");
}

#[test]
fn a_crash_mid_change_is_undone_when_the_history_opens_again() {
    let (dir, mut device, clock, history) = setup();
    // As if the process died after the first step of two.
    let journal = Journal {
        approval: "a".repeat(64),
        status: JournalStatus::Applying,
        started: 1,
        steps: vec![
            JournalStep::Setting {
                key: "animator_duration_scale".into(),
                original: None,
                target: "0.5".into(),
            },
            JournalStep::Setting {
                key: "window_animation_scale".into(),
                original: None,
                target: "0.5".into(),
            },
        ],
        pending: vec![],
    };
    write_toml(&history.file("journal.toml"), &journal).unwrap();
    device
        .settings
        .insert("animator_duration_scale".into(), "0.5".into());
    assert!(history.busy().unwrap());
    let observe = message(&Message::Observe {
        observation: Observation::default(),
    });
    let mut busy = history;
    let result = busy.commit(CommitKind::Message, &observe, &device, &clock, &id);
    assert_eq!(result.error.as_deref(), Some("TV_BUSY"));
    drop(busy);

    let history = TvHistory::open(&dir.0, &mut device, &clock, &id).unwrap();
    assert!(!history.busy().unwrap());
    assert_eq!(device.settings["animator_duration_scale"], "1");
}

#[test]
fn an_applied_journal_without_its_entry_is_committed_on_open() {
    let (dir, mut device, clock, history) = setup();
    let journal = Journal {
        approval: "b".repeat(64),
        status: JournalStatus::Applied,
        started: 1,
        steps: vec![JournalStep::Setting {
            key: "animator_duration_scale".into(),
            original: None,
            target: "0.5".into(),
        }],
        pending: vec![package()],
    };
    write_toml(&history.file("journal.toml"), &journal).unwrap();
    device
        .settings
        .insert("animator_duration_scale".into(), "0.5".into());
    drop(history);
    let history = TvHistory::open(&dir.0, &mut device, &clock, &id).unwrap();
    let head = history.store().log(1).unwrap().remove(0);
    assert_eq!(
        head.meta.approved_by,
        Some(ApprovedBy::TvCode("b".repeat(64)))
    );
    assert_eq!(history.pending(&clock).unwrap().approval.len(), 1);
}

#[test]
fn a_pending_approval_is_consumed_once_within_two_hours_of_one_boot() {
    let (_dir, mut device, clock, mut history) = setup();
    history
        .apply(&[package()], None, &mut device, &clock, &id)
        .unwrap();
    let hash = history.pending(&clock).unwrap().approval[0].hash.clone();
    let consume = message(&Message::Consume {
        approval: hash.clone(),
    });
    assert_eq!(
        history
            .commit(CommitKind::Message, &consume, &device, &clock, &id)
            .error,
        None
    );
    assert_eq!(
        history
            .commit(CommitKind::Message, &consume, &device, &clock, &id)
            .error
            .as_deref(),
        Some("PENDING_INVALID")
    );
    // A note may now say the TV approved it.
    let note = |consumed: Option<String>| {
        message(&Message::Note {
            note: Note {
                plan: "history-restore".into(),
                journal: "1-history-restore.toml".into(),
                approved_by: NoteApproval::Tv,
                outcome: Outcome::Applied,
                consumed,
                observation: Observation::default(),
            },
        })
    };
    assert_eq!(
        history
            .commit(
                CommitKind::Message,
                &note(Some("c".repeat(64))),
                &device,
                &clock,
                &id
            )
            .error
            .as_deref(),
        Some("HISTORY_INVALID")
    );
    assert_eq!(
        history
            .commit(
                CommitKind::Message,
                &note(Some(hash.clone())),
                &device,
                &clock,
                &id
            )
            .error,
        None
    );
    let head = history.store().log(1).unwrap().remove(0);
    assert_eq!(head.meta.approved_by, Some(ApprovedBy::TvCode(hash)));

    // Expiry by uptime, and a reboot ends every approval.
    history
        .apply(&[package()], None, &mut device, &clock, &id)
        .unwrap();
    clock.advance(PENDING_VALID_MS + 1);
    assert!(history.pending(&clock).unwrap().approval.is_empty());
    history
        .apply(&[package()], None, &mut device, &clock, &id)
        .unwrap();
    clock.reboot(1_000);
    assert!(history.pending(&clock).unwrap().approval.is_empty());
}

#[test]
fn the_door_records_drift_and_is_capped() {
    let (_dir, mut device, clock, mut history) = setup();
    let mut observation = Observation::default();
    observation
        .packages
        .insert("com.example.telemetry".into(), PackageState::Disabled);
    let observe = message(&Message::Observe { observation });
    assert_eq!(
        history
            .commit(CommitKind::Message, &observe, &device, &clock, &id)
            .error,
        None
    );
    let head = history.store().log(1).unwrap().remove(0);
    assert_eq!(head.meta.author, Author::Drift);
    // An observation never changes the launcher's own state.
    device.launcher.update_check = false;
    let state = history.store().state(&head.id).unwrap();
    assert!(state.launcher.update_check);

    for _ in 1..DOOR_PER_HOUR {
        assert_eq!(
            history
                .commit(CommitKind::Message, &observe, &device, &clock, &id)
                .error,
            None
        );
    }
    assert_eq!(
        history
            .commit(CommitKind::Message, &observe, &device, &clock, &id)
            .error
            .as_deref(),
        Some("HISTORY_RATE_LIMITED")
    );
    clock.advance(HOUR_MS);
    assert_eq!(
        history
            .commit(CommitKind::Message, &observe, &device, &clock, &id)
            .error,
        None
    );
    let _ = &mut device;
}

#[test]
fn junk_from_the_door_is_refused_without_harm() {
    let (_dir, device, clock, mut history) = setup();
    let before = history.store().head().unwrap();
    for junk in [
        &b""[..],
        b"\xff\xfe",
        b"kind = \"observe\"",
        b"kind = \"launcher\"\norder = []",
    ] {
        let result = history.commit(CommitKind::Message, junk, &device, &clock, &id);
        assert!(result.error.is_some());
        let result = history.commit(CommitKind::Import, junk, &device, &clock, &id);
        assert!(result.error.is_some());
    }
    assert_eq!(history.store().head().unwrap(), before);
}

#[test]
fn a_history_is_put_back_only_into_an_empty_one() {
    let (_dir, mut device, clock, history) = setup();
    let export = history.export(None).unwrap();
    let fresh = TempDir::new();
    let mut fresh_history = TvHistory::open(&fresh.0, &mut device, &clock, &id).unwrap();
    let other_id = || HistoryId::parse("11111111-2222-4333-8444-555555555555").unwrap();
    assert_eq!(
        fresh_history
            .commit(CommitKind::Import, &export, &device, &clock, &other_id)
            .error,
        None
    );
    assert_eq!(fresh_history.store().id().unwrap(), Some(id()));
    let log = fresh_history.store().log(10).unwrap();
    assert_eq!(log[0].meta.author, Author::Import);
    assert_eq!(log.len(), history.store().log(10).unwrap().len() + 1);
    assert_eq!(
        fresh_history
            .commit(CommitKind::Import, &export, &device, &clock, &other_id)
            .error
            .as_deref(),
        Some("HISTORY_NOT_EMPTY")
    );
}

#[test]
fn export_says_which_history_and_refuses_an_unknown_since() {
    let (_dir, _device, _clock, history) = setup();
    let bytes = history.export(None).unwrap();
    let (header, pack) = ExportHeader::split(&bytes).unwrap();
    assert_eq!(header.history, Some(id()));
    assert_eq!(header.head, history.store().head().unwrap());
    assert!(!pack.is_empty());
    assert_eq!(
        history.export(Some(&"0".repeat(40))).unwrap_err().code,
        "HISTORY_NOT_FOUND"
    );
    assert_eq!(
        history.export(Some("../x")).unwrap_err().code,
        "HISTORY_INVALID"
    );
}

#[test]
fn wrong_codes_lock_approvals_for_longer_each_time() {
    let (_dir, _device, clock, history) = setup();
    assert_eq!(history.locked_ms(&clock).unwrap(), 0);
    assert_eq!(history.wrong_code(&clock).unwrap(), 0);
    assert_eq!(history.wrong_code(&clock).unwrap(), 0);
    assert_eq!(history.wrong_code(&clock).unwrap(), LOCKOUT_MS);
    clock.advance(LOCKOUT_MS / 2);
    assert_eq!(history.locked_ms(&clock).unwrap(), LOCKOUT_MS / 2);
    // A reboot doesn't end it: only uptime counts.
    clock.reboot(10 * 60 * 1000);
    assert_eq!(
        history.locked_ms(&clock).unwrap(),
        LOCKOUT_MS / 2 - 10 * 60 * 1000
    );
    clock.advance(LOCKOUT_MS);
    assert_eq!(history.locked_ms(&clock).unwrap(), 0);
    for _ in 0..2 {
        history.wrong_code(&clock).unwrap();
    }
    assert_eq!(history.wrong_code(&clock).unwrap(), 2 * LOCKOUT_MS);
    history.correct_code().unwrap();
    assert_eq!(history.locked_ms(&clock).unwrap(), 0);
}

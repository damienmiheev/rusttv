//! The launcher's side of the history (HISTORY.md): one thread owns the
//! [`TvHistory`]; the UI and the door send it jobs.
//!
//! The UI never waits on it on the key path (only for the Settings page's
//! data and for an approved change); the door's binder threads wait for
//! their answer. Packing runs here at background priority while the
//! launcher is off screen, and stops when it comes back (HISTORY.md § 9).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;

use rusttv_history::door::{ApprovedChange, CommitKind, CommitResult};
use rusttv_history::tv::{ApplyOutcome, Refused, TvClock, TvDevice, TvHistory};
use rusttv_history::{Author, EntryId, HistoryError, HistoryId, SETTING_KEYS, State, diff};

use crate::settings::{Draft, EntryView, Overview};

/// Entries the Settings page lists.
const PAGE_ENTRIES: usize = 200;

/// What the history thread needs from the platform.
pub(crate) trait HistoryPlatform: Send {
    /// The launcher's files directory.
    fn root(&self) -> PathBuf;
    /// The device and the clock, borrowed together.
    fn parts(&mut self) -> (&mut dyn TvDevice, &dyn TvClock);
    /// A new random history id.
    fn new_id(&self) -> HistoryId;
    /// Whether this build is debuggable or profileable (approvals off).
    fn test_build(&self) -> bool;
    /// Lowers this thread's CPU priority (packing must not slow Home).
    fn lower_priority(&self) {}
}

type Reply<T> = Sender<T>;

/// Work for the history thread.
enum Job {
    LauncherEdited,
    Pack,
    Overview(Reply<Overview>),
    RestoreDraft(EntryId, Reply<Result<Draft, String>>),
    Apply(Draft, Reply<Result<ApplyOutcome, Refused>>),
    WrongCode(Reply<u64>),
    CorrectCode,
    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    Export(Option<String>, Reply<Result<Vec<u8>, Refused>>),
    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    Pending(Reply<Result<String, Refused>>),
    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    Commit(CommitKind, Vec<u8>, Reply<CommitResult>),
}

/// The handle to the history thread.
#[derive(Debug, Clone)]
pub(crate) struct History {
    jobs: Sender<Job>,
    /// Set while a TV change's journal exists: the UI doesn't write the
    /// order file then (HISTORY.md § 4.1 step 7).
    busy: Arc<AtomicBool>,
    /// Stops a running pack when the launcher comes back.
    interrupt: Arc<AtomicBool>,
    /// Set when recovery or a change rewrote the launcher's state: the UI
    /// reloads it.
    launcher_changed: Arc<AtomicBool>,
}

static GLOBAL: OnceLock<History> = OnceLock::new();

impl History {
    /// The process's history thread, started on first use with the
    /// platform `make` builds (the activity or the door, whichever is first).
    pub fn global(make: impl FnOnce() -> Box<dyn HistoryPlatform>) -> &'static Self {
        GLOBAL.get_or_init(|| Self::start(make()))
    }

    /// The process's history thread, if something started it.
    pub fn get() -> Option<&'static Self> {
        GLOBAL.get()
    }

    /// Starts a history thread (the process's own goes through [`Self::global`]).
    pub fn start(platform: Box<dyn HistoryPlatform>) -> Self {
        let (jobs, receiver) = mpsc::channel();
        let history = Self {
            jobs,
            busy: Arc::default(),
            interrupt: Arc::default(),
            launcher_changed: Arc::default(),
        };
        let shared = history.clone();
        let spawned = thread::Builder::new()
            .name("rusttv-history".to_owned())
            .spawn(move || work(platform, &receiver, &shared));
        if let Err(error) = spawned {
            log::warn!("history is off: starting its thread failed: {error}");
        }
        history
    }

    fn send(&self, job: Job) {
        // Fails only when the thread is gone; history is then off.
        let _ = self.jobs.send(job);
    }

    fn ask<T>(&self, job: impl FnOnce(Reply<T>) -> Job) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.send(job(reply));
        answer.recv().ok()
    }

    /// The person moved, hid or showed an app: record it.
    pub fn launcher_edited(&self) {
        self.send(Job::LauncherEdited);
    }

    /// Packs if it is time (ui.rs calls this a few seconds after a pause).
    pub fn pack(&self) {
        self.interrupt.store(false, Ordering::SeqCst);
        self.send(Job::Pack);
    }

    /// The launcher is back on screen: stop packing.
    pub fn resumed(&self) {
        self.interrupt.store(true, Ordering::SeqCst);
    }

    /// Whether a TV change is in progress.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Whether the launcher's state was rewritten since the last call.
    pub fn take_launcher_changed(&self) -> bool {
        self.launcher_changed.swap(false, Ordering::SeqCst)
    }

    /// The Settings page's data.
    pub fn overview(&self) -> Overview {
        self.ask(Job::Overview).unwrap_or_else(|| Overview {
            error: Some("history is off".to_owned()),
            ..Overview::default()
        })
    }

    /// The changes from now back to an entry's state.
    pub fn restore_draft(&self, id: EntryId) -> Result<Draft, String> {
        self.ask(|reply| Job::RestoreDraft(id, reply))
            .unwrap_or_else(|| Err("history is off".to_owned()))
    }

    /// Applies an approved draft.
    pub fn apply(&self, draft: Draft) -> Result<ApplyOutcome, Refused> {
        self.ask(|reply| Job::Apply(draft, reply))
            .unwrap_or_else(|| Err(off()))
    }

    /// A wrong code; returns how long approvals are locked now.
    pub fn wrong_code(&self) -> u64 {
        self.ask(Job::WrongCode).unwrap_or(0)
    }

    /// A correct code.
    pub fn correct_code(&self) {
        self.send(Job::CorrectCode);
    }

    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    /// The door's `export` read.
    pub fn export(&self, since: Option<String>) -> Result<Vec<u8>, Refused> {
        self.ask(|reply| Job::Export(since, reply))
            .unwrap_or_else(|| Err(off()))
    }

    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    /// The door's `pending` read.
    pub fn pending(&self) -> Result<String, Refused> {
        self.ask(Job::Pending).unwrap_or_else(|| Err(off()))
    }

    #[cfg_attr(
        not(target_os = "android"),
        expect(dead_code, reason = "the door is the Android provider")
    )]
    /// The door's `commit` call.
    pub fn commit(&self, kind: CommitKind, bytes: Vec<u8>) -> CommitResult {
        self.ask(|reply| Job::Commit(kind, bytes, reply))
            .unwrap_or(CommitResult {
                error: Some("HISTORY_STORAGE".to_owned()),
                head: None,
            })
    }
}

fn off() -> Refused {
    Refused {
        code: "HISTORY_STORAGE",
        message: "history is off".to_owned(),
    }
}

fn work(mut platform: Box<dyn HistoryPlatform>, jobs: &Receiver<Job>, shared: &History) {
    platform.lower_priority();
    let mut tv: Option<TvHistory> = None;
    for job in jobs {
        if tv.is_none() {
            let root = platform.root();
            let id = platform.new_id();
            let (device, clock) = platform.parts();
            // Recovery runs here, whatever started the process.
            match TvHistory::open(&root, device, clock, &|| id.clone()) {
                Ok(opened) => {
                    tv = Some(opened);
                    shared.launcher_changed.store(true, Ordering::SeqCst);
                }
                Err(error) => {
                    // No app names in the log (HISTORY.md § 1, privacy).
                    log::warn!("opening the history failed: {error}");
                    reply_failed(job, &error.to_string());
                    continue;
                }
            }
        }
        if let Some(history) = tv.as_mut() {
            run(history, job, platform.as_mut(), shared);
            shared
                .busy
                .store(history.busy().unwrap_or(true), Ordering::SeqCst);
        }
    }
}

fn run(history: &mut TvHistory, job: Job, platform: &mut dyn HistoryPlatform, shared: &History) {
    let id = platform.new_id();
    let new_id = || id.clone();
    let test_build = platform.test_build();
    let (device, clock) = platform.parts();
    match job {
        Job::LauncherEdited => {
            if let Err(error) = history.record_launcher_edit(device, clock, &new_id) {
                log::warn!("history: {error}");
            }
        }
        Job::Pack => {
            if let Err(error) = history.pack(&shared.interrupt) {
                log::info!("history: packing stopped: {error}");
            }
        }
        Job::Overview(reply) => {
            let _ = reply.send(overview(history, device, clock, test_build));
        }
        Job::RestoreDraft(id, reply) => {
            let _ =
                reply.send(restore_draft(history, device, &id).map_err(|error| error.to_string()));
        }
        Job::Apply(draft, reply) => {
            let outcome = if test_build {
                Err(Refused {
                    code: "TEST_BUILD",
                    message: "approvals are off in test builds".to_owned(),
                })
            } else {
                history.apply(
                    &draft.changes,
                    draft.launcher.as_ref(),
                    device,
                    clock,
                    &new_id,
                )
            };
            if draft.launcher.is_some() {
                shared.launcher_changed.store(true, Ordering::SeqCst);
            }
            let _ = reply.send(outcome);
        }
        Job::WrongCode(reply) => {
            let _ = reply.send(history.wrong_code(clock).unwrap_or(0));
        }
        Job::CorrectCode => {
            if let Err(error) = history.correct_code() {
                log::warn!("history: {error}");
            }
        }
        Job::Export(since, reply) => {
            let _ = reply.send(history.export(since.as_deref()));
        }
        Job::Pending(reply) => {
            let list = history
                .pending(clock)
                .and_then(|list| list.to_toml())
                .map_err(Refused::from);
            let _ = reply.send(list);
        }
        Job::Commit(kind, bytes, reply) => {
            let _ = reply.send(history.commit(kind, &bytes, device, clock, &new_id));
        }
    }
}

fn reply_failed(job: Job, message: &str) {
    let refused = || Refused {
        code: "HISTORY_STORAGE",
        message: message.to_owned(),
    };
    match job {
        Job::Overview(reply) => {
            let _ = reply.send(Overview {
                error: Some(message.to_owned()),
                ..Overview::default()
            });
        }
        Job::RestoreDraft(_, reply) => {
            let _ = reply.send(Err(message.to_owned()));
        }
        Job::Apply(_, reply) => {
            let _ = reply.send(Err(refused()));
        }
        Job::WrongCode(reply) => {
            let _ = reply.send(0);
        }
        Job::Export(_, reply) => {
            let _ = reply.send(Err(refused()));
        }
        Job::Pending(reply) => {
            let _ = reply.send(Err(refused()));
        }
        Job::Commit(_, _, reply) => {
            let _ = reply.send(CommitResult {
                error: Some("HISTORY_STORAGE".to_owned()),
                head: None,
            });
        }
        Job::LauncherEdited | Job::Pack | Job::CorrectCode => {}
    }
}

fn overview(
    history: &TvHistory,
    device: &dyn TvDevice,
    clock: &dyn TvClock,
    test_build: bool,
) -> Overview {
    let read = || -> Result<Overview, HistoryError> {
        let mut settings = std::collections::BTreeMap::new();
        for key in SETTING_KEYS {
            if let Some(value) = device.setting(key)? {
                settings.insert(key.to_owned(), value);
            }
        }
        Ok(Overview {
            settings,
            launcher: device.launcher()?,
            can_write_settings: device.can_write_settings(),
            entries: entry_views(history)?,
            pending: history.pending(clock)?.approval,
            locked_ms: history.locked_ms(clock)?,
            test_build,
            error: None,
        })
    };
    read().unwrap_or_else(|error| Overview {
        error: Some(error.to_string()),
        test_build,
        ..Overview::default()
    })
}

/// The newest entries with what each changed, ready for the page.
fn entry_views(history: &TvHistory) -> Result<Vec<EntryView>, HistoryError> {
    let store = history.store();
    let entries = store.log(PAGE_ENTRIES)?;
    let newest_import = entries
        .iter()
        .position(|entry| entry.meta.author == Author::Import);
    let mut views = Vec::with_capacity(entries.len());
    let mut state = None;
    for (index, entry) in entries.iter().enumerate() {
        let current = match state.take() {
            Some(state) => state,
            None => store.state(&entry.id)?,
        };
        let parent = match &entry.parent {
            Some(parent) => store.state(parent)?,
            None => State::new(),
        };
        views.push(EntryView {
            id: entry.id.clone(),
            time_unix: entry.time_unix,
            author: entry.meta.author,
            imported: newest_import.is_some_and(|import| index > import),
            changes: diff(&parent, &current),
        });
        state = Some(parent);
    }
    Ok(views)
}

/// The changes from the device now to an entry's state (HISTORY.md § 4.6):
/// settings and the launcher's state as they are now, packages as the
/// computer last saw them; device facts are never restored.
fn restore_draft(
    history: &TvHistory,
    device: &dyn TvDevice,
    id: &EntryId,
) -> Result<Draft, HistoryError> {
    let store = history.store();
    let target = store.state(id)?;
    let head = store.head_state()?.unwrap_or_else(State::new);
    let mut draft = Draft::default();
    for key in SETTING_KEYS {
        let now = device.setting(key)?;
        let want = target.settings.global.get(key).cloned();
        let number = |value: &Option<String>| value.as_deref().unwrap_or("1").parse::<f64>().ok();
        if number(&now) != number(&want) {
            draft.changes.push(ApprovedChange::Setting {
                key: key.to_owned(),
                old: now,
                new: want,
            });
        }
    }
    for (name, want) in &target.packages {
        let seen = head.packages.get(name).copied();
        if seen != Some(*want) {
            draft.changes.push(ApprovedChange::Package {
                name: name.clone(),
                old: seen,
                new: *want,
            });
        }
    }
    let launcher = device.launcher()?;
    if launcher != target.launcher {
        draft.launcher = Some(target.launcher);
    }
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::time::Instant;

    use rusttv_history::Launcher;

    use super::*;

    #[derive(Debug, Default)]
    struct Device {
        launcher: Launcher,
        settings: BTreeMap<String, String>,
    }

    impl TvDevice for Device {
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
            true
        }
        fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
            self.settings.insert(key.to_owned(), value.to_owned());
            Ok(())
        }
    }

    struct Clock(Instant);

    impl TvClock for Clock {
        fn elapsed_ms(&self) -> u64 {
            u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
        }
        fn boot_count(&self) -> u64 {
            1
        }
        fn now_unix(&self) -> i64 {
            1_790_000_000
        }
    }

    struct Platform {
        root: PathBuf,
        device: Device,
        clock: Clock,
        test_build: bool,
    }

    impl HistoryPlatform for Platform {
        fn root(&self) -> PathBuf {
            self.root.clone()
        }
        fn parts(&mut self) -> (&mut dyn TvDevice, &dyn TvClock) {
            (&mut self.device, &self.clock)
        }
        fn new_id(&self) -> HistoryId {
            HistoryId::parse("0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69").unwrap()
        }
        fn test_build(&self) -> bool {
            self.test_build
        }
    }

    fn start(test_build: bool) -> (History, PathBuf) {
        static NEXT: Mutex<u32> = Mutex::new(0);
        let mut next = NEXT.lock().unwrap();
        *next += 1;
        let root = std::env::temp_dir().join(format!(
            "rusttv-launcher-history-{}-{next}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let platform = Platform {
            root: root.clone(),
            device: Device::default(),
            clock: Clock(Instant::now()),
            test_build,
        };
        (History::start(Box::new(platform)), root)
    }

    #[test]
    fn records_lists_and_restores_through_the_thread() {
        let (history, root) = start(false);
        history.launcher_edited();
        let draft = Draft {
            changes: vec![ApprovedChange::Setting {
                key: "animator_duration_scale".into(),
                old: None,
                new: Some("0.5".into()),
            }],
            launcher: None,
        };
        assert!(matches!(
            history.apply(draft),
            Ok(ApplyOutcome::Applied { .. })
        ));
        let overview = history.overview();
        assert_eq!(overview.error, None);
        assert_eq!(overview.entries.len(), 2);
        assert_eq!(overview.settings["animator_duration_scale"], "0.5");
        let first = overview.entries[1].id.clone();
        let back = history.restore_draft(first).unwrap();
        assert_eq!(back.changes.len(), 1);
        assert!(!history.busy());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn test_builds_cannot_approve() {
        let (history, root) = start(true);
        let draft = Draft {
            changes: Vec::new(),
            launcher: Some(Launcher::default()),
        };
        assert_eq!(history.apply(draft).unwrap_err().code, "TEST_BUILD");
        assert!(history.overview().test_build);
        let _ = std::fs::remove_dir_all(root);
    }
}

/// The history on a computer (the desktop dev mode): the real order file,
/// settings kept in memory, a monotonic clock.
#[cfg(not(target_os = "android"))]
pub(crate) mod desktop {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use rusttv_history::tv::{TvClock, TvDevice};
    use rusttv_history::{HistoryError, HistoryId, Launcher};

    use super::HistoryPlatform;
    use crate::order_file;

    struct Device {
        dir: PathBuf,
        settings: BTreeMap<String, String>,
    }

    impl TvDevice for Device {
        fn launcher(&self) -> Result<Launcher, HistoryError> {
            Ok(order_file::launcher_state(&self.dir))
        }
        fn write_launcher(&mut self, launcher: &Launcher) -> Result<(), HistoryError> {
            order_file::write_launcher_state(&self.dir, launcher)
                .map_err(|error| HistoryError::Storage(error.to_string()))
        }
        fn setting(&self, key: &str) -> Result<Option<String>, HistoryError> {
            Ok(self.settings.get(key).cloned())
        }
        fn can_write_settings(&self) -> bool {
            true
        }
        fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
            self.settings.insert(key.to_owned(), value.to_owned());
            Ok(())
        }
    }

    struct Clock(Instant);

    impl TvClock for Clock {
        fn elapsed_ms(&self) -> u64 {
            u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
        }
        fn boot_count(&self) -> u64 {
            1
        }
        fn now_unix(&self) -> i64 {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| {
                    i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
                })
        }
    }

    struct Platform {
        device: Device,
        clock: Clock,
    }

    impl HistoryPlatform for Platform {
        fn root(&self) -> PathBuf {
            self.device.dir.clone()
        }
        fn parts(&mut self) -> (&mut dyn TvDevice, &dyn TvClock) {
            (&mut self.device, &self.clock)
        }
        fn new_id(&self) -> HistoryId {
            use std::hash::{BuildHasher, Hasher};
            // Dev mode only: the hasher's random keys are enough here.
            let mut bytes = [0; 16];
            for (index, chunk) in bytes.chunks_mut(8).enumerate() {
                let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
                hasher.write_usize(index);
                chunk.copy_from_slice(&hasher.finish().to_le_bytes());
            }
            HistoryId::from_random(bytes)
        }
        fn test_build(&self) -> bool {
            false
        }
    }

    /// Starts the process's history thread over `dir` (where the order file is).
    pub(crate) fn start(dir: PathBuf) {
        super::History::global(|| {
            Box::new(Platform {
                device: Device {
                    dir,
                    settings: BTreeMap::new(),
                },
                clock: Clock(Instant::now()),
            })
        });
    }
}

#[cfg(test)]
mod containment {
    /// Rust reaches system settings only through `SecureSettings` (the one
    /// JNI call by index in android/history.rs); no other code names the
    /// settings provider or its classes (HISTORY.md § 7).
    #[test]
    fn no_rust_code_writes_system_settings() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let this_file = src.join("history.rs");
        let mut files = vec![src];
        let mut offenders = Vec::new();
        while let Some(path) = files.pop() {
            if path.is_dir() {
                files.extend(
                    std::fs::read_dir(&path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for needle in [
                "android/provider/Settings",
                "content://settings",
                "putString",
                "Settings$",
            ] {
                // This test names them itself.
                if text.contains(needle) && path != this_file {
                    offenders.push(format!("{}: {needle}", path.display()));
                }
            }
        }
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}

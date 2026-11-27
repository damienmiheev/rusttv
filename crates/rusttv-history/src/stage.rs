//! The staging door (SECURITY.md "Packages: staging and install", risk
//! class RS): how the CLI hands the launcher one inert package.
//!
//! Three steps, all through the launcher's door provider:
//! 1. `content call --method stage --arg begin:<size>:<sha256>` answers a
//!    one-time nonce, valid for 60 s;
//! 2. `content write --uri …/stage/<nonce>` streams the bytes, capped at the
//!    announced size;
//! 3. `content call --method stage --arg end:<nonce>` keeps them in the slot
//!    only if their length and SHA-256 match what step 1 announced.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use crate::HistoryError;
use crate::door::{is_hash, is_nonce};

/// Version of the staging answers.
pub const STAGE_VERSION: u32 = 1;

/// The largest package that can be staged: PANEL_STUDIO.md § 5.1's
/// compressed cap.
pub const MAX_STAGED: u64 = 4 << 20;

/// Every code the staging door answers with.
pub const STAGE_ERRORS: [&str; 6] = [
    "STAGING_BUSY",
    "STAGING_TOO_BIG",
    "STAGING_MISMATCH",
    "STAGING_EXPIRED",
    "STAGING_INVALID",
    "STAGING_STORAGE",
];

/// The `--arg` of `content call … stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageArg<'a> {
    /// Announces the bytes that follow: their size and SHA-256 (64 hex).
    Begin {
        /// Bytes, 1..=[`MAX_STAGED`].
        size: u64,
        /// Lowercase hex.
        sha256: &'a str,
    },
    /// The bytes for this nonce are written: check and keep them.
    End {
        /// 16 lowercase hex.
        nonce: &'a str,
    },
}

impl<'a> StageArg<'a> {
    /// The argument text.
    pub fn encode(&self) -> String {
        match self {
            Self::Begin { size, sha256 } => format!("begin:{size}:{sha256}"),
            Self::End { nonce } => format!("end:{nonce}"),
        }
    }

    /// Parses an argument; only well-formed, in-range values pass.
    ///
    /// # Errors
    ///
    /// [`HistoryError::Invalid`] for anything else.
    pub fn parse(arg: &'a str) -> Result<Self, HistoryError> {
        let bad = || HistoryError::invalid(format!("bad stage argument {arg:?}"));
        match arg.split(':').collect::<Vec<_>>().as_slice() {
            ["begin", size, sha256] if is_hash(sha256) => {
                let size: u64 = size
                    .parse()
                    .ok()
                    .filter(|n| (1..=MAX_STAGED).contains(n))
                    .ok_or_else(bad)?;
                // Digits only: `parse` also takes a leading `+`.
                if !size_digits(arg) {
                    return Err(bad());
                }
                Ok(Self::Begin { size, sha256 })
            }
            ["end", nonce] if is_nonce(nonce) => Ok(Self::End { nonce }),
            _ => Err(bad()),
        }
    }
}

/// The size field of a `begin:` argument is plain digits.
fn size_digits(arg: &str) -> bool {
    arg.split(':')
        .nth(1)
        .is_some_and(|size| size.bytes().all(|b| b.is_ascii_digit()))
}

/// What a stage call answered: one line in the result bundle, key `rusttv`:
/// `rusttv-stage 1 ok <nonce|none>` or `rusttv-stage 1 error <CODE>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageResult {
    /// `None` when it worked, else one of [`STAGE_ERRORS`].
    pub error: Option<String>,
    /// The nonce `begin` handed out.
    pub nonce: Option<String>,
}

impl StageResult {
    /// It worked; `nonce` for `begin`.
    pub fn ok(nonce: Option<String>) -> Self {
        Self { error: None, nonce }
    }

    /// It failed with `code`.
    pub fn error(code: &str) -> Self {
        Self {
            error: Some(code.to_owned()),
            nonce: None,
        }
    }

    /// The result line.
    pub fn encode(&self) -> String {
        match &self.error {
            None => format!(
                "rusttv-stage {STAGE_VERSION} ok {}",
                self.nonce.as_deref().unwrap_or("none")
            ),
            Some(code) => format!("rusttv-stage {STAGE_VERSION} error {code}"),
        }
    }

    /// Finds and parses the result line in `content call`'s output
    /// (`Result: Bundle[{rusttv=rusttv-stage …}]`).
    ///
    /// # Errors
    ///
    /// [`HistoryError::Invalid`] when there is no well-formed line, as with
    /// a launcher too old to stage.
    pub fn parse(output: &str) -> Result<Self, HistoryError> {
        let bad = || HistoryError::invalid(format!("no RustTV stage result in {output:?}"));
        let start = output.find("rusttv-stage ").ok_or_else(bad)?;
        let line: String = output[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
            .collect();
        match line.split(' ').collect::<Vec<_>>().as_slice() {
            ["rusttv-stage", "1", "ok", "none"] => Ok(Self::ok(None)),
            ["rusttv-stage", "1", "ok", nonce] if is_nonce(nonce) => {
                Ok(Self::ok(Some((*nonce).to_owned())))
            }
            ["rusttv-stage", "1", "error", code] if STAGE_ERRORS.contains(code) => {
                Ok(Self::error(code))
            }
            _ => Err(bad()),
        }
    }
}

// The slot: the TV's side of the door. One inert package waits here until a
// person reviews it. Everything lives in files under one folder, so the door
// provider (which may run in a process the launcher didn't start) and Home
// see the same slot:
// - `pending`: the `begin` in flight: nonce, size, SHA-256, when;
// - `incoming-<nonce>`: its bytes while they arrive;
// - `slot` and `slot.meta`: the package waiting, and its size, SHA-256 and
//   when it was staged;
// - `rate`: when each `begin` of the last hour came.
// Times are milliseconds of TV uptime with the boot they belong to: a file
// from another boot is dropped, so the slot is gone after a reboot.

/// How long a nonce from `begin` can be written to (SECURITY.md rule 2).
const NONCE_MS: u64 = 60_000;
/// How long a staged package waits (rule 2, default).
const SLOT_MS: u64 = 24 * 60 * 60 * 1000;
/// Stagings an hour (rule 3, default).
const PER_HOUR: usize = 10;
const HOUR_MS: u64 = 60 * 60 * 1000;

/// One staging at a time: the provider answers on several binder threads.
static LOCK: Mutex<()> = Mutex::new(());

/// A moment: TV uptime in ms, in boot `boot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    /// Which boot (Android's `Settings.Global.BOOT_COUNT`).
    pub boot: u64,
    /// Uptime, ms (`SystemClock.elapsedRealtime`).
    pub ms: u64,
}

/// The package waiting in the slot: only facts the launcher derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// Bytes.
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// The staging slot's folder, as the TV's launcher keeps it.
#[derive(Debug, Clone)]
pub struct Slot {
    dir: PathBuf,
}

/// Where `begin` stands: nonce, size, SHA-256, when.
struct Pending {
    nonce: String,
    size: u64,
    sha256: String,
    at: Moment,
    /// Its one write has started.
    written: bool,
}

impl Slot {
    /// The slot in `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Answers `content call … stage --arg <arg>`; `nonce` is a fresh random
    /// 16-hex nonce for a `begin`.
    pub fn call(&self, arg: &str, nonce: &str, at: Moment) -> StageResult {
        let Ok(_guard) = LOCK.lock() else {
            return StageResult::error("STAGING_STORAGE");
        };
        let result = if at.boot == 0 {
            // No boot count: a reboot couldn't be told apart (rule 2).
            Err("STAGING_STORAGE")
        } else if arg.starts_with("begin:") {
            // Every begin counts, malformed and failed ones too (rule 3).
            self.count(at).and_then(|()| match StageArg::parse(arg) {
                Ok(StageArg::Begin { size, sha256 }) => self
                    .begin(size, sha256, nonce, at)
                    .map(|()| Some(nonce.to_owned())),
                _ => Err("STAGING_INVALID"),
            })
        } else {
            match StageArg::parse(arg) {
                Ok(StageArg::End { nonce }) => self.end(nonce, at).map(|()| None),
                _ => Err("STAGING_INVALID"),
            }
        };
        match result {
            Ok(nonce) => StageResult::ok(nonce),
            Err(code) => StageResult::error(code),
        }
    }

    /// Where the bytes for `nonce` go, and at most how many: only for the
    /// latest `begin`, within its 60 s, and only once.
    pub fn incoming(&self, nonce: &str, at: Moment) -> Option<(PathBuf, u64)> {
        let _guard = LOCK.lock().ok()?;
        let pending = self.pending(at)?;
        if pending.nonce != nonce || pending.written {
            return None;
        }
        let line = format!(
            "{} {} {} {} {} written\n",
            pending.nonce, pending.size, pending.sha256, pending.at.boot, pending.at.ms
        );
        write_atomic(&self.dir.join("pending"), line.as_bytes()).ok()?;
        Some((self.incoming_path(nonce), pending.size))
    }

    /// The package waiting, if any; an expired one is deleted.
    pub fn waiting(&self, at: Moment) -> Option<Waiting> {
        let _guard = LOCK.lock().ok()?;
        let meta = fs::read_to_string(self.dir.join("slot.meta")).unwrap_or_default();
        let fields: Vec<&str> = meta.split_whitespace().collect();
        let parsed = match fields.as_slice() {
            [size, sha256, boot, ms] => size.parse::<u64>().ok().and_then(|size| {
                Some((
                    size,
                    (*sha256).to_owned(),
                    Moment {
                        boot: boot.parse().ok()?,
                        ms: ms.parse().ok()?,
                    },
                ))
            }),
            _ => None,
        };
        // The slot's bytes must be the ones the meta describes.
        let on_disk = fs::metadata(self.dir.join("slot"))
            .map(|meta| meta.len())
            .ok();
        match parsed {
            Some((size, sha256, staged))
                if at.boot != 0 && fresh(staged, at, SLOT_MS) && on_disk == Some(size) =>
            {
                Some(Waiting { size, sha256 })
            }
            _ => {
                self.remove_slot();
                None
            }
        }
    }

    /// Copies the package waiting into the review's own file (rule 5), so a
    /// later staging can't change what is reviewed; `None` when nothing
    /// waits. The copy's bytes are checked against the facts by whoever
    /// reads it.
    ///
    /// # Errors
    ///
    /// Copying failed.
    pub fn copy_for_review(&self, at: Moment) -> io::Result<Option<Waiting>> {
        let Some(waiting) = self.waiting(at) else {
            return Ok(None);
        };
        // Under the lock: `end` replaces the bytes and meta in two renames.
        let _guard = LOCK
            .lock()
            .map_err(|_| io::Error::other("the slot lock is poisoned"))?;
        let meta = fs::read_to_string(self.dir.join("slot.meta")).unwrap_or_default();
        if !meta.contains(&waiting.sha256) {
            // Replaced between the two reads: review the newer one later.
            return Ok(None);
        }
        let bytes = fs::read(self.dir.join("slot"))?;
        drop(_guard);
        crate::packages::write_synced(&self.review_path(), &bytes)?;
        Ok(Some(waiting))
    }

    /// The review's copy of the package.
    pub fn review_path(&self) -> PathBuf {
        self.dir.join("review")
    }

    /// Sets the crash marker while the package with `sha256` is checked,
    /// or clears it (`None`).
    pub fn set_checking(&self, sha256: Option<&str>) -> io::Result<()> {
        let path = self.dir.join("checking");
        match sha256 {
            Some(sha256) => crate::packages::write_synced(&path, sha256.as_bytes()),
            None => match fs::remove_file(path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            },
        }
    }

    /// Deletes the review copy, and the slot too when it still holds
    /// `sha256` (a newer staging stays).
    /// Returns whether the slot was deleted.
    pub fn discard(&self, sha256: Option<&str>) -> bool {
        let _ = fs::remove_file(self.review_path());
        let Some(sha256) = sha256 else {
            return false;
        };
        let Ok(_guard) = LOCK.lock() else {
            return false;
        };
        let meta = fs::read_to_string(self.dir.join("slot.meta")).unwrap_or_default();
        let held = meta.split_whitespace().nth(1) == Some(sha256);
        if held {
            self.remove_slot();
        }
        held
    }

    /// At the launcher's start, before anything reads the slot: a review
    /// lives only in memory, so its copy goes; a crash marker means the
    /// launcher died checking a package, which is then deleted and noted
    /// (crash quarantine, rule 5). Returns whether that happened.
    pub fn after_start(&self) -> bool {
        for stale in ["review", "review.new", "checking.new"] {
            let _ = fs::remove_file(self.dir.join(stale));
        }
        let marker = self.dir.join("checking");
        let Ok(sha256) = fs::read_to_string(&marker) else {
            return false;
        };
        let deleted = self.discard(Some(sha256.trim()));
        if deleted {
            let _ = fs::write(self.dir.join("quarantined"), b"");
        }
        let _ = fs::remove_file(marker);
        deleted
    }

    /// Whether a package was deleted because checking it stopped the
    /// launcher, since the last staging or [`Self::clear_quarantined`].
    pub fn quarantined(&self) -> bool {
        self.dir.join("quarantined").exists()
    }

    /// The person has seen the quarantine notice.
    pub fn clear_quarantined(&self) {
        let _ = fs::remove_file(self.dir.join("quarantined"));
    }

    /// Counts a begin; refuses past [`PER_HOUR`] in the last hour.
    fn count(&self, at: Moment) -> Result<(), &'static str> {
        fs::create_dir_all(&self.dir).map_err(|_| "STAGING_STORAGE")?;
        let mut times = self.rate(at);
        if times.len() >= PER_HOUR {
            return Err("STAGING_BUSY");
        }
        times.push(at.ms);
        let rate: String = times
            .iter()
            .map(|ms| format!("{} {ms}\n", at.boot))
            .collect();
        write_atomic(&self.dir.join("rate"), rate.as_bytes()).map_err(|_| "STAGING_STORAGE")
    }

    fn begin(&self, size: u64, sha256: &str, nonce: &str, at: Moment) -> Result<(), &'static str> {
        // A newer begin replaces an unfinished one, and no stray incoming
        // file outlives it. A quarantine notice is for the package before.
        let _ = fs::remove_file(self.dir.join("pending"));
        let _ = fs::remove_file(self.dir.join("quarantined"));
        if let Ok(entries) = fs::read_dir(&self.dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with("incoming-") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let line = format!("{nonce} {size} {sha256} {} {}\n", at.boot, at.ms);
        write_atomic(&self.dir.join("pending"), line.as_bytes()).map_err(|_| "STAGING_STORAGE")
    }

    fn end(&self, nonce: &str, at: Moment) -> Result<(), &'static str> {
        let pending = self
            .pending(at)
            .filter(|pending| pending.nonce == nonce)
            .ok_or("STAGING_EXPIRED")?;
        let incoming = self.incoming_path(nonce);
        let _ = fs::remove_file(self.dir.join("pending"));
        let checked = hash_file(&incoming, pending.size);
        let result = match checked {
            Ok(Some((size, sha256))) if size == pending.size && sha256 == pending.sha256 => {
                // Only now that the bytes match is the slot replaced (rule 2):
                // its meta goes first, so a slot without a matching meta is
                // never shown, then the bytes in one rename in the same
                // folder, then the new meta.
                let meta = format!("{size} {sha256} {} {}\n", at.boot, at.ms);
                let tmp = self.dir.join("slot.meta.new");
                fs::write(&tmp, meta.as_bytes())
                    .and_then(|()| {
                        let _ = fs::remove_file(self.dir.join("slot.meta"));
                        fs::rename(&incoming, self.dir.join("slot"))
                    })
                    .and_then(|()| fs::rename(&tmp, self.dir.join("slot.meta")))
                    .map_err(|_| "STAGING_STORAGE")
            }
            Ok(None) => Err("STAGING_TOO_BIG"),
            Ok(Some(_)) => Err("STAGING_MISMATCH"),
            // Nothing was written.
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err("STAGING_MISMATCH"),
            Err(_) => Err("STAGING_STORAGE"),
        };
        let _ = fs::remove_file(&incoming);
        result
    }

    /// The `begin` in flight, if it is from this boot; else it is dropped.
    fn pending(&self, at: Moment) -> Option<Pending> {
        let text = fs::read_to_string(self.dir.join("pending")).ok()?;
        let fields: Vec<&str> = text.split_whitespace().collect();
        let parse = |nonce: &str, size: &str, sha256: &str, boot: &str, ms: &str, written| {
            Some(Pending {
                nonce: nonce.to_owned(),
                size: size.parse().ok()?,
                sha256: sha256.to_owned(),
                at: Moment {
                    boot: boot.parse().ok()?,
                    ms: ms.parse().ok()?,
                },
                written,
            })
        };
        let pending = match fields.as_slice() {
            [nonce, size, sha256, boot, ms] => parse(nonce, size, sha256, boot, ms, false),
            [nonce, size, sha256, boot, ms, "written"] => {
                parse(nonce, size, sha256, boot, ms, true)
            }
            _ => None,
        };
        match pending {
            Some(pending) if fresh(pending.at, at, NONCE_MS) => Some(pending),
            other => {
                if let Some(old) = other {
                    let _ = fs::remove_file(self.incoming_path(&old.nonce));
                }
                let _ = fs::remove_file(self.dir.join("pending"));
                None
            }
        }
    }

    /// The begins of the last hour of this boot.
    fn rate(&self, at: Moment) -> Vec<u64> {
        let text = fs::read_to_string(self.dir.join("rate")).unwrap_or_default();
        text.lines()
            .filter_map(|line| {
                let (boot, ms) = line.split_once(' ')?;
                let then = Moment {
                    boot: boot.parse().ok()?,
                    ms: ms.parse().ok()?,
                };
                fresh(then, at, HOUR_MS).then_some(then.ms)
            })
            .collect()
    }

    fn incoming_path(&self, nonce: &str) -> PathBuf {
        self.dir.join(format!("incoming-{nonce}"))
    }

    fn remove_slot(&self) {
        let _ = fs::remove_file(self.dir.join("slot"));
        let _ = fs::remove_file(self.dir.join("slot.meta"));
    }
}

/// `then` is in this boot, not in the future, and less than `limit` ago.
fn fresh(then: Moment, now: Moment, limit: u64) -> bool {
    then.boot == now.boot && then.ms <= now.ms && now.ms - then.ms <= limit
}

/// The SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The file's size and SHA-256, or `None` when it holds more than `cap`.
pub(crate) fn hash_file(path: &Path, cap: u64) -> io::Result<Option<(u64, String)>> {
    let file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buf = [0_u8; 64 * 1024];
    let mut reader = file.take(cap + 1);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    if size > cap {
        return Ok(None);
    }
    let hex = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(Some((size, hex)))
}

/// Writes `bytes` beside `path`, then renames it into place.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    #[test]
    fn arguments_round_trip_and_bad_ones_are_refused() {
        for arg in [
            StageArg::Begin {
                size: 1,
                sha256: HASH,
            },
            StageArg::Begin {
                size: MAX_STAGED,
                sha256: HASH,
            },
            StageArg::End {
                nonce: "0123456789abcdef",
            },
        ] {
            assert_eq!(StageArg::parse(&arg.encode()).unwrap(), arg);
        }
        for bad in [
            "begin:0:".to_owned() + HASH,
            format!("begin:{}:{HASH}", MAX_STAGED + 1),
            format!("begin:+5:{HASH}"),
            "begin:5:ABCD".to_owned(),
            format!("begin:5:{HASH}:x"),
            "end:0123".to_owned(),
            "end:0123456789ABCDEF".to_owned(),
            "commit:0123456789abcdef".to_owned(),
            String::new(),
        ] {
            assert!(StageArg::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn results_round_trip_inside_the_bundle_text() {
        for result in [
            StageResult::ok(Some("0123456789abcdef".to_owned())),
            StageResult::ok(None),
            StageResult::error("STAGING_BUSY"),
        ] {
            let output = format!("Result: Bundle[{{rusttv={}}}]\n", result.encode());
            assert_eq!(StageResult::parse(&output).unwrap(), result);
        }
        // An older launcher has no stage method: no result line.
        assert!(StageResult::parse("Error while accessing provider").is_err());
        assert!(StageResult::parse("rusttv-stage 1 error SOMETHING_ELSE").is_err());
        assert!(StageResult::parse("rusttv-stage 2 ok none").is_err());
    }
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rusttv-staging-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const NONCE: &str = "0123456789abcdef";

    fn at(ms: u64) -> Moment {
        Moment { boot: 7, ms }
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn begin(staging: &Slot, bytes: &[u8], nonce: &str, ms: u64) -> StageResult {
        let arg = StageArg::Begin {
            size: bytes.len() as u64,
            sha256: &sha(bytes),
        };
        staging.call(&arg.encode(), nonce, at(ms))
    }

    fn end(staging: &Slot, nonce: &str, ms: u64) -> StageResult {
        staging.call(
            &StageArg::End { nonce }.encode(),
            "ffffffffffffffff",
            at(ms),
        )
    }

    /// Writes `bytes` where `incoming` says, as the provider's pipe does.
    fn write(staging: &Slot, nonce: &str, bytes: &[u8], ms: u64) {
        let (path, cap) = staging.incoming(nonce, at(ms)).expect("writable");
        assert_eq!(cap, bytes.len() as u64);
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn bytes_that_match_wait_in_the_slot_until_they_expire() {
        let dir = TempDir::new("match");
        let staging = Slot::new(&dir.0);
        let bytes = b"PK fake package";
        assert_eq!(
            begin(&staging, bytes, NONCE, 1_000).nonce.as_deref(),
            Some(NONCE)
        );
        write(&staging, NONCE, bytes, 2_000);
        assert_eq!(end(&staging, NONCE, 3_000), StageResult::ok(None));
        let waiting = staging.waiting(at(4_000)).unwrap();
        assert_eq!(waiting.size, bytes.len() as u64);
        assert_eq!(waiting.sha256, sha(bytes));
        assert_eq!(fs::read(dir.0.join("slot")).unwrap(), bytes);
        // 24 h later it is gone.
        assert!(staging.waiting(at(3_000 + SLOT_MS + 1)).is_none());
        assert!(!dir.0.join("slot").exists());
    }

    fn stage(staging: &Slot, bytes: &[u8], ms: u64) {
        begin(staging, bytes, NONCE, ms);
        write(staging, NONCE, bytes, ms);
        assert_eq!(end(staging, NONCE, ms), StageResult::ok(None));
    }

    #[test]
    fn the_review_copy_does_not_follow_a_later_staging() {
        let dir = TempDir::new("review");
        let staging = Slot::new(&dir.0);
        stage(&staging, b"first", 1_000);
        let waiting = staging.copy_for_review(at(2_000)).unwrap().unwrap();
        assert_eq!(waiting.sha256, sha(b"first"));
        stage(&staging, b"second", 3_000);
        assert_eq!(fs::read(staging.review_path()).unwrap(), b"first");
        // Done with the first: the newer staging stays.
        staging.discard(Some(&sha(b"first")));
        assert!(!staging.review_path().exists());
        assert_eq!(staging.waiting(at(4_000)).unwrap().sha256, sha(b"second"));
        // Nothing waiting, nothing to copy.
        staging.discard(Some(&sha(b"second")));
        assert_eq!(staging.copy_for_review(at(5_000)).unwrap(), None);
    }

    #[test]
    fn a_crash_while_checking_deletes_that_package_at_the_next_start() {
        let dir = TempDir::new("quarantine");
        let staging = Slot::new(&dir.0);
        stage(&staging, b"bad", 1_000);
        staging.copy_for_review(at(1_000)).unwrap();
        staging.set_checking(Some(&sha(b"bad"))).unwrap();
        // The launcher dies here; at the next start:
        assert!(staging.after_start());
        assert!(staging.quarantined());
        assert!(staging.waiting(at(2_000)).is_none());
        assert!(!staging.review_path().exists());
        assert!(!staging.after_start(), "only once");
        // A new staging clears the notice.
        stage(&staging, b"good", 3_000);
        assert!(!staging.quarantined());
    }

    #[test]
    fn a_crash_marker_spares_a_newer_staging_and_a_start_drops_old_copies() {
        let dir = TempDir::new("spare");
        let staging = Slot::new(&dir.0);
        stage(&staging, b"old", 1_000);
        staging.copy_for_review(at(1_000)).unwrap();
        staging.set_checking(Some(&sha(b"old"))).unwrap();
        stage(&staging, b"new", 2_000);
        // Nothing was deleted, so there is no notice.
        assert!(!staging.after_start());
        assert!(!staging.quarantined());
        assert_eq!(staging.waiting(at(3_000)).unwrap().sha256, sha(b"new"));
        // Without a marker a start only drops the review copy.
        staging.copy_for_review(at(3_000)).unwrap();
        staging.clear_quarantined();
        assert!(!staging.after_start());
        assert!(!staging.review_path().exists());
        assert!(!staging.quarantined());
        assert!(staging.waiting(at(4_000)).is_some());
    }

    #[test]
    fn a_reboot_empties_the_slot() {
        let dir = TempDir::new("reboot");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        write(&staging, NONCE, b"abc", 1_000);
        end(&staging, NONCE, 1_000);
        let rebooted = Moment { boot: 8, ms: 5_000 };
        assert!(staging.waiting(rebooted).is_none());
    }

    #[test]
    fn wrong_bytes_keep_the_previous_slot() {
        let dir = TempDir::new("mismatch");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"first", NONCE, 1_000);
        write(&staging, NONCE, b"first", 1_000);
        end(&staging, NONCE, 1_000);
        let second = "1111111111111111";
        begin(&staging, b"second", second, 2_000);
        let (path, _) = staging.incoming(second, at(2_000)).unwrap();
        fs::write(&path, b"SECOND").unwrap();
        assert_eq!(
            end(&staging, second, 2_000),
            StageResult::error("STAGING_MISMATCH")
        );
        assert!(!path.exists(), "the partial file goes");
        assert_eq!(staging.waiting(at(3_000)).unwrap().sha256, sha(b"first"));
        // More bytes than announced.
        let third = "2222222222222222";
        begin(&staging, b"abc", third, 3_000);
        let (path, _) = staging.incoming(third, at(3_000)).unwrap();
        fs::write(&path, b"abcd").unwrap();
        assert_eq!(
            end(&staging, third, 3_000),
            StageResult::error("STAGING_TOO_BIG")
        );
        assert_eq!(staging.waiting(at(3_000)).unwrap().sha256, sha(b"first"));
    }

    #[test]
    fn a_nonce_is_good_for_one_write_within_a_minute() {
        let dir = TempDir::new("nonce");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        assert!(staging.incoming("1111111111111111", at(1_000)).is_none());
        assert!(staging.incoming(NONCE, at(1_000 + NONCE_MS + 1)).is_none());
        assert_eq!(
            end(&staging, NONCE, 1_000 + NONCE_MS + 1),
            StageResult::error("STAGING_EXPIRED")
        );
        // A newer begin replaces the older nonce.
        begin(&staging, b"abc", NONCE, 100_000);
        begin(&staging, b"abc", "1111111111111111", 100_001);
        assert!(staging.incoming(NONCE, at(100_002)).is_none());
        // And end uses it up.
        write(&staging, "1111111111111111", b"abc", 100_002);
        assert!(end(&staging, "1111111111111111", 100_003).error.is_none());
        assert_eq!(
            end(&staging, "1111111111111111", 100_004),
            StageResult::error("STAGING_EXPIRED")
        );
    }

    #[test]
    fn ten_begins_an_hour_then_busy() {
        let dir = TempDir::new("rate");
        let staging = Slot::new(&dir.0);
        for n in 0..PER_HOUR as u64 {
            assert!(begin(&staging, b"abc", NONCE, 1_000 + n).error.is_none());
        }
        assert_eq!(
            begin(&staging, b"abc", NONCE, 2_000),
            StageResult::error("STAGING_BUSY")
        );
        assert!(
            begin(&staging, b"abc", NONCE, 1_000 + HOUR_MS + 10)
                .error
                .is_none()
        );
    }

    #[test]
    fn bad_arguments_are_refused() {
        let dir = TempDir::new("bad");
        let staging = Slot::new(&dir.0);
        assert_eq!(
            staging.call("begin:5:xyz", NONCE, at(1)),
            StageResult::error("STAGING_INVALID")
        );
        assert!(staging.waiting(at(1)).is_none());
    }

    #[test]
    fn a_nonce_opens_one_write_only() {
        let dir = TempDir::new("one-write");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        assert!(staging.incoming(NONCE, at(1_000)).is_some());
        assert!(
            staging.incoming(NONCE, at(1_001)).is_none(),
            "a second write"
        );
    }

    #[test]
    fn an_end_with_nothing_written_is_a_mismatch() {
        let dir = TempDir::new("nothing");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        assert_eq!(
            end(&staging, NONCE, 1_000),
            StageResult::error("STAGING_MISMATCH")
        );
    }

    #[test]
    fn a_begin_clears_stray_incoming_files() {
        let dir = TempDir::new("stray");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        let (path, _) = staging.incoming(NONCE, at(1_000)).unwrap();
        fs::write(&path, b"ab").unwrap();
        begin(&staging, b"abc", "1111111111111111", 2_000);
        assert!(!path.exists());
    }

    #[test]
    fn malformed_begins_count_toward_the_limit() {
        let dir = TempDir::new("rate-bad");
        let staging = Slot::new(&dir.0);
        for n in 0..PER_HOUR as u64 {
            assert_eq!(
                staging.call("begin:x:y", NONCE, at(1_000 + n)),
                StageResult::error("STAGING_INVALID")
            );
        }
        assert_eq!(
            begin(&staging, b"abc", NONCE, 2_000),
            StageResult::error("STAGING_BUSY")
        );
    }

    #[test]
    fn without_a_boot_count_nothing_is_staged() {
        let dir = TempDir::new("no-boot");
        let staging = Slot::new(&dir.0);
        let zero = Moment { boot: 0, ms: 5 };
        let arg = StageArg::Begin {
            size: 3,
            sha256: &sha(b"abc"),
        };
        assert_eq!(
            staging.call(&arg.encode(), NONCE, zero),
            StageResult::error("STAGING_STORAGE")
        );
        assert!(staging.waiting(zero).is_none());
    }

    #[test]
    fn a_slot_whose_meta_does_not_match_is_dropped() {
        let dir = TempDir::new("bad-meta");
        let staging = Slot::new(&dir.0);
        begin(&staging, b"abc", NONCE, 1_000);
        write(&staging, NONCE, b"abc", 1_000);
        end(&staging, NONCE, 1_000);
        fs::write(dir.0.join("slot.meta"), "x y 7 1000\n").unwrap();
        assert!(staging.waiting(at(2_000)).is_none());
        assert!(!dir.0.join("slot").exists());
        // Bytes without their meta are never shown either.
        begin(&staging, b"abc", "1111111111111111", 3_000);
        write(&staging, "1111111111111111", b"abc", 3_000);
        end(&staging, "1111111111111111", 3_000);
        fs::write(dir.0.join("slot"), b"abcd").unwrap();
        assert!(staging.waiting(at(4_000)).is_none());
    }
}

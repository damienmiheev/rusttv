//! A fake TV for the CLI's integration tests: the state `adb shell` changes
//! (settings, disabled packages, the launcher and its grant) kept in
//! `<dir>/device.toml`, and the history door answered by the real TV side,
//! `rusttv_history::tv::TvHistory`, over `<dir>/tv`.
//!
//! `tests/fixtures/fake-adb` runs it as `rusttv-fake-tv <dir> <adb args
//! after -s serial>`. Two more calls are for the tests themselves:
//! `rusttv-fake-tv <dir> approve` reads a TOML change list on stdin and
//! approves it as a person on the TV would (packages become a pending
//! approval), and `rusttv-fake-tv <dir> edit` records the launcher's own
//! edit from `<dir>/launcher.toml`. It never talks to a device.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rusttv_history::door::{ApprovedChange, CommitKind, read_error};
use rusttv_history::tv::{TvClock, TvDevice, TvHistory};
use rusttv_history::{HistoryError, HistoryId, Launcher};
use serde::{Deserialize, Serialize};

const LAUNCHER: &str = "ai.rusttv.launcher";
const PERMISSION: &str = "android.permission.WRITE_SECURE_SETTINGS";
const URI: &str = "content://ai.rusttv.launcher.history";
const NEW_ID: &str = "0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69";

/// What `adb shell` can change on the fake TV.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Device {
    serialno: String,
    launcher_installed: bool,
    debuggable: bool,
    granted: bool,
    disabled: BTreeSet<String>,
    global: BTreeMap<String, String>,
    secure: BTreeMap<String, String>,
}

/// The TV's clocks, from `<dir>/clock.toml` (tests move them).
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
struct Clock {
    elapsed_ms: u64,
    boot_count: u64,
    now_unix: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            elapsed_ms: 60_000,
            boot_count: 1,
            now_unix: 1_792_843_149,
        }
    }
}

impl TvClock for Clock {
    fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    fn boot_count(&self) -> u64 {
        self.boot_count
    }

    fn now_unix(&self) -> i64 {
        self.now_unix
    }
}

/// The launcher's view of the fake TV.
struct Tv {
    dir: PathBuf,
    device: Device,
}

impl TvDevice for Tv {
    fn launcher(&self) -> Result<Launcher, HistoryError> {
        match fs::read_to_string(self.dir.join("launcher.toml")) {
            Ok(text) => {
                toml::from_str(&text).map_err(|error| HistoryError::Storage(error.to_string()))
            }
            Err(_) => Ok(Launcher::default()),
        }
    }

    fn write_launcher(&mut self, launcher: &Launcher) -> Result<(), HistoryError> {
        let text =
            toml::to_string(launcher).map_err(|error| HistoryError::Storage(error.to_string()))?;
        fs::write(self.dir.join("launcher.toml"), text)
            .map_err(|error| HistoryError::Storage(error.to_string()))
    }

    fn setting(&self, key: &str) -> Result<Option<String>, HistoryError> {
        Ok(self.device.global.get(key).cloned())
    }

    fn can_write_settings(&self) -> bool {
        self.device.granted
    }

    fn put_setting(&mut self, key: &str, value: &str) -> Result<(), HistoryError> {
        self.device.global.insert(key.to_owned(), value.to_owned());
        save(&self.dir, &self.device).map_err(HistoryError::Storage)
    }
}

fn read<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|error| format!("{}: {error}", path.display())),
        Err(_) => Ok(T::default()),
    }
}

fn save(dir: &Path, device: &Device) -> Result<(), String> {
    let text = toml::to_string(device).map_err(|error| error.to_string())?;
    fs::write(dir.join("device.toml"), text).map_err(|error| error.to_string())
}

/// The id of a new history: `<dir>/new-id` if the test wrote one, else
/// [`NEW_ID`] (the launcher makes a random UUID).
fn new_id(dir: &Path) -> HistoryId {
    fs::read_to_string(dir.join("new-id"))
        .ok()
        .and_then(|text| HistoryId::parse(text.trim()).ok())
        .or_else(|| HistoryId::parse(NEW_ID).ok())
        .unwrap_or_else(unreachable_id)
}

#[expect(clippy::panic, reason = "a test fixture; NEW_ID is a valid UUID")]
fn unreachable_id() -> HistoryId {
    panic!("NEW_ID is not a history id")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((dir, rest)) = args.split_first() else {
        return fail("usage: rusttv-fake-tv <dir> <adb args>");
    };
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    match run(Path::new(dir), &rest) {
        Ok(out) => {
            let mut stdout = std::io::stdout();
            if stdout.write_all(&out).is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(message) => fail(&message),
    }
}

#[expect(clippy::print_stderr, reason = "adb prints its errors on stderr")]
fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}

fn run(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut device: Device = read(&dir.join("device.toml"))?;
    let clock: Clock = read(&dir.join("clock.toml"))?;
    let text = |text: String| Ok(text.into_bytes());
    match args {
        ["shell", "getprop", "ro.serialno"] => text(format!("{}\n", device.serialno)),
        ["shell", "settings", "get", namespace, key] => {
            let map = if *namespace == "secure" {
                &device.secure
            } else {
                &device.global
            };
            text(format!(
                "{}\n",
                map.get(*key).map_or("null", String::as_str)
            ))
        }
        ["shell", "settings", "put", "global", key, value] => {
            device.global.insert((*key).to_owned(), (*value).to_owned());
            save(dir, &device)?;
            text(String::new())
        }
        ["shell", "settings", "delete", "global", key] => {
            device.global.remove(*key);
            save(dir, &device)?;
            text("Deleted 1 rows\n".to_owned())
        }
        ["shell", "pm", "list", "packages", "-d"] => text(
            device
                .disabled
                .iter()
                .map(|package| format!("package:{package}\n"))
                .collect(),
        ),
        ["shell", "pm", "disable-user", "--user", "0", package] => {
            device.disabled.insert((*package).to_owned());
            save(dir, &device)?;
            text(format!("Package {package} new state: disabled-user\n"))
        }
        ["shell", "pm", "enable", "--user", "0", package] => {
            device.disabled.remove(*package);
            save(dir, &device)?;
            text(format!("Package {package} new state: enabled\n"))
        }
        [
            "shell",
            "pm",
            verb @ ("grant" | "revoke"),
            LAUNCHER,
            PERMISSION,
        ] => {
            if !device.launcher_installed {
                return Err(format!("Unknown package: {LAUNCHER}"));
            }
            device.granted = *verb == "grant";
            save(dir, &device)?;
            text(String::new())
        }
        ["shell", "pm", "uninstall", LAUNCHER] => {
            device.launcher_installed = false;
            device.granted = false;
            save(dir, &device)?;
            // Android deletes the launcher's data with it.
            let _ = fs::remove_dir_all(dir.join("tv"));
            text("Success\n".to_owned())
        }
        ["shell", "dumpsys", "package", LAUNCHER] => text(dumpsys(&device)),
        ["exec-out", "content", "read", "--uri", uri] => {
            let history = open(dir, device)?;
            let path = uri
                .strip_prefix(URI)
                .ok_or_else(|| format!("unknown URI {uri}"))?;
            match path {
                "/pending" => Ok(history
                    .pending(&clock)
                    .and_then(|list| list.to_toml())
                    .map_or_else(|_| read_error("HISTORY_STORAGE"), String::into_bytes)),
                _ => {
                    let since = path
                        .strip_prefix("/export/")
                        .ok_or_else(|| format!("unknown URI {uri}"))?;
                    let since = (since != "all").then_some(since);
                    Ok(history
                        .export(since)
                        .unwrap_or_else(|refused| read_error(refused.code)))
                }
            }
        }
        ["exec-in", "content", "write", "--uri", uri] => {
            let nonce = uri
                .strip_prefix(&format!("{URI}/inbox/"))
                .ok_or_else(|| format!("unknown URI {uri}"))?;
            let mut bytes = Vec::new();
            std::io::stdin()
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            fs::create_dir_all(dir.join("inbox")).map_err(|error| error.to_string())?;
            fs::write(dir.join("inbox").join(nonce), bytes).map_err(|error| error.to_string())?;
            Ok(Vec::new())
        }
        [
            "shell",
            "content",
            "call",
            "--uri",
            URI,
            "--method",
            "commit",
            "--arg",
            arg,
        ] => {
            let (kind, nonce) = CommitKind::parse_arg(arg).map_err(|error| error.to_string())?;
            let inbox = dir.join("inbox").join(nonce);
            let bytes = fs::read(&inbox).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(&inbox);
            let tv = Tv {
                dir: dir.to_owned(),
                device: read(&dir.join("device.toml"))?,
            };
            let mut history = open(dir, device)?;
            let result = history.commit(kind, &bytes, &tv, &clock, &|| new_id(dir));
            text(format!("Result: Bundle[{{rusttv={}}}]\n", result.encode()))
        }
        ["approve"] => {
            #[derive(Deserialize)]
            struct List {
                change: Vec<ApprovedChange>,
            }
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|error| error.to_string())?;
            let list: List = toml::from_str(&input).map_err(|error| error.to_string())?;
            let mut tv = Tv {
                dir: dir.to_owned(),
                device: read(&dir.join("device.toml"))?,
            };
            let mut history = open(dir, device)?;
            let outcome = history
                .apply(&list.change, None, &mut tv, &clock, &|| new_id(dir))
                .map_err(|refused| refused.to_string())?;
            text(format!("{outcome:?}\n"))
        }
        ["edits", count] => {
            // `count` edits recorded as the launcher's own: two tiles
            // added, then the first two swapped each time.
            let count: usize = count.parse().map_err(|_| "bad count".to_owned())?;
            let mut tv = Tv {
                dir: dir.to_owned(),
                device: read(&dir.join("device.toml"))?,
            };
            let mut history = open(dir, device)?;
            for _ in 0..count {
                let mut launcher = tv.launcher().map_err(|error| error.to_string())?;
                if launcher.order.len() < 2 {
                    let tile = format!("com.example.app{}/.Main", launcher.order.len());
                    launcher.order.push(tile);
                } else {
                    launcher.order.swap(0, 1);
                }
                tv.write_launcher(&launcher)
                    .map_err(|error| error.to_string())?;
                history
                    .record_launcher_edit(&tv, &clock, &|| new_id(dir))
                    .map_err(|error| error.to_string())?;
            }
            text(String::new())
        }
        _ => Err(format!(
            "rusttv-fake-tv: unexpected call: {}",
            args.join(" ")
        )),
    }
}

/// The TV's history, as the launcher opens it on the first door call.
fn open(dir: &Path, device: Device) -> Result<TvHistory, String> {
    if !device.launcher_installed {
        return Err(format!(
            "Error: Could not find provider: {}",
            URI.trim_start_matches("content://")
        ));
    }
    let mut tv = Tv {
        dir: dir.to_owned(),
        device,
    };
    let clock: Clock = read(&dir.join("clock.toml"))?;
    TvHistory::open(&dir.join("tv"), &mut tv, &clock, &|| new_id(dir))
        .map_err(|error| error.to_string())
}

/// `dumpsys package ai.rusttv.launcher`, trimmed to what RustTV reads.
fn dumpsys(device: &Device) -> String {
    if !device.launcher_installed {
        return "Unable to find package: ai.rusttv.launcher\n".to_owned();
    }
    format!(
        "Packages:\n  Package [{LAUNCHER}] (5e2f0d4):\n    versionName=0.1.0\n    flags=[ {}HAS_CODE \
         ALLOW_CLEAR_USER_DATA ]\n    install permissions:\n      {PERMISSION}: granted={}\n",
        if device.debuggable { "DEBUGGABLE " } else { "" },
        device.granted
    )
}

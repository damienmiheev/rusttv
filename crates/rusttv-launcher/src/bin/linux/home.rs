//! The home screen's binary on Linux; see `src/bin/linux.rs`.

use std::cell::RefCell;
use std::error::Error;
use std::io::{Read as _, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;

use rusttv_launcher::{
    AppEvent, DeviceSettings as _, Lifecycle, Limits, LinuxDevice, LinuxPlatform, RunOptions,
    VolumeChange, run, start_desktop_history,
};

/// Logs info, warnings and errors of the launcher to stderr (the session's
/// journal).
struct Stderr;

impl log::Log for Stderr {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info
    }

    #[expect(clippy::print_stderr, reason = "stderr is the launcher's log on Linux")]
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) && record.target().starts_with("rusttv") {
            eprintln!("{}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

thread_local! {
    /// The running home screen's lifecycle, for app events from other threads.
    static LIFECYCLE: RefCell<Option<Lifecycle>> = const { RefCell::new(None) };
}

/// The directory for the launcher's files.
fn state_dir() -> Result<PathBuf, Box<dyn Error>> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let base = match (var("XDG_STATE_HOME"), var("HOME")) {
        (Some(state), _) => PathBuf::from(state),
        (None, Some(home)) => PathBuf::from(home).join(".local/state"),
        (None, None) => return Err("neither XDG_STATE_HOME nor HOME is set".into()),
    };
    Ok(base.join("rusttv"))
}

/// A BCP 47 tag from `RUSTTV_LANG` or the POSIX locale (`de_DE.UTF-8` → `de-DE`).
fn language() -> Option<String> {
    let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    if let Some(tag) = var("RUSTTV_LANG") {
        return Some(tag);
    }
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| var(name))?;
    let base = locale.split(['.', '@']).next()?;
    (base != "C" && base != "POSIX").then(|| base.replace('_', "-"))
}

/// What happened, for the home screen on the UI thread.
#[derive(Debug, Clone)]
enum Signal {
    /// An app started or ended.
    App(AppEvent),
    /// Home was pressed (`--home`).
    Home,
    /// Home was held (`--app-menu`).
    AppMenu,
    /// A volume key (`--volume`).
    Volume(VolumeChange),
    /// The power key (`--power`).
    Power,
}

/// Hands `signal` to the home screen on the UI thread, where the lifecycle
/// lives.
fn deliver(signal: Signal) {
    let what = format!("{signal:?}");
    let delivered = slint::invoke_from_event_loop(move || {
        LIFECYCLE.with(|slot| {
            if let Some(lifecycle) = slot.borrow().as_ref() {
                match signal {
                    // The splash stays until the app's window takes the
                    // focus (the launcher then pauses itself).
                    Signal::App(AppEvent::Started(_)) => {}
                    Signal::App(AppEvent::Exited(id)) => lifecycle.app_ended(id),
                    // Like Android's Home: leave every page, then come back.
                    Signal::Home => lifecycle.home(),
                    Signal::AppMenu => lifecycle.app_menu(),
                    Signal::Volume(change) => lifecycle.volume(change),
                    Signal::Power => lifecycle.power_menu(),
                }
            }
        });
    });
    if let Err(error) = delivered {
        log::warn!("{what} not delivered: {error}");
    }
}

/// `$XDG_CONFIG_HOME/rusttv/linux.toml` (default `~/.config/...`): how many
/// apps may run.
///
/// ```toml
/// # Apps kept running besides the launcher; opening one more closes the
/// # least recently used.
/// max_apps = 2
/// # Close background apps while less memory than this (MiB) is available.
/// min_available_mib = 120
/// # Stop apps in the background (they keep their memory, take no CPU);
/// # also pauses music playing in them. Off unless set: a device with
/// # little CPU (the HY320F image) turns it on.
/// freeze_background = false
/// ```
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    max_apps: Option<usize>,
    min_available_mib: Option<u64>,
    freeze_background: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_apps: Some(3),
            min_available_mib: Some(120),
            freeze_background: false,
        }
    }
}

fn config() -> Config {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let Some(base) = var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")))
    else {
        return Config::default();
    };
    let path = base.join("rusttv/linux.toml");
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|error| {
            log::warn!("{}: {error}; using the defaults", path.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    }
}

/// The socket `--home` writes to, in `$XDG_RUNTIME_DIR`.
fn home_socket() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("rusttv-launcher.sock"))
}

/// Accepts `home` messages from `--home`, `power` from `--power` and
/// `volume <up|down|mute>` from `--volume` for as long as the launcher runs.
fn listen_for_home(path: PathBuf) {
    // A socket left by an earlier run would make bind fail.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            log::warn!("no Home socket at {}: {error}", path.display());
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name("rusttv-home".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut message = String::new();
                if stream.take(32).read_to_string(&mut message).is_err() {
                    continue;
                }
                let message = message.trim();
                if message == "home" {
                    deliver(Signal::Home);
                } else if message == "app-menu" {
                    deliver(Signal::AppMenu);
                } else if message == "power" {
                    deliver(Signal::Power);
                } else if let Some(change) = message
                    .strip_prefix("volume ")
                    .and_then(VolumeChange::parse)
                {
                    deliver(Signal::Volume(change));
                }
            }
        });
    if let Err(error) = spawned {
        log::warn!("no thread for the Home socket: {error}");
    }
}

/// `--home` / `--power`: tells the running launcher that Home or the power
/// key was pressed.
fn send_key(message: &str) -> Result<(), Box<dyn Error>> {
    let path = home_socket().ok_or("XDG_RUNTIME_DIR is not set")?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|error| format!("no launcher at {}: {error}", path.display()))?;
    stream.write_all(format!("{message}\n").as_bytes())?;
    Ok(())
}

/// `--volume up|down|mute`: the running launcher changes the volume and
/// shows its volume bar; without one, the volume changes all the same.
fn send_volume(change: &str) -> Result<(), Box<dyn Error>> {
    let parsed = VolumeChange::parse(change).ok_or(USAGE)?;
    let sent = home_socket()
        .and_then(|path| UnixStream::connect(path).ok())
        .is_some_and(|mut stream| {
            stream
                .write_all(format!("volume {change}\n").as_bytes())
                .is_ok()
        });
    if !sent && LinuxDevice::detect().volume(parsed).is_none() {
        return Err("no volume control (wpctl not found or failed)".into());
    }
    Ok(())
}

const USAGE: &str =
    "usage: rusttv-launcher-linux [--home | --app-menu | --power | --volume up|down|mute]";

pub(crate) fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => {}
        ["--home"] => return send_key("home"),
        ["--app-menu"] => return send_key("app-menu"),
        ["--power"] => return send_key("power"),
        ["--volume", change] => return send_volume(change),
        _ => return Err(USAGE.into()),
    }
    if log::set_logger(&Stderr).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let dir = state_dir()?;
    std::fs::create_dir_all(&dir)?;
    start_desktop_history(dir.clone(), true);

    let lifecycle = Lifecycle::default();
    LIFECYCLE.with(|slot| *slot.borrow_mut() = Some(lifecycle.clone()));
    if let Some(path) = home_socket() {
        listen_for_home(path);
    }
    let events = Arc::new(|event: AppEvent| deliver(Signal::App(event)));
    let config = config();
    let limits = Limits {
        max_apps: config.max_apps,
        min_available_mib: config.min_available_mib,
        freeze_background: config.freeze_background,
    };
    log::info!("app limits: {limits:?}");
    // The launcher's keyboard for text fields in other apps.
    rusttv_launcher::start_keyboard();
    // Game controllers steer the home screen like the remote.
    rusttv_launcher::start_gamepads();
    let platform = LinuxPlatform::from_env()
        .with_events(events)
        .with_tasks(limits)
        .with_sizes(dir.join("app-sizes.toml"));
    let language = language();
    let right_to_left = language
        .as_deref()
        .is_some_and(|tag| tag.starts_with("ar") || tag.starts_with("he") || tag.starts_with("iw"));
    let options = RunOptions {
        language,
        right_to_left,
        simulated_pause: None,
        order_file: Some(dir.join("order.toml")),
        lifecycle,
        remote_pairing: false,
        remote_port: None,
        no_key_hints: true,
    };
    Ok(run(platform, options)?)
}

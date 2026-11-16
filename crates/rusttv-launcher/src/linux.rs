//! Linux platform: the apps of the XDG desktop entries, started as child
//! processes (feature `linux`).
//!
//! Apps are the `Type=Application` entries under `applications/` in
//! `$XDG_DATA_HOME` and `$XDG_DATA_DIRS` (Desktop Entry Specification 1.5),
//! so packages and Flatpaks show up without extra work. Icons come from the
//! `hicolor` theme, `pixmaps`, or the Breeze and Adwaita themes (PNG, else
//! SVG). Updates are the system package
//! manager's job, so the launcher's updater does nothing here.

mod device;
mod keyboard;
mod tasks;
mod toplevels;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::BufReader;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::SystemTime;

pub use device::LinuxDevice;
pub use keyboard::start as start_keyboard;
pub use tasks::Limits;
use tasks::Tasks;
use toplevels::Toplevels;

use crate::device::{DeviceSettings, QuickPanel};
use crate::model::{App, Category, QuickAction};
use crate::platform::{AppEntry, Artwork, Bitmap, Platform, PlatformError, UpdateStatus};

/// What happened to an app the launcher started (by desktop file ID).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppEvent {
    /// The app's process started; its window follows.
    Started(String),
    /// The app's process ended.
    Exited(String),
}

/// Receives [`AppEvent`]s; `Exited` arrives on a waiter thread.
pub type AppEvents = Arc<dyn Fn(AppEvent) + Send + Sync>;

/// How long an app may take to show its window (Kodi on a slow TV box takes
/// about ten seconds); after that the launch counts as failed.
const LAUNCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Icon sizes to try, best first: big enough for a tile, small enough to keep
/// the decoded picture small.
const ICON_SIZES: [&str; 7] = [
    "128x128", "256x256", "96x96", "192x192", "512x512", "64x64", "48x48",
];

/// Where SVG app icons are, under a data dir: the hicolor fallback theme,
/// then the themes KDE and GNOME apps ship their icons in.
const SVG_ICON_DIRS: [&str; 4] = [
    "icons/hicolor/scalable/apps",
    "icons/breeze/apps/48",
    "icons/breeze-dark/apps/48",
    "icons/Adwaita/scalable/apps",
];

/// The longer side, in pixels, SVG icons are drawn at.
const SVG_PIXELS: f32 = 256.0;

/// KDE Connect's desktop entry, opened by its entry in the top-right menu.
const KDE_CONNECT: &str = "org.kde.kdeconnect.app.desktop";

/// Where packages put the launcher's licence notices.
const NOTICES: &str = "/usr/share/doc/rusttv-launcher/NOTICES";

/// [`Platform`] over the XDG desktop entries of this machine.
pub struct LinuxPlatform {
    data_dirs: Vec<PathBuf>,
    desktops: Vec<String>,
    language: Option<String>,
    events: Option<AppEvents>,
    entries: RefCell<HashMap<String, DesktopEntry>>,
    seen: RefCell<Vec<Option<SystemTime>>>,
    tasks: Option<Arc<Tasks>>,
    windows: Option<Toplevels>,
    /// Audio, networks and power through the system's tools.
    device: Arc<LinuxDevice>,
}

impl fmt::Debug for LinuxPlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinuxPlatform")
            .field("data_dirs", &self.data_dirs)
            .field("desktops", &self.desktops)
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

impl LinuxPlatform {
    /// Reads the XDG directories, `XDG_CURRENT_DESKTOP` and the message
    /// language from the environment.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
        let home = var("HOME").map(PathBuf::from);
        let data_home = var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|home| home.join(".local/share")));
        let data_dirs =
            var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        let mut dirs: Vec<PathBuf> = data_home.into_iter().collect();
        dirs.extend(
            data_dirs
                .split(':')
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from),
        );
        let desktops = var("XDG_CURRENT_DESKTOP")
            .map(|value| value.split(':').map(str::to_owned).collect())
            .unwrap_or_default();
        let language = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| var(name))
            .filter(|value| value != "C" && value != "POSIX");
        Self::new(dirs, desktops, language)
    }

    /// A platform over these data directories (most important first), desktop
    /// names and POSIX locale (e.g. `de_DE.UTF-8`).
    pub fn new(data_dirs: Vec<PathBuf>, desktops: Vec<String>, language: Option<String>) -> Self {
        Self {
            data_dirs,
            desktops,
            language,
            events: None,
            entries: RefCell::default(),
            seen: RefCell::default(),
            tasks: None,
            windows: None,
            device: Arc::new(LinuxDevice::detect()),
        }
    }

    /// Manages the apps it starts: opening a running app brings its window to
    /// the front instead of starting it again, and `limits` caps how many
    /// run. Needs a compositor with `wlr-foreign-toplevel-management` to find
    /// the windows.
    #[must_use]
    pub fn with_tasks(mut self, limits: Limits) -> Self {
        let tasks = Tasks::start(limits);
        let used = tasks.clone();
        self.windows = Toplevels::start(Box::new(move |app_id| used.activated(app_id)));
        self.tasks = Some(tasks);
        self
    }

    /// Reports app starts and exits to `events`, e.g. to pause and resume
    /// the home screen.
    #[must_use]
    pub fn with_events(mut self, events: AppEvents) -> Self {
        self.events = Some(events);
        self
    }

    fn application_dirs(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.data_dirs.iter().map(|dir| dir.join("applications"))
    }

    /// Modification times of the application directories: installing or
    /// removing a desktop entry changes its directory's.
    fn stamps(&self) -> Vec<Option<SystemTime>> {
        self.application_dirs()
            .map(|dir| fs::metadata(dir).and_then(|meta| meta.modified()).ok())
            .collect()
    }

    /// Every visible desktop entry by desktop file ID; an ID found in an
    /// earlier data directory hides later ones.
    fn scan(&self) -> HashMap<String, DesktopEntry> {
        let mut found = HashMap::new();
        let mut taken = HashSet::new();
        for dir in self.application_dirs() {
            let mut files = Vec::new();
            collect_desktop_files(&dir, &dir, &mut files);
            for (id, path) in files {
                if !taken.insert(id.clone()) {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(entry) = DesktopEntry::parse(&text, &path, self.language.as_deref())
                    && entry.is_visible(&self.desktops)
                {
                    found.insert(id, entry);
                }
            }
        }
        found
    }

    /// A PNG of the icon in the hicolor theme or `pixmaps`, else an SVG
    /// there or in the Breeze or Adwaita theme.
    fn find_icon(&self, icon: &str) -> Option<PathBuf> {
        if icon.contains('/') {
            let path = PathBuf::from(icon);
            return path.is_file().then_some(path);
        }
        let png = format!("{icon}.png");
        let svg = format!("{icon}.svg");
        let pngs = self.data_dirs.iter().flat_map(|dir| {
            ICON_SIZES
                .iter()
                .map(|size| dir.join("icons/hicolor").join(size).join("apps").join(&png))
                .chain(std::iter::once(dir.join("pixmaps").join(&png)))
        });
        let svgs = self.data_dirs.iter().flat_map(|dir| {
            SVG_ICON_DIRS
                .iter()
                .map(|place| dir.join(place).join(&svg))
                .chain(std::iter::once(dir.join("pixmaps").join(&svg)))
        });
        pngs.chain(svgs).find(|path| path.is_file())
    }
}

impl Platform for LinuxPlatform {
    fn list_apps(&self) -> Result<Vec<AppEntry>, PlatformError> {
        *self.seen.borrow_mut() = self.stamps();
        let entries = self.scan();
        let apps = entries
            .iter()
            .map(|(id, entry)| AppEntry {
                app: App {
                    id: id.clone(),
                    label: entry.name.clone(),
                    category: entry.category,
                },
                artwork: if entry.icon.is_some() {
                    Artwork::Later
                } else {
                    Artwork::Missing
                },
            })
            .collect();
        *self.entries.borrow_mut() = entries;
        Ok(apps)
    }

    /// Desktop apps such as Kodi can take many seconds on a small device.
    fn launch_timeout(&self) -> std::time::Duration {
        LAUNCH_TIMEOUT
    }

    fn launch(&mut self, id: &str) -> Result<(), PlatformError> {
        let entries = self.entries.borrow();
        let entry = entries
            .get(id)
            .ok_or_else(|| PlatformError::not_launchable(id))?;
        // Its window, if it runs but was not started by this launcher (e.g.
        // before a restart of the launcher).
        if self
            .tasks
            .as_ref()
            .is_some_and(|tasks| !tasks.is_running(id))
            && self
                .windows
                .as_ref()
                .is_some_and(|windows| windows.activate(&entry.window_keys(id)))
        {
            log::info!("{id} was already open; raised");
            return Ok(());
        }
        if let Some(keys) = self.tasks.as_ref().and_then(|tasks| tasks.reuse(id)) {
            // Already running: its window comes to the front. Without one
            // yet, it is still starting; the splash waits for it.
            let raised = self
                .windows
                .as_ref()
                .is_some_and(|windows| windows.activate(&keys));
            log::info!(
                "{id} is running; {}",
                if raised { "raised" } else { "no window yet" }
            );
            return Ok(());
        }
        let argv = entry.argv();
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| PlatformError::not_launchable(id))?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(dir) = &entry.working_dir {
            command.current_dir(dir);
        }
        // A process group of its own, so closing the app also ends the
        // processes it started (e.g. a wrapper script's real program).
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| PlatformError::call("launch", format!("{id}: {error}")))?;
        let group = child.id();
        log::info!("started {id} (pid {group})");
        if let Some(tasks) = &self.tasks {
            tasks.started(id, group, entry.window_keys(id));
        }
        if let Some(events) = &self.events {
            events(AppEvent::Started(id.to_owned()));
        }
        let events = self.events.clone();
        let tasks = self.tasks.clone();
        let name = id.to_owned();
        let waiter = std::thread::Builder::new()
            .name("rusttv-app-wait".into())
            .spawn(move || {
                match child.wait() {
                    Ok(status) => log::info!("{name} ended ({status})"),
                    Err(error) => log::warn!("waiting for {name} failed: {error}"),
                }
                if let Some(tasks) = tasks {
                    tasks.ended(group);
                }
                if let Some(events) = events {
                    events(AppEvent::Exited(name));
                }
            });
        if let Err(error) = waiter {
            log::warn!("no thread to wait for {id}: {error}");
        }
        Ok(())
    }

    fn take_apps_changed(&mut self) -> Result<bool, PlatformError> {
        let now = self.stamps();
        let changed = *self.seen.borrow() != now;
        *self.seen.borrow_mut() = now;
        Ok(changed)
    }

    fn artwork(&self, id: &str) -> Result<Artwork, PlatformError> {
        let entries = self.entries.borrow();
        let entry = entries
            .get(id)
            .ok_or_else(|| PlatformError::not_launchable(id))?;
        let Some(path) = entry.icon.as_deref().and_then(|icon| self.find_icon(icon)) else {
            return Ok(Artwork::Missing);
        };
        let decoded = if path.extension().is_some_and(|extension| extension == "svg") {
            decode_svg(&path)
        } else {
            decode_png(&path)
        };
        match decoded {
            Ok(bitmap) => Ok(Artwork::Icon(bitmap)),
            Err(error) => {
                log::warn!("icon {} of {id}: {error}", path.display());
                Ok(Artwork::Missing)
            }
        }
    }

    fn notices(&self) -> Result<String, PlatformError> {
        let path = std::env::var_os("RUSTTV_NOTICES")
            .map_or_else(|| PathBuf::from(NOTICES), PathBuf::from);
        fs::read_to_string(&path)
            .map_err(|error| PlatformError::call("notices", format!("{}: {error}", path.display())))
    }

    fn artwork_loaded(&self) {}

    fn update_status(&self) -> Result<UpdateStatus, PlatformError> {
        Ok(UpdateStatus::Idle)
    }

    /// Nothing to check: the system package manager updates the launcher.
    fn check_for_update(&mut self) -> Result<(), PlatformError> {
        Ok(())
    }

    fn install_update(&mut self) -> Result<(), PlatformError> {
        Err(PlatformError::call(
            "install_update",
            "the system package manager updates the launcher",
        ))
    }

    /// The quick panels whose tools are installed, and KDE Connect when
    /// its desktop entry is.
    fn quick_actions(&self) -> Vec<QuickAction> {
        let mut actions = Vec::new();
        for (action, panel) in [
            (QuickAction::Audio, QuickPanel::Audio),
            (QuickAction::Network, QuickPanel::Network),
            (QuickAction::Power, QuickPanel::Power),
        ] {
            if self.device.has(panel) {
                actions.push(action);
            }
        }
        if device::has_desktop_entry(&self.data_dirs, KDE_CONNECT) {
            actions.push(QuickAction::KdeConnect);
        }
        actions
    }

    fn quick_action_app(&self, action: QuickAction) -> Option<String> {
        (action == QuickAction::KdeConnect).then(|| KDE_CONNECT.to_owned())
    }

    fn device_settings(&self) -> Option<Arc<dyn DeviceSettings>> {
        Some(self.device.clone())
    }
}

/// Collects `(desktop file ID, path)` for every `.desktop` file under `dir`;
/// the ID is the path below `root` with `/` replaced by `-`.
fn collect_desktop_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_desktop_files(root, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "desktop")
            && let Ok(relative) = path.strip_prefix(root)
        {
            let id = relative.to_string_lossy().replace('/', "-");
            out.push((id, path));
        }
    }
}

/// The parts of a desktop entry the launcher uses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopEntry {
    name: String,
    exec: String,
    icon: Option<String>,
    category: Category,
    working_dir: Option<PathBuf>,
    path: PathBuf,
    hidden: bool,
    only_show_in: Vec<String>,
    not_show_in: Vec<String>,
    try_exec: Option<String>,
    wm_class: Option<String>,
}

impl DesktopEntry {
    /// Reads the `[Desktop Entry]` group; `None` unless it is an application
    /// that can be started from a launcher (no terminal apps).
    fn parse(text: &str, path: &Path, language: Option<&str>) -> Option<Self> {
        let mut values: HashMap<&str, &str> = HashMap::new();
        let mut in_entry = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_entry = line == "[Desktop Entry]";
                continue;
            }
            if in_entry && let Some((key, value)) = line.split_once('=') {
                values.entry(key.trim_end()).or_insert(value.trim_start());
            }
        }
        let get = |key: &str| values.get(key).map(|value| unescape(value));
        let flag = |key: &str| values.get(key).is_some_and(|value| *value == "true");
        if get("Type").as_deref() != Some("Application") || flag("Terminal") {
            return None;
        }
        let name = localized(&values, "Name", language)?;
        let exec = get("Exec").filter(|exec| !exec.is_empty())?;
        let list = |key: &str| -> Vec<String> {
            get(key)
                .map(|value| {
                    value
                        .split(';')
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Self {
            name,
            exec,
            icon: get("Icon").filter(|icon| !icon.is_empty()),
            category: category(&list("Categories")),
            working_dir: get("Path").filter(|dir| !dir.is_empty()).map(PathBuf::from),
            path: path.to_path_buf(),
            hidden: flag("Hidden") || flag("NoDisplay"),
            only_show_in: list("OnlyShowIn"),
            not_show_in: list("NotShowIn"),
            try_exec: get("TryExec").filter(|program| !program.is_empty()),
            wm_class: get("StartupWMClass").filter(|class| !class.is_empty()),
        })
    }

    /// Lower-case app IDs the app's windows may have: `StartupWMClass`, the
    /// desktop file ID without `.desktop` (and its last part for reverse-DNS
    /// IDs), and the program's name.
    fn window_keys(&self, id: &str) -> Vec<String> {
        let stem = id.strip_suffix(".desktop").unwrap_or(id);
        let program = split_exec(&self.exec)
            .first()
            .and_then(|program| Path::new(program).file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let mut keys: Vec<String> = [
            self.wm_class.clone(),
            Some(stem.to_owned()),
            stem.rsplit('.').next().map(str::to_owned),
            program,
        ]
        .into_iter()
        .flatten()
        .map(|key| key.to_lowercase())
        .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// Whether a launcher on these desktops shows the entry.
    fn is_visible(&self, desktops: &[String]) -> bool {
        let on = |list: &[String]| list.iter().any(|name| desktops.contains(name));
        !self.hidden
            && (self.only_show_in.is_empty() || on(&self.only_show_in))
            && !on(&self.not_show_in)
            && self.try_exec.as_deref().is_none_or(program_exists)
            && !self.is_launcher()
    }

    /// The launcher's own entry, which the home screen leaves out.
    fn is_launcher(&self) -> bool {
        split_exec(&self.exec)
            .first()
            .and_then(|program| Path::new(program).file_name())
            .is_some_and(|name| name.to_string_lossy().starts_with("rusttv-launcher"))
    }

    /// The command line, with field codes expanded for a start without files.
    fn argv(&self) -> Vec<String> {
        let mut argv = Vec::new();
        for arg in split_exec(&self.exec) {
            match arg.as_str() {
                "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                "%i" => {
                    if let Some(icon) = &self.icon {
                        argv.extend(["--icon".to_owned(), icon.clone()]);
                    }
                }
                "%c" => argv.push(self.name.clone()),
                "%k" => argv.push(self.path.to_string_lossy().into_owned()),
                _ => argv.push(expand_percent(&arg, self)),
            }
        }
        argv
    }
}

/// `%c`, `%k` and `%%` inside a longer argument; other codes are dropped.
fn expand_percent(arg: &str, entry: &DesktopEntry) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('c') => out.push_str(&entry.name),
            Some('k') => out.push_str(&entry.path.to_string_lossy()),
            _ => {}
        }
    }
    out
}

/// The value of `key` in the best matching locale: `lang_COUNTRY@MODIFIER`,
/// `lang_COUNTRY`, `lang@MODIFIER`, `lang`, then the plain key.
fn localized(values: &HashMap<&str, &str>, key: &str, language: Option<&str>) -> Option<String> {
    if let Some(language) = language {
        let locale = language.split('.').next().unwrap_or(language);
        let (base, modifier) = match locale.split_once('@') {
            Some((base, modifier)) => (base, Some(modifier)),
            None => (locale, None),
        };
        let lang = base.split('_').next().unwrap_or(base);
        let mut tries = Vec::new();
        if let Some(modifier) = modifier {
            tries.push(format!("{base}@{modifier}"));
        }
        tries.push(base.to_owned());
        if let Some(modifier) = modifier {
            tries.push(format!("{lang}@{modifier}"));
        }
        tries.push(lang.to_owned());
        for locale in tries {
            if let Some(value) = values.get(format!("{key}[{locale}]").as_str()) {
                return Some(unescape(value));
            }
        }
    }
    values.get(key).map(|value| unescape(value))
}

/// The launcher category for the first freedesktop main or additional
/// category that has one.
fn category(categories: &[String]) -> Category {
    categories
        .iter()
        .find_map(|name| match name.as_str() {
            "Game" => Some(Category::Game),
            "Audio" | "Music" => Some(Category::Audio),
            "Video" | "TV" | "AudioVideo" | "Player" => Some(Category::Video),
            "Graphics" | "Photography" | "Viewer" => Some(Category::Image),
            "Chat" | "InstantMessaging" | "VideoConference" => Some(Category::Social),
            "News" | "Feed" => Some(Category::News),
            "Maps" => Some(Category::Maps),
            "Office" => Some(Category::Productivity),
            "Accessibility" => Some(Category::Accessibility),
            _ => None,
        })
        .unwrap_or_default()
}

/// Undoes the string escapes of a desktop entry value (`\s`, `\n`, `\t`,
/// `\r`, `\\`).
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Splits an `Exec` value into arguments: spaces separate them, double
/// quotes group them, and inside quotes `\"`, `` \` ``, `\$` and `\\` stand
/// for the character.
fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// Whether `program` is an executable path or found in `PATH`.
fn program_exists(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Decodes a PNG file into an RGBA [`Bitmap`].
/// Draws an SVG icon with its longer side [`SVG_PIXELS`] long.
fn decode_svg(path: &Path) -> Result<Bitmap, String> {
    use resvg::{tiny_skia, usvg};

    let data = fs::read(path).map_err(|error| error.to_string())?;
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default())
        .map_err(|error| error.to_string())?;
    let size = tree.size();
    let scale = SVG_PIXELS / size.width().max(size.height());
    let side = |length: f32| (length * scale).ceil().clamp(1.0, SVG_PIXELS) as u32;
    let (width, height) = (side(size.width()), side(size.height()));
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or("empty icon")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia keeps premultiplied alpha; bitmaps are straight.
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    Bitmap::new(width, height, rgba).ok_or_else(|| "wrong pixel count".to_owned())
}

fn decode_png(path: &Path) -> Result<Bitmap, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "too large".to_owned())?;
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| error.to_string())?;
    pixels.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels,
        png::ColorType::Rgb => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[red, green, blue]| [red, green, blue, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[gray, alpha]| [gray, gray, gray, alpha])
            .collect(),
        png::ColorType::Grayscale => pixels
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::Indexed => return Err("indexed colours left after expansion".into()),
    };
    Bitmap::new(info.width, info.height, rgba).ok_or_else(|| "wrong pixel count".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> Option<DesktopEntry> {
        DesktopEntry::parse(
            text,
            Path::new("/usr/share/applications/test.desktop"),
            Some("de_DE.UTF-8"),
        )
    }

    #[test]
    fn an_svg_icon_is_drawn_256_pixels_wide_with_straight_alpha() {
        let path = std::env::temp_dir().join(format!("rusttv-icon-{}.svg", std::process::id()));
        fs::write(
            &path,
            "<svg xmlns='http://www.w3.org/2000/svg' width='48' height='24'>             <rect width='24' height='24' fill='#ff0000' fill-opacity='0.5'/></svg>",
        )
        .unwrap();
        let bitmap = decode_svg(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!((bitmap.width(), bitmap.height()), (256, 128));
        // Top left: half-transparent red, not premultiplied; top right: empty.
        let first = &bitmap.rgba()[..4];
        assert_eq!(first[0], 255);
        assert!((126..=129).contains(&first[3]), "{first:?}");
        assert_eq!(bitmap.rgba()[255 * 4 + 3], 0);
    }

    #[test]
    fn parses_an_application() {
        let parsed = entry(
            "[Desktop Entry]\nType=Application\nName=Player\nName[de]=Abspieler\n\
             Exec=mpv --player-operation-mode=pseudo-gui -- %U\nIcon=mpv\n\
             Categories=AudioVideo;Video;\n[Desktop Action new]\nName=Other\n",
        )
        .expect("an application");
        assert_eq!(parsed.name, "Abspieler");
        assert_eq!(parsed.category, Category::Video);
        assert_eq!(parsed.icon.as_deref(), Some("mpv"));
        assert_eq!(
            parsed.argv(),
            ["mpv", "--player-operation-mode=pseudo-gui", "--"]
        );
    }

    #[test]
    fn skips_terminal_apps_links_and_entries_without_exec() {
        assert!(
            entry("[Desktop Entry]\nType=Application\nName=Top\nExec=htop\nTerminal=true\n")
                .is_none()
        );
        assert!(
            entry("[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org\n").is_none()
        );
        assert!(entry("[Desktop Entry]\nType=Application\nName=Nothing\n").is_none());
    }

    #[test]
    fn honours_visibility_keys() {
        let visible = |text: &str, desktops: &[&str]| {
            let desktops: Vec<String> = desktops.iter().map(|name| (*name).to_owned()).collect();
            entry(text).is_some_and(|parsed| parsed.is_visible(&desktops))
        };
        let base = "[Desktop Entry]\nType=Application\nName=A\nExec=true\n";
        assert!(visible(base, &[]));
        assert!(!visible(&format!("{base}NoDisplay=true\n"), &[]));
        assert!(!visible(&format!("{base}OnlyShowIn=KDE;\n"), &["labwc"]));
        assert!(visible(
            &format!("{base}OnlyShowIn=KDE;labwc;\n"),
            &["labwc"]
        ));
        assert!(!visible(&format!("{base}NotShowIn=labwc;\n"), &["labwc"]));
        assert!(!visible(
            "[Desktop Entry]\nType=Application\nName=Me\nExec=/usr/bin/rusttv-launcher-linux\n",
            &[]
        ));
    }

    #[test]
    fn splits_and_expands_exec() {
        let parsed = entry(
            "[Desktop Entry]\nType=Application\nName=My App\nIcon=app\n\
             Exec=\"/opt/my app/run\" --title %c \"say \\\\\"hi\\\\\"\" %i 100%% %f\n",
        )
        .expect("an application");
        assert_eq!(
            parsed.argv(),
            [
                "/opt/my app/run",
                "--title",
                "My App",
                "say \"hi\"",
                "--icon",
                "app",
                "100%"
            ]
        );
    }

    #[test]
    fn maps_categories() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(category(&names(&["Game", "ArcadeGame"])), Category::Game);
        assert_eq!(category(&names(&["AudioVideo", "Audio"])), Category::Video);
        assert_eq!(
            category(&names(&["Network", "WebBrowser"])),
            Category::Other
        );
    }

    #[test]
    fn lists_apps_and_notices_changes() {
        let root = std::env::temp_dir().join(format!("rusttv-linux-{}", std::process::id()));
        let apps = root.join("applications");
        fs::create_dir_all(apps.join("kde")).expect("temp dir");
        fs::write(
            apps.join("one.desktop"),
            "[Desktop Entry]\nType=Application\nName=One\nExec=true\n",
        )
        .expect("write");
        fs::write(
            apps.join("kde/two.desktop"),
            "[Desktop Entry]\nType=Application\nName=Two\nExec=true\n",
        )
        .expect("write");
        let mut platform = LinuxPlatform::new(vec![root.clone()], Vec::new(), None);
        let mut ids: Vec<String> = platform
            .list_apps()
            .expect("list")
            .into_iter()
            .map(|entry| entry.app.id)
            .collect();
        ids.sort();
        assert_eq!(ids, ["kde-two.desktop", "one.desktop"]);
        assert!(!platform.take_apps_changed().expect("changed"));
        assert!(platform.launch("missing.desktop").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}

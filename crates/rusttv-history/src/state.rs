//! The snapshot of what RustTV manages on a TV (`state.toml`, HISTORY.md § 1).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::HistoryError;

/// Version of the snapshot format. Bump on any incompatible change.
///
/// 2 added `[launcher.folders]`, 3 `[launcher.phones]`, 4
/// `reduced_effects`, 5 tile names of Linux desktop entries
/// (`org.kde.dolphin.desktop`) and `remote`, 6 panel tiles' names
/// (`panel:history`, PANELS.md § 6), 7 built-in and panel tiles hidden or
/// in a folder (`hidden = ["about"]`), 8 `ambient_off` (the idle screen
/// turned off, PANELS.md § 9 Q6), 9 `safe_look` (the emergency Home in
/// place of the person's Home, PANEL_STUDIO.md § 10.4), 10 `[themes]`
/// (theme packages installed on the TV, SECURITY.md "Packages: staging
/// and install"). A snapshot is written in the oldest
/// format that holds it (no folders, no phones, full effects, only Android
/// names: format 1), so older readers keep reading it until it needs more.
pub const STATE_FORMAT: u32 = 10;

/// The longest folder name, in characters (the launcher's limit).
const FOLDER_NAME_MAX: usize = 40;

/// The longest phone name, in characters (RustTV Remote's limit).
const PHONE_NAME_MAX: usize = 40;

/// The allowlisted global settings, by name (the CLI's `SettingKey`).
pub const SETTING_KEYS: [&str; 3] = [
    "animator_duration_scale",
    "transition_animation_scale",
    "window_animation_scale",
];

/// What an unset animation scale means: Android's default, 1.
const SETTING_DEFAULT: &str = "1";

/// One snapshot. Parsed strictly: unknown fields are an error. Two
/// snapshots are equal when their contents are, whatever their `format`.
#[derive(Debug, Clone, Default, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    /// [`STATE_FORMAT`] when written.
    pub format: u32,
    /// The launcher's own state; only the TV changes it.
    #[serde(default)]
    pub launcher: Launcher,
    /// Allowlisted global settings, by name; a missing key is unset.
    #[serde(default)]
    pub settings: Settings,
    /// Catalog packages and their state, as the Mac last saw them.
    #[serde(default)]
    pub packages: BTreeMap<String, PackageState>,
    /// Theme packages installed on the TV, by id (format 10; only written
    /// when there is one). Recorded, not restored yet.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub themes: BTreeMap<String, Theme>,
    /// Facts the Mac observed; never restored.
    #[serde(default)]
    pub device: Device,
}

/// The launcher's own state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    /// Tile names in order ([`is_tile_name`]): apps, built-in tiles
    /// ([`BUILTIN_NAMES`]), panel tiles (`panel:<id>`), `folder:<number>`.
    #[serde(default)]
    pub order: Vec<String>,
    /// Folders by tile name (`folder:<number>`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub folders: BTreeMap<String, Folder>,
    /// Names of tiles kept off Home: apps' components, and built-in and
    /// panel tiles' names (format 7).
    #[serde(default)]
    pub hidden: BTreeSet<String>,
    /// Phones paired with RustTV Remote: id (32 hex digits) → name. Never
    /// their keys; a restore can only remove phones (REMOTE.md § 6).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub phones: BTreeMap<String, String>,
    /// Whether the launcher checks for updates by itself.
    #[serde(default = "yes")]
    pub update_check: bool,
    /// Whether the launcher draws fewer effects (no shadows, no backdrop)
    /// for a weak GPU.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reduced_effects: bool,
    /// Whether the idle screen is off: Home never dims to its clock after
    /// a while (format 8; on by default, so only written when off).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ambient_off: bool,
    /// Whether the safe look is on: the plain emergency Home shows in place
    /// of the person's Home (format 9; off by default, so only written when
    /// on; PANEL_STUDIO.md § 10.4).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub safe_look: bool,
}

impl Launcher {
    /// Every tile name it holds: the order, hidden tiles, folders and their
    /// tiles.
    fn tile_names(&self) -> impl Iterator<Item = &String> {
        self.order
            .iter()
            .chain(&self.hidden)
            .chain(self.folders.keys())
            .chain(self.folders.values().flat_map(|folder| &folder.apps))
    }
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            folders: BTreeMap::new(),
            hidden: BTreeSet::new(),
            phones: BTreeMap::new(),
            update_check: true,
            reduced_effects: false,
            ambient_off: false,
            safe_look: false,
        }
    }
}

/// An installed theme package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// `<major>.<minor>.<patch>`.
    pub version: String,
    /// The package's SHA-256, lowercase hex.
    pub sha256: String,
}

/// A folder on Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Folder {
    /// Its name, as the person sees it.
    pub name: String,
    /// Its tiles' names, in order: apps' components, and built-in and
    /// panel tiles' names (format 7).
    pub apps: Vec<String>,
}

fn yes() -> bool {
    true
}

/// The `[settings]` table: only `global` exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// `Settings.Global` keys from [`SETTING_KEYS`].
    #[serde(default)]
    pub global: BTreeMap<String, String>,
}

/// Whether a package is enabled for user 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageState {
    /// Enabled.
    Enabled,
    /// Disabled for user 0.
    Disabled,
}

impl fmt::Display for PackageState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        })
    }
}

/// Facts the Mac observed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// `ro.build.fingerprint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Whether a Home takeover is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub takeover: Option<bool>,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.launcher == other.launcher
            && self.settings == other.settings
            && self.packages == other.packages
            && self.themes == other.themes
            && self.device == other.device
    }
}

impl State {
    /// An empty snapshot in the current format.
    pub fn new() -> Self {
        Self {
            format: STATE_FORMAT,
            ..Self::default()
        }
    }

    /// Parses and validates `state.toml`.
    pub fn parse(text: &str) -> Result<Self, HistoryError> {
        // Peek at the format first, so a newer file says so instead of
        // failing on a field this version doesn't know.
        #[derive(Deserialize)]
        struct Format {
            format: u32,
        }
        let format = toml::from_str::<Format>(text)
            .map_err(|error| HistoryError::invalid(format!("state.toml: {error}")))?
            .format;
        if format > STATE_FORMAT {
            return Err(HistoryError::FormatNewer(format));
        }
        let state: Self = toml::from_str(text)
            .map_err(|error| HistoryError::invalid(format!("state.toml: {error}")))?;
        state.validate()?;
        Ok(state)
    }

    /// The `state.toml` text, in the oldest format that holds it. Only ever
    /// hashed as written, never re-made to compare (HISTORY.md § 1).
    pub fn to_toml(&self) -> Result<String, HistoryError> {
        let state = Self {
            format: self.needed_format(),
            ..self.clone()
        };
        state.validate()?;
        toml::to_string(&state).map_err(|error| HistoryError::invalid(error.to_string()))
    }

    /// The oldest format that can hold this snapshot.
    pub fn needed_format(&self) -> u32 {
        let launcher = &self.launcher;
        let off_home = launcher
            .hidden
            .iter()
            .chain(launcher.folders.values().flat_map(|folder| &folder.apps));
        if !self.themes.is_empty() {
            10
        } else if launcher.safe_look {
            9
        } else if launcher.ambient_off {
            8
        } else if off_home.into_iter().any(|name| is_builtin_name(name)) {
            7
        } else if launcher.tile_names().any(|name| is_panel_name(name)) {
            6
        } else if self.launcher.tile_names().any(needs_format_five) {
            5
        } else if self.launcher.reduced_effects {
            4
        } else if !self.launcher.phones.is_empty() {
            3
        } else if !self.launcher.folders.is_empty() {
            2
        } else {
            1
        }
    }

    /// Checks names and values against the allowlists.
    pub fn validate(&self) -> Result<(), HistoryError> {
        if !(self.needed_format()..=STATE_FORMAT).contains(&self.format) {
            return Err(HistoryError::invalid(format!(
                "state format {} (this build reads up to {STATE_FORMAT}; these contents need {})",
                self.format,
                self.needed_format()
            )));
        }
        for (key, value) in &self.settings.global {
            if !SETTING_KEYS.contains(&key.as_str()) {
                return Err(HistoryError::invalid(format!(
                    "setting {key} is not allowed"
                )));
            }
            if !is_setting_value(value) {
                return Err(HistoryError::invalid(format!(
                    "{key} = {value:?} is not a valid value"
                )));
            }
        }
        for name in self.packages.keys() {
            if !is_package_name(name) {
                return Err(HistoryError::invalid(format!(
                    "{name:?} is not a package name"
                )));
            }
        }
        for name in self.launcher.tile_names() {
            if !is_tile_name(name) {
                return Err(HistoryError::invalid(format!(
                    "{name:?} is not a tile name"
                )));
            }
        }
        for (key, folder) in &self.launcher.folders {
            if !key.starts_with("folder:") {
                return Err(HistoryError::invalid(format!("{key:?} is not a folder")));
            }
            if !is_folder_name(&folder.name) {
                return Err(HistoryError::invalid(format!(
                    "the name of {key} is not valid"
                )));
            }
        }
        for (id, name) in &self.launcher.phones {
            if id.len() != 32 || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return Err(HistoryError::invalid(format!("{id:?} is not a phone id")));
            }
            if !is_phone_name(name) {
                return Err(HistoryError::invalid(format!(
                    "the name of phone {id} is not valid"
                )));
            }
        }
        if self.themes.len() > crate::packages::MAX_PACKAGES {
            return Err(HistoryError::invalid("too many themes".to_owned()));
        }
        for (id, theme) in &self.themes {
            if rusttv_panels::PackageId::parse(id).is_err()
                || rusttv_panels::Version::parse(&theme.version).is_err()
                || !crate::door::is_hash(&theme.sha256)
            {
                return Err(HistoryError::invalid(format!(
                    "{id:?} is not a valid theme entry"
                )));
            }
        }
        if let Some(fingerprint) = &self.device.fingerprint
            && (fingerprint.len() > 256 || fingerprint.chars().any(char::is_control))
        {
            return Err(HistoryError::invalid("fingerprint is not valid".to_owned()));
        }
        Ok(())
    }

    /// Whether two snapshots describe the same state: an unset animation
    /// scale equals "1" (HISTORY.md § 1).
    pub fn same_as(&self, other: &Self) -> bool {
        let settings = |state: &Self| -> BTreeMap<&str, String> {
            SETTING_KEYS
                .iter()
                .map(|key| (*key, state.setting(key)))
                .collect()
        };
        self.launcher == other.launcher
            && settings(self) == settings(other)
            && self.packages == other.packages
            && self.themes == other.themes
            && self.device == other.device
    }

    /// A setting's value, an unset key read as its default.
    pub fn setting(&self, key: &str) -> String {
        self.settings
            .global
            .get(key)
            .map_or_else(|| SETTING_DEFAULT.to_owned(), |value| normalize(value))
    }
}

/// "1.0" and "1" are the same scale; compares numbers, not text.
fn normalize(value: &str) -> String {
    let trimmed = value.trim_end_matches('0').trim_end_matches('.');
    if value.contains('.') { trimmed } else { value }.to_owned()
}

/// The CLI's rule for an animation scale: `[0-9]{1,2}(\.[0-9]{1,3})?`.
pub fn is_setting_value(value: &str) -> bool {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let digits = |text: &str, min: usize, max: usize| {
        (min..=max).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    digits(whole, 1, 2) && (!value.contains('.') || digits(fraction, 1, 3))
}

/// An Android package name: dot-separated Java identifiers.
pub fn is_package_name(name: &str) -> bool {
    name.len() <= 255
        && name.split('.').count() >= 2
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// A folder's name: 1 to 40 characters, none of them control characters.
/// Shown only, never run.
pub fn is_folder_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= FOLDER_NAME_MAX
        && !name.chars().any(char::is_control)
}

/// A phone's name: 1 to 40 characters, none of them control characters.
/// Shown only, never run.
pub fn is_phone_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= PHONE_NAME_MAX
        && !name.chars().any(char::is_control)
}

/// Whether a tile name is one builds before format 5 refuse: a Linux
/// desktop entry's id or `remote` (see [`is_tile_name`]).
fn needs_format_five(name: &String) -> bool {
    !name.contains('/') && (name == "remote" || name.ends_with(".desktop"))
}

/// The built-in tiles' names (format 1, `remote` format 5). The launcher
/// never hides or folders `hidden-apps` (the Hidden apps tile), but a
/// hand-edited snapshot could, so it counts like the others.
const BUILTIN_NAMES: [&str; 4] = ["about", "settings", "hidden-apps", "remote"];

/// Whether a tile name is a built-in or panel tile's. Hidden or in a
/// folder, it needs format 7: builds before it accept the name there
/// ([`is_tile_name`]) but would show the tile anyway and drop it from
/// its folder.
fn is_builtin_name(name: &str) -> bool {
    BUILTIN_NAMES.contains(&name) || is_panel_name(name)
}

/// Whether a tile name is a panel tile's, `panel:<id>` (format 6; builds
/// before it would read it as an app and drop it). The id is 1 to 40
/// lowercase letters, digits and single '-' between them, so it never
/// holds a `/` and can't clash with an app.
fn is_panel_name(name: &str) -> bool {
    name.strip_prefix("panel:").is_some_and(|id| {
        (1..=40).contains(&id.len())
            && id.starts_with(|c: char| c.is_ascii_lowercase())
            && !id.ends_with('-')
            && !id.contains("--")
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    })
}

/// A tile name: `about`, `settings`, `hidden-apps`, `remote`,
/// `panel:<id>` (format 6, [`is_panel_name`]), `folder:<number>`, a Linux
/// desktop entry's id (`<id>.desktop`, format 5)
/// or a component `<package>/<class>`. Tile names are only shown, never run, so
/// the package part is checked by its characters (not
/// [`is_package_name`]'s stricter shape): one bad name must not stop the
/// whole history.
pub fn is_tile_name(name: &str) -> bool {
    match name.split_once('/') {
        None => {
            is_builtin_name(name)
                || name
                    .strip_prefix("folder:")
                    .is_some_and(|number| number.parse::<u32>().is_ok() && !number.starts_with('+'))
                // A Linux desktop entry's id (`org.kde.dolphin.desktop`).
                || name.strip_suffix(".desktop").is_some_and(|stem| {
                    !stem.is_empty()
                        && name.len() <= 255
                        && stem
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                })
        }
        Some((package, class)) => {
            !package.is_empty()
                && package.len() <= 255
                && package
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
                && !class.is_empty()
                && class.len() <= 255
                && class
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$'))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> State {
        let mut state = State::new();
        state.launcher.order = vec!["com.netflix.ninja/.MainActivity".into(), "about".into()];
        state.launcher.hidden.insert("com.example.app/.Main".into());
        state
            .settings
            .global
            .insert("animator_duration_scale".into(), "0.5".into());
        state
            .packages
            .insert("com.example.telemetry".into(), PackageState::Disabled);
        state.device.fingerprint =
            Some("Philips/PH1M_WW_9972/PH1M:14/UKN2/1:user/release-keys".into());
        state.device.takeover = Some(true);
        state
    }

    #[test]
    fn round_trips_through_toml() {
        let state = sample();
        let text = state.to_toml().unwrap();
        assert_eq!(State::parse(&text).unwrap(), state);
    }

    #[test]
    fn an_unset_scale_is_the_same_as_one() {
        let mut a = State::new();
        let mut b = State::new();
        b.settings
            .global
            .insert("window_animation_scale".into(), "1.0".into());
        assert!(a.same_as(&b));
        a.settings
            .global
            .insert("window_animation_scale".into(), "0.5".into());
        assert!(!a.same_as(&b));
    }

    #[test]
    fn refuses_keys_values_and_names_outside_the_allowlists() {
        for text in [
            "format = 1\n[settings.global]\nadb_enabled = \"1\"\n",
            "format = 1\n[settings.global]\nanimator_duration_scale = \"0.5; rm\"\n",
            "format = 1\n[packages]\n\"com.example; reboot\" = \"disabled\"\n",
            "format = 1\n[launcher]\norder = [\"../../etc\"]\n",
            "format = 1\n[launcher]\norder = [\"a/b\\nc\"]\n",
            "format = 1\nextra = 1\n",
        ] {
            assert!(
                matches!(State::parse(text), Err(HistoryError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn a_newer_format_says_so() {
        assert!(matches!(
            State::parse("format = 11\nsomething_new = true\n"),
            Err(HistoryError::FormatNewer(11))
        ));
    }

    #[test]
    fn folders_need_format_two_and_others_stay_at_one() {
        let state = sample();
        assert!(state.to_toml().unwrap().starts_with("format = 1\n"));

        let mut with_folder = sample();
        with_folder.launcher.order.push("folder:1".into());
        with_folder.launcher.folders.insert(
            "folder:1".into(),
            Folder {
                name: "Ігри \"2\"".into(),
                apps: vec!["com.example.game/.Main".into()],
            },
        );
        let text = with_folder.to_toml().unwrap();
        assert!(text.starts_with("format = 2\n"), "{text}");
        let parsed = State::parse(&text).unwrap();
        assert_eq!(parsed.launcher, with_folder.launcher);
        // Format 1 can't say it has folders.
        let downgraded = text.replacen("format = 2", "format = 1", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
    }

    #[test]
    fn reduced_effects_need_format_four() {
        let mut reduced = sample();
        reduced.launcher.reduced_effects = true;
        let text = reduced.to_toml().unwrap();
        assert!(
            text.starts_with(
                "format = 4
"
            ),
            "{text}"
        );
        assert!(text.contains("reduced_effects = true"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, reduced.launcher);
        let downgraded = text.replacen("format = 4", "format = 3", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        let full = sample().to_toml().unwrap();
        assert!(!full.contains("reduced_effects"), "{full}");
    }

    #[test]
    fn desktop_entries_need_format_five() {
        let mut linux = sample();
        linux.launcher.order.push("org.kde.dolphin.desktop".into());
        let text = linux.to_toml().unwrap();
        assert!(text.starts_with("format = 5\n"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, linux.launcher);
        let downgraded = text.replacen("format = 5", "format = 4", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        // Also in a folder or hidden; Android names stay at format 1.
        let mut hidden = sample();
        hidden.launcher.hidden.insert("foot.desktop".into());
        assert_eq!(hidden.needed_format(), 5);
        // So does the RustTV Remote tile, which every Home has.
        let mut remote = sample();
        remote.launcher.order.push("remote".into());
        assert_eq!(remote.needed_format(), 5);
        let downgraded = remote
            .to_toml()
            .unwrap()
            .replacen("format = 5", "format = 4", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        assert_eq!(sample().needed_format(), 1);
        assert!(sample().to_toml().unwrap().starts_with("format = 1\n"));
    }

    #[test]
    fn panel_tiles_need_format_six() {
        let mut history = sample();
        history.launcher.order.push("panel:history".into());
        let text = history.to_toml().unwrap();
        assert!(text.starts_with("format = 6\n"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, history.launcher);
        // A build that reads up to format 5 must refuse it, not read the
        // panel as an app it then drops.
        let downgraded = text.replacen("format = 6", "format = 5", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        // Also with every older format's contents.
        let mut all = history.clone();
        all.launcher.order.push("remote".into());
        all.launcher.reduced_effects = true;
        assert_eq!(all.needed_format(), 6);
        assert!(is_tile_name("panel:history"));
        assert!(is_tile_name("panel:a1-b"));
        for bad in [
            "panel:",
            "panel:History",
            "panel:-a",
            "panel:a-",
            "panel:a--b",
            "panel:1a",
            "panel:a/b",
            "panel:a b",
        ] {
            assert!(!is_tile_name(bad), "{bad}");
        }
        assert!(is_tile_name(&format!("panel:{}", "a".repeat(40))));
        assert!(!is_tile_name(&format!("panel:{}", "a".repeat(41))));
    }

    /// A built-in or panel tile hidden or in a folder needs format 7:
    /// builds that read up to 6 would take the name there and still show
    /// the tile on Home (and drop it from the folder), so they must refuse.
    #[test]
    fn built_in_tiles_hidden_or_in_folders_need_format_seven() {
        // An Android Home: every built-in tile in the order, an app hidden.
        let home = |order: &[&str]| {
            let mut state = sample();
            state.launcher.order = order.iter().map(|name| (*name).to_owned()).collect();
            state
        };
        let all = [
            "com.example.a/.Main",
            "about",
            "settings",
            "remote",
            "panel:history",
        ];
        let normal = home(&all);
        assert_eq!(normal.needed_format(), 6, "built-in tiles in the order");
        assert!(normal.to_toml().unwrap().starts_with("format = 6\n"));
        for name in [
            "about",
            "settings",
            "remote",
            "panel:history",
            "hidden-apps",
        ] {
            let others: Vec<&str> = all.iter().copied().filter(|tile| *tile != name).collect();
            let mut hidden = home(&others);
            hidden.launcher.hidden.insert(name.into());
            let mut in_folder = home(&[others.as_slice(), &["folder:1"]].concat());
            in_folder.launcher.folders.insert(
                "folder:1".into(),
                Folder {
                    name: "RustTV".into(),
                    apps: vec![name.into(), "com.example.b/.Main".into()],
                },
            );
            for state in [hidden, in_folder] {
                assert_eq!(state.needed_format(), 7, "{name}");
                let text = state.to_toml().unwrap();
                assert!(text.starts_with("format = 7\n"), "{text}");
                assert_eq!(State::parse(&text).unwrap().launcher, state.launcher);
                let downgraded = text.replacen("format = 7", "format = 6", 1);
                assert!(
                    matches!(State::parse(&downgraded), Err(HistoryError::Invalid(_))),
                    "{name}"
                );
            }
        }
    }

    /// An installed theme needs format 10; none is never written.
    #[test]
    fn installed_themes_need_format_ten() {
        let mut state = sample();
        state.themes.insert(
            "user.dusk".to_owned(),
            Theme {
                version: "1.2.0".to_owned(),
                sha256: "a".repeat(64),
            },
        );
        let text = state.to_toml().unwrap();
        assert!(text.starts_with("format = 10\n"), "{text}");
        assert_eq!(State::parse(&text).unwrap(), state);
        let downgraded = text.replacen("format = 10", "format = 9", 1);
        assert!(State::parse(&downgraded).is_err());
        for bad in ["[themes.\"dusk\"]", "[themes.\"user.Dusk\"]"] {
            let broken = text.replacen("[themes.\"user.dusk\"]", bad, 1);
            assert!(State::parse(&broken).is_err(), "{broken}");
        }
        assert!(!sample().to_toml().unwrap().contains("themes"));
    }

    /// The safe look on needs format 9; off (the default) is never written.
    #[test]
    fn the_safe_look_on_needs_format_nine() {
        let mut on = sample();
        on.launcher.safe_look = true;
        assert_eq!(on.needed_format(), 9);
        let text = on.to_toml().unwrap();
        assert!(text.starts_with("format = 9\n"), "{text}");
        assert!(text.contains("safe_look = true"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, on.launcher);
        let downgraded = text.replacen("format = 9", "format = 8", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        assert!(!sample().to_toml().unwrap().contains("safe_look"));
    }

    /// The idle screen turned off needs format 8; on (the default) is never
    /// written, so a snapshot with it on stays in its older format.
    #[test]
    fn the_idle_screen_off_needs_format_eight() {
        let mut off = sample();
        off.launcher.ambient_off = true;
        assert_eq!(off.needed_format(), 8);
        let text = off.to_toml().unwrap();
        assert!(text.starts_with("format = 8\n"), "{text}");
        assert!(text.contains("ambient_off = true"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, off.launcher);
        // A build that reads up to format 7 must refuse it, not turn the
        // idle screen back on when it writes the snapshot again.
        let downgraded = text.replacen("format = 8", "format = 7", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        let on = sample().to_toml().unwrap();
        assert!(!on.contains("ambient_off"), "{on}");
        assert!(on.starts_with("format = 1\n"), "{on}");
    }

    /// Snapshots written by older builds still load in this one.
    #[test]
    fn older_snapshots_still_load() {
        for text in [
            "format = 1\n[launcher]\norder = [\"a.b/.M\", \"about\", \"settings\"]\nhidden = [\"c.d/.M\"]\n",
            "format = 5\n[launcher]\norder = [\"a.b/.M\", \"about\", \"settings\", \"remote\"]\n",
            "format = 6\n[launcher]\norder = [\"a.b/.M\", \"remote\", \"panel:history\"]\n\n[launcher.folders.\"folder:1\"]\nname = \"V\"\napps = [\"e.f/.M\"]\n",
            "format = 7\n[launcher]\norder = [\"a.b/.M\", \"remote\"]\nhidden = [\"panel:history\"]\n",
        ] {
            let state = State::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"));
            assert!(state.needed_format() <= state.format, "{text}");
        }
    }

    #[test]
    fn phones_need_format_three() {
        let mut with_phone = sample();
        with_phone
            .launcher
            .phones
            .insert("ab".repeat(16), "iPhone (Safari)".into());
        let text = with_phone.to_toml().unwrap();
        assert!(text.starts_with("format = 3\n"), "{text}");
        assert_eq!(State::parse(&text).unwrap().launcher, with_phone.launcher);
        let downgraded = text.replacen("format = 3", "format = 2", 1);
        assert!(matches!(
            State::parse(&downgraded),
            Err(HistoryError::Invalid(_))
        ));
        for text in [
            "format = 3\n[launcher.phones]\nabc = \"x\"\n",
            "format = 3\n[launcher.phones]\nABABABABABABABABABABABABABABABAB = \"x\"\n",
            "format = 3\n[launcher.phones]\nabababababababababababababababab = \"\"\n",
            "format = 3\n[launcher.phones]\nabababababababababababababababab = \"a\\nb\"\n",
        ] {
            assert!(
                matches!(State::parse(text), Err(HistoryError::Invalid(_))),
                "{text}"
            );
        }
    }

    #[test]
    fn folder_names_and_keys_are_checked() {
        for text in [
            "format = 2\n[launcher.folders.\"folder:1\"]\nname = \"\"\napps = []\n",
            "format = 2\n[launcher.folders.\"folder:1\"]\nname = \"a\\nb\"\napps = []\n",
            "format = 2\n[launcher.folders.\"folder:x\"]\nname = \"a\"\napps = []\n",
            "format = 2\n[launcher.folders.about]\nname = \"a\"\napps = []\n",
            "format = 2\n[launcher.folders.\"folder:1\"]\nname = \"a\"\napps = [\"../../x\"]\n",
        ] {
            assert!(
                matches!(State::parse(text), Err(HistoryError::Invalid(_))),
                "{text}"
            );
        }
        assert!(is_folder_name(&"я".repeat(40)));
        assert!(!is_folder_name(&"я".repeat(41)));
        assert!(is_tile_name("folder:12"));
        assert!(!is_tile_name("folder:+1"));
        assert!(!is_tile_name("folder:"));
        assert!(is_tile_name("remote"));
        assert!(is_tile_name("foot.desktop"));
        assert!(is_tile_name("org.kde.dolphin.desktop"));
        assert!(is_tile_name("vlc-player.desktop"));
        assert!(!is_tile_name(".desktop"));
        assert!(!is_tile_name("a b.desktop"));
    }

    #[test]
    fn setting_values_follow_the_cli_rule() {
        for ok in ["0", "1", "0.5", "10", "1.25", "99.999"] {
            assert!(is_setting_value(ok), "{ok}");
        }
        for bad in ["", ".5", "100", "1.", "1.2345", "-1", "1e3", " 1"] {
            assert!(!is_setting_value(bad), "{bad}");
        }
    }
}

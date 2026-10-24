//! The snapshot of what RustTV manages on a TV (`state.toml`, HISTORY.md § 1).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::HistoryError;

/// Version of the snapshot format. Bump on any incompatible change.
pub const STATE_FORMAT: u32 = 1;

/// The allowlisted global settings, by name (the CLI's `SettingKey`).
pub const SETTING_KEYS: [&str; 3] = [
    "animator_duration_scale",
    "transition_animation_scale",
    "window_animation_scale",
];

/// What an unset animation scale means: Android's default, 1.
const SETTING_DEFAULT: &str = "1";

/// One snapshot. Parsed strictly: unknown fields are an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Facts the Mac observed; never restored.
    #[serde(default)]
    pub device: Device,
}

/// The launcher's own state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    /// Tile names in order: app component names, `about`, `settings`.
    #[serde(default)]
    pub order: Vec<String>,
    /// Component names of apps kept off Home.
    #[serde(default)]
    pub hidden: BTreeSet<String>,
    /// Whether the launcher checks for updates by itself.
    #[serde(default = "yes")]
    pub update_check: bool,
}

impl Default for Launcher {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            hidden: BTreeSet::new(),
            update_check: true,
        }
    }
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

    /// The `state.toml` text. Only ever hashed as written, never re-made to
    /// compare (HISTORY.md § 1).
    pub fn to_toml(&self) -> Result<String, HistoryError> {
        self.validate()?;
        toml::to_string(self).map_err(|error| HistoryError::invalid(error.to_string()))
    }

    /// Checks names and values against the allowlists.
    pub fn validate(&self) -> Result<(), HistoryError> {
        if self.format != STATE_FORMAT {
            return Err(HistoryError::invalid(format!(
                "state format {} (this build reads {STATE_FORMAT})",
                self.format
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
        for name in self.launcher.order.iter().chain(&self.launcher.hidden) {
            if !is_tile_name(name) {
                return Err(HistoryError::invalid(format!(
                    "{name:?} is not a tile name"
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

/// A tile name: `about`, `settings`, `hidden-apps`, or a component
/// `<package>/<class>`. Tile names are only shown, never run, so the package
/// part is checked by its characters (not [`is_package_name`]'s stricter
/// shape): one bad name must not stop the whole history.
pub fn is_tile_name(name: &str) -> bool {
    match name.split_once('/') {
        None => matches!(name, "about" | "settings" | "hidden-apps"),
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
            State::parse("format = 2\nsomething_new = true\n"),
            Err(HistoryError::FormatNewer(2))
        ));
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

//! Typed device commands: the only way RustTV can reach a device.
//!
//! There is deliberately no "run this shell string" command. Every command is
//! a [`DeviceCommand`] variant with a fixed argument list and a [`RiskClass`]
//! (SECURITY.md § Risk classes).

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::launcher::RUSTTV_LAUNCHER;

/// How much a command can change on the device.
///
/// Ordered from least to most risky, so policies can compare with `<=`.
/// There is no variant for forbidden operations (R4): those have no command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RiskClass {
    /// R0: reads state, changes nothing.
    Read,
    /// R1: changes runtime state only; an app restart or reboot undoes it.
    Transient,
    /// R2: persistent but trivially reversible setting.
    Setting,
    /// R3: enables or disables a package for the user.
    PackageState,
}

impl RiskClass {
    /// Short code used in SECURITY.md and JSON output: `R0` to `R3`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Read => "R0",
            Self::Transient => "R1",
            Self::Setting => "R2",
            Self::PackageState => "R3",
        }
    }
}

impl fmt::Display for RiskClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "R0 (read-only)",
            Self::Transient => "R1 (transient)",
            Self::Setting => "R2 (setting)",
            Self::PackageState => "R3 (package state)",
        })
    }
}

/// See [`DeviceCommand::timeout`]. A cold launcher start takes well under
/// 2 s even on a slow TV; 30 s only catches a start that will never finish.
const START_ACTIVITY_TIMEOUT: Duration = Duration::from_secs(30);

/// A command RustTV may run on a device.
///
/// Serialized (in plans and journals) as `{kind = "...", arg = "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "arg", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeviceCommand {
    /// All system properties (`getprop`).
    GetProperties,
    /// All installed packages with their APK paths (`pm list packages -f`).
    ListPackages,
    /// Packages not part of the system image (`pm list packages -3`).
    ListThirdPartyPackages,
    /// Disabled packages (`pm list packages -d`).
    ListDisabledPackages,
    /// System-wide memory counters (`cat /proc/meminfo`).
    MemInfo,
    /// Size and free space of the user data partition (`df -k /data`).
    DataStorage,
    /// Cumulative CPU time counters (`cat /proc/stat`).
    CpuStat,
    /// All processes with their resident memory (`ps -A -o PID,RSS,NAME`).
    ProcessList,
    /// The activity that handles the Home button (`cmd package resolve-activity ...`).
    HomeActivity,
    /// Every activity that can handle Home, with priorities (`cmd package query-activities ...`).
    HomeCandidates,
    /// Memory usage of one app process (`dumpsys meminfo <package>`).
    PackageMemInfo(PackageName),
    /// One package's state, including its `versionName` (`dumpsys package
    /// <package>`); prints no package section when it isn't installed.
    PackageInfo(PackageName),
    /// Cumulative CPU time of every process (`ps -A -o PID,TIME+,NAME`).
    ProcessCpuTimes,
    /// Stops an app's processes (`am force-stop <package>`). Transient.
    ForceStop(PackageName),
    /// Starts an activity and waits for its first frame (`am start -W -n <component>`). Transient.
    StartActivity(Component),
    /// Activity manager state, including the resumed activity (`dumpsys activity activities`).
    Activities,
    /// Presses the Home button (`input keyevent KEYCODE_HOME`). Transient.
    PressHome,
    /// Power manager state, including whether the screen is awake or showing
    /// the screensaver (`dumpsys power`).
    PowerState,
    /// Wakes the screen and ends the screensaver, as any key press would
    /// (`input keyevent KEYCODE_WAKEUP`). Transient.
    WakeUp,
    /// Every input method, enabled or not, one id per line (`ime list -a -s`).
    InputMethods,
    /// Enabled accessibility services, `:`-separated components or `null`
    /// (`settings get secure enabled_accessibility_services`).
    EnabledAccessibilityServices,
    /// TV input state, including which TV input services the system has
    /// bound (`dumpsys tv_input`).
    TvInputs,
    /// One allowlisted global setting (`settings get global <key>`); prints
    /// `null` when the key is unset.
    GetSetting(SettingKey),
    /// Writes one allowlisted global setting (OPTIMIZER.md § 4): `to` a value
    /// is `settings put global <key> <value>`, `to` unset is `settings delete
    /// global <key>`. `from` is the state read when planning, so the command
    /// carries its own inverse. Setting (R2); only optimizer plans use it.
    SetSetting {
        /// The setting.
        key: SettingKey,
        /// The state this command leaves.
        to: SettingState,
        /// The state before, which the inverse brings back.
        from: SettingState,
    },
    /// Disables a package for user 0 (`pm disable-user --user 0 <package>`).
    /// Package state; only Home takeover and privacy plans use it (SECURITY.md).
    DisablePackage(PackageName),
    /// Re-enables a package for user 0 (`pm enable --user 0 <package>`). Package state.
    EnablePackage(PackageName),
    /// Installs the RustTV launcher from a local APK, which must contain
    /// package `ai.rusttv.launcher` (`adb install --pkg ai.rusttv.launcher
    /// <apk>`). A host-side adb command, not `adb shell`. Package state.
    InstallLauncher(ApkPath),
    /// Replaces an installed RustTV launcher, keeping its data
    /// (`adb install -r --pkg ai.rusttv.launcher <apk>`). Package state;
    /// can't be undone (see [`DeviceCommand::is_irreversible`]).
    UpgradeLauncher(ApkPath),
    /// Uninstalls the RustTV launcher and its data (`pm uninstall
    /// ai.rusttv.launcher`). Takes no package: it can remove nothing else.
    /// Package state; can't be undone (see [`DeviceCommand::is_irreversible`]).
    UninstallLauncher,
}

impl DeviceCommand {
    /// How risky this command is.
    pub fn risk(&self) -> RiskClass {
        match self {
            Self::GetProperties
            | Self::ListPackages
            | Self::ListThirdPartyPackages
            | Self::ListDisabledPackages
            | Self::MemInfo
            | Self::DataStorage
            | Self::CpuStat
            | Self::ProcessList
            | Self::HomeActivity
            | Self::HomeCandidates
            | Self::PackageMemInfo(_)
            | Self::PackageInfo(_)
            | Self::ProcessCpuTimes
            | Self::Activities
            | Self::PowerState
            | Self::InputMethods
            | Self::EnabledAccessibilityServices
            | Self::TvInputs
            | Self::GetSetting(_) => RiskClass::Read,
            Self::ForceStop(_) | Self::StartActivity(_) | Self::PressHome | Self::WakeUp => {
                RiskClass::Transient
            }
            Self::SetSetting { .. } => RiskClass::Setting,
            Self::DisablePackage(_)
            | Self::EnablePackage(_)
            | Self::InstallLauncher(_)
            | Self::UpgradeLauncher(_)
            | Self::UninstallLauncher => RiskClass::PackageState,
        }
    }

    /// Commands above R1 that have no inverse on purpose: they replace or
    /// remove RustTV's own launcher, and putting the previous version back
    /// needs an APK RustTV doesn't have. Plans allow them only as their last
    /// step and say so in the dry run (`crate::plan::Plan::new`).
    pub fn is_irreversible(&self) -> bool {
        matches!(self, Self::UpgradeLauncher(_) | Self::UninstallLauncher)
    }

    /// The command that undoes this one, if there is one.
    ///
    /// Every R2/R3 command must have an inverse: plans refuse steps that can't
    /// be undone (SECURITY.md § Change lifecycle).
    pub fn inverse(&self) -> Option<Self> {
        match self {
            Self::DisablePackage(package) => Some(Self::EnablePackage(package.clone())),
            Self::EnablePackage(package) => Some(Self::DisablePackage(package.clone())),
            Self::InstallLauncher(_) => Some(Self::UninstallLauncher),
            Self::SetSetting { key, to, from } => Some(Self::SetSetting {
                key: *key,
                to: from.clone(),
                from: to.clone(),
            }),
            _ => None,
        }
    }

    /// How long [`crate::SystemAdb`] lets this command run before stopping
    /// it, for commands that wait on the device's screen.
    ///
    /// `am start -W` waits for the activity's first frame, which never comes
    /// while the screensaver covers it.
    pub fn timeout(&self) -> Option<Duration> {
        match self {
            Self::StartActivity(_) => Some(START_ACTIVITY_TIMEOUT),
            _ => None,
        }
    }

    /// Arguments passed to `adb` after the `-s <serial>` device selector.
    ///
    /// All start with `shell` except the launcher installs, which run
    /// `adb install` on this computer with a validated [`ApkPath`].
    pub fn adb_args(&self) -> Vec<&str> {
        match self {
            Self::GetProperties => vec!["shell", "getprop"],
            Self::ListPackages => vec!["shell", "pm", "list", "packages", "-f"],
            Self::ListThirdPartyPackages => vec!["shell", "pm", "list", "packages", "-3"],
            Self::ListDisabledPackages => vec!["shell", "pm", "list", "packages", "-d"],
            Self::MemInfo => vec!["shell", "cat", "/proc/meminfo"],
            Self::DataStorage => vec!["shell", "df", "-k", "/data"],
            Self::CpuStat => vec!["shell", "cat", "/proc/stat"],
            Self::ProcessList => vec!["shell", "ps", "-A", "-o", "PID,RSS,NAME"],
            Self::HomeActivity => vec![
                "shell",
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.HOME",
            ],
            Self::HomeCandidates => vec![
                "shell",
                "cmd",
                "package",
                "query-activities",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.HOME",
            ],
            Self::PackageMemInfo(package) => vec!["shell", "dumpsys", "meminfo", package.as_str()],
            Self::PackageInfo(package) => vec!["shell", "dumpsys", "package", package.as_str()],
            Self::ProcessCpuTimes => vec!["shell", "ps", "-A", "-o", "PID,TIME+,NAME"],
            Self::ForceStop(package) => vec!["shell", "am", "force-stop", package.as_str()],
            Self::StartActivity(component) => {
                vec!["shell", "am", "start", "-W", "-n", component.as_str()]
            }
            Self::Activities => vec!["shell", "dumpsys", "activity", "activities"],
            Self::PressHome => vec!["shell", "input", "keyevent", "KEYCODE_HOME"],
            Self::PowerState => vec!["shell", "dumpsys", "power"],
            Self::WakeUp => vec!["shell", "input", "keyevent", "KEYCODE_WAKEUP"],
            Self::InputMethods => vec!["shell", "ime", "list", "-a", "-s"],
            Self::EnabledAccessibilityServices => vec![
                "shell",
                "settings",
                "get",
                "secure",
                "enabled_accessibility_services",
            ],
            Self::TvInputs => vec!["shell", "dumpsys", "tv_input"],
            Self::GetSetting(key) => vec!["shell", "settings", "get", "global", key.as_str()],
            Self::SetSetting {
                key,
                to: SettingState::Value(value),
                ..
            } => vec![
                "shell",
                "settings",
                "put",
                "global",
                key.as_str(),
                value.as_str(),
            ],
            Self::SetSetting {
                key,
                to: SettingState::Unset,
                ..
            } => vec!["shell", "settings", "delete", "global", key.as_str()],
            Self::DisablePackage(package) => {
                vec![
                    "shell",
                    "pm",
                    "disable-user",
                    "--user",
                    "0",
                    package.as_str(),
                ]
            }
            Self::EnablePackage(package) => {
                vec!["shell", "pm", "enable", "--user", "0", package.as_str()]
            }
            // `--pkg` makes Android refuse an APK whose package is not RustTV's.
            Self::InstallLauncher(apk) => vec!["install", "--pkg", RUSTTV_LAUNCHER, apk.as_str()],
            Self::UpgradeLauncher(apk) => {
                vec!["install", "-r", "--pkg", RUSTTV_LAUNCHER, apk.as_str()]
            }
            Self::UninstallLauncher => vec!["shell", "pm", "uninstall", RUSTTV_LAUNCHER],
        }
    }
}

/// A validated Android package name, safe to pass to `adb shell`.
///
/// `adb shell` joins its arguments into one string that the device's shell
/// interprets, so any value we pass must not contain shell syntax. Android
/// package names are dot-separated segments of ASCII letters, digits and `_`,
/// each starting with a letter; that rules out every shell metacharacter.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PackageName(String);

impl PackageName {
    /// Validates `name` against Android's package name rules.
    pub fn new(name: &str) -> Result<Self, InvalidPackageName> {
        let valid_segment = |segment: &str| {
            let mut chars = segment.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic())
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        };
        // Android requires at least two segments, e.g. `com.example`.
        if name.contains('.') && name.split('.').all(valid_segment) {
            Ok(Self(name.to_owned()))
        } else {
            Err(InvalidPackageName(name.to_owned()))
        }
    }

    /// The package name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `ai.rusttv.launcher`. Valid by construction; a test checks it.
    pub(crate) fn rusttv_launcher() -> Self {
        Self(RUSTTV_LAUNCHER.to_owned())
    }
}

impl TryFrom<String> for PackageName {
    type Error = InvalidPackageName;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::new(&name)
    }
}

impl From<PackageName> for String {
    fn from(package: PackageName) -> Self {
        package.0
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returned by [`PackageName::new`] for a string that is not a valid package name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a valid Android package name")]
pub struct InvalidPackageName(String);

/// A global setting RustTV may read and write (OPTIMIZER.md § 1). The
/// namespace is fixed (`global`) and there is no free-form key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKey {
    /// `Settings.Global.WINDOW_ANIMATION_SCALE`: window open/close animations.
    WindowAnimationScale,
    /// `Settings.Global.TRANSITION_ANIMATION_SCALE`: activity transitions.
    TransitionAnimationScale,
    /// `Settings.Global.ANIMATOR_DURATION_SCALE`: `ValueAnimator`s in every app.
    AnimatorDurationScale,
}

impl SettingKey {
    /// Every key, in the order `optimize analyze` shows them.
    pub const ALL: [Self; 3] = [
        Self::WindowAnimationScale,
        Self::TransitionAnimationScale,
        Self::AnimatorDurationScale,
    ];

    /// The key's name in `Settings.Global`, e.g. `window_animation_scale`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WindowAnimationScale => "window_animation_scale",
            Self::TransitionAnimationScale => "transition_animation_scale",
            Self::AnimatorDurationScale => "animator_duration_scale",
        }
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A value for a [`SettingKey`], safe to pass to `adb shell`: a decimal of
/// one or two digits and up to three decimals, `[0-9]{1,2}(\.[0-9]{1,3})?`
/// (every allowlisted key is an animation scale). No shell syntax fits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SettingValue(String);

impl SettingValue {
    /// Validates `value`.
    pub fn new(value: &str) -> Result<Self, InvalidSettingValue> {
        let digits = |part: &str, max: usize| {
            (1..=max).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_digit())
        };
        let valid = match value.split_once('.') {
            Some((whole, fraction)) => digits(whole, 2) && digits(fraction, 3),
            None => digits(value, 2),
        };
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidSettingValue(value.to_owned()))
        }
    }

    /// The value as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SettingValue {
    type Error = InvalidSettingValue;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<SettingValue> for String {
    fn from(value: SettingValue) -> Self {
        value.0
    }
}

impl fmt::Display for SettingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returned by [`SettingValue::new`] for a value outside the allowed form.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a valid setting value; expected a decimal like `0.5` or `1.0`")]
pub struct InvalidSettingValue(String);

/// Whether a global setting is stored, and with which value.
///
/// Serialized as `"unset"` or `{ value = "0.5" }`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingState {
    /// Not stored; Android uses the default (1.0 for the animation scales).
    Unset,
    /// Stored with this value.
    Value(SettingValue),
}

impl SettingState {
    /// Reads the output of `settings get global <key>`: `null` is unset
    /// (also for a stored string `null`, which Android reads as the default
    /// too; OPTIMIZER.md § 4); anything else must be a valid [`SettingValue`].
    pub fn from_output(output: &str) -> Result<Self, InvalidSettingValue> {
        match output.trim() {
            "null" => Ok(Self::Unset),
            value => SettingValue::new(value).map(Self::Value),
        }
    }
}

impl fmt::Display for SettingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unset => f.write_str("unset"),
            Self::Value(value) => value.fmt(f),
        }
    }
}

/// A local APK file for `adb install`, as an absolute UTF-8 path.
///
/// `adb install` runs on this computer and gets the path as one process
/// argument (no host shell), so directories may contain spaces. Only the
/// file name can reach the device: older adb versions copy the APK to
/// `/data/local/tmp/<file name>` and pass that to `pm`. So the file name is
/// limited to ASCII letters, digits, `.`, `_` and `-`, ending in `.apk`, and
/// no part of the path may contain control characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApkPath(String);

impl ApkPath {
    /// Validates the shape of `path` without touching the file system (used
    /// when reading journals, where the APK may be gone).
    pub fn new(path: &str) -> Result<Self, InvalidApk> {
        let invalid = |reason: &str| InvalidApk(format!("`{path}` {reason}"));
        if !Path::new(path).is_absolute() {
            return Err(invalid("is not an absolute path"));
        }
        if path.chars().any(char::is_control) {
            return Err(invalid("contains control characters"));
        }
        let name = path.rsplit('/').next().unwrap_or_default();
        let stem = name.strip_suffix(".apk").unwrap_or_default();
        let valid_name = !stem.is_empty()
            && !name.starts_with(['-', '.'])
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !valid_name {
            return Err(invalid(
                "needs a file name of ASCII letters, digits, `.`, `_` and `-` ending in `.apk`",
            ));
        }
        Ok(Self(path.to_owned()))
    }

    /// An existing regular file at `path` (relative to the current directory
    /// or absolute), resolved to its absolute path with symlinks followed.
    pub fn from_local_file(path: &Path) -> Result<Self, InvalidApk> {
        let shown = path.display();
        let resolved = path
            .canonicalize()
            .map_err(|error| InvalidApk(format!("`{shown}`: {error}")))?;
        if !resolved.is_file() {
            return Err(InvalidApk(format!("`{shown}` is not a file")));
        }
        let text = resolved
            .to_str()
            .ok_or_else(|| InvalidApk(format!("`{shown}` is not valid UTF-8")))?;
        Self::new(text)
    }

    /// The absolute path as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ApkPath {
    type Error = InvalidApk;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        Self::new(&path)
    }
}

impl From<ApkPath> for String {
    fn from(apk: ApkPath) -> Self {
        apk.0
    }
}

impl fmt::Display for ApkPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returned for an APK path that is missing, not a file, or unsafe to pass on.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid APK: {0}")]
pub struct InvalidApk(String);

impl InvalidApk {
    pub(crate) fn new(reason: String) -> Self {
        Self(reason)
    }
}

/// A validated activity component, `<package>/<activity>`, e.g.
/// `ai.rusttv.spike/.LauncherActivity`. Safe to pass to `adb shell`.
///
/// The activity is a class name, relative (`.Main`) or fully qualified
/// (`com.example.Main`): ASCII letters, digits, `_` and `.`. Inner classes
/// (`$`) are rejected because `$` is shell syntax.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Component {
    /// `package/activity`, kept whole so it can be borrowed as one argument.
    text: String,
    package_len: usize,
}

impl Component {
    /// The package part.
    pub fn package(&self) -> PackageName {
        // Validated in `from_str`.
        PackageName(self.text[..self.package_len].to_owned())
    }

    /// The whole component as `package/activity`.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl std::str::FromStr for Component {
    type Err = InvalidComponent;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidComponent(input.to_owned());
        let (package, activity) = input.split_once('/').ok_or_else(invalid)?;
        PackageName::new(package).map_err(|_| invalid())?;
        let class = activity.strip_prefix('.').unwrap_or(activity);
        let valid_class = !class.is_empty()
            && class.split('.').all(|segment| {
                let mut chars = segment.chars();
                chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        if !valid_class {
            return Err(invalid());
        }
        Ok(Self {
            text: input.to_owned(),
            package_len: package.len(),
        })
    }
}

impl TryFrom<String> for Component {
    type Error = InvalidComponent;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Component> for String {
    fn from(component: Component) -> Self {
        component.text
    }
}

impl fmt::Display for Component {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Returned when parsing a [`Component`] fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not a valid activity component; expected `package/.Activity`, e.g. `ai.rusttv.spike/.LauncherActivity`"
)]
pub struct InvalidComponent(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// One instance of every command. The `match` stops compiling when a
    /// variant is added, so new commands can't skip the safety tests below.
    fn every_command() -> Vec<DeviceCommand> {
        let all = vec![
            DeviceCommand::GetProperties,
            DeviceCommand::ListPackages,
            DeviceCommand::ListThirdPartyPackages,
            DeviceCommand::ListDisabledPackages,
            DeviceCommand::MemInfo,
            DeviceCommand::DataStorage,
            DeviceCommand::CpuStat,
            DeviceCommand::ProcessList,
            DeviceCommand::HomeActivity,
            DeviceCommand::HomeCandidates,
            DeviceCommand::PackageMemInfo(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::PackageInfo(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::ProcessCpuTimes,
            DeviceCommand::ForceStop(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::StartActivity("com.example.app/.MainActivity".parse().unwrap()),
            DeviceCommand::Activities,
            DeviceCommand::PressHome,
            DeviceCommand::PowerState,
            DeviceCommand::WakeUp,
            DeviceCommand::InputMethods,
            DeviceCommand::EnabledAccessibilityServices,
            DeviceCommand::TvInputs,
            DeviceCommand::GetSetting(SettingKey::WindowAnimationScale),
            DeviceCommand::SetSetting {
                key: SettingKey::TransitionAnimationScale,
                to: value("0.5"),
                from: SettingState::Unset,
            },
            DeviceCommand::SetSetting {
                key: SettingKey::AnimatorDurationScale,
                to: value("0.5"),
                from: value("1.0"),
            },
            DeviceCommand::DisablePackage(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::EnablePackage(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::InstallLauncher(apk()),
            DeviceCommand::UpgradeLauncher(apk()),
            DeviceCommand::UninstallLauncher,
        ];
        for command in &all {
            match command {
                DeviceCommand::GetProperties
                | DeviceCommand::ListPackages
                | DeviceCommand::ListThirdPartyPackages
                | DeviceCommand::ListDisabledPackages
                | DeviceCommand::MemInfo
                | DeviceCommand::DataStorage
                | DeviceCommand::CpuStat
                | DeviceCommand::ProcessList
                | DeviceCommand::HomeActivity
                | DeviceCommand::HomeCandidates
                | DeviceCommand::PackageMemInfo(_)
                | DeviceCommand::PackageInfo(_)
                | DeviceCommand::ProcessCpuTimes
                | DeviceCommand::ForceStop(_)
                | DeviceCommand::StartActivity(_)
                | DeviceCommand::Activities
                | DeviceCommand::PressHome
                | DeviceCommand::PowerState
                | DeviceCommand::WakeUp
                | DeviceCommand::InputMethods
                | DeviceCommand::EnabledAccessibilityServices
                | DeviceCommand::TvInputs
                | DeviceCommand::GetSetting(_)
                | DeviceCommand::SetSetting { .. }
                | DeviceCommand::DisablePackage(_)
                | DeviceCommand::EnablePackage(_)
                | DeviceCommand::InstallLauncher(_)
                | DeviceCommand::UpgradeLauncher(_)
                | DeviceCommand::UninstallLauncher => {}
            }
        }
        all
    }

    fn apk() -> ApkPath {
        ApkPath::new("/Users/me/Application Support/app-arm64-v8a-release.apk").unwrap()
    }

    fn value(text: &str) -> SettingState {
        SettingState::Value(SettingValue::new(text).unwrap())
    }

    #[test]
    fn only_package_state_and_launcher_install_commands_are_above_transient() {
        // This used to assert that nothing above R1 existed, because R2/R3
        // commands needed the plan/approve/journal engine (SECURITY.md). The
        // engine now exists (`crate::plan`): Home takeover and privacy
        // profiles need two R3 commands and `launcher install`/`uninstall`
        // three more, limited to RustTV's own package. Anything else above R1 must be added here on purpose.
        // The optimizer (OPTIMIZER.md § 4) adds the only R2 command,
        // `SetSetting`, limited to the allowlisted keys.
        for command in every_command() {
            let expected = match command {
                DeviceCommand::ForceStop(_)
                | DeviceCommand::StartActivity(_)
                | DeviceCommand::PressHome
                | DeviceCommand::WakeUp => RiskClass::Transient,
                DeviceCommand::SetSetting { .. } => RiskClass::Setting,
                DeviceCommand::DisablePackage(_)
                | DeviceCommand::EnablePackage(_)
                | DeviceCommand::InstallLauncher(_)
                | DeviceCommand::UpgradeLauncher(_)
                | DeviceCommand::UninstallLauncher => RiskClass::PackageState,
                _ => RiskClass::Read,
            };
            assert_eq!(command.risk(), expected, "{command:?}");
        }
    }

    #[test]
    fn every_command_above_transient_can_be_undone_unless_declared_irreversible() {
        for command in every_command() {
            if command.risk() <= RiskClass::Transient {
                continue;
            }
            match command.inverse() {
                None => assert!(command.is_irreversible(), "{command:?} needs an inverse"),
                Some(inverse) if inverse.is_irreversible() => {
                    // Installing is undone by uninstalling, which can't be undone.
                    assert_eq!(command, DeviceCommand::InstallLauncher(apk()));
                }
                Some(inverse) => {
                    assert_eq!(inverse.inverse(), Some(command.clone()), "{command:?}")
                }
            }
        }
        assert_eq!(
            PackageName::rusttv_launcher(),
            PackageName::new(RUSTTV_LAUNCHER).unwrap()
        );
    }

    #[test]
    fn commands_round_trip_through_toml_and_reject_invalid_packages() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Wrapper {
            command: DeviceCommand,
        }
        for command in every_command() {
            let text = toml::to_string(&Wrapper {
                command: command.clone(),
            })
            .unwrap();
            let back: Wrapper = toml::from_str(&text).unwrap();
            assert_eq!(back.command, command, "{text}");
        }
        let evil = "command = { kind = \"disable_package\", arg = \"com.x; reboot\" }";
        assert!(toml::from_str::<Wrapper>(evil).is_err());
    }

    #[test]
    fn set_setting_renders_put_or_delete_and_inverts_from_its_own_original() {
        let set = DeviceCommand::SetSetting {
            key: SettingKey::WindowAnimationScale,
            to: value("0.5"),
            from: SettingState::Unset,
        };
        assert_eq!(
            set.adb_args(),
            [
                "shell",
                "settings",
                "put",
                "global",
                "window_animation_scale",
                "0.5"
            ]
        );
        let undo = set.inverse().unwrap();
        assert_eq!(
            undo.adb_args(),
            [
                "shell",
                "settings",
                "delete",
                "global",
                "window_animation_scale"
            ]
        );
        assert_eq!(
            DeviceCommand::GetSetting(SettingKey::AnimatorDurationScale).adb_args(),
            [
                "shell",
                "settings",
                "get",
                "global",
                "animator_duration_scale"
            ]
        );
    }

    #[test]
    fn setting_values_are_short_decimals_only() {
        for good in ["0", "1", "0.5", "1.0", "10", "99.999", "0.25"] {
            assert!(SettingValue::new(good).is_ok(), "{good:?}");
        }
        for bad in [
            "",
            ".5",
            "5.",
            "100",
            "0.5000",
            "-1",
            "+1",
            "1e3",
            "0,5",
            " 0.5",
            "0.5 ",
            "0.5;reboot",
            "0.5\nreboot",
            "$(reboot)",
            "`id`",
            "null",
            "NaN",
            "inf",
            "1..0",
            "0.5.1",
            "٣",
        ] {
            assert!(SettingValue::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn settings_get_output_null_is_unset_and_anything_else_must_validate() {
        assert_eq!(SettingState::from_output("null\n"), Ok(SettingState::Unset));
        assert_eq!(
            SettingState::from_output("null\r\n"),
            Ok(SettingState::Unset)
        );
        assert_eq!(SettingState::from_output("1.0\n"), Ok(value("1.0")));
        assert!(SettingState::from_output("").is_err());
        assert!(SettingState::from_output("fast\n").is_err());
        assert!(SettingState::from_output("0.5; reboot").is_err());
    }

    #[test]
    fn tampered_setting_commands_do_not_deserialize() {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            #[expect(dead_code, reason = "only deserialization is tested")]
            command: DeviceCommand,
        }
        let good = r#"command = { kind = "set_setting", arg = { key = "window_animation_scale", to = { value = "0.5" }, from = "unset" } }"#;
        assert!(toml::from_str::<Wrapper>(good).is_ok());
        for bad in [
            good.replace("window_animation_scale", "adb_enabled"),
            good.replace("\"0.5\"", "\"0.5; reboot\""),
            good.replace("\"unset\"", "\"null\""),
            good.replace("set_setting", "put_setting"),
            good.replace(" }, from", ", namespace = \"secure\" }, from"),
        ] {
            assert!(toml::from_str::<Wrapper>(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn components_accept_relative_and_qualified_activities() {
        let component: Component = "ai.rusttv.spike/.LauncherActivity".parse().unwrap();
        assert_eq!(component.package().as_str(), "ai.rusttv.spike");
        assert!(
            "com.google.android.apps.tv.launcherx/com.google.android.apps.tv.launcherx.home.HomeActivity"
                .parse::<Component>()
                .is_ok()
        );
    }

    #[test]
    fn components_with_shell_syntax_or_bad_shape_are_rejected() {
        for input in [
            "",
            "ai.rusttv.spike",
            "ai.rusttv.spike/",
            "ai.rusttv.spike/.",
            "/.Main",
            "ai.rusttv.spike/.Main$Inner",
            "ai.rusttv.spike/.Main;reboot",
            "ai.rusttv.spike/.Main reboot",
            "evil;reboot/.Main",
            "ai.rusttv.spike/..Main",
        ] {
            assert!(input.parse::<Component>().is_err(), "{input:?}");
        }
    }

    #[test]
    fn commands_run_through_adb_shell_without_shell_syntax() {
        const SHELL_SYNTAX: &[char] = &[
            ';', '|', '&', '$', '`', '<', '>', '(', ')', '\'', '"', '\\', '*', '?', '\n', ' ',
        ];
        for command in every_command() {
            let mut args = command.adb_args();
            if args.first() == Some(&"install") {
                // Host-side: the APK path is one argument; only its file name
                // may reach the device, and it is checked like the rest.
                let path = args.pop().unwrap();
                assert!(path.ends_with("/app-arm64-v8a-release.apk"), "{path}");
                args.push(path.rsplit('/').next().unwrap());
            } else {
                assert_eq!(args.first(), Some(&"shell"), "{command:?}");
            }
            for arg in args {
                assert!(!arg.contains(SHELL_SYNTAX), "{command:?}: {arg:?}");
            }
        }
    }

    #[test]
    fn apk_paths_must_be_absolute_with_a_plain_apk_file_name() {
        for path in [
            "/a/app.apk",
            "/Users/me/My Builds/app-x86_64-release.apk",
            "/a/app_1.2-x.apk",
        ] {
            assert!(ApkPath::new(path).is_ok(), "{path:?}");
        }
        for path in [
            "",
            "app.apk",
            "android/app.apk",
            "/a/app.zip",
            "/a/.apk",
            "/a/-r.apk",
            "/a/my app.apk",
            "/a/app;reboot.apk",
            "/a/$(reboot).apk",
            "/a/app\n.apk",
            "/a\n/app.apk",
            "/a/äpp.apk",
            "/a/",
        ] {
            assert!(ApkPath::new(path).is_err(), "{path:?}");
        }
    }

    #[test]
    fn apk_files_are_resolved_and_must_exist() {
        let dir = std::env::temp_dir().join(format!("rusttv-apk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("app-x86_64-release.apk");
        std::fs::write(&file, b"not really an APK").unwrap();

        let apk = ApkPath::from_local_file(&file).unwrap();
        assert!(Path::new(apk.as_str()).is_absolute());
        assert!(apk.as_str().ends_with("/app-x86_64-release.apk"));
        assert!(ApkPath::from_local_file(&dir.join("missing.apk")).is_err());
        assert!(ApkPath::from_local_file(&dir).is_err(), "a directory");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn package_name_is_passed_as_a_single_argument() {
        let package = PackageName::new("com.google.android.tvlauncher").unwrap();
        // `adb_args` borrows from the command, so the command must outlive `args`.
        let command = DeviceCommand::PackageMemInfo(package);
        let args = command.adb_args();
        assert_eq!(args.last(), Some(&"com.google.android.tvlauncher"));
    }

    #[test]
    fn real_package_names_are_accepted() {
        for name in [
            "com.example",
            "com.google.android.tvlauncher",
            "ai.rusttv.launcher",
            "a.b_2",
        ] {
            assert!(PackageName::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn shell_injection_and_malformed_names_are_rejected() {
        for name in [
            "",
            "launcher",
            "com.example; reboot",
            "com.example && rm -rf /",
            "com.example|cat",
            "$(reboot).x",
            "com.`id`",
            "com.example\nreboot",
            "com..example",
            ".com.example",
            "com.example.",
            "com.1example",
            "com.exämple",
        ] {
            assert!(
                PackageName::new(name).is_err(),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn risk_classes_are_ordered_by_severity() {
        assert!(RiskClass::Read < RiskClass::Transient);
        assert!(RiskClass::Transient < RiskClass::Setting);
        assert!(RiskClass::Setting < RiskClass::PackageState);
    }
}

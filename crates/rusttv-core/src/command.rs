//! Typed device commands: the only way RustTV can reach a device.
//!
//! There is deliberately no "run this shell string" command. Every command is
//! a [`DeviceCommand`] variant with a fixed argument list and a [`RiskClass`]
//! (SECURITY.md § Risk classes).

use std::fmt;

use serde::{Deserialize, Serialize};

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
    /// Disables a package for user 0 (`pm disable-user --user 0 <package>`).
    /// Package state; only the Home takeover plan uses it (SECURITY.md).
    DisablePackage(PackageName),
    /// Re-enables a package for user 0 (`pm enable --user 0 <package>`). Package state.
    EnablePackage(PackageName),
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
            | Self::ProcessCpuTimes
            | Self::Activities => RiskClass::Read,
            Self::ForceStop(_) | Self::StartActivity(_) | Self::PressHome => RiskClass::Transient,
            Self::DisablePackage(_) | Self::EnablePackage(_) => RiskClass::PackageState,
        }
    }

    /// The command that undoes this one, if there is one.
    ///
    /// Every R2/R3 command must have an inverse: plans refuse steps that can't
    /// be undone (SECURITY.md § Change lifecycle).
    pub fn inverse(&self) -> Option<Self> {
        match self {
            Self::DisablePackage(package) => Some(Self::EnablePackage(package.clone())),
            Self::EnablePackage(package) => Some(Self::DisablePackage(package.clone())),
            _ => None,
        }
    }

    /// Arguments passed to `adb` after the `-s <serial>` device selector.
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
            Self::ProcessCpuTimes => vec!["shell", "ps", "-A", "-o", "PID,TIME+,NAME"],
            Self::ForceStop(package) => vec!["shell", "am", "force-stop", package.as_str()],
            Self::StartActivity(component) => {
                vec!["shell", "am", "start", "-W", "-n", component.as_str()]
            }
            Self::Activities => vec!["shell", "dumpsys", "activity", "activities"],
            Self::PressHome => vec!["shell", "input", "keyevent", "KEYCODE_HOME"],
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
            DeviceCommand::ProcessCpuTimes,
            DeviceCommand::ForceStop(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::StartActivity("com.example.app/.MainActivity".parse().unwrap()),
            DeviceCommand::Activities,
            DeviceCommand::PressHome,
            DeviceCommand::DisablePackage(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::EnablePackage(PackageName::new("com.example.app").unwrap()),
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
                | DeviceCommand::ProcessCpuTimes
                | DeviceCommand::ForceStop(_)
                | DeviceCommand::StartActivity(_)
                | DeviceCommand::Activities
                | DeviceCommand::PressHome
                | DeviceCommand::DisablePackage(_)
                | DeviceCommand::EnablePackage(_) => {}
            }
        }
        all
    }

    #[test]
    fn only_package_enable_and_disable_are_above_transient() {
        // This used to assert that nothing above R1 existed, because R2/R3
        // commands needed the plan/approve/journal engine (SECURITY.md). The
        // engine now exists (`crate::plan`), and Home takeover needs exactly
        // two R3 commands. Anything else above R1 must be added here on purpose.
        for command in every_command() {
            let expected = match command {
                DeviceCommand::ForceStop(_)
                | DeviceCommand::StartActivity(_)
                | DeviceCommand::PressHome => RiskClass::Transient,
                DeviceCommand::DisablePackage(_) | DeviceCommand::EnablePackage(_) => {
                    RiskClass::PackageState
                }
                _ => RiskClass::Read,
            };
            assert_eq!(command.risk(), expected, "{command:?}");
        }
    }

    #[test]
    fn every_command_above_transient_can_be_undone() {
        for command in every_command() {
            if command.risk() > RiskClass::Transient {
                let inverse = command.inverse().expect("R2/R3 commands need an inverse");
                assert_eq!(inverse.inverse(), Some(command.clone()), "{command:?}");
            }
        }
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
            let args = command.adb_args();
            assert_eq!(args.first(), Some(&"shell"), "{command:?}");
            for arg in args {
                assert!(!arg.contains(SHELL_SYNTAX), "{command:?}: {arg:?}");
            }
        }
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

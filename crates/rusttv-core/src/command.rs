//! Typed device commands: the only way RustTV can reach a device.
//!
//! There is deliberately no "run this shell string" command. Every command is
//! a [`DeviceCommand`] variant with a fixed argument list and a [`RiskClass`]
//! (SECURITY.md § Risk classes).

use std::fmt;

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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
    /// Memory usage of one app process (`dumpsys meminfo <package>`).
    PackageMemInfo(PackageName),
    /// Cumulative CPU time of every process (`ps -A -o PID,TIME+,NAME`).
    ProcessCpuTimes,
    /// Stops an app's processes (`am force-stop <package>`). Transient.
    ForceStop(PackageName),
    /// Starts an activity and waits for its first frame (`am start -W -n <component>`). Transient.
    StartActivity(Component),
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
            | Self::PackageMemInfo(_)
            | Self::ProcessCpuTimes => RiskClass::Read,
            Self::ForceStop(_) | Self::StartActivity(_) => RiskClass::Transient,
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
            Self::PackageMemInfo(package) => vec!["shell", "dumpsys", "meminfo", package.as_str()],
            Self::ProcessCpuTimes => vec!["shell", "ps", "-A", "-o", "PID,TIME+,NAME"],
            Self::ForceStop(package) => vec!["shell", "am", "force-stop", package.as_str()],
            Self::StartActivity(component) => {
                vec!["shell", "am", "start", "-W", "-n", component.as_str()]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
            DeviceCommand::PackageMemInfo(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::ProcessCpuTimes,
            DeviceCommand::ForceStop(PackageName::new("com.example.app").unwrap()),
            DeviceCommand::StartActivity("com.example.app/.MainActivity".parse().unwrap()),
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
                | DeviceCommand::PackageMemInfo(_)
                | DeviceCommand::ProcessCpuTimes
                | DeviceCommand::ForceStop(_)
                | DeviceCommand::StartActivity(_) => {}
            }
        }
        all
    }

    #[test]
    fn nothing_above_transient_exists_yet() {
        // R2/R3 commands need the plan/approve/journal engine (SECURITY.md),
        // which doesn't exist yet. Only stopping and starting apps is R1.
        for command in every_command() {
            let expected = match command {
                DeviceCommand::ForceStop(_) | DeviceCommand::StartActivity(_) => {
                    RiskClass::Transient
                }
                _ => RiskClass::Read,
            };
            assert_eq!(command.risk(), expected, "{command:?}");
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

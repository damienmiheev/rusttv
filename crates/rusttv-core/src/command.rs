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
    /// Memory usage of one app process (`dumpsys meminfo <package>`).
    PackageMemInfo(PackageName),
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
            | Self::PackageMemInfo(_) => RiskClass::Read,
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
            Self::PackageMemInfo(package) => vec!["shell", "dumpsys", "meminfo", package.as_str()],
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
            DeviceCommand::PackageMemInfo(PackageName::new("com.example.app").unwrap()),
        ];
        for command in &all {
            match command {
                DeviceCommand::GetProperties
                | DeviceCommand::ListPackages
                | DeviceCommand::ListThirdPartyPackages
                | DeviceCommand::ListDisabledPackages
                | DeviceCommand::MemInfo
                | DeviceCommand::DataStorage
                | DeviceCommand::PackageMemInfo(_) => {}
            }
        }
        all
    }

    #[test]
    fn milestone_1_commands_are_read_only() {
        for command in every_command() {
            assert_eq!(command.risk(), RiskClass::Read, "{command:?}");
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

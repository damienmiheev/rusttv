//! `rusttv apps list`: installed packages and their status.

use std::collections::HashSet;

use serde::Serialize;

use crate::adb::Adb;
use crate::command::DeviceCommand;
use crate::error::AdbError;

/// One installed package. Serialized in `rusttv apps list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    /// Package name, e.g. `com.google.android.youtube.tv`.
    pub package: String,
    /// Path of the base APK on the device.
    pub apk_path: Option<String>,
    /// Whether the package ships with the firmware.
    pub kind: AppKind,
    /// `false` if the package is disabled for the user.
    pub enabled: bool,
}

/// Where a package comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKind {
    /// Part of the system image, including system apps updated from the store.
    System,
    /// Installed by the user or a store; can be uninstalled normally.
    ThirdParty,
}

/// Lists installed packages, sorted by name.
///
/// System vs third-party comes from Android's own `-3` filter rather than the
/// APK path: updated system apps live under `/data/app` like user apps.
pub fn list_apps(adb: &dyn Adb, serial: &str) -> Result<Vec<AppInfo>, AdbError> {
    let all = adb.run(serial, &DeviceCommand::ListPackages)?;
    let third_party = adb.run(serial, &DeviceCommand::ListThirdPartyPackages)?;
    let disabled = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;

    let third_party = package_names(&third_party)?;
    let disabled = package_names(&disabled)?;

    let mut apps: Vec<AppInfo> = parse_package_list(&all)?
        .into_iter()
        .map(|(package, apk_path)| AppInfo {
            package: package.to_owned(),
            apk_path: apk_path.map(str::to_owned),
            kind: if third_party.contains(package) {
                AppKind::ThirdParty
            } else {
                AppKind::System
            },
            enabled: !disabled.contains(package),
        })
        .collect();
    apps.sort_by(|a, b| a.package.cmp(&b.package));
    Ok(apps)
}

/// `pm list packages` printed an error instead of a package list.
///
/// Kept apart from an empty list, which `pm` prints as no output at all, so
/// a failed query never reads as "not installed" or "none disabled".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`pm list packages` printed an error instead of a package list: {line}")]
pub struct PackageListError {
    line: String,
}

impl PackageListError {
    /// The first line of the output that shows the failure.
    pub fn line(&self) -> &str {
        &self.line
    }
}

/// How `pm` (or `cmd`, which `pm` runs) starts the lines it prints when it
/// fails, per AOSP `cmd.cpp`, `ShellCommand` and `PackageManagerShellCommand`.
/// Without adb's shell protocol (`shell_v2`), these arrive on stdout with exit
/// code 0; with it, the non-zero exit already fails as `COMMAND_FAILED`.
const PM_ERROR_PREFIXES: &[&str] = &[
    "Error:",
    "Exception",
    "Failure",
    "cmd: ",
    "java.",
    "Security exception",
];

/// Parses `pm list packages` output into `(package, apk_path)` pairs.
///
/// Lines are `package:<name>`, or `package:<path>=<name>` with `-f`. The
/// path itself can contain `=` (e.g. `/data/app/~~abc==/...`), so the name
/// is whatever follows the last `=`.
///
/// Empty output is an empty list. It is an error if any line looks like a
/// `pm` failure ([`PM_ERROR_PREFIXES`]), or if there is output but no
/// `package:` line at all (e.g. `/system/bin/sh: pm: not found`). Other
/// stray lines next to packages, such as linker warnings, are ignored.
pub fn parse_package_list(output: &str) -> Result<Vec<(&str, Option<&str>)>, PackageListError> {
    let mut packages = Vec::new();
    let mut unrecognized = None;
    for line in output.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let Some(entry) = line.strip_prefix("package:") else {
            if PM_ERROR_PREFIXES
                .iter()
                .any(|prefix| line.starts_with(prefix))
            {
                return Err(PackageListError {
                    line: line.to_owned(),
                });
            }
            unrecognized.get_or_insert(line);
            continue;
        };
        let (path, name) = match entry.rsplit_once('=') {
            Some((path, name)) => (Some(path), name),
            None => (None, entry),
        };
        if !name.is_empty() {
            packages.push((name, path));
        }
    }
    match unrecognized {
        Some(line) if packages.is_empty() => Err(PackageListError {
            line: line.to_owned(),
        }),
        _ => Ok(packages),
    }
}

/// The package names in `pm list packages` output.
pub(crate) fn package_names(output: &str) -> Result<HashSet<&str>, PackageListError> {
    Ok(parse_package_list(output)?
        .into_iter()
        .map(|(name, _)| name)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;

    fn emulator() -> FakeAdb {
        FakeAdb::new()
            .respond(
                DeviceCommand::ListPackages,
                include_str!("fixtures/emulator-pixel3a-api34/packages-f.txt"),
            )
            .respond(
                DeviceCommand::ListThirdPartyPackages,
                include_str!("fixtures/emulator-pixel3a-api34/packages-3.txt"),
            )
            .respond(
                DeviceCommand::ListDisabledPackages,
                include_str!("fixtures/emulator-pixel3a-api34/packages-d.txt"),
            )
    }

    fn find<'a>(apps: &'a [AppInfo], package: &str) -> &'a AppInfo {
        apps.iter().find(|app| app.package == package).unwrap()
    }

    #[test]
    fn lists_every_package_from_a_real_capture() {
        let apps = list_apps(&emulator(), "emulator-5554").unwrap();

        assert_eq!(apps.len(), 224);
        assert!(
            apps.windows(2)
                .all(|pair| pair[0].package < pair[1].package)
        );
        let third_party = apps
            .iter()
            .filter(|app| app.kind == AppKind::ThirdParty)
            .count();
        assert_eq!(third_party, 3);
        let disabled: Vec<&str> = apps
            .iter()
            .filter(|app| !app.enabled)
            .map(|app| app.package.as_str())
            .collect();
        assert_eq!(
            disabled,
            ["com.android.nfc", "com.google.android.gms.supervision"]
        );
    }

    #[test]
    fn updated_system_app_under_data_app_is_still_system() {
        let apps = list_apps(&emulator(), "emulator-5554").unwrap();
        let webview = find(&apps, "com.google.android.webview");

        assert!(
            webview
                .apk_path
                .as_deref()
                .unwrap()
                .starts_with("/data/app/")
        );
        assert_eq!(webview.kind, AppKind::System);
        assert!(webview.enabled);
    }

    #[test]
    fn apk_path_containing_equals_signs_is_split_at_the_last_one() {
        let line = "package:/data/app/~~R0xxg-XMHSOyj_E0xw53FQ==/com.redphx.deviceid-ipg_7VwytiEx_LTHvbhrjg==/base.apk=com.redphx.deviceid";
        let parsed = parse_package_list(line).unwrap();
        assert_eq!(
            parsed,
            [(
                "com.redphx.deviceid",
                Some(
                    "/data/app/~~R0xxg-XMHSOyj_E0xw53FQ==/com.redphx.deviceid-ipg_7VwytiEx_LTHvbhrjg==/base.apk"
                )
            )]
        );
    }

    #[test]
    fn noise_lines_are_ignored() {
        let parsed =
            parse_package_list("\nWARNING: linker: something\npackage:com.example\npackage:\n")
                .unwrap();
        assert_eq!(parsed, [("com.example", None)]);
    }

    #[test]
    fn empty_output_is_an_empty_list() {
        let output = include_str!("fixtures/synthetic-home-takeover/packages-d-none.txt");
        assert_eq!(parse_package_list(output), Ok(vec![]));
    }

    #[test]
    fn each_pm_failure_shape_is_an_error() {
        let failures = [
            (
                include_str!("fixtures/synthetic-pm-list-errors/cant-find-service.txt"),
                "cmd: Can't find service: package",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/failure-calling-service.txt"),
                "Failure calling service package: Broken pipe (32)",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/exception.txt"),
                "Exception occurred while executing 'list':",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/error-unknown-option.txt"),
                "Error: Unknown option: -x",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/error-not-running.txt"),
                "Error: Could not access the Package Manager.  Is the system running?",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/sh-pm-not-found.txt"),
                "/system/bin/sh: pm: not found",
            ),
            (
                include_str!("fixtures/synthetic-pm-list-errors/packages-then-exception.txt"),
                "Exception occurred while executing 'list':",
            ),
        ];
        for (output, line) in failures {
            let error = parse_package_list(output).unwrap_err();
            assert_eq!(error.line(), line);
            assert_eq!(AdbError::from(error).code(), "PACKAGE_LIST_FAILED");
        }
    }

    #[test]
    fn a_failing_query_fails_the_listing_instead_of_hiding_packages() {
        for failing in [
            DeviceCommand::ListPackages,
            DeviceCommand::ListThirdPartyPackages,
            DeviceCommand::ListDisabledPackages,
        ] {
            let adb = emulator().respond(
                failing,
                include_str!("fixtures/synthetic-pm-list-errors/cant-find-service.txt"),
            );
            let error = list_apps(&adb, "emulator-5554").unwrap_err();
            assert_eq!(error.code(), "PACKAGE_LIST_FAILED");
            assert!(error.to_string().contains("Can't find service"), "{error}");
        }
    }
}

#[cfg(test)]
mod odd_input {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::{adb_output, line_ending};

    proptest! {
        #[test]
        fn parser_never_panics(output in adb_output()) {
            if let Ok(packages) = parse_package_list(&output) {
                for (name, _) in packages {
                    prop_assert!(!name.is_empty());
                }
            }
        }

        #[test]
        fn package_lines_parse_back_to_their_parts(
            packages in prop::collection::vec(
                ("[a-z][a-z0-9_]{0,10}(\\.[a-z][a-z0-9_]{0,10}){0,4}", prop::option::of("/data/app/[~A-Za-z0-9_=-]{0,30}/base\\.apk")),
                0..20,
            ),
            ending in line_ending(),
            trailing in " {0,3}",
        ) {
            let output: String = packages
                .iter()
                .map(|(name, path)| match path {
                    Some(path) => format!("package:{path}={name}{trailing}{ending}"),
                    None => format!("package:{name}{trailing}{ending}"),
                })
                .collect();
            let parsed = parse_package_list(&output).unwrap();
            let expected: Vec<(&str, Option<&str>)> = packages
                .iter()
                .map(|(name, path)| (name.as_str(), path.as_deref()))
                .collect();
            prop_assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn empty_output_has_no_packages() {
        assert_eq!(parse_package_list(""), Ok(vec![]));
        assert_eq!(parse_package_list("\r\n\n"), Ok(vec![]));
    }

    #[test]
    fn crlf_trailing_spaces_and_a_truncated_last_line() {
        let parsed =
            parse_package_list("package:com.a\r\npackage:com.b  \r\r\npackage:com.c").unwrap();
        assert_eq!(parsed, [("com.a", None), ("com.b", None), ("com.c", None)]);
    }

    #[test]
    fn error_output_is_an_error_not_an_empty_list() {
        let output = "Error: java.lang.SecurityException: Shell does not have permission\n";
        assert!(parse_package_list(output).is_err());
    }
}

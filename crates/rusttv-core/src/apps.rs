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

    let third_party: HashSet<&str> = parse_package_list(&third_party)
        .map(|(name, _)| name)
        .collect();
    let disabled: HashSet<&str> = parse_package_list(&disabled)
        .map(|(name, _)| name)
        .collect();

    let mut apps: Vec<AppInfo> = parse_package_list(&all)
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

/// Parses `pm list packages` output into `(package, apk_path)` pairs.
///
/// Lines are `package:<name>`, or `package:<path>=<name>` with `-f`. The
/// path itself can contain `=` (e.g. `/data/app/~~abc==/...`), so the name
/// is whatever follows the last `=`.
pub fn parse_package_list(output: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    output.lines().filter_map(|line| {
        let entry = line.trim().strip_prefix("package:")?;
        let (path, name) = match entry.rsplit_once('=') {
            Some((path, name)) => (Some(path), name),
            None => (None, entry),
        };
        (!name.is_empty()).then_some((name, path))
    })
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
        let parsed: Vec<_> = parse_package_list(line).collect();
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
        let parsed: Vec<_> =
            parse_package_list("\nWARNING: linker: something\npackage:com.example\npackage:\n")
                .collect();
        assert_eq!(parsed, [("com.example", None)]);
    }
}

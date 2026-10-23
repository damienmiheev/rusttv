//! `rusttv launcher status`: which launcher the Home button opens.

use serde::Serialize;

use crate::adb::Adb;
use crate::command::DeviceCommand;
use crate::error::AdbError;
use crate::perf::parse_home_activity;

/// Package name of the RustTV launcher.
pub const RUSTTV_LAUNCHER: &str = "ai.rusttv.launcher";

/// Which launcher handles Home. Serialized in `rusttv launcher status --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LauncherStatus {
    /// Activity the Home button opens now, `package/activity`; `None` if nothing does.
    pub home: Option<String>,
    /// Every activity that can handle Home, highest priority first.
    pub candidates: Vec<HomeCandidate>,
    /// Whether the RustTV launcher is installed (and enabled) as a Home candidate.
    pub rusttv_installed: bool,
    /// Whether Home opens the RustTV launcher now.
    pub rusttv_is_home: bool,
}

/// An activity that can handle Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HomeCandidate {
    /// `package/activity` as Android prints it (the activity may be shortened to `.Name`).
    pub component: String,
    /// Intent filter priority. Android picks the highest without asking; on
    /// Google TV the stock launcher has 2 and other launchers 0 (ARCHITECTURE.md D12).
    pub priority: i32,
}

impl HomeCandidate {
    /// The package part of [`HomeCandidate::component`].
    pub fn package(&self) -> &str {
        self.component
            .split_once('/')
            .map_or(self.component.as_str(), |(package, _)| package)
    }
}

/// Reads which launcher handles Home and which others could. Read-only.
pub fn launcher_status(adb: &dyn Adb, serial: &str) -> Result<LauncherStatus, AdbError> {
    let home = adb.run(serial, &DeviceCommand::HomeActivity)?;
    let candidates = adb.run(serial, &DeviceCommand::HomeCandidates)?;

    let home =
        parse_home_activity(&home).map(|(package, activity)| format!("{package}/{activity}"));
    let candidates = parse_home_candidates(&candidates);
    let is_rusttv = |component: &str| {
        component.split_once('/').map(|(package, _)| package) == Some(RUSTTV_LAUNCHER)
    };
    Ok(LauncherStatus {
        rusttv_installed: candidates
            .iter()
            .any(|candidate| candidate.package() == RUSTTV_LAUNCHER),
        rusttv_is_home: home.as_deref().is_some_and(is_rusttv),
        home,
        candidates,
    })
}

/// Parses `cmd package query-activities --brief`: blocks of a
/// `priority=<n> ...` line followed by a `package/activity` line.
/// Sorted by priority, highest first; ties keep Android's order.
pub fn parse_home_candidates(output: &str) -> Vec<HomeCandidate> {
    let mut candidates = Vec::new();
    let mut priority = None;
    for line in output.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("priority=") {
            priority = rest
                .split_whitespace()
                .next()
                .and_then(|value| value.parse().ok());
        } else if line.contains('/')
            && !line.contains(' ')
            && let Some(priority) = priority.take()
        {
            candidates.push(HomeCandidate {
                component: line.to_owned(),
                priority,
            });
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.priority));
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;

    fn tv() -> FakeAdb {
        FakeAdb::new()
            .respond(
                DeviceCommand::HomeActivity,
                include_str!("fixtures/google-tv-emulator-api34/home-activity.txt"),
            )
            .respond(
                DeviceCommand::HomeCandidates,
                include_str!("fixtures/google-tv-emulator-api34/home-candidates.txt"),
            )
    }

    #[test]
    fn google_tv_home_is_the_stock_launcher_with_rusttv_installed() {
        let status = launcher_status(&tv(), "emulator-5554").unwrap();
        assert_eq!(
            status.home.as_deref(),
            Some("com.google.android.apps.tv.launcherx/.home.HomeActivity")
        );
        let candidates: Vec<(&str, i32)> = status
            .candidates
            .iter()
            .map(|candidate| (candidate.component.as_str(), candidate.priority))
            .collect();
        assert_eq!(
            candidates,
            [
                ("com.google.android.apps.tv.launcherx/.home.HomeActivity", 2),
                ("ai.rusttv.launcher/.LauncherActivity", 0),
                ("ai.rusttv.spike/.LauncherActivity", 0),
                ("com.android.tv.settings/.system.FallbackHome", -1000),
            ]
        );
        assert!(status.rusttv_installed);
        assert!(!status.rusttv_is_home);
    }

    #[test]
    fn rusttv_as_home_is_recognised_by_package() {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::HomeActivity,
                "priority=0 preferredOrder=0\nai.rusttv.launcher/.LauncherActivity\n",
            )
            .respond(DeviceCommand::HomeCandidates, "No activities found\n");
        let status = launcher_status(&adb, "x").unwrap();
        assert!(status.rusttv_is_home);
        assert!(!status.rusttv_installed);
        assert!(status.candidates.is_empty());
    }

    #[test]
    fn a_similar_package_name_is_not_rusttv() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::HomeActivity, "ai.rusttv.launcherx/.Home\n")
            .respond(
                DeviceCommand::HomeCandidates,
                "priority=0\nai.rusttv.launcherx/.Home\n",
            );
        let status = launcher_status(&adb, "x").unwrap();
        assert!(!status.rusttv_is_home);
        assert!(!status.rusttv_installed);
    }

    #[test]
    fn nothing_handling_home_gives_no_home() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::HomeActivity, "No activity found\n")
            .respond(DeviceCommand::HomeCandidates, "No activities found\n");
        let status = launcher_status(&adb, "x").unwrap();
        assert_eq!(status.home, None);
        assert!(status.candidates.is_empty());
    }

    #[test]
    fn a_component_without_a_priority_line_is_skipped() {
        let parsed =
            parse_home_candidates("Activity #0:\n  com.example/.Home\npriority=x\nother/.A\n");
        assert!(parsed.is_empty());
    }
}

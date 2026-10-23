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

/// Parses `dumpsys activity activities` for the resumed (front) activity,
/// `package/activity`, from its `topResumedActivity=ActivityRecord{<id> u0
/// <component> t<task>}` line (or `ResumedActivity:` / `mResumedActivity:`
/// on other Android versions). `None` if no activity is resumed.
pub fn parse_resumed_activity(output: &str) -> Option<&str> {
    output.lines().find_map(|line| {
        let line = line.trim_start();
        let record = [
            "topResumedActivity=",
            "ResumedActivity:",
            "mResumedActivity:",
        ]
        .iter()
        .find_map(|label| line.strip_prefix(label))?;
        let record = record.trim().strip_prefix("ActivityRecord{")?;
        record
            .split_whitespace()
            .find(|token| token.contains('/'))
            .map(|component| component.trim_end_matches('}'))
    })
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
    fn resumed_activity_is_read_from_dumpsys() {
        let rusttv = include_str!("fixtures/synthetic-home-takeover/activities-rusttv-front.txt");
        assert_eq!(
            parse_resumed_activity(rusttv),
            Some("ai.rusttv.launcher/.LauncherActivity")
        );
        let stock = include_str!("fixtures/synthetic-home-takeover/activities-stock-front.txt");
        assert_eq!(
            parse_resumed_activity(stock),
            Some("com.google.android.apps.tv.launcherx/.home.HomeActivity")
        );
        assert_eq!(
            parse_resumed_activity("ACTIVITY MANAGER\n  topResumedActivity=null\n"),
            None
        );
    }

    #[test]
    fn a_component_without_a_priority_line_is_skipped() {
        let parsed =
            parse_home_candidates("Activity #0:\n  com.example/.Home\npriority=x\nother/.A\n");
        assert!(parsed.is_empty());
    }
}

#[cfg(test)]
mod odd_input {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::{adb_output, line_ending};

    const COMPONENT: &str =
        "[a-z][a-z0-9_]{0,8}(\\.[a-z][a-z0-9_]{0,8}){0,3}/\\.?[A-Za-z][A-Za-z0-9_.]{0,20}";

    proptest! {
        #[test]
        fn parsers_never_panic(output in adb_output()) {
            let candidates = parse_home_candidates(&output);
            prop_assert!(candidates.windows(2).all(|pair| pair[0].priority >= pair[1].priority));
            for candidate in &candidates {
                prop_assert!(candidate.component.contains('/'));
                let _ = candidate.package();
            }
            parse_resumed_activity(&output);
        }

        #[test]
        fn candidates_parse_back_sorted_by_priority(
            blocks in prop::collection::vec((any::<i32>(), COMPONENT), 0..10),
            ending in line_ending(),
        ) {
            let mut output = format!("{} activities found:{ending}", blocks.len());
            for (index, (priority, component)) in blocks.iter().enumerate() {
                output.push_str(&format!(
                    "  Activity #{index}:{ending}    priority={priority} preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=true{ending}    {component}{ending}"
                ));
            }
            let mut expected: Vec<HomeCandidate> = blocks
                .iter()
                .map(|(priority, component)| HomeCandidate { component: component.clone(), priority: *priority })
                .collect();
            // Stable sort: ties keep Android's order.
            expected.sort_by_key(|candidate| std::cmp::Reverse(candidate.priority));
            prop_assert_eq!(parse_home_candidates(&output), expected);
        }

        #[test]
        fn resumed_activity_parses_back(
            component in COMPONENT,
            label in "(  topResumedActivity=|    ResumedActivity: |  mResumedActivity: )",
            ending in line_ending(),
        ) {
            let output = format!(
                "ACTIVITY MANAGER ACTIVITIES{ending}{label}ActivityRecord{{1a2b3c u0 {component} t12}}{ending}"
            );
            prop_assert_eq!(parse_resumed_activity(&output), Some(component.as_str()));
        }
    }

    #[test]
    fn empty_output_has_no_candidates_or_resumed_activity() {
        assert!(parse_home_candidates("").is_empty());
        assert_eq!(parse_resumed_activity(""), None);
    }

    #[test]
    fn candidates_with_crlf_and_a_truncated_last_block() {
        let output = "2 activities found:\r\n  Activity #0:\r\n    priority=2 preferredOrder=0\r\n    com.a/.Home  \r\n  Activity #1:\r\n    priority=0 pre";
        assert_eq!(
            parse_home_candidates(output),
            [HomeCandidate {
                component: "com.a/.Home".to_owned(),
                priority: 2
            }]
        );
    }

    #[test]
    fn a_component_without_a_readable_priority_is_skipped() {
        // Never invent a priority; it decides which launcher Android picks.
        let output = "priority=99999999999 x\ncom.a/.Home\npriority=1\ncom.b/.Home\n";
        assert_eq!(
            parse_home_candidates(output),
            [HomeCandidate {
                component: "com.b/.Home".to_owned(),
                priority: 1
            }]
        );
    }

    #[test]
    fn resumed_activity_with_crlf_and_truncated_record() {
        assert_eq!(
            parse_resumed_activity("  topResumedActivity=ActivityRecord{1 u0 com.a/.B t3}\r\n"),
            Some("com.a/.B")
        );
        assert_eq!(
            parse_resumed_activity("  topResumedActivity=ActivityRecord{1 u0 com.a/.B"),
            Some("com.a/.B")
        );
        assert_eq!(parse_resumed_activity("  topResumedActivity=null\n"), None);
    }
}

//! `rusttv performance snapshot`: a point-in-time view of device load.
//!
//! Commands are chosen to be cheap so measuring doesn't load the device. A
//! full `dumpsys meminfo` measures every process: ~1 s on an idle emulator,
//! but it hit its 10 s timeout on a busy one, exactly when a snapshot
//! matters most. So we use `ps` (~0.4 s) for all processes and
//! `dumpsys meminfo` only for the home app.

use std::cmp::Reverse;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::adb::Adb;
use crate::command::{DeviceCommand, PackageName};
use crate::error::AdbError;
use crate::inspect::{MemoryInfo, parse_meminfo, parse_properties};

/// How many processes `top_processes` lists.
pub const TOP_PROCESS_COUNT: usize = 10;

/// Serialized as the `data` of `rusttv performance snapshot --json`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PerformanceSnapshot {
    /// adb serial the snapshot was taken from.
    pub serial: String,
    /// Host time when the snapshot started, in milliseconds since the Unix epoch.
    pub taken_at_unix_ms: u64,
    /// Firmware build, so snapshots are only compared within one build.
    pub build_fingerprint: Option<String>,
    /// System memory from `/proc/meminfo`.
    pub memory: Option<MemoryInfo>,
    /// CPU usage over the sample interval.
    pub cpu: Option<CpuUsage>,
    /// The app that handles the Home button (the launcher) and its memory.
    pub home_app: Option<HomeApp>,
    /// Processes using the most resident memory, largest first.
    pub top_processes: Vec<ProcessMemory>,
}

/// Share of CPU time spent not idle, across all cores.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CpuUsage {
    /// 0–100.
    pub busy_percent: f64,
    /// Length of the sample the percentage covers.
    pub sample_ms: u64,
}

/// The current launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HomeApp {
    /// Package name, e.g. `com.google.android.apps.tv.launcherx`.
    pub package: String,
    /// Activity component as reported by Android, e.g. `.MainActivity`.
    pub activity: String,
    /// Proportional set size; `None` if the launcher isn't running.
    pub pss_kib: Option<u64>,
    /// Resident set size; `None` if the launcher isn't running.
    pub rss_kib: Option<u64>,
}

/// One row of `ps`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessMemory {
    /// Process ID.
    pub pid: u32,
    /// Process name, usually the package name for apps.
    pub name: String,
    /// Resident set size.
    pub rss_kib: u64,
}

/// Takes a performance snapshot. Read-only.
///
/// CPU usage is measured by reading `/proc/stat` twice, `cpu_sample` apart.
/// Pass `Duration::ZERO` to skip the wait (CPU usage will then be `None`).
pub fn snapshot(
    adb: &dyn Adb,
    serial: &str,
    cpu_sample: Duration,
) -> Result<PerformanceSnapshot, AdbError> {
    let taken_at_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        });

    let cpu_before = adb.run(serial, &DeviceCommand::CpuStat)?;
    thread::sleep(cpu_sample);
    let cpu_after = adb.run(serial, &DeviceCommand::CpuStat)?;
    let cpu = cpu_busy_percent(&cpu_before, &cpu_after).map(|busy_percent| CpuUsage {
        busy_percent,
        sample_ms: u64::try_from(cpu_sample.as_millis()).unwrap_or(u64::MAX),
    });

    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let memory = parse_meminfo(&adb.run(serial, &DeviceCommand::MemInfo)?);
    let mut processes = parse_process_list(&adb.run(serial, &DeviceCommand::ProcessList)?);
    processes.sort_by_key(|process| Reverse(process.rss_kib));
    processes.truncate(TOP_PROCESS_COUNT);

    let home_app = match parse_home_activity(&adb.run(serial, &DeviceCommand::HomeActivity)?) {
        Some((package, activity)) => {
            // The package name comes from the device and goes back into an
            // `adb shell` argument, so it must pass validation first.
            let usage = match PackageName::new(package) {
                Ok(name) => {
                    parse_app_memory(&adb.run(serial, &DeviceCommand::PackageMemInfo(name))?)
                }
                Err(_) => None,
            };
            Some(HomeApp {
                package: package.to_owned(),
                activity: activity.to_owned(),
                pss_kib: usage.map(|(pss, _)| pss),
                rss_kib: usage.and_then(|(_, rss)| rss),
            })
        }
        None => None,
    };

    Ok(PerformanceSnapshot {
        serial: serial.to_owned(),
        taken_at_unix_ms,
        build_fingerprint: properties.get("ro.build.fingerprint").cloned(),
        memory,
        cpu,
        home_app,
        top_processes: processes,
    })
}

/// CPU busy percentage between two `/proc/stat` readings.
///
/// Uses the aggregate `cpu` line: user nice system idle iowait irq softirq
/// steal (guest time is already included in user). Idle time is idle +
/// iowait. Returns `None` when no time passed or the input is malformed.
pub fn cpu_busy_percent(before: &str, after: &str) -> Option<f64> {
    let (busy_before, total_before) = cpu_times(before)?;
    let (busy_after, total_after) = cpu_times(after)?;
    let total = total_after.checked_sub(total_before)?;
    let busy = busy_after.checked_sub(busy_before)?;
    // Display-only ratio; jiffy counts are far below f64's exact range.
    (total > 0).then(|| busy as f64 * 100.0 / total as f64)
}

/// Returns (busy, total) jiffies from the aggregate `cpu` line.
fn cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let values: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [user, nice, system, idle, iowait, irq, softirq, steal] = values.as_slice() else {
        return None;
    };
    let idle = idle + iowait;
    let total = user + nice + system + idle + irq + softirq + steal;
    Some((total - idle, total))
}

/// Parses `ps -A -o PID,RSS,NAME`. Kernel threads (RSS 0) are skipped.
pub fn parse_process_list(output: &str) -> Vec<ProcessMemory> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let pid = columns.next()?.parse().ok()?;
            let rss_kib = columns.next()?.parse().ok()?;
            let name = columns.next()?.to_owned();
            (rss_kib > 0).then_some(ProcessMemory { pid, name, rss_kib })
        })
        .collect()
}

/// Parses `cmd package resolve-activity --brief`: the last line is
/// `<package>/<activity>`. Returns `None` if nothing handles Home.
pub fn parse_home_activity(output: &str) -> Option<(&str, &str)> {
    let component = output.lines().rev().find(|line| !line.trim().is_empty())?;
    component.trim().split_once('/')
}

/// Parses `dumpsys meminfo <package>` into (PSS, RSS) in KiB, from the line
/// `TOTAL PSS:    44081            TOTAL RSS:    78828 ...`.
/// Returns `None` when the app isn't running ("No process found for: ...").
pub fn parse_app_memory(output: &str) -> Option<(u64, Option<u64>)> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with("TOTAL PSS:"))?;
    let value_after = |label: &str| {
        let rest = &line[line.find(label)? + label.len()..];
        rest.split_whitespace().next()?.parse::<u64>().ok()
    };
    Some((value_after("TOTAL PSS:")?, value_after("TOTAL RSS:")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;

    macro_rules! capture {
        ($file:literal) => {
            include_str!(concat!("fixtures/emulator-pixel3a-api34/", $file))
        };
    }

    #[test]
    fn cpu_usage_from_two_samples() {
        // Synthetic: 100 jiffies pass, 25 of them busy.
        let before = "cpu  100 0 100 800 0 0 0 0 0 0\n";
        let after = "cpu  110 5 110 870 5 0 0 0 0 0\n";
        assert_eq!(cpu_busy_percent(before, after), Some(25.0));
    }

    #[test]
    fn cpu_usage_needs_time_to_pass() {
        let sample = capture!("proc-stat-1.txt");
        assert_eq!(cpu_busy_percent(sample, sample), None);
        assert_eq!(cpu_busy_percent("garbage", sample), None);
    }

    #[test]
    fn fully_busy_second_right_after_boot() {
        // Real capture, 1 s apart, taken seconds after boot: the idle counter
        // didn't move at all, so the CPU really was 100% busy. (Later samples
        // on the same emulator showed normal idle time.)
        let busy = cpu_busy_percent(capture!("proc-stat-1.txt"), capture!("proc-stat-2.txt"));
        assert_eq!(busy, Some(100.0));
    }

    #[test]
    fn process_list_skips_kernel_threads() {
        let processes = parse_process_list(capture!("ps.txt"));
        assert!(processes.iter().all(|process| process.rss_kib > 0));
        assert!(
            processes
                .iter()
                .any(|process| process.name == "system_server")
        );
        assert!(!processes.iter().any(|process| process.name == "[kthreadd]"));
    }

    #[test]
    fn home_activity_is_the_last_line() {
        assert_eq!(
            parse_home_activity(capture!("home-activity.txt")),
            Some((
                "com.google.android.apps.nexuslauncher",
                ".NexusLauncherActivity"
            ))
        );
        assert_eq!(parse_home_activity("No activity found\n"), None);
    }

    #[test]
    fn app_memory_totals_or_none_when_not_running() {
        assert_eq!(
            parse_app_memory(capture!("meminfo-launcher.txt")),
            Some((44_081, Some(78_828)))
        );
        assert_eq!(parse_app_memory(capture!("meminfo-not-running.txt")), None);
    }

    #[test]
    fn snapshot_of_a_real_emulator_capture() {
        let launcher = PackageName::new("com.google.android.apps.nexuslauncher").unwrap();
        let adb = FakeAdb::new()
            .respond(DeviceCommand::CpuStat, capture!("proc-stat-1.txt"))
            .respond(DeviceCommand::GetProperties, capture!("getprop.txt"))
            .respond(DeviceCommand::MemInfo, capture!("meminfo.txt"))
            .respond(DeviceCommand::ProcessList, capture!("ps.txt"))
            .respond(DeviceCommand::HomeActivity, capture!("home-activity.txt"))
            .respond(
                DeviceCommand::PackageMemInfo(launcher),
                capture!("meminfo-launcher.txt"),
            );

        let snapshot = snapshot(&adb, "emulator-5554", Duration::ZERO).unwrap();

        assert!(snapshot.taken_at_unix_ms > 0);
        assert!(
            snapshot
                .build_fingerprint
                .unwrap()
                .starts_with("google/sdk_gphone64_arm64")
        );
        assert_eq!(snapshot.memory.unwrap().total_kib, 2_026_264);
        assert_eq!(snapshot.cpu, None, "zero-length sample");
        assert_eq!(
            snapshot.home_app,
            Some(HomeApp {
                package: "com.google.android.apps.nexuslauncher".to_owned(),
                activity: ".NexusLauncherActivity".to_owned(),
                pss_kib: Some(44_081),
                rss_kib: Some(78_828),
            })
        );
        assert_eq!(snapshot.top_processes.len(), TOP_PROCESS_COUNT);
        assert!(
            snapshot
                .top_processes
                .windows(2)
                .all(|pair| pair[0].rss_kib >= pair[1].rss_kib)
        );
    }

    #[test]
    fn a_home_package_that_fails_validation_is_never_sent_back_to_the_device() {
        // FakeAdb has no PackageMemInfo response: if snapshot() tried to run
        // one, the snapshot would fail.
        let adb = FakeAdb::new()
            .respond(DeviceCommand::CpuStat, capture!("proc-stat-1.txt"))
            .respond(DeviceCommand::GetProperties, "")
            .respond(DeviceCommand::MemInfo, "")
            .respond(DeviceCommand::ProcessList, "")
            .respond(DeviceCommand::HomeActivity, "evil;reboot/.Activity\n");

        let home = snapshot(&adb, "tv", Duration::ZERO)
            .unwrap()
            .home_app
            .unwrap();
        assert_eq!(home.package, "evil;reboot");
        assert_eq!(home.pss_kib, None);
    }
}

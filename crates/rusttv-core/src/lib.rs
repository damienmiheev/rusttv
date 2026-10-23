//! Shared core for RustTV.
//!
//! All device access goes through this crate: build a [`DeviceCommand`] and
//! run it with an [`Adb`] implementation. Anything that changes a device
//! persistently goes through the safety engine ([`Plan`], [`apply`]). See ARCHITECTURE.md for the modules
//! still to come (`device`, `packages`, `perf`, ...).

mod adb;
mod apps;
mod bench;
mod command;
mod compat;
mod device;
mod error;
mod inspect;
mod install;
mod journal;
mod launcher;
mod perf;
mod plan;
mod takeover;
#[cfg(test)]
mod test_support;

#[cfg(any(test, feature = "test-util"))]
pub use adb::FakeAdb;
pub use adb::{Adb, SystemAdb};
pub use apps::{AppInfo, AppKind, list_apps, parse_package_list};
pub use bench::{
    AmStart, AppResult, BenchmarkDevice, BenchmarkError, BenchmarkOptions, BenchmarkReport,
    LaunchKind, LaunchRun, Stats, app_cpu_seconds, parse_am_start, parse_cpu_time, run_benchmark,
};
pub use command::{
    ApkPath, Component, DeviceCommand, InvalidApk, InvalidComponent, InvalidPackageName,
    PackageName, RiskClass,
};
pub use compat::{
    DeviceKind, DeviceProfile, MissingField, PROFILE_SCHEMA_VERSION, ProfileDevice,
    ProfileFirmware, ProfileHardware, ProfileLauncher, ProfileTested, bundled_profiles,
    firmware_family, profile_file_name, profile_from_info,
};
pub use device::{
    ConnectOutcome, DEFAULT_ADB_PORT, DeviceAddress, DeviceEntry, DeviceSelectionError,
    DeviceState, InvalidDeviceAddress, parse_connect_output, parse_device_list, select_device,
};
pub use error::{AdbError, ERROR_CODES};
pub use inspect::{
    DeviceInfo, MemoryInfo, StorageInfo, inspect, parse_df, parse_meminfo, parse_properties,
};
pub use install::{
    LAUNCHER_INSTALL, LAUNCHER_UNINSTALL, default_apk, install_plan, uninstall_plan,
};
pub use journal::{
    FileJournal, JOURNAL_SCHEMA_VERSION, Journal, JournalStatus, JournalStep, JournalStore,
    StepStatus, find_restorable_journal, journal_file_name, journals_dir, load_journal,
};
pub use launcher::{
    HomeCandidate, LauncherStatus, RUSTTV_LAUNCHER, launcher_status, parse_home_candidates,
    parse_resumed_activity,
};
pub use plan::{
    Approval, Check, DryRun, DryRunStep, Plan, PlanError, Step, apply, device_fingerprint, restore,
    restore_plan,
};
pub use takeover::{HOME_TAKEOVER, takeover_plan, takeover_restore_plan};

pub use perf::{
    CpuUsage, HomeApp, PerformanceSnapshot, ProcessMemory, TOP_PROCESS_COUNT, cpu_busy_percent,
    parse_app_memory, parse_home_activity, parse_process_list, snapshot,
};
/// Version of the core crate, shown by `rusttv --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn error_codes_list_matches_the_codes_errors_produce() {
        let produced: BTreeSet<&str> = error::every_adb_error()
            .iter()
            .map(AdbError::code)
            .chain(
                device::every_selection_error()
                    .iter()
                    .map(DeviceSelectionError::code),
            )
            .chain(
                bench::every_benchmark_error()
                    .iter()
                    .map(BenchmarkError::code),
            )
            .chain(plan::every_plan_error().iter().map(PlanError::code))
            .collect();
        let listed: BTreeSet<&str> = ERROR_CODES.iter().copied().collect();

        assert_eq!(
            listed.len(),
            ERROR_CODES.len(),
            "duplicate entries in ERROR_CODES"
        );
        assert_eq!(produced, listed);
    }
}

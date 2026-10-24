//! Shared core for RustTV.
//!
//! All device access goes through this crate: build a [`DeviceCommand`] and
//! run it with an [`Adb`] implementation. Anything that changes a device
//! persistently goes through the safety engine ([`Plan`], [`apply`]). See ARCHITECTURE.md for the modules
//! still to come (`device`, `packages`, `perf`, ...).

mod adb;
mod apps;
mod bench;
mod catalog;
mod command;
mod compat;
mod device;
mod download;
mod error;
mod grant;
mod history;
mod inspect;
mod terminal;
mod install;
mod journal;
mod launcher;
mod optimizer;
mod perf;
mod plan;
mod privacy;
mod takeover;
#[cfg(any(test, feature = "test-util"))]
pub mod test_release;
#[cfg(test)]
mod test_support;
mod update;

#[cfg(any(test, feature = "test-util"))]
pub use adb::FakeAdb;
pub use adb::{Adb, SystemAdb};
pub use apps::{AppInfo, AppKind, PackageListError, list_apps, parse_package_list};
pub use bench::{
    AmStart, AppResult, BenchmarkDevice, BenchmarkError, BenchmarkOptions, BenchmarkReport,
    LaunchKind, LaunchRun, Stats, app_cpu_seconds, parse_am_start, parse_cpu_time, run_benchmark,
};
pub use catalog::{
    CATALOG_SCHEMA_VERSION, CatalogEntry, LoadedChoices, PrivacyChoices, bundled_catalog,
    config_dir, load_choices,
};
pub use command::{
    ApkPath, Component, DeviceCommand, DoorKind, HistoryNonce, HistorySince, InboxPayload,
    InvalidApk, InvalidComponent, InvalidHistoryValue, InvalidPackageName, InvalidSettingValue,
    PackageName, RiskClass, SettingKey, SettingState, SettingValue, WRITE_SECURE_SETTINGS,
    WatchKey,
};
pub use history::{
    HISTORY_ALLOW_TV_SETTINGS, HISTORY_RESTORE, HISTORY_RESTORE_TV, HISTORY_REVOKE_TV_SETTINGS,
    RestoreTarget, TvChange, approve_by_tv, history_restore_plan, history_restore_undo_plan,
    is_export, launcher_grant, launcher_test_build, refuse_overlapping_journals,
    restore_tv_plan, same_scale, tv_settings_plan,
};
#[cfg(all(feature = "test-util", debug_assertions))]
pub use terminal::TEST_TERMINAL_ENV;
pub use terminal::stdin_is_terminal;
pub use compat::{
    DeviceKind, DeviceProfile, InvalidPrivacyCategory, MissingField, PROFILE_SCHEMA_VERSION,
    PrivacyCategory, ProfileDevice, ProfileFirmware, ProfileHardware, ProfileLauncher,
    ProfileOptimizer, ProfilePrivacy, ProfileTested, TestedPackage, bundled_profiles,
    firmware_family, profile_file_name, profile_from_info,
};
pub use device::{
    ConnectOutcome, DEFAULT_ADB_PORT, DeviceAddress, DeviceEntry, DeviceSelectionError,
    DeviceState, InvalidDeviceAddress, parse_connect_output, parse_device_list, select_device,
};
pub use download::{Curl, LocalRelease, ReleaseSource, download_verified, latest_release};
pub use error::{AdbError, ERROR_CODES};
pub use grant::{
    GRANT_SCHEMA_VERSION, Grant, GrantScope, InvalidGrantScope, MAX_GRANT_DURATION, grant_path,
    load_grant, revoke_grant, save_grant,
};
pub use inspect::{
    DeviceInfo, MemoryInfo, StorageInfo, inspect, parse_df, parse_meminfo, parse_properties,
};
pub use install::{
    LAUNCHER_INSTALL, LAUNCHER_UNINSTALL, default_apk, install_apk, install_plan,
    release_install_plan, uninstall_plan,
};
pub use journal::{
    ApprovedBy, FileJournal, JOURNAL_SCHEMA_VERSION, Journal, JournalSearch, JournalStatus,
    JournalStep, JournalStore, RestorableJournals, StepStatus, UnreadableJournal,
    find_restorable_journal, find_restorable_journal_where, journal_file_name, journals_dir,
    load_journal, restorable_journals, state_dir,
};
pub use launcher::{
    HomeCandidate, LauncherStatus, RUSTTV_LAUNCHER, launcher_status, parse_home_candidates,
    parse_resumed_activity,
};
pub use optimizer::{
    InvalidOptimizeKind, OPTIMIZE_PLAN_PREFIX, OPTIMIZER_SCHEMA_VERSION, OptimizeJournalSearch,
    OptimizeKind, OptimizerProfile, OptimizerState, OptimizerStatus, ProfileSetting,
    REDUCED_ANIMATIONS, SettingStatus, bundled_optimizer_profile, bundled_optimizer_profiles,
    find_optimize_journal, optimize_restore_plan, optimizer_plan, optimizer_restore_plan,
    profile_of_plan,
};
pub use plan::{
    Approval, Check, DryRun, DryRunStep, Plan, PlanError, Step, apply, device_fingerprint, restore,
    restore_plan,
};
pub use privacy::{
    OptimizeAnalysis, PRIVACY, PackageSource, PackageState, PrivacyData, PrivacyPackageStatus,
    optimize_analyze, privacy_plan, privacy_restore_plan, statically_protected,
};
pub use takeover::{
    HOME_TAKEOVER, takeover_journal_from_profile, takeover_plan, takeover_restore_plan,
};

#[cfg(all(feature = "test-util", debug_assertions))]
pub use update::TEST_SIGNING_KEY_ENV;
pub use update::{
    LauncherOnDevice, MAX_ASSET_BYTES, MAX_METADATA_BYTES, RELEASE_SIGNING_KEY,
    RELEASES_LATEST_URL, Release, ReleaseAsset, SUMS_FILE, SUMS_SIGNATURE_FILE, Sums, UpdateError,
    Version, cli_archive_dir, cli_archive_name, launcher_apk_name, launcher_on_device,
    launcher_update_asset, parse_release, parse_sums, parse_version_name, release_signing_key,
    sha256, verify_asset, verify_sums,
};

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
            .chain(update::every_update_error().iter().map(UpdateError::code))
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

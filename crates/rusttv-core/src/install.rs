//! `rusttv launcher install` / `uninstall`: put the RustTV launcher on a
//! device and take it off again (SECURITY.md § Home takeover, rule 6).
//!
//! Both are [`Plan`]s run by the safety engine in [`crate::plan`]. They only
//! touch RustTV's own package: the commands take no package name.

use std::path::{Path, PathBuf};

use crate::adb::Adb;
use crate::apps::package_names;
use crate::command::{ApkPath, DeviceCommand, PackageName};
use crate::inspect::parse_properties;
use crate::journal::find_restorable_journal;
use crate::launcher::{RUSTTV_LAUNCHER, launcher_status};
use crate::plan::{Check, Plan, PlanError, Step, device_fingerprint};
use crate::takeover::HOME_TAKEOVER;
use crate::update::Version;

/// Plan name of `launcher install`, also used in journal file names.
pub const LAUNCHER_INSTALL: &str = "launcher-install";
/// Plan name of `launcher uninstall`, also used in journal file names.
pub const LAUNCHER_UNINSTALL: &str = "launcher-uninstall";

/// ABIs `android/build.sh` builds a launcher APK for.
const APK_ABIS: &[&str] = &["arm64-v8a", "armeabi-v7a", "x86_64"];

/// The APK `android/build.sh` builds for `abi`, relative to the repository
/// root: `android/app/build/outputs/apk/release/app-<abi>-release.apk`.
/// `None` for an ABI no APK is built for.
pub fn default_apk(abi: &str) -> Option<PathBuf> {
    APK_ABIS.contains(&abi).then(|| {
        PathBuf::from(format!(
            "android/app/build/outputs/apk/release/app-{abi}-release.apk"
        ))
    })
}

/// Plans installing (or upgrading) the RustTV launcher. Read-only.
///
/// `apk` is the APK to install; without it, [`default_apk`] for the device's
/// primary ABI (first in `ro.product.cpu.abilist`), relative to the current
/// directory. `journals` is the journal directory, to find an active Home
/// takeover.
///
/// - Not installed yet: one step, `adb install`, undone by uninstalling.
/// - Already installed: an upgrade (`adb install -r`), which can't be undone
///   (see [`DeviceCommand::is_irreversible`]) and is refused while Home
///   depends on RustTV, since a broken upgrade would leave no working Home.
///
/// Either way the step is verified by RustTV being installed and a Home candidate.
pub fn install_plan(
    adb: &dyn Adb,
    serial: &str,
    apk: Option<&Path>,
    journals: &Path,
) -> Result<Plan, PlanError> {
    let source = match apk {
        Some(path) => ApkSource::Given(path),
        None => ApkSource::Built,
    };
    plan_install(adb, serial, source, journals)
}

/// Plans installing or upgrading the RustTV launcher from `apk`, the APK of
/// release `version` that `rusttv update launcher` downloaded for the
/// device's primary ABI and verified. The same plan as [`install_plan`] with
/// `Some(apk)`; only its notes say where the APK comes from. Read-only.
pub fn release_install_plan(
    adb: &dyn Adb,
    serial: &str,
    apk: &Path,
    version: Version,
    journals: &Path,
) -> Result<Plan, PlanError> {
    plan_install(adb, serial, ApkSource::Release(apk, version), journals)
}

/// Where the APK of an install plan comes from.
enum ApkSource<'a> {
    /// `android/build.sh`'s output for the device's primary ABI.
    Built,
    /// `--apk <file>`.
    Given(&'a Path),
    /// Downloaded from a release and verified (`rusttv update launcher`).
    Release(&'a Path, Version),
}

fn plan_install(
    adb: &dyn Adb,
    serial: &str,
    source: ApkSource<'_>,
    journals: &Path,
) -> Result<Plan, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let fingerprint = properties
        .get("ro.build.fingerprint")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            PlanError::precondition("the device doesn't report `ro.build.fingerprint`".to_owned())
        })?;
    let abis = properties
        .get("ro.product.cpu.abilist")
        .map_or("", String::as_str);

    let mut notes = Vec::new();
    let apk = match source {
        ApkSource::Given(path) => {
            notes.push(format!(
                "The APK was chosen with --apk; its ABI isn't checked against the device's \
                 (`{abis}`)."
            ));
            ApkPath::from_local_file(path)?
        }
        ApkSource::Release(path, version) => {
            notes.push(format!(
                "APK of release {version} for the device's primary ABI, downloaded and verified \
                 (signed SHA256SUMS, SHA-256, size); Android also refuses an upgrade signed with \
                 another key than the installed launcher's."
            ));
            ApkPath::from_local_file(path)?
        }
        ApkSource::Built => {
            let (apk, primary) = built_apk(abis)?;
            notes.push(format!("APK for the device's primary ABI, {primary}."));
            apk
        }
    };

    let rusttv = PackageName::rusttv_launcher();
    let checks = vec![Check::HomeCandidate(rusttv)];
    let step = if is_installed(adb, serial)? {
        home_without_rusttv(adb, serial, &fingerprint, journals)?;
        notes.insert(
            0,
            format!(
                "{RUSTTV_LAUNCHER} is already installed: this is an upgrade (`adb install -r`), \
                 which keeps its data. It can't be undone automatically. If installing fails, \
                 Android keeps the installed version; if the new version installs but fails the \
                 checks, it stays installed: then `rusttv launcher uninstall` and install an \
                 older APK."
            ),
        );
        Step::new(DeviceCommand::UpgradeLauncher(apk), checks)
    } else {
        notes.push(format!(
            "If the checks fail, {RUSTTV_LAUNCHER} is uninstalled again. Remove it later with \
             `rusttv launcher uninstall`."
        ));
        Step::new(DeviceCommand::InstallLauncher(apk), checks)
    };
    notes.push(
        "Installing doesn't change which app Home opens or touch the stock launcher.".to_owned(),
    );
    Plan::new(
        LAUNCHER_INSTALL,
        serial,
        &fingerprint,
        "install",
        vec![step],
        notes,
    )
}

/// The APK [`install_plan`] with the same `apk` would install on the
/// device, checked the same way (`INVALID_APK` if there is none). Read-only;
/// lets `rusttv launcher upgrade` refuse before it changes anything.
pub fn install_apk(adb: &dyn Adb, serial: &str, apk: Option<&Path>) -> Result<ApkPath, PlanError> {
    match apk {
        Some(path) => Ok(ApkPath::from_local_file(path)?),
        None => {
            let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
            let abis = properties
                .get("ro.product.cpu.abilist")
                .map_or("", String::as_str);
            Ok(built_apk(abis)?.0)
        }
    }
}

/// `android/build.sh`'s APK for the primary ABI of `abis`
/// (`ro.product.cpu.abilist`), and that ABI.
fn built_apk(abis: &str) -> Result<(ApkPath, &str), PlanError> {
    let primary = abis.split(',').next().unwrap_or_default();
    if primary.is_empty() {
        return Err(PlanError::invalid_apk(
            "the device doesn't report `ro.product.cpu.abilist`; pass --apk <file>".to_owned(),
        ));
    }
    let path = default_apk(primary).ok_or_else(|| {
        PlanError::invalid_apk(format!(
            "no RustTV APK is built for the device's primary ABI `{primary}` \
             (android/build.sh builds {}); pass --apk <file>",
            APK_ABIS.join(", ")
        ))
    })?;
    let apk = ApkPath::from_local_file(&path).map_err(|error| {
        PlanError::invalid_apk(format!(
            "{error}; build it with android/build.sh and run rusttv from the repository \
             root, or pass --apk <file>"
        ))
    })?;
    Ok((apk, primary))
}

/// Plans uninstalling the RustTV launcher. Read-only.
///
/// Refused while a Home takeover is active (SECURITY.md rule 6): by a
/// restorable takeover journal in `journals` for this firmware, or, for a
/// takeover from elsewhere, because no system launcher other than RustTV can
/// handle Home. The person runs `rusttv launcher restore` first, so the
/// stock launcher is back before RustTV goes. After uninstalling, Home must
/// open that launcher.
pub fn uninstall_plan(adb: &dyn Adb, serial: &str, journals: &Path) -> Result<Plan, PlanError> {
    let fingerprint = device_fingerprint(adb, serial)?;
    if !is_installed(adb, serial)? {
        return Err(PlanError::precondition(format!(
            "the RustTV launcher ({RUSTTV_LAUNCHER}) isn't installed; nothing to uninstall"
        )));
    }
    let home = home_without_rusttv(adb, serial, &fingerprint, journals)?;
    let notes = vec![
        format!(
            "Uninstalls {RUSTTV_LAUNCHER} and deletes its data (saved tile order). This can't be \
             undone automatically; reinstall with `rusttv launcher install`."
        ),
        format!("The stock launcher isn't touched; Home must open {home} afterwards."),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
    ];
    Plan::new(
        LAUNCHER_UNINSTALL,
        serial,
        &fingerprint,
        "uninstall",
        vec![Step::new(
            DeviceCommand::UninstallLauncher,
            vec![Check::HomeIs(home)],
        )],
        notes,
    )
}

fn is_installed(adb: &dyn Adb, serial: &str) -> Result<bool, PlanError> {
    Ok(Check::PackageInstalled(PackageName::rusttv_launcher()).holds(adb, serial)?)
}

/// The package Home will open without RustTV; refuses if that would be
/// Android's fallback Home (a Home takeover is active).
fn home_without_rusttv(
    adb: &dyn Adb,
    serial: &str,
    fingerprint: &str,
    journals: &Path,
) -> Result<PackageName, PlanError> {
    let search = find_restorable_journal(journals, HOME_TAKEOVER, fingerprint);
    match search.found {
        Ok((path, journal)) => {
            return Err(PlanError::precondition(format!(
                "a Home takeover is still active (journal {}, {}), so Home depends on RustTV; \
                 run `rusttv launcher restore` first",
                path.display(),
                journal.status
            )));
        }
        Err(error) if error.code() == "NO_JOURNAL" => {}
        Err(error) => return Err(error),
    }
    // An unreadable file may be an unrestored takeover journal: refuse rather
    // than guess (SECURITY.md rule 6).
    if !search.unreadable.is_empty() {
        let paths: Vec<String> = search
            .unreadable
            .iter()
            .map(|file| file.path.display().to_string())
            .collect();
        let reasons: Vec<String> = search
            .unreadable
            .iter()
            .map(|file| file.error.to_string())
            .collect();
        return Err(PlanError::precondition(format!(
            "can't read journal file(s) {}, and any of them may be an unrestored Home takeover. \
             Check each file; if a takeover is still active, run `rusttv launcher restore \
             --journal <file>` (after fixing it) or `rusttv launcher restore --from-profile`. \
             Then move the file out of {}. Reason: {}",
            paths.join(", "),
            journals.display(),
            reasons.join("; ")
        )));
    }

    let status = launcher_status(adb, serial)?;
    let third_party = adb.run(serial, &DeviceCommand::ListThirdPartyPackages)?;
    let third_party = package_names(&third_party)?;
    // Candidates are sorted highest priority first. Android's fallback Home
    // (TV Settings' FallbackHome) has a negative priority.
    let system_launcher = status
        .candidates
        .iter()
        .find(|candidate| {
            let package = candidate.package();
            package != RUSTTV_LAUNCHER && candidate.priority >= 0 && !third_party.contains(package)
        })
        .map(|candidate| candidate.package())
        .ok_or_else(|| {
            PlanError::precondition(
                "no system launcher other than RustTV can handle Home (is the stock launcher \
                 disabled?), so without RustTV only Android's fallback Home would be left. \
                 Enable the stock launcher first: `rusttv launcher restore`, or Settings > Apps \
                 > See all apps > Show system apps > Enable"
                    .to_owned(),
            )
        })?;
    // Uninstalling an app that isn't Home doesn't change Home; a chooser
    // (`android/...ResolverActivity`) or RustTV gives way to the system launcher.
    let current = status
        .home
        .as_deref()
        .and_then(|home| home.split_once('/'))
        .map(|(package, _)| package)
        .filter(|&package| package != RUSTTV_LAUNCHER && package != "android");
    let expected = current.unwrap_or(system_launcher);
    PackageName::new(expected)
        .map_err(|error| PlanError::precondition(format!("Home resolver: {error}")))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::adb::FakeAdb;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{FileJournal, Journal, JournalStatus, JournalStep, JournalStore};
    use crate::plan::{apply, restore_plan};
    use crate::{RiskClass, StepStatus};

    const SERIAL: &str = "emulator-5554";

    macro_rules! real {
        ($file:literal) => {
            include_str!(concat!("fixtures/google-tv-emulator-api34/", $file)).to_owned()
        };
    }
    macro_rules! synthetic {
        ($file:literal) => {
            include_str!(concat!("fixtures/synthetic-launcher-install/", $file)).to_owned()
        };
    }
    macro_rules! takeover {
        ($file:literal) => {
            include_str!(concat!("fixtures/synthetic-home-takeover/", $file)).to_owned()
        };
    }

    /// A temp directory holding `app-arm64-v8a-release.apk` (not a real APK)
    /// and an empty `journals/`.
    struct Dir(PathBuf);

    impl Dir {
        fn new(test: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("rusttv-install-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("journals")).unwrap();
            fs::write(dir.join("app-arm64-v8a-release.apk"), b"synthetic").unwrap();
            Self(dir)
        }
        fn apk(&self) -> PathBuf {
            self.0.join("app-arm64-v8a-release.apk")
        }
        fn journals(&self) -> PathBuf {
            self.0.join("journals")
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn apk_path(dir: &Dir) -> ApkPath {
        ApkPath::from_local_file(&dir.apk()).unwrap()
    }

    /// The Google TV emulator. `packages` is `pm list packages -f` on each
    /// call, `candidates` the Home candidates on each call.
    fn tv(dir: &Dir, packages: Vec<String>, candidates: Vec<String>) -> FakeAdb {
        FakeAdb::new()
            .respond(DeviceCommand::GetProperties, takeover!("getprop.txt"))
            .respond_sequence(DeviceCommand::ListPackages, packages)
            .respond_sequence(DeviceCommand::HomeCandidates, candidates)
            .respond(DeviceCommand::HomeActivity, real!("home-activity.txt"))
            .respond(
                DeviceCommand::ListThirdPartyPackages,
                takeover!("packages-3.txt"),
            )
            .respond(
                DeviceCommand::InstallLauncher(apk_path(dir)),
                synthetic!("adb-install.txt"),
            )
            .respond(
                DeviceCommand::UpgradeLauncher(apk_path(dir)),
                synthetic!("adb-install.txt"),
            )
            .respond(
                DeviceCommand::UninstallLauncher,
                synthetic!("pm-uninstall.txt"),
            )
    }

    fn changes(adb: &FakeAdb) -> Vec<DeviceCommand> {
        adb.calls()
            .into_iter()
            .filter(|command| command.risk() > RiskClass::Read)
            .collect()
    }

    fn takeover_journal(dir: &Dir, status: JournalStatus) {
        let stock = PackageName::new("com.google.android.apps.tv.launcherx").unwrap();
        let fingerprint = "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240625.001.K5/12345678:userdebug/dev-keys";
        FileJournal::new(dir.journals().join("0000000000001-home-takeover.toml"))
            .save(&Journal {
                schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
                plan: HOME_TAKEOVER.to_owned(),
                serial: SERIAL.to_owned(),
                fingerprint: fingerprint.to_owned(),
                started_at_unix_ms: 1,
                status,
                approved_by: None,
                restore_approved_by: None,
                steps: vec![JournalStep {
                    adb: String::new(),
                    command: DeviceCommand::DisablePackage(stock.clone()),
                    original_state: Some(Check::PackageEnabled(stock)),
                    status: StepStatus::Done,
                }],
            })
            .unwrap();
    }

    #[test]
    fn default_apks_follow_the_build_script_names() {
        assert_eq!(
            default_apk("armeabi-v7a").unwrap(),
            Path::new("android/app/build/outputs/apk/release/app-armeabi-v7a-release.apk")
        );
        assert!(default_apk("x86_64").is_some());
        assert!(default_apk("arm64-v8a").is_some());
        assert_eq!(default_apk("x86"), None);
        assert_eq!(default_apk("../../evil"), None);
    }

    #[test]
    fn install_plan_installs_verifies_and_undoes_by_uninstalling() {
        let dir = Dir::new("plan");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-no-rusttv.txt")],
            vec![synthetic!("home-candidates-no-rusttv.txt")],
        );
        let plan = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap();
        let dry_run = plan.dry_run();

        assert_eq!(dry_run.plan, "launcher-install");
        assert_eq!(dry_run.max_risk, "R3");
        assert_eq!(dry_run.confirmation, "install");
        let [step] = dry_run.steps.as_slice() else {
            panic!("expected one step: {dry_run:?}");
        };
        assert_eq!(
            step.command,
            format!(
                "adb -s emulator-5554 install --pkg ai.rusttv.launcher {}",
                apk_path(&dir)
            )
        );
        assert_eq!(
            step.records.as_deref(),
            Some("ai.rusttv.launcher is not installed")
        );
        assert_eq!(
            step.verify,
            [
                "ai.rusttv.launcher is installed",
                "ai.rusttv.launcher can handle Home"
            ]
        );
        assert_eq!(
            step.undo.as_deref(),
            Some("adb -s emulator-5554 shell pm uninstall ai.rusttv.launcher")
        );
        assert!(changes(&adb).is_empty(), "planning is read-only");
    }

    #[test]
    fn a_release_install_plan_is_the_same_plan_with_its_own_note() {
        let dir = Dir::new("release-plan");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-no-rusttv.txt")],
            vec![synthetic!("home-candidates-no-rusttv.txt")],
        );
        let given = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap();
        let version = Version::parse("0.2.0").unwrap();
        let release =
            release_install_plan(&adb, SERIAL, &dir.apk(), version, &dir.journals()).unwrap();
        let (given, release) = (given.dry_run(), release.dry_run());

        assert_eq!(release.plan, given.plan);
        assert_eq!(release.confirmation, given.confirmation);
        assert_eq!(release.steps, given.steps);
        assert!(given.notes[0].contains("--apk"), "{:?}", given.notes);
        assert!(
            release.notes[0].contains("release 0.2.0"),
            "{:?}",
            release.notes
        );
        assert!(!release.notes.iter().any(|note| note.contains("--apk")));
        assert!(changes(&adb).is_empty(), "planning is read-only");
    }

    #[test]
    fn install_applies_and_verifies() {
        let dir = Dir::new("apply");
        let adb = tv(
            &dir,
            vec![
                synthetic!("packages-f-no-rusttv.txt"),
                synthetic!("packages-f-no-rusttv.txt"),
                synthetic!("packages-f-rusttv.txt"),
            ],
            vec![real!("home-candidates.txt")],
        );
        let plan = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap();
        let approval = plan.approve("install").unwrap();
        let mut store = MemoryJournal::default();

        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(
            changes(&adb),
            [DeviceCommand::InstallLauncher(apk_path(&dir))]
        );
    }

    #[test]
    fn an_install_that_is_no_home_candidate_is_uninstalled_again() {
        let dir = Dir::new("rollback");
        let adb = tv(
            &dir,
            vec![
                synthetic!("packages-f-no-rusttv.txt"),
                synthetic!("packages-f-no-rusttv.txt"),
                synthetic!("packages-f-rusttv.txt"),
                synthetic!("packages-f-no-rusttv.txt"),
            ],
            // Installed, but no activity handles Home (e.g. a wrong build).
            vec![synthetic!("home-candidates-no-rusttv.txt")],
        );
        let plan = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap();
        let approval = plan.approve("install").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(error.to_string().contains("can handle Home"), "{error}");
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::InstallLauncher(apk_path(&dir)),
                DeviceCommand::UninstallLauncher
            ]
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(store.last().steps[0].status, StepStatus::Undone);
    }

    #[test]
    fn installing_over_an_installed_launcher_is_an_irreversible_upgrade() {
        let dir = Dir::new("upgrade");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-rusttv.txt")],
            vec![real!("home-candidates.txt")],
        );
        let plan = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap();
        let dry_run = plan.dry_run();
        let step = &dry_run.steps[0];
        assert!(
            step.command
                .contains(" install -r --pkg ai.rusttv.launcher ")
        );
        assert_eq!(step.undo, None);
        assert_eq!(step.records, None);
        assert!(dry_run.notes[0].contains("upgrade"), "{:?}", dry_run.notes);
        assert!(dry_run.notes[0].contains("can't be undone"));
    }

    #[test]
    fn no_upgrade_or_uninstall_while_a_takeover_journal_is_active() {
        for status in [JournalStatus::Applied, JournalStatus::RollbackFailed] {
            let dir = Dir::new("takeover-journal");
            takeover_journal(&dir, status);
            let adb = tv(
                &dir,
                vec![synthetic!("packages-f-rusttv.txt")],
                vec![real!("home-candidates.txt")],
            );
            let error = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap_err();
            assert_eq!(error.code(), "PRECONDITION_FAILED");
            assert!(error.to_string().contains("launcher restore"), "{error}");
            let error = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap_err();
            assert!(error.to_string().contains("launcher restore"), "{error}");
        }
        // A restored takeover is over.
        let dir = Dir::new("takeover-restored");
        takeover_journal(&dir, JournalStatus::Restored);
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-rusttv.txt")],
            vec![real!("home-candidates.txt")],
        );
        assert!(uninstall_plan(&adb, SERIAL, &dir.journals()).is_ok());
    }

    #[test]
    fn no_upgrade_or_uninstall_while_a_journal_file_is_unreadable() {
        // Even next to a restored takeover journal: the unreadable file may be
        // a newer takeover that was never restored.
        let dir = Dir::new("unreadable-journal");
        takeover_journal(&dir, JournalStatus::Restored);
        let corrupt = dir.journals().join("0000000000002-home-takeover.toml");
        fs::write(&corrupt, "not a journal").unwrap();
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-rusttv.txt")],
            vec![real!("home-candidates.txt")],
        );
        let error = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        let message = error.to_string();
        assert!(
            message.contains(&corrupt.display().to_string()),
            "{message}"
        );
        assert!(message.contains("--from-profile"), "{message}");
        let error = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");

        // Moved aside, the uninstall plans again.
        fs::remove_file(&corrupt).unwrap();
        assert!(uninstall_plan(&adb, SERIAL, &dir.journals()).is_ok());
    }

    #[test]
    fn no_upgrade_or_uninstall_when_the_stock_launcher_is_disabled() {
        // A takeover from another computer: no journal here, but only RustTV,
        // a third-party spike and the fallback Home can handle Home.
        let dir = Dir::new("stock-disabled");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-rusttv.txt")],
            vec![synthetic!("home-candidates-stock-disabled.txt")],
        )
        .respond(
            DeviceCommand::HomeActivity,
            takeover!("home-activity-rusttv.txt"),
        );
        let error = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("fallback Home"), "{error}");
        let error = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap_err();
        assert!(error.to_string().contains("fallback Home"), "{error}");
    }

    #[test]
    fn uninstall_removes_rusttv_and_checks_home_opens_the_stock_launcher() {
        let dir = Dir::new("uninstall");
        let adb = tv(
            &dir,
            vec![
                synthetic!("packages-f-rusttv.txt"),
                synthetic!("packages-f-no-rusttv.txt"),
            ],
            vec![real!("home-candidates.txt")],
        );
        let plan = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap();
        let dry_run = plan.dry_run();
        assert_eq!(dry_run.plan, "launcher-uninstall");
        assert_eq!(dry_run.confirmation, "uninstall");
        assert_eq!(
            dry_run.steps[0].command,
            "adb -s emulator-5554 shell pm uninstall ai.rusttv.launcher"
        );
        assert_eq!(
            dry_run.steps[0].verify,
            [
                "ai.rusttv.launcher is not installed",
                "Home opens com.google.android.apps.tv.launcherx"
            ]
        );
        assert_eq!(dry_run.steps[0].undo, None);

        let approval = plan.approve("uninstall").unwrap();
        let mut store = MemoryJournal::default();
        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(changes(&adb), [DeviceCommand::UninstallLauncher]);

        // An uninstall journal can't be "restored".
        let error = restore_plan(&journal, SERIAL, "f").unwrap_err();
        assert_eq!(error.code(), "INVALID_PLAN");
    }

    #[test]
    fn rusttv_as_home_gives_way_to_the_system_launcher() {
        // A device where RustTV is Home by the user's choice, not a takeover.
        let dir = Dir::new("rusttv-home");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-rusttv.txt")],
            vec![real!("home-candidates.txt")],
        )
        .respond(
            DeviceCommand::HomeActivity,
            takeover!("home-activity-rusttv.txt"),
        );
        let plan = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap();
        assert_eq!(
            plan.dry_run().steps[0].verify[1],
            "Home opens com.google.android.apps.tv.launcherx"
        );
    }

    #[test]
    fn uninstall_of_a_missing_launcher_is_refused() {
        let dir = Dir::new("missing");
        let adb = tv(
            &dir,
            vec![synthetic!("packages-f-no-rusttv.txt")],
            vec![synthetic!("home-candidates-no-rusttv.txt")],
        );
        let error = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(
            error.to_string().contains("nothing to uninstall"),
            "{error}"
        );
    }

    #[test]
    fn a_failing_package_list_is_not_read_as_rusttv_missing() {
        let dir = Dir::new("pm-fails");
        let failure =
            include_str!("fixtures/synthetic-pm-list-errors/error-not-running.txt").to_owned();
        let adb = tv(&dir, vec![failure], vec![real!("home-candidates.txt")]);

        let error = uninstall_plan(&adb, SERIAL, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PACKAGE_LIST_FAILED");
        let error = install_plan(&adb, SERIAL, Some(&dir.apk()), &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "PACKAGE_LIST_FAILED");
        assert!(changes(&adb).is_empty());
    }

    #[test]
    fn a_missing_or_unsafe_apk_is_refused_before_reading_packages() {
        let dir = Dir::new("bad-apk");
        let adb = tv(&dir, vec![], vec![]);
        let error = install_plan(
            &adb,
            SERIAL,
            Some(&dir.0.join("missing.apk")),
            &dir.journals(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "INVALID_APK");

        let unsafe_name = dir.0.join("app;reboot.apk");
        fs::write(&unsafe_name, b"x").unwrap();
        let error = install_plan(&adb, SERIAL, Some(&unsafe_name), &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "INVALID_APK");
        assert_eq!(
            adb.calls(),
            [DeviceCommand::GetProperties, DeviceCommand::GetProperties]
        );
    }

    #[test]
    fn the_default_apk_needs_a_built_abi() {
        let dir = Dir::new("abi");
        let x86 = tv(&dir, vec![], vec![]).respond(
            DeviceCommand::GetProperties,
            takeover!("getprop.txt").replace("[arm64-v8a]", "[x86,armeabi-v7a]"),
        );
        let error = install_plan(&x86, SERIAL, None, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "INVALID_APK");
        assert!(error.to_string().contains("primary ABI `x86`"), "{error}");

        // arm64: the default path is relative to the current directory, where
        // tests run without a build, so it isn't there.
        let arm64 = tv(&dir, vec![], vec![]);
        let error = install_plan(&arm64, SERIAL, None, &dir.journals()).unwrap_err();
        assert_eq!(error.code(), "INVALID_APK");
        assert!(
            error.to_string().contains("app-arm64-v8a-release.apk"),
            "{error}"
        );
    }
}

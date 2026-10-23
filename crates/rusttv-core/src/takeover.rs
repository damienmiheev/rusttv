//! Home takeover: make RustTV the Home screen by disabling (never
//! uninstalling) the stock launcher, and undo it (ARCHITECTURE.md D12,
//! SECURITY.md § Home takeover).
//!
//! Both directions are [`Plan`]s run by the safety engine in [`crate::plan`].

use std::collections::HashSet;
use std::time::Duration;

use crate::adb::Adb;
use crate::apps::parse_package_list;
use crate::command::{DeviceCommand, PackageName};
use crate::compat::DeviceProfile;
use crate::inspect::parse_properties;
use crate::journal::Journal;
use crate::launcher::{RUSTTV_LAUNCHER, launcher_status};
use crate::plan::{Check, Plan, PlanError, Step, device_fingerprint, restore_plan};

/// Plan name of a Home takeover, also used in journal file names.
pub const HOME_TAKEOVER: &str = "home-takeover";

/// How long RustTV gets to come to the front after the HOME key press.
const FRONT_TIMEOUT: Duration = Duration::from_secs(10);

/// Plans disabling the stock launcher so Home opens RustTV. Read-only.
///
/// Rules from SECURITY.md § Home takeover:
/// - 2: one of `profiles` (normally [`crate::bundled_profiles`]) matches the
///   device's model and firmware family, names the stock launcher, and
///   records a tested takeover. Otherwise `TAKEOVER_NOT_TESTED`.
/// - 3: the RustTV launcher is installed and not yet Home; Home opens a
///   system app that outranks RustTV by intent priority (D12). That app is
///   the only package the plan touches.
/// - 5: after disabling, Home must resolve to RustTV; then HOME is pressed
///   and RustTV must be the resumed activity with a live process within
///   10 s. Otherwise the plan rolls back.
pub fn takeover_plan(
    adb: &dyn Adb,
    serial: &str,
    profiles: &[DeviceProfile],
) -> Result<Plan, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
    let Some(profile) = profiles.iter().find(|profile| profile.matches(&properties)) else {
        return Err(PlanError::not_tested(format!(
            "no compatibility profile in compat/devices/ matches this device ({} {}, `{}`, \
             firmware `{}`); takeover is only offered on devices where it was tested",
            property("ro.product.manufacturer"),
            property("ro.product.model"),
            property("ro.product.device"),
            property("ro.build.fingerprint"),
        )));
    };
    let profile_name = crate::compat::profile_file_name(profile);
    let Some(launcher) = profile
        .launcher
        .as_ref()
        .filter(|launcher| launcher.takeover_tested)
    else {
        return Err(PlanError::not_tested(format!(
            "the compatibility profile {profile_name} doesn't record a tested takeover \
             (`[launcher]` with `stock_package` and `takeover_tested = true`)"
        )));
    };

    let status = launcher_status(adb, serial)?;
    if status.rusttv_is_home {
        return Err(PlanError::precondition(
            "Home already opens the RustTV launcher; nothing to do".to_owned(),
        ));
    }
    if !status.rusttv_installed {
        return Err(PlanError::precondition(format!(
            "the RustTV launcher ({RUSTTV_LAUNCHER}) isn't installed; install it first"
        )));
    }
    let home = status
        .home
        .as_deref()
        .ok_or_else(|| PlanError::precondition("nothing handles Home on this device".to_owned()))?;
    let stock = home.split_once('/').map_or(home, |(package, _)| package);
    if stock != launcher.stock_package {
        return Err(PlanError::not_tested(format!(
            "Home opens `{stock}`, but the profile {profile_name} names `{}` as the stock \
             launcher; takeover was tested only for that package",
            launcher.stock_package
        )));
    }

    let priority_of = |package: &str| {
        status
            .candidates
            .iter()
            .filter(|candidate| candidate.package() == package)
            .map(|candidate| candidate.priority)
            .max()
    };
    let outranks_rusttv = matches!(
        (priority_of(stock), priority_of(RUSTTV_LAUNCHER)),
        (Some(stock), Some(rusttv)) if stock > rusttv && stock > 0
    );
    if !outranks_rusttv {
        return Err(PlanError::precondition(format!(
            "Home opens `{home}`, which doesn't outrank RustTV by priority; takeover is only for \
             a stock launcher that always wins Home (ARCHITECTURE.md D12)"
        )));
    }

    let third_party = adb.run(serial, &DeviceCommand::ListThirdPartyPackages)?;
    let third_party: HashSet<&str> = parse_package_list(&third_party)
        .map(|(name, _)| name)
        .collect();
    if third_party.contains(stock) {
        return Err(PlanError::precondition(format!(
            "Home opens `{stock}`, which isn't a system app; takeover only replaces the stock launcher"
        )));
    }

    let stock = PackageName::new(stock)
        .map_err(|error| PlanError::precondition(format!("Home resolver: {error}")))?;
    let rusttv = PackageName::new(RUSTTV_LAUNCHER)
        .map_err(|error| PlanError::precondition(error.to_string()))?;
    let fingerprint = properties
        .get("ro.build.fingerprint")
        .cloned()
        .unwrap_or_default();
    let notes = vec![
        "Disables, never uninstalls, the stock launcher. Undo with `rusttv launcher restore`."
            .to_owned(),
        format!(
            "Without ADB: Settings > Apps > See all apps > Show system apps > {stock} > Enable."
        ),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
        format!("Takeover was tested on this model and firmware family ({profile_name})."),
    ];
    Plan::new(
        HOME_TAKEOVER,
        serial,
        &fingerprint,
        "takeover",
        vec![
            Step::new(
                DeviceCommand::DisablePackage(stock),
                vec![Check::HomeIs(rusttv.clone())],
            ),
            Step::new(DeviceCommand::PressHome, vec![Check::InFront(rusttv)])
                .polling_for(FRONT_TIMEOUT),
        ],
        notes,
    )
}

/// Plans undoing a Home takeover recorded in `journal`. Read-only.
///
/// The journal is a file anyone could edit, so the plan is refused unless
/// every step only re-enables a package.
pub fn takeover_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    journal: &Journal,
) -> Result<Plan, PlanError> {
    if journal.plan != HOME_TAKEOVER {
        return Err(PlanError::precondition(format!(
            "this is a `{}` journal, not a {HOME_TAKEOVER} journal",
            journal.plan
        )));
    }
    let fingerprint = device_fingerprint(adb, serial)?;
    let plan = restore_plan(journal, serial, &fingerprint)?;
    if !plan
        .steps()
        .iter()
        .all(|step| matches!(step.command(), DeviceCommand::EnablePackage(_)))
    {
        return Err(PlanError::precondition(
            "the journal asks for more than re-enabling packages; refusing to restore from it"
                .to_owned(),
        ));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::adb::FakeAdb;
    use crate::compat::{
        DeviceKind, ProfileDevice, ProfileFirmware, ProfileHardware, ProfileLauncher, ProfileTested,
    };
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{JournalStatus, StepStatus};
    use crate::plan::{apply, restore};

    const SERIAL: &str = "emulator-5554";
    const STOCK: &str = "com.google.android.apps.tv.launcherx";

    macro_rules! real {
        ($file:literal) => {
            include_str!(concat!("fixtures/google-tv-emulator-api34/", $file)).to_owned()
        };
    }
    macro_rules! synthetic {
        ($file:literal) => {
            include_str!(concat!("fixtures/synthetic-home-takeover/", $file)).to_owned()
        };
    }

    fn stock() -> PackageName {
        PackageName::new(STOCK).unwrap()
    }

    /// A profile for the synthetic Google TV device, with `launcher` as given.
    /// Hand-written for tests; no committed profile is takeover-tested.
    fn profile(launcher: Option<ProfileLauncher>) -> DeviceProfile {
        DeviceProfile {
            schema_version: crate::compat::PROFILE_SCHEMA_VERSION,
            device: ProfileDevice {
                manufacturer: "Google".to_owned(),
                brand: "google".to_owned(),
                model: "sdk_google_atv64_arm64".to_owned(),
                codename: "emu64a".to_owned(),
                kind: DeviceKind::Tv,
            },
            firmware: ProfileFirmware {
                android_version: "14".to_owned(),
                api_level: 34,
                security_patch: "2024-06-05".to_owned(),
                // Same family as the synthetic getprop, older build.
                build_fingerprint:
                    "google/sdk_google_atv64_arm64/emu64a:14/UTT3.240101.001/1:userdebug/dev-keys"
                        .to_owned(),
            },
            hardware: ProfileHardware {
                abis: vec!["arm64-v8a".to_owned()],
                memory_total_mib: 2000,
                data_storage_total_mib: 6000,
            },
            tested: ProfileTested {
                rusttv_version: "0.0.0".to_owned(),
                notes: String::new(),
            },
            launcher,
        }
    }

    fn tested() -> Vec<DeviceProfile> {
        vec![profile(Some(ProfileLauncher {
            stock_package: STOCK.to_owned(),
            takeover_tested: true,
        }))]
    }

    /// The Google TV emulator before a takeover. `home` is what Home resolves
    /// to on each call (the first is for planning), `disabled` the output of
    /// `pm list packages -d` on each call, `activities` of
    /// `dumpsys activity activities` on each call.
    fn tv(home: Vec<String>, disabled: Vec<String>, activities: Vec<String>) -> FakeAdb {
        FakeAdb::new()
            .respond_sequence(DeviceCommand::HomeActivity, home)
            .respond(DeviceCommand::HomeCandidates, real!("home-candidates.txt"))
            .respond(
                DeviceCommand::ListThirdPartyPackages,
                synthetic!("packages-3.txt"),
            )
            .respond(DeviceCommand::GetProperties, synthetic!("getprop.txt"))
            .respond_sequence(DeviceCommand::ListDisabledPackages, disabled)
            .respond(
                DeviceCommand::DisablePackage(stock()),
                synthetic!("pm-disable-user.txt"),
            )
            .respond(
                DeviceCommand::EnablePackage(stock()),
                synthetic!("pm-enable.txt"),
            )
            .respond(
                DeviceCommand::PressHome,
                synthetic!("input-keyevent-home.txt"),
            )
            .respond_sequence(DeviceCommand::Activities, activities)
            .respond(
                DeviceCommand::ProcessList,
                synthetic!("ps-rusttv-running.txt"),
            )
    }

    /// Plans against a stock device: only the planning reads matter.
    fn planning_tv() -> FakeAdb {
        tv(vec![real!("home-activity.txt")], vec![], vec![])
    }

    fn changes(adb: &FakeAdb) -> Vec<DeviceCommand> {
        adb.calls()
            .into_iter()
            .filter(|command| command.risk() > crate::RiskClass::Read)
            .collect()
    }

    #[test]
    fn plan_disables_the_stock_launcher_then_presses_home() {
        let adb = planning_tv();
        let plan = takeover_plan(&adb, SERIAL, &tested()).unwrap();
        let dry_run = plan.dry_run();

        assert_eq!(dry_run.max_risk, "R3");
        assert_eq!(dry_run.confirmation, "takeover");
        let [disable, home] = dry_run.steps.as_slice() else {
            panic!("expected two steps: {dry_run:?}");
        };
        assert_eq!(
            disable.command,
            "adb -s emulator-5554 shell pm disable-user --user 0 com.google.android.apps.tv.launcherx"
        );
        assert_eq!(
            disable.records.as_deref(),
            Some("com.google.android.apps.tv.launcherx is enabled")
        );
        assert_eq!(
            disable.verify,
            [
                "com.google.android.apps.tv.launcherx is disabled",
                "Home opens ai.rusttv.launcher"
            ]
        );
        assert_eq!(
            disable.undo.as_deref(),
            Some(
                "adb -s emulator-5554 shell pm enable --user 0 com.google.android.apps.tv.launcherx"
            )
        );
        assert_eq!(home.risk, "R1");
        assert_eq!(
            home.command,
            "adb -s emulator-5554 shell input keyevent KEYCODE_HOME"
        );
        assert_eq!(home.verify, ["ai.rusttv.launcher is in front and running"]);
        assert_eq!(home.poll_ms, Some(10_000));
        assert_eq!(home.undo, None);
        // Planning is read-only.
        assert!(changes(&adb).is_empty());
    }

    #[test]
    fn takeover_applies_and_verifies_then_restore_undoes_it() {
        let adb = tv(
            vec![
                real!("home-activity.txt"),
                synthetic!("home-activity-rusttv.txt"),
            ],
            vec![
                synthetic!("packages-d-none.txt"),
                synthetic!("packages-d-launcher-disabled.txt"),
                synthetic!("packages-d-none.txt"),
            ],
            // RustTV comes to the front on the second look.
            vec![
                synthetic!("activities-stock-front.txt"),
                synthetic!("activities-rusttv-front.txt"),
            ],
        );
        let plan = takeover_plan(&adb, SERIAL, &tested()).unwrap();
        let approval = plan.approve("takeover").unwrap();
        let mut store = MemoryJournal::default();
        let sleeps = Cell::new(0);

        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {
            sleeps.set(sleeps.get() + 1);
        })
        .unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(sleeps.get(), 1);
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(stock()),
                DeviceCommand::PressHome
            ]
        );

        // Restore: from the saved journal; only the package is re-enabled.
        let mut journal = store.last().clone();
        let plan = takeover_restore_plan(&adb, SERIAL, &journal).unwrap();
        assert_eq!(
            plan.steps().iter().map(Step::command).collect::<Vec<_>>(),
            [&DeviceCommand::EnablePackage(stock())]
        );
        let approval = plan.approve("restore").unwrap();
        restore(&adb, &plan, &approval, &mut journal, &mut store).unwrap();
        assert_eq!(store.last().status, JournalStatus::Restored);
        assert_eq!(store.last().steps[0].status, StepStatus::Undone);
    }

    #[test]
    fn home_showing_a_chooser_instead_of_rusttv_rolls_back() {
        // On the captured emulator the spike launcher also has priority 0, so
        // without the stock launcher Android would ask which Home to use.
        let adb = tv(
            vec![
                real!("home-activity.txt"),
                synthetic!("home-activity-chooser.txt"),
            ],
            vec![
                synthetic!("packages-d-none.txt"),
                synthetic!("packages-d-launcher-disabled.txt"),
                synthetic!("packages-d-none.txt"),
            ],
            vec![],
        );
        let plan = takeover_plan(&adb, SERIAL, &tested()).unwrap();
        let approval = plan.approve("takeover").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error.to_string().contains("Home opens ai.rusttv.launcher"),
            "{error}"
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(store.last().steps[0].status, StepStatus::Undone);
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(stock()),
                DeviceCommand::EnablePackage(stock())
            ],
            "HOME is never pressed after a failed first step"
        );
        assert_eq!(
            adb.calls().last(),
            Some(&DeviceCommand::ListDisabledPackages),
            "the re-enable must be verified"
        );
    }

    #[test]
    fn rusttv_not_coming_to_the_front_within_10_s_rolls_back() {
        let adb = tv(
            vec![
                real!("home-activity.txt"),
                synthetic!("home-activity-rusttv.txt"),
            ],
            vec![
                synthetic!("packages-d-none.txt"),
                synthetic!("packages-d-launcher-disabled.txt"),
                synthetic!("packages-d-none.txt"),
            ],
            vec![synthetic!("activities-stock-front.txt")],
        );
        let plan = takeover_plan(&adb, SERIAL, &tested()).unwrap();
        let approval = plan.approve("takeover").unwrap();
        let mut store = MemoryJournal::default();
        let waited = Cell::new(Duration::ZERO);

        let error = apply(&adb, &plan, &approval, &mut store, &|duration| {
            waited.set(waited.get() + duration);
        })
        .unwrap_err();

        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error
                .to_string()
                .contains("in front and running` within 10 s"),
            "{error}"
        );
        assert_eq!(waited.get(), Duration::from_secs(10));
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(stock()),
                DeviceCommand::PressHome,
                DeviceCommand::EnablePackage(stock())
            ]
        );
        let last = store.last();
        assert_eq!(last.status, JournalStatus::RolledBack);
        assert_eq!(last.steps[0].status, StepStatus::Undone);
        // Pressing HOME changed nothing persistent, so it has nothing to undo.
        assert_eq!(last.steps[1].status, StepStatus::Started);
    }

    #[test]
    fn rusttv_in_front_without_a_live_process_rolls_back() {
        let adb = tv(
            vec![
                real!("home-activity.txt"),
                synthetic!("home-activity-rusttv.txt"),
            ],
            vec![
                synthetic!("packages-d-none.txt"),
                synthetic!("packages-d-launcher-disabled.txt"),
                synthetic!("packages-d-none.txt"),
            ],
            vec![synthetic!("activities-rusttv-front.txt")],
        )
        .respond(
            DeviceCommand::ProcessList,
            synthetic!("ps-rusttv-not-running.txt"),
        );
        let plan = takeover_plan(&adb, SERIAL, &tested()).unwrap();
        let approval = plan.approve("takeover").unwrap();

        let error = apply(
            &adb,
            &plan,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert_eq!(
            changes(&adb).last(),
            Some(&DeviceCommand::EnablePackage(stock()))
        );
    }

    #[test]
    fn no_takeover_without_a_matching_profile() {
        let error = takeover_plan(&planning_tv(), SERIAL, &[]).unwrap_err();
        assert_eq!(error.code(), "TAKEOVER_NOT_TESTED");
        let message = error.to_string();
        assert!(message.contains("no compatibility profile"), "{message}");
        assert!(message.contains("sdk_google_atv64_arm64"), "{message}");

        // The bundled profiles (the phone emulator) don't match a TV either.
        let error = takeover_plan(&planning_tv(), SERIAL, &crate::bundled_profiles()).unwrap_err();
        assert_eq!(error.code(), "TAKEOVER_NOT_TESTED");
    }

    #[test]
    fn no_takeover_when_the_profile_does_not_record_a_tested_takeover() {
        for launcher in [
            None,
            Some(ProfileLauncher {
                stock_package: STOCK.to_owned(),
                takeover_tested: false,
            }),
        ] {
            let error = takeover_plan(&planning_tv(), SERIAL, &[profile(launcher)]).unwrap_err();
            assert_eq!(error.code(), "TAKEOVER_NOT_TESTED");
            assert!(
                error.to_string().contains("takeover_tested = true"),
                "{error}"
            );
        }
    }

    #[test]
    fn no_takeover_when_home_is_not_the_profiled_stock_launcher() {
        let profiles = vec![profile(Some(ProfileLauncher {
            stock_package: "com.google.android.tvlauncher".to_owned(),
            takeover_tested: true,
        }))];
        let error = takeover_plan(&planning_tv(), SERIAL, &profiles).unwrap_err();
        assert_eq!(error.code(), "TAKEOVER_NOT_TESTED");
        assert!(
            error
                .to_string()
                .contains("names `com.google.android.tvlauncher`"),
            "{error}"
        );
    }

    #[test]
    fn no_takeover_on_a_new_android_version() {
        let adb = planning_tv().respond(
            DeviceCommand::GetProperties,
            synthetic!("getprop.txt").replace("emu64a:14/", "emu64a:15/"),
        );
        let error = takeover_plan(&adb, SERIAL, &tested()).unwrap_err();
        assert_eq!(error.code(), "TAKEOVER_NOT_TESTED");
    }

    #[test]
    fn no_takeover_when_rusttv_is_missing_or_already_home() {
        let missing = planning_tv().respond(
            DeviceCommand::HomeCandidates,
            "priority=2\ncom.google.android.apps.tv.launcherx/.home.HomeActivity\n",
        );
        let error = takeover_plan(&missing, SERIAL, &tested()).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("isn't installed"), "{error}");

        let home = tv(vec![synthetic!("home-activity-rusttv.txt")], vec![], vec![]);
        let error = takeover_plan(&home, SERIAL, &tested()).unwrap_err();
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    #[test]
    fn no_takeover_of_a_launcher_that_does_not_outrank_rusttv() {
        // A profile naming the spike as stock launcher, to get past rule 2.
        let profiles = vec![profile(Some(ProfileLauncher {
            stock_package: "ai.rusttv.spike".to_owned(),
            takeover_tested: true,
        }))];
        let adb = tv(
            vec!["priority=0\nai.rusttv.spike/.LauncherActivity\n".to_owned()],
            vec![],
            vec![],
        );
        let error = takeover_plan(&adb, SERIAL, &profiles).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("doesn't outrank"), "{error}");
    }

    #[test]
    fn no_takeover_of_a_third_party_home_app() {
        let adb = planning_tv().respond(
            DeviceCommand::ListThirdPartyPackages,
            "package:com.google.android.apps.tv.launcherx\npackage:ai.rusttv.launcher\n",
        );
        let error = takeover_plan(&adb, SERIAL, &tested()).unwrap_err();
        assert!(error.to_string().contains("isn't a system app"), "{error}");
    }

    #[test]
    fn restore_refuses_a_journal_that_would_disable_something() {
        // A tampered journal: "undoing" an enable would disable Settings.
        let settings = PackageName::new("com.android.tv.settings").unwrap();
        let journal = Journal {
            schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
            plan: HOME_TAKEOVER.to_owned(),
            serial: SERIAL.to_owned(),
            fingerprint: "x".to_owned(),
            started_at_unix_ms: 0,
            status: JournalStatus::Applied,
            steps: vec![crate::journal::JournalStep {
                adb: String::new(),
                command: DeviceCommand::EnablePackage(settings),
                original_state: None,
                status: StepStatus::Done,
            }],
        };
        let error = takeover_restore_plan(&planning_tv(), SERIAL, &journal).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("refusing"), "{error}");
    }
}

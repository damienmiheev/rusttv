//! Home takeover: make RustTV the Home screen by disabling (never
//! uninstalling) the stock launcher, and undo it (ARCHITECTURE.md D12,
//! SECURITY.md § Home takeover).
//!
//! Both directions are [`Plan`]s run by the safety engine in [`crate::plan`].

use std::collections::HashSet;

use crate::adb::Adb;
use crate::apps::parse_package_list;
use crate::command::{DeviceCommand, PackageName};
use crate::journal::Journal;
use crate::launcher::{RUSTTV_LAUNCHER, launcher_status};
use crate::plan::{Check, Plan, PlanError, Step, device_fingerprint, restore_plan};

/// Plan name of a Home takeover, also used in journal file names.
pub const HOME_TAKEOVER: &str = "home-takeover";

/// Plans disabling the stock launcher so Home opens RustTV. Read-only.
///
/// Preconditions (SECURITY.md § Home takeover, rule 3): the RustTV launcher
/// is installed and not yet Home; Home opens a system app that outranks
/// RustTV by intent priority (the Google TV situation of D12). That app is
/// the only package the plan touches. The plan verifies that Home then
/// opens RustTV, and rolls back if not.
///
/// Not checked yet: rule 2 (compat profile records a tested takeover for
/// this firmware), and from rule 3 and 5 that RustTV draws frames and comes
/// to the front on a HOME key press.
pub fn takeover_plan(adb: &dyn Adb, serial: &str) -> Result<Plan, PlanError> {
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
    let fingerprint = device_fingerprint(adb, serial)?;
    let notes = vec![
        "Disables, never uninstalls, the stock launcher. Undo with `rusttv launcher restore`."
            .to_owned(),
        format!(
            "Without ADB: Settings > Apps > See all apps > Show system apps > {stock} > Enable."
        ),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
        "Not checked by this version: whether this device and firmware have a tested takeover \
         (SECURITY.md § Home takeover, rule 2)."
            .to_owned(),
    ];
    Plan::new(
        HOME_TAKEOVER,
        serial,
        &fingerprint,
        "takeover",
        vec![Step::new(
            DeviceCommand::DisablePackage(stock),
            vec![Check::HomeIs(rusttv)],
        )],
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
    use super::*;
    use crate::adb::FakeAdb;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{JournalStatus, StepStatus};
    use crate::plan::{apply, restore};

    const SERIAL: &str = "emulator-5554";

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
        PackageName::new("com.google.android.apps.tv.launcherx").unwrap()
    }

    /// The Google TV emulator before a takeover. `home` is what Home resolves
    /// to on each call (the first is for planning), `disabled` the output of
    /// `pm list packages -d` on each call.
    fn tv(home: Vec<String>, disabled: Vec<String>) -> FakeAdb {
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
    }

    #[test]
    fn plan_disables_only_the_stock_launcher_and_checks_home() {
        let adb = tv(vec![real!("home-activity.txt")], vec![]);
        let plan = takeover_plan(&adb, SERIAL).unwrap();
        let dry_run = plan.dry_run();

        assert_eq!(dry_run.max_risk, "R3");
        assert_eq!(dry_run.confirmation, "takeover");
        assert_eq!(dry_run.steps.len(), 1);
        let step = &dry_run.steps[0];
        assert_eq!(
            step.command,
            "adb -s emulator-5554 shell pm disable-user --user 0 com.google.android.apps.tv.launcherx"
        );
        assert_eq!(
            step.records.as_deref(),
            Some("com.google.android.apps.tv.launcherx is enabled")
        );
        assert_eq!(
            step.verify,
            [
                "com.google.android.apps.tv.launcherx is disabled",
                "Home opens ai.rusttv.launcher"
            ]
        );
        assert_eq!(
            step.undo.as_deref(),
            Some(
                "adb -s emulator-5554 shell pm enable --user 0 com.google.android.apps.tv.launcherx"
            )
        );
        // Planning is read-only.
        assert!(
            adb.calls()
                .iter()
                .all(|command| command.risk() == crate::RiskClass::Read)
        );
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
        );
        let plan = takeover_plan(&adb, SERIAL).unwrap();
        let approval = plan.approve("takeover").unwrap();
        let mut store = MemoryJournal::default();

        let journal = apply(&adb, &plan, &approval, &mut store).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(journal.steps[0].status, StepStatus::Done);
        assert_eq!(
            journal.steps[0].original_state,
            Some(Check::PackageEnabled(stock()))
        );

        // Restore: the restore plan comes from the saved journal.
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
        );
        let plan = takeover_plan(&adb, SERIAL).unwrap();
        let approval = plan.approve("takeover").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error.to_string().contains("Home opens ai.rusttv.launcher"),
            "{error}"
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(store.last().steps[0].status, StepStatus::Undone);
        assert_eq!(
            adb.calls().last(),
            Some(&DeviceCommand::ListDisabledPackages),
            "the re-enable must be verified"
        );
        assert!(adb.calls().contains(&DeviceCommand::EnablePackage(stock())));
    }

    #[test]
    fn no_takeover_when_rusttv_is_missing_or_already_home() {
        let missing = tv(vec![real!("home-activity.txt")], vec![]).respond(
            DeviceCommand::HomeCandidates,
            "priority=2\ncom.google.android.apps.tv.launcherx/.home.HomeActivity\n",
        );
        let error = takeover_plan(&missing, SERIAL).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("isn't installed"), "{error}");

        let home = tv(vec![synthetic!("home-activity-rusttv.txt")], vec![]);
        let error = takeover_plan(&home, SERIAL).unwrap_err();
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    #[test]
    fn no_takeover_of_a_launcher_that_does_not_outrank_rusttv() {
        // e.g. a phone-style device where the user simply picks a default Home.
        let adb = tv(
            vec!["priority=0\nai.rusttv.spike/.LauncherActivity\n".to_owned()],
            vec![],
        );
        let error = takeover_plan(&adb, SERIAL).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("doesn't outrank"), "{error}");
    }

    #[test]
    fn no_takeover_of_a_third_party_home_app() {
        let adb = tv(vec![real!("home-activity.txt")], vec![]).respond(
            DeviceCommand::ListThirdPartyPackages,
            "package:com.google.android.apps.tv.launcherx\npackage:ai.rusttv.launcher\n",
        );
        let error = takeover_plan(&adb, SERIAL).unwrap_err();
        assert!(error.to_string().contains("isn't a system app"), "{error}");
    }

    #[test]
    fn restore_refuses_a_journal_that_would_disable_something() {
        // A tampered journal: "undoing" an enable would disable Settings.
        let adb = tv(vec![real!("home-activity.txt")], vec![]);
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
        let error = takeover_restore_plan(&adb, SERIAL, &journal).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("refusing"), "{error}");
    }
}

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::adb::FakeAdb;
use crate::command::{RiskClass, SettingValue};
use crate::compat::bundled_profiles;
use crate::journal::tests::MemoryJournal;
use crate::journal::{FileJournal, JOURNAL_SCHEMA_VERSION, JournalStep, JournalStore};
use crate::plan::apply;

const SERIAL: &str = "tv";
const GETPROP: &str = include_str!("../fixtures/philips-65oled818-api34/getprop.txt");
const HOME: &str = include_str!("../fixtures/philips-65oled818-api34/home-activity.txt");
const CANDIDATES: &str =
    include_str!("../fixtures/philips-65oled818-api34/home-candidates-rusttv-installed.txt");
const IME: &str = include_str!("../fixtures/synthetic-privacy/ime-list.txt");
const PACKAGES: &str = include_str!("../fixtures/synthetic-privacy/packages-f.txt");

/// Tested on the Philips (its profile's `[[privacy.tested]]`).
const CANDEEBUG: &str = "org.droidtv.candeebug";
/// In the catalog, installed in the fixture, not tested on the Philips.
const USAGELOGGER: &str = "org.droidtv.usagelogger";
/// Disabled in these tests.
const GEL: &str = "org.droidtv.gel";
const STOCK: &str = "com.google.android.apps.tv.launcherx";

fn package(name: &str) -> PackageName {
    PackageName::new(name).unwrap()
}

fn value(text: &str) -> SettingState {
    SettingState::Value(SettingValue::new(text).unwrap())
}

/// The Philips: window scale 1.0, the others unset, `org.droidtv.gel`
/// disabled, nothing protected beyond the fixture's keyboard.
fn tv() -> FakeAdb {
    FakeAdb::new()
        .respond(DeviceCommand::GetProperties, GETPROP)
        .respond(DeviceCommand::HomeActivity, HOME)
        .respond(DeviceCommand::HomeCandidates, CANDIDATES)
        .respond(DeviceCommand::InputMethods, IME)
        .respond(DeviceCommand::EnabledAccessibilityServices, "null\n")
        .respond(DeviceCommand::ListPackages, PACKAGES)
        .respond(
            DeviceCommand::ListDisabledPackages,
            format!("package:{GEL}\n"),
        )
        .respond(
            DeviceCommand::GetSetting(SettingKey::WindowAnimationScale),
            "1.0\n",
        )
        .respond(
            DeviceCommand::GetSetting(SettingKey::TransitionAnimationScale),
            "null\n",
        )
        .respond(
            DeviceCommand::GetSetting(SettingKey::AnimatorDurationScale),
            "null\n",
        )
}

struct Dir(PathBuf);

impl Dir {
    fn new(test: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("rusttv-history-core-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target(settings: &[(SettingKey, SettingState)], packages: &[(&str, bool)]) -> RestoreTarget {
    RestoreTarget {
        settings: settings.to_vec(),
        packages: packages
            .iter()
            .map(|(name, enabled)| (package(name), *enabled))
            .collect(),
    }
}

fn plan(adb: &FakeAdb, target: &RestoreTarget, dir: &Dir) -> Result<Plan, PlanError> {
    history_restore_plan(adb, SERIAL, target, &bundled_profiles(), &dir.0, false)
}

#[test]
fn a_restore_plans_settings_and_catalog_packages_and_skips_the_rest() {
    let dir = Dir::new("plan");
    let target = target(
        &[
            (SettingKey::WindowAnimationScale, value("0.5")),
            // Unset ≡ 1: nothing to do for these.
            (SettingKey::TransitionAnimationScale, value("1")),
            (SettingKey::AnimatorDurationScale, SettingState::Unset),
        ],
        &[
            (CANDEEBUG, false),
            (GEL, true),
            (STOCK, false),
            ("com.example.notcatalog", false),
            ("com.android.printspooler", true),
        ],
    );
    let plan = plan(&tv(), &target, &dir).unwrap();
    assert_eq!(plan.name(), HISTORY_RESTORE);
    assert_eq!(plan.confirmation(), "restore");
    let commands: Vec<&DeviceCommand> = plan.steps().iter().map(Step::command).collect();
    assert_eq!(
        commands,
        [
            &DeviceCommand::SetSetting {
                key: SettingKey::WindowAnimationScale,
                to: value("0.5"),
                from: value("1.0"),
            },
            &DeviceCommand::DisablePackage(package(CANDEEBUG)),
            &DeviceCommand::EnablePackage(package(GEL)),
        ]
    );
    // Disabling re-checks the protected-package policy right before.
    assert_eq!(
        plan.steps()[1].requirements(),
        [Check::NotProtected(package(CANDEEBUG))]
    );
    let notes = plan.dry_run().notes.join("\n");
    assert!(notes.contains("rusttv launcher takeover"), "{notes}");
    assert!(notes.contains("not in RustTV's package catalog"), "{notes}");
    assert!(plan.max_risk() == RiskClass::PackageState);
}

#[test]
fn protected_and_untested_packages_follow_the_privacy_rules() {
    let dir = Dir::new("rules");
    let adb = tv().respond(
        DeviceCommand::EnabledAccessibilityServices,
        format!("{CANDEEBUG}/.Service\n"),
    );
    let error = plan(&adb, &target(&[], &[(CANDEEBUG, false)]), &dir).unwrap_err();
    assert_eq!(error.code(), "PACKAGE_PROTECTED");

    let untested = target(&[], &[(USAGELOGGER, false)]);
    let error = plan(&tv(), &untested, &dir).unwrap_err();
    assert_eq!(error.code(), "PROFILE_NOT_TESTED");
    let plan =
        history_restore_plan(&tv(), SERIAL, &untested, &bundled_profiles(), &dir.0, true).unwrap();
    assert!(plan.dry_run().notes.join(" ").contains("UNTESTED"));

    let error = plan_nothing(&dir);
    assert_eq!(error.code(), "PRECONDITION_FAILED");
}

fn plan_nothing(dir: &Dir) -> PlanError {
    plan(&tv(), &target(&[], &[(GEL, false)]), dir).unwrap_err()
}

fn journal(plan: &str, steps: Vec<DeviceCommand>) -> Journal {
    Journal {
        schema_version: JOURNAL_SCHEMA_VERSION,
        plan: plan.to_owned(),
        serial: SERIAL.to_owned(),
        fingerprint: "f".to_owned(),
        started_at_unix_ms: 1,
        status: JournalStatus::Applied,
        approved_by: None,
        restore_approved_by: None,
        steps: steps
            .into_iter()
            .map(|command| JournalStep {
                adb: String::new(),
                command,
                original_state: None,
                status: StepStatus::Done,
            })
            .collect(),
    }
}

#[test]
fn restores_and_optimizer_or_privacy_journals_refuse_each_other_on_the_same_keys() {
    let dir = Dir::new("overlap");
    let optimizer = DeviceCommand::SetSetting {
        key: SettingKey::WindowAnimationScale,
        to: value("0.5"),
        from: value("1.0"),
    };
    FileJournal::new(dir.0.join("1-optimize-reduced-animations-v1.toml"))
        .save(&journal("optimize-reduced-animations-v1", vec![optimizer]))
        .unwrap();
    let settings = target(&[(SettingKey::WindowAnimationScale, value("0.7"))], &[]);
    let error = plan(&tv(), &settings, &dir).unwrap_err();
    assert_eq!(error.code(), "PRECONDITION_FAILED");
    assert!(
        error.to_string().contains("window_animation_scale"),
        "{error}"
    );
    // Another key doesn't overlap.
    let other = target(&[(SettingKey::AnimatorDurationScale, value("0.5"))], &[]);
    let restore_plan = plan(&tv(), &other, &dir).unwrap();

    // The other way round: an optimizer or privacy plan is refused while a
    // restorable history restore holds its keys or packages.
    let dir = Dir::new("overlap-back");
    FileJournal::new(dir.0.join("1-history-restore.toml"))
        .save(&journal(
            HISTORY_RESTORE,
            vec![restore_plan.steps()[0].command().clone()],
        ))
        .unwrap();
    let privacy = Plan::new(
        PRIVACY,
        SERIAL,
        "f",
        "privacy",
        vec![Step::new(
            DeviceCommand::DisablePackage(package(CANDEEBUG)),
            Vec::new(),
        )],
        Vec::new(),
    )
    .unwrap();
    refuse_overlapping_journals(&privacy, &dir.0).unwrap();
    let optimizer = Plan::new(
        "optimize-reduced-animations-v1",
        SERIAL,
        "f",
        "optimize",
        vec![Step::new(
            DeviceCommand::SetSetting {
                key: SettingKey::AnimatorDurationScale,
                to: value("0.5"),
                from: SettingState::Unset,
            },
            Vec::new(),
        )],
        Vec::new(),
    )
    .unwrap();
    let error = refuse_overlapping_journals(&optimizer, &dir.0).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("rusttv history restore --journal"),
        "{error}"
    );

    // An unreadable journal may be such a change: a restore refuses.
    fs::write(dir.0.join("2-broken.toml"), "not a journal").unwrap();
    let error = plan(&tv(), &other, &dir).unwrap_err();
    assert!(
        error.to_string().contains("can't read journal file"),
        "{error}"
    );
}

#[test]
fn a_tv_approval_covers_exactly_its_changes_with_their_old_values() {
    let dir = Dir::new("tv-approval");
    let target = target(
        &[(SettingKey::WindowAnimationScale, value("0.5"))],
        &[(CANDEEBUG, false)],
    );
    let plan = plan(&tv(), &target, &dir).unwrap();
    let hash = "ab".repeat(32);
    let exact = [
        TvChange::Package {
            package: package(CANDEEBUG),
            old: Some(true),
            new: false,
        },
        TvChange::Setting {
            key: SettingKey::WindowAnimationScale,
            // "1" and "1.0" are the same scale.
            old: value("1"),
            new: value("0.5"),
        },
    ];
    let approval = approve_by_tv(&plan, &hash, &exact).unwrap();

    let adb = tv()
        .respond_sequence(
            DeviceCommand::GetSetting(SettingKey::WindowAnimationScale),
            ["1.0\n".to_owned(), "0.5\n".to_owned()],
        )
        .respond_sequence(
            DeviceCommand::ListDisabledPackages,
            [
                format!("package:{GEL}\n"),
                format!("package:{GEL}\npackage:{CANDEEBUG}\n"),
            ],
        )
        .respond(plan.steps()[0].command().clone(), "")
        .respond(plan.steps()[1].command().clone(), "");
    let mut store = MemoryJournal::default();
    apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
    assert_eq!(
        store.last().approved_by,
        Some(ApprovedBy::Tv {
            approval: hash.clone()
        })
    );

    // Another old value, a missing change, an extra one, a bad hash: no.
    let mut changed = exact.clone();
    changed[1] = TvChange::Setting {
        key: SettingKey::WindowAnimationScale,
        old: value("0.7"),
        new: value("0.5"),
    };
    assert_eq!(
        approve_by_tv(&plan, &hash, &changed).unwrap_err().code(),
        "NOT_APPROVED"
    );
    assert!(approve_by_tv(&plan, &hash, &exact[..1]).is_err());
    let mut more = exact.to_vec();
    more.push(TvChange::Package {
        package: package(GEL),
        old: Some(false),
        new: true,
    });
    assert!(approve_by_tv(&plan, &hash, &more).is_err());
    assert!(approve_by_tv(&plan, "nothex", &exact).is_err());
}

#[test]
fn a_restore_journal_is_undone_only_with_settings_and_catalog_packages() {
    let enable = DeviceCommand::EnablePackage(package(GEL));
    let set = DeviceCommand::SetSetting {
        key: SettingKey::WindowAnimationScale,
        to: value("1.0"),
        from: value("0.5"),
    };
    // GEL enabled now; window at what the restore set.
    let adb = tv().respond(DeviceCommand::ListDisabledPackages, "");
    let mut recorded = journal(HISTORY_RESTORE, vec![set, enable]);
    let undo = history_restore_undo_plan(&adb, SERIAL, &mut recorded, &bundled_profiles()).unwrap();
    let commands: Vec<&DeviceCommand> = undo.steps().iter().map(Step::command).collect();
    assert_eq!(
        commands,
        [
            &DeviceCommand::DisablePackage(package(GEL)),
            &DeviceCommand::SetSetting {
                key: SettingKey::WindowAnimationScale,
                to: value("0.5"),
                from: value("1.0"),
            },
        ]
    );

    // Only history-restore journals; only catalog packages, never takeover ones.
    let mut other = journal("privacy", vec![DeviceCommand::DisablePackage(package(GEL))]);
    assert!(history_restore_undo_plan(&adb, SERIAL, &mut other, &bundled_profiles()).is_err());
    let mut stock = journal(
        HISTORY_RESTORE,
        vec![DeviceCommand::DisablePackage(package(STOCK))],
    );
    let error =
        history_restore_undo_plan(&adb, SERIAL, &mut stock, &bundled_profiles()).unwrap_err();
    assert!(error.to_string().contains("refusing to restore"), "{error}");

    // Something changed since everywhere: nothing left, marked restored.
    let adb = tv()
        .respond(
            DeviceCommand::ListDisabledPackages,
            format!("package:{GEL}\n"),
        )
        .respond(
            DeviceCommand::GetSetting(SettingKey::WindowAnimationScale),
            "0.7\n",
        );
    let mut recorded = journal(
        HISTORY_RESTORE,
        vec![
            DeviceCommand::SetSetting {
                key: SettingKey::WindowAnimationScale,
                to: value("1.0"),
                from: value("0.5"),
            },
            DeviceCommand::EnablePackage(package(GEL)),
        ],
    );
    let error =
        history_restore_undo_plan(&adb, SERIAL, &mut recorded, &bundled_profiles()).unwrap_err();
    assert_eq!(error.code(), "PRECONDITION_FAILED");
    assert_eq!(recorded.status, JournalStatus::Restored);
}

#[test]
fn restore_tv_writes_the_export_and_imports_it_with_launcher_private_steps() {
    let head = HistorySince::entry(&"cd".repeat(20)).unwrap();
    let nonce = HistoryNonce::new("0123456789abcdef").unwrap();
    let export = b"rusttv-export 1 0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69 x 0\nPACK".to_vec();
    let adb = tv();
    let plan = restore_tv_plan(&adb, SERIAL, export.clone(), &head, nonce.clone(), vec![]).unwrap();
    assert_eq!(plan.max_risk(), RiskClass::LauncherPrivate);
    assert_eq!(plan.confirmation(), "history");
    assert!(plan.steps().iter().all(|step| step.undo().is_none()));

    let commit = plan.steps()[1].command().clone();
    let adb = adb
        .respond_binary(plan.steps()[0].command().clone(), [Vec::new()])
        .respond(commit, "Result: Bundle[{rusttv=rusttv-result 1 ok none}]")
        .respond_binary(
            DeviceCommand::HistoryExport(head.clone()),
            [b"rusttv-export 1 0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69 none 0\n".to_vec()],
        );
    let approval = plan.approve("history").unwrap();
    let mut store = MemoryJournal::default();
    apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
    // The export went to the inbox on stdin.
    assert_eq!(adb.inputs()[0], export);

    // A refusal fails the check: the plan stops (nothing to undo).
    let adb = tv()
        .respond_binary(plan.steps()[0].command().clone(), [Vec::new()])
        .respond(
            plan.steps()[1].command().clone(),
            "Result: Bundle[{rusttv=rusttv-result 1 error HISTORY_NOT_EMPTY none}]",
        )
        .respond_binary(
            DeviceCommand::HistoryExport(head),
            [b"rusttv-error HISTORY_NOT_FOUND\n".to_vec()],
        );
    let error = apply(
        &adb,
        &plan,
        &approval,
        &mut MemoryJournal::default(),
        &|_| {},
    )
    .unwrap_err();
    assert_eq!(error.code(), "ROLLED_BACK");

    // A journal keeps the payload's digest only, which can't be sent again.
    let text = toml::to_string(&store.last()).unwrap();
    assert!(text.contains("sha256"), "{text}");
    assert!(!text.contains("PACK"), "{text}");
    let back: Journal = toml::from_str(&text).unwrap();
    assert_eq!(back.steps[0].command.stdin(), None);
    assert!(crate::adb::run_step(&tv(), SERIAL, &back.steps[0].command).is_err());
}

fn dumpsys(granted: bool, flags: &str) -> String {
    format!(
        "Packages:\n  Package [ai.rusttv.launcher] (1):\n    flags=[ {flags} ]\n    install \
         permissions:\n      android.permission.WRITE_SECURE_SETTINGS: granted={granted}\n"
    )
}

#[test]
fn the_settings_grant_is_a_plan_undone_by_its_revoke() {
    let info = DeviceCommand::PackageInfo(PackageName::rusttv_launcher());
    let adb = tv()
        .respond_sequence(
            info.clone(),
            [
                dumpsys(false, "HAS_CODE"),
                dumpsys(false, "HAS_CODE"),
                dumpsys(true, "HAS_CODE"),
            ],
        )
        .respond(DeviceCommand::GrantSecureSettings, "");
    let plan = tv_settings_plan(&adb, SERIAL, true).unwrap();
    assert_eq!(plan.name(), HISTORY_ALLOW_TV_SETTINGS);
    assert_eq!(plan.confirmation(), "allow");
    assert_eq!(plan.max_risk(), RiskClass::Setting);
    let dry = plan.dry_run();
    assert!(dry.steps[0].undo.as_deref().unwrap().contains("pm revoke"));
    let approval = plan.approve("allow").unwrap();
    apply(
        &adb,
        &plan,
        &approval,
        &mut MemoryJournal::default(),
        &|_| {},
    )
    .unwrap();

    let granted = tv().respond(info.clone(), dumpsys(true, "HAS_CODE"));
    assert_eq!(
        tv_settings_plan(&granted, SERIAL, true).unwrap_err().code(),
        "PRECONDITION_FAILED"
    );
    let plan = tv_settings_plan(&granted, SERIAL, false).unwrap();
    assert_eq!(plan.confirmation(), "revoke");
    let missing = tv().respond(info, "Unable to find package: ai.rusttv.launcher\n");
    assert!(tv_settings_plan(&missing, SERIAL, true).is_err());
}

#[test]
fn launcher_flags_and_grant_are_read_from_dumpsys() {
    assert_eq!(launcher_grant(&dumpsys(true, "HAS_CODE")), Some(true));
    assert_eq!(launcher_grant(&dumpsys(false, "HAS_CODE")), Some(false));
    assert_eq!(launcher_grant("Unable to find package"), None);
    assert_eq!(
        launcher_test_build(&dumpsys(false, "HAS_CODE")),
        Some(false)
    );
    assert_eq!(
        launcher_test_build(&dumpsys(false, "DEBUGGABLE HAS_CODE")),
        Some(true)
    );
    let profileable = format!(
        "{}    privateFlags=[ PROFILEABLE_BY_SHELL ]\n",
        dumpsys(false, "HAS_CODE")
    );
    assert_eq!(launcher_test_build(&profileable), Some(true));
    // No flags line: unknown, which callers treat as a test build.
    assert_eq!(
        launcher_test_build("Packages:\n  Package [ai.rusttv.launcher] (1):\n"),
        None
    );
    assert!(is_export(b"rusttv-export 1 none none 0\n"));
    assert!(!is_export(b"rusttv-error HISTORY_NOT_FOUND\n"));
}

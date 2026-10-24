//! Plans around RustTV's history (HISTORY.md § 4.5–4.6, § 6.3, § 7):
//! restoring a snapshot's settings and packages (`history-restore`) and
//! undoing that from its journal, putting the history back on the TV
//! (`history-restore-tv`, launcher-private steps), and granting or revoking
//! the launcher's `WRITE_SECURE_SETTINGS`.
//!
//! The snapshots themselves live in the `rusttv-history` crate; this module
//! takes plain targets, so the core doesn't depend on it.

use std::collections::{BTreeSet, HashSet};
use std::io;
use std::path::Path;

use crate::adb::Adb;
use crate::catalog::bundled_catalog;
use crate::command::{
    Component, DeviceCommand, DoorKind, HistoryNonce, HistorySince, InboxPayload, PackageName,
    SettingKey, SettingState,
};
use crate::compat::{DeviceProfile, profile_file_name};
use crate::inspect::parse_properties;
use crate::journal::{ApprovedBy, Journal, JournalStatus, StepStatus, restorable_journals};
use crate::launcher::RUSTTV_LAUNCHER;
use crate::optimizer::OPTIMIZE_PLAN_PREFIX;
use crate::plan::{Approval, Check, Plan, PlanError, Step, device_fingerprint, restore_plan};
use crate::privacy::{LiveState, PRIVACY, PackageState, PackageStates};

/// Plan name of `rusttv history restore` and `apply-pending`.
pub const HISTORY_RESTORE: &str = "history-restore";
/// Plan name of `rusttv history restore-tv`.
pub const HISTORY_RESTORE_TV: &str = "history-restore-tv";
/// Plan name of `rusttv history allow-tv-settings`.
pub const HISTORY_ALLOW_TV_SETTINGS: &str = "history-allow-tv-settings";
/// Plan name of `rusttv history revoke-tv-settings`.
pub const HISTORY_REVOKE_TV_SETTINGS: &str = "history-revoke-tv-settings";

/// What a `history-restore` plan should leave on the device: the settings
/// and catalog packages of a snapshot, or of a TV approval.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreTarget {
    /// Each setting and the state to leave it in (unset ≡ `1`).
    pub settings: Vec<(SettingKey, SettingState)>,
    /// Each package and whether it should be enabled.
    pub packages: Vec<(PackageName, bool)>,
}

/// One change a person approved on the TV, with the value it had then
/// (HISTORY.md § 6.2 item 7): what [`approve_by_tv`] compares a plan with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TvChange {
    /// A global setting.
    Setting {
        /// The key.
        key: SettingKey,
        /// Its state when approved.
        old: SettingState,
        /// The state to set.
        new: SettingState,
    },
    /// A catalog package.
    Package {
        /// The package.
        package: PackageName,
        /// Enabled when approved (as last seen by this computer); `None` if
        /// it was never seen.
        old: Option<bool>,
        /// Whether to enable it.
        new: bool,
    },
}

/// Whether two setting states are the same animation scale (unset ≡ 1).
pub fn same_scale(a: &SettingState, b: &SettingState) -> bool {
    let number = |state: &SettingState| match state {
        SettingState::Unset => Some(1.0),
        SettingState::Value(value) => value.as_str().parse::<f64>().ok(),
    };
    match (number(a), number(b)) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        _ => a == b,
    }
}

/// Packages a Home takeover changes, in any bundled profile: only `rusttv
/// launcher takeover|restore` may touch them (HISTORY.md § 1).
fn takeover_packages(profiles: &[DeviceProfile]) -> HashSet<String> {
    profiles
        .iter()
        .filter_map(|profile| profile.launcher.as_ref())
        .flat_map(|launcher| {
            std::iter::once(launcher.stock_package.clone())
                .chain(launcher.disable_home_packages.iter().cloned())
        })
        .collect()
}

/// Plans putting the device's settings and catalog packages as `target`
/// says (`history-restore`, HISTORY.md § 4.6). Read-only on the device.
///
/// - Settings already at their value (unset ≡ 1) are skipped. A current
///   value that isn't a valid setting value refuses the plan: it couldn't
///   be put back.
/// - Only catalog packages; never the packages a Home takeover changes
///   (named in the notes, with `rusttv launcher takeover|restore`); packages
///   not installed or already in their state are skipped.
/// - Disabling follows the protected-package policy: rules 1–3 are checked
///   live now (any protected package refuses the plan, `PACKAGE_PROTECTED`)
///   and again right before its step; rule 4 leaves untested packages out
///   unless `allow_untested`, and refuses the plan when nothing else is
///   left (`PROFILE_NOT_TESTED`).
/// - Refused while a restorable optimizer or privacy journal in `journals`
///   touches the same keys or packages, or a journal file can't be read.
/// - After the last step Home must still open the activity it opens now.
pub fn history_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    target: &RestoreTarget,
    profiles: &[DeviceProfile],
    journals: &Path,
    allow_untested: bool,
) -> Result<Plan, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let fingerprint = properties
        .get("ro.build.fingerprint")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            PlanError::precondition("the device doesn't report `ro.build.fingerprint`".to_owned())
        })?;
    let profile = profiles.iter().find(|profile| profile.matches(&properties));
    let tested: Vec<&str> = profile
        .and_then(|profile| profile.privacy.as_ref())
        .map(|privacy| {
            privacy
                .tested
                .iter()
                .map(|tested| tested.package.as_str())
                .collect()
        })
        .unwrap_or_default();
    let tested_on = profile.map_or_else(
        || "no compat profile matches this device".to_owned(),
        |profile| format!("this model's profile ({})", profile_file_name(profile)),
    );

    let mut planned = Vec::new();
    let mut notes = Vec::new();
    let mut skipped = Vec::new();
    for (key, to) in &target.settings {
        let output = adb.run(serial, &DeviceCommand::GetSetting(*key))?;
        let from = SettingState::from_output(&output).map_err(|error| {
            PlanError::precondition(format!(
                "{key} holds a value RustTV can't validate, so it couldn't be put back ({error}); \
                 nothing is planned"
            ))
        })?;
        if same_scale(&from, to) {
            continue;
        }
        notes.push(format!("{key}: {from} → {to}."));
        planned.push(DeviceCommand::SetSetting {
            key: *key,
            to: to.clone(),
            from,
        });
    }

    let catalog: HashSet<String> = bundled_catalog()
        .into_iter()
        .map(|entry| String::from(entry.package))
        .collect();
    let takeover = takeover_packages(profiles);
    let live = LiveState::read(adb, serial)?;
    let states = PackageStates::read(adb, serial)?;
    let mut refused = Vec::new();
    let mut untested = Vec::new();
    for (package, enabled) in &target.packages {
        let name = package.as_str();
        if takeover.contains(name) {
            skipped.push(format!(
                "Skipped {name}: a Home takeover changes it; only `rusttv launcher takeover` and \
                 `rusttv launcher restore` do."
            ));
            continue;
        }
        if !catalog.contains(name) {
            skipped.push(format!("Skipped {name}: not in RustTV's package catalog."));
            continue;
        }
        match (states.of(name), *enabled) {
            (PackageState::Absent, _) => {
                skipped.push(format!("Skipped {name}: not installed."));
            }
            (PackageState::Enabled, true) | (PackageState::Disabled, false) => {}
            (PackageState::Disabled, true) => {
                notes.push(format!("{name}: disabled → enabled."));
                planned.push(DeviceCommand::EnablePackage(package.clone()));
            }
            (PackageState::Enabled, false) => {
                if let Some(why) = live.protection(name) {
                    refused.push(format!("`{name}`: {why}"));
                } else if !tested.contains(&name) && !allow_untested {
                    untested.push(name.to_owned());
                } else {
                    let mark = if tested.contains(&name) {
                        ""
                    } else {
                        " UNTESTED on this model (--allow-untested)."
                    };
                    notes.push(format!("{name}: enabled → disabled.{mark}"));
                    planned.push(DeviceCommand::DisablePackage(package.clone()));
                }
            }
        }
    }
    if !refused.is_empty() {
        return Err(PlanError::protected(format!(
            "the snapshot disables protected packages, so nothing is planned: {}",
            refused.join("; ")
        )));
    }
    if planned.is_empty() {
        if !untested.is_empty() {
            return Err(PlanError::profile_not_tested(format!(
                "the only changes left disable packages not tested on this model ({tested_on}): \
                 {}. Pass --allow-untested to plan them anyway",
                untested.join(", ")
            )));
        }
        return Err(PlanError::precondition(format!(
            "nothing to restore: the device's settings and catalog packages already match{}",
            if skipped.is_empty() {
                String::new()
            } else {
                format!(" ({})", skipped.join(" "))
            }
        )));
    }
    notes.extend(skipped);
    if !untested.is_empty() {
        notes.push(format!(
            "Not planned, UNTESTED on this model ({tested_on}; pass --allow-untested to include \
             them): {}.",
            untested.join(", ")
        ));
    }

    let home = live
        .home
        .as_deref()
        .ok_or_else(|| PlanError::precondition("nothing handles Home on this device".to_owned()))?;
    let home: Component = home
        .parse()
        .map_err(|error| PlanError::precondition(format!("Home: {error}")))?;
    notes.extend([
        "Only settings and catalog packages: the launcher's own state (order, hidden apps, the \
         update switch) is restored on the TV, in Settings > History."
            .to_owned(),
        "Packages are checked against the protected-package policy now and again right before \
         each step; if one fails, the steps done so far are undone."
            .to_owned(),
        format!(
            "After the last step Home must still open {home}; otherwise every step is undone."
        ),
        "Undo with `rusttv history restore --journal <file>` (the journal this writes); a value \
         changed since is left as is."
            .to_owned(),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
    ]);
    let last = planned.len() - 1;
    let steps = planned
        .into_iter()
        .enumerate()
        .map(|(index, command)| {
            let checks = if index == last {
                vec![Check::HomeActivityIs(home.clone())]
            } else {
                Vec::new()
            };
            let requirements = match &command {
                DeviceCommand::DisablePackage(package) => vec![Check::NotProtected(package.clone())],
                _ => Vec::new(),
            };
            Step::new(command, checks).requiring(requirements)
        })
        .collect();
    let plan = Plan::new(
        HISTORY_RESTORE,
        serial,
        &fingerprint,
        "restore",
        steps,
        notes,
    )?;
    refuse_overlapping_journals(&plan, journals)?;
    Ok(plan)
}

/// What a step touches, for comparing plans with journals.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Touched {
    Setting(&'static str),
    Package(String),
}

impl std::fmt::Display for Touched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Setting(key) => f.write_str(key),
            Self::Package(package) => f.write_str(package),
        }
    }
}

fn touched(command: &DeviceCommand) -> Option<Touched> {
    match command {
        DeviceCommand::SetSetting { key, .. } => Some(Touched::Setting(key.as_str())),
        DeviceCommand::DisablePackage(package) | DeviceCommand::EnablePackage(package) => {
            Some(Touched::Package(package.as_str().to_owned()))
        }
        _ => None,
    }
}

/// Refuses `plan` (`PRECONDITION_FAILED`) while a restorable journal in
/// `journals` of the other kind touches the same settings or packages: for
/// a `history-restore` plan, optimizer and privacy journals; for an
/// optimizer or privacy plan, `history-restore` journals (HISTORY.md § 4.6).
/// Restoring them in another order would put back values the other change
/// set. For a `history-restore` plan, a journal file that can't be read
/// refuses it too (it may be such a journal).
pub fn refuse_overlapping_journals(plan: &Plan, journals: &Path) -> Result<(), PlanError> {
    let history = plan.name() == HISTORY_RESTORE;
    let others = |name: &str| {
        if history {
            name == PRIVACY || name.starts_with(OPTIMIZE_PLAN_PREFIX)
        } else {
            name == HISTORY_RESTORE
        }
    };
    let found = match restorable_journals(journals, others) {
        Ok(found) => found,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PlanError::journal(format!(
                "{}: {error}",
                journals.display()
            )));
        }
    };
    if history && let Some(file) = found.unreadable.first() {
        return Err(PlanError::precondition(format!(
            "can't read journal file {}, which may be an unrestored optimizer or privacy change \
             to the same settings or packages: {}. Check it, restore it if needed, then move it \
             out of {}",
            file.path.display(),
            file.error,
            journals.display()
        )));
    }
    let mine: BTreeSet<Touched> = plan
        .steps()
        .iter()
        .filter_map(|step| touched(step.command()))
        .collect();
    for (path, journal) in &found.journals {
        let both: Vec<String> = journal
            .steps
            .iter()
            .filter(|step| step.status.may_have_run())
            .filter_map(|step| touched(&step.command))
            .filter(|item| mine.contains(item))
            .collect::<BTreeSet<_>>()
            .iter()
            .map(ToString::to_string)
            .collect();
        if both.is_empty() {
            continue;
        }
        let undo = if journal.plan == HISTORY_RESTORE {
            "rusttv history restore --journal"
        } else {
            "rusttv optimize restore --journal"
        };
        return Err(PlanError::precondition(format!(
            "the {} change in {} ({}) still holds {}; restore it first with `{undo} {}`, then \
             plan again",
            journal.plan,
            path.display(),
            journal.status,
            both.join(", "),
            path.display()
        )));
    }
    Ok(())
}

/// Approves a `history-restore` plan by a pending approval made on the TV
/// (HISTORY.md § 6.3): only if its steps are exactly `changes`, each with
/// the value it had when approved (settings compare as animation scales,
/// unset ≡ 1). The journal then records `approved_by = { by = "tv",
/// approval = "<hash>" }`.
///
/// Only the CLI calls this, after it consumed the approval through the
/// door, which the TV allows once. Otherwise `NOT_APPROVED`, saying what
/// differs, and the CLI asks for the typed confirmation instead.
pub fn approve_by_tv(
    plan: &Plan,
    approval: &str,
    changes: &[TvChange],
) -> Result<Approval, PlanError> {
    let refuse = |reason: &str| {
        Err(PlanError::not_approved(&format!(
            "the plan isn't exactly what was approved on the TV: {reason}"
        )))
    };
    if plan.name() != HISTORY_RESTORE {
        return refuse(&format!("a `{}` plan", plan.name()));
    }
    if approval.len() != 64
        || !approval
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return refuse("the approval isn't a SHA-256");
    }
    if plan.steps().len() != changes.len() {
        return refuse(&format!(
            "{} steps for {} approved changes",
            plan.steps().len(),
            changes.len()
        ));
    }
    let mut left: Vec<&TvChange> = changes.iter().collect();
    for step in plan.steps() {
        let position = left.iter().position(|change| match (step.command(), change) {
            (
                DeviceCommand::SetSetting { key, to, from },
                TvChange::Setting {
                    key: approved,
                    old,
                    new,
                },
            ) => key == approved && same_scale(from, old) && same_scale(to, new),
            (
                DeviceCommand::DisablePackage(package),
                TvChange::Package {
                    package: approved,
                    old: Some(true),
                    new: false,
                },
            )
            | (
                DeviceCommand::EnablePackage(package),
                TvChange::Package {
                    package: approved,
                    old: Some(false),
                    new: true,
                },
            ) => package == approved,
            _ => false,
        });
        match position {
            Some(index) => {
                left.remove(index);
            }
            None => {
                return refuse(&format!(
                    "`{}` wasn't approved, or the value it starts from changed since",
                    crate::plan::adb_command(plan.serial(), step.command())
                ));
            }
        }
    }
    Ok(plan.approval_by(ApprovedBy::Tv {
        approval: approval.to_owned(),
    }))
}

/// Plans undoing a `history-restore` plan recorded in `journal`. Read-only
/// on the device.
///
/// The journal is untrusted: refused unless its plan is `history-restore`
/// and every undo is a `settings put/delete` of an allowlisted key (checked
/// when it was read) or enabling or disabling a catalog package that a Home
/// takeover doesn't change. A package re-disabled by the undo must pass
/// rules 1–3 now (`PACKAGE_PROTECTED` otherwise). A setting or package
/// already back is skipped; a setting changed since is left as is. Both are
/// marked undone in `journal` (in memory; [`crate::restore`] saves it) and
/// named in the notes; if nothing is left, the journal is marked restored
/// (in memory) and `PRECONDITION_FAILED` returned.
pub fn history_restore_undo_plan(
    adb: &dyn Adb,
    serial: &str,
    journal: &mut Journal,
    profiles: &[DeviceProfile],
) -> Result<Plan, PlanError> {
    if journal.plan != HISTORY_RESTORE {
        return Err(PlanError::precondition(format!(
            "this is a `{}` journal, not a {HISTORY_RESTORE} journal",
            journal.plan
        )));
    }
    let fingerprint = device_fingerprint(adb, serial)?;
    let plan = restore_plan(journal, serial, &fingerprint)?;
    let catalog: HashSet<String> = bundled_catalog()
        .into_iter()
        .map(|entry| String::from(entry.package))
        .collect();
    let takeover = takeover_packages(profiles);
    for step in plan.steps() {
        let allowed = match step.command() {
            DeviceCommand::SetSetting { .. } => true,
            DeviceCommand::EnablePackage(package) | DeviceCommand::DisablePackage(package) => {
                catalog.contains(package.as_str()) && !takeover.contains(package.as_str())
            }
            _ => false,
        };
        if !allowed {
            return Err(PlanError::precondition(format!(
                "the journal asks for more than putting back settings and catalog packages \
                 (`{}`); refusing to restore from it",
                crate::plan::adb_command(serial, step.command())
            )));
        }
    }

    let disabled = PackageStates::read(adb, serial)?;
    let live = LiveState::read(adb, serial)?;
    let mut skipped = Vec::new();
    let mut refused = Vec::new();
    for step in &mut journal.steps {
        if !step.status.may_have_run() {
            continue;
        }
        match &step.command {
            DeviceCommand::SetSetting { key, to, from } => {
                let output = adb.run(serial, &DeviceCommand::GetSetting(*key))?;
                let Ok(now) = SettingState::from_output(&output) else {
                    continue;
                };
                if now == *from {
                    step.status = StepStatus::Undone;
                    skipped.push(format!("Skipped {key}: it is {from} again already."));
                } else if now != *to {
                    step.status = StepStatus::Undone;
                    skipped.push(format!(
                        "Skipped {key}: it is {now} now, not the {to} RustTV set (changed \
                         since); left as is."
                    ));
                }
            }
            DeviceCommand::DisablePackage(package) => {
                if disabled.of(package.as_str()) != PackageState::Disabled {
                    step.status = StepStatus::Undone;
                    skipped.push(format!(
                        "Skipped {package}: it is enabled now (re-enabled since); left as is."
                    ));
                }
            }
            DeviceCommand::EnablePackage(package) => {
                if disabled.of(package.as_str()) != PackageState::Enabled {
                    step.status = StepStatus::Undone;
                    skipped.push(format!(
                        "Skipped {package}: it isn't enabled now (changed since); left as is."
                    ));
                } else if let Some(why) = live.protection(package.as_str()) {
                    refused.push(format!("`{package}`: {why}"));
                }
            }
            _ => {}
        }
    }
    if !refused.is_empty() {
        return Err(PlanError::protected(format!(
            "undoing would disable protected packages, so nothing is planned: {}",
            refused.join("; ")
        )));
    }
    if !journal.steps.iter().any(|step| step.status.may_have_run()) {
        journal.status = JournalStatus::Restored;
        return Err(PlanError::precondition(format!(
            "nothing to restore: {}",
            skipped.join(" ")
        )));
    }
    let plan = restore_plan(journal, serial, &fingerprint)?;
    Ok(plan.with_notes(skipped.into_iter().chain([
        "Right before each step a setting is read again; one changed since is left as is."
            .to_owned(),
    ])))
}

/// Plans putting this computer's copy of a history back into the TV's empty
/// history (`history-restore-tv`, HISTORY.md § 4.5): the export is written to
/// the door's inbox and imported, two launcher-private (RL) steps that change
/// no setting or app. After the import the TV's history must hold `head`
/// (this copy's newest entry). The launcher accepts it only into an empty
/// history (`HISTORY_NOT_EMPTY` otherwise, and the step fails).
pub fn restore_tv_plan(
    adb: &dyn Adb,
    serial: &str,
    export: Vec<u8>,
    head: &HistorySince,
    nonce: HistoryNonce,
    notes: Vec<String>,
) -> Result<Plan, PlanError> {
    let fingerprint = device_fingerprint(adb, serial)?;
    let steps = vec![
        Step::new(
            DeviceCommand::HistoryInbox {
                nonce: nonce.clone(),
                payload: InboxPayload::new(export),
            },
            Vec::new(),
        ),
        Step::new(
            DeviceCommand::HistoryCommit {
                kind: DoorKind::Import,
                nonce,
            },
            vec![Check::HistoryContinues(head.clone())],
        ),
    ];
    Plan::new(
        HISTORY_RESTORE_TV,
        serial,
        &fingerprint,
        "history",
        steps,
        notes,
    )
}

/// Plans granting (`grant`) or revoking the launcher's
/// `WRITE_SECURE_SETTINGS` (HISTORY.md § 7): one R2 step, undone by the
/// other. Refused when the launcher isn't installed or already is that way.
pub fn tv_settings_plan(adb: &dyn Adb, serial: &str, grant: bool) -> Result<Plan, PlanError> {
    let fingerprint = device_fingerprint(adb, serial)?;
    let info = adb.run(
        serial,
        &DeviceCommand::PackageInfo(PackageName::rusttv_launcher()),
    )?;
    let Some(granted) = launcher_grant(&info) else {
        return Err(PlanError::precondition(format!(
            "the RustTV launcher ({RUSTTV_LAUNCHER}) isn't installed; install it with `rusttv \
             launcher install`"
        )));
    };
    if granted == grant {
        return Err(PlanError::precondition(format!(
            "nothing to do: the RustTV launcher {} WRITE_SECURE_SETTINGS already",
            if grant { "holds" } else { "doesn't hold" }
        )));
    }
    let (name, word, command, notes) = if grant {
        (
            HISTORY_ALLOW_TV_SETTINGS,
            "allow",
            DeviceCommand::GrantSecureSettings,
            vec![
                "Lets the RustTV launcher change the three animation scales itself, after you \
                 approve each change on the TV with the code and hold OK. It writes nothing \
                 else: one Kotlin function with the keys as an enum (HISTORY.md § 7)."
                    .to_owned(),
                "Android drops the permission when the launcher is uninstalled; `rusttv history \
                 sync` says so."
                    .to_owned(),
                "Undo with `rusttv history revoke-tv-settings`.".to_owned(),
            ],
        )
    } else {
        (
            HISTORY_REVOKE_TV_SETTINGS,
            "revoke",
            DeviceCommand::RevokeSecureSettings,
            vec![
                "Settings approved on the TV then wait for your computer (`rusttv history \
                 apply-pending`)."
                    .to_owned(),
            ],
        )
    };
    Plan::new(
        name,
        serial,
        &fingerprint,
        word,
        vec![Step::new(command, Vec::new())],
        notes,
    )
}

/// The launcher's package section in `dumpsys package ai.rusttv.launcher`,
/// or `None` when it isn't installed.
fn launcher_section(output: &str) -> Option<&str> {
    let start = output.find(&format!("Package [{RUSTTV_LAUNCHER}]"))?;
    Some(&output[start..])
}

/// Whether the launcher holds `WRITE_SECURE_SETTINGS`, from `dumpsys package
/// ai.rusttv.launcher`; `None` when it isn't installed.
pub fn launcher_grant(output: &str) -> Option<bool> {
    let section = launcher_section(output)?;
    let line = format!("{}: granted=", crate::command::WRITE_SECURE_SETTINGS);
    Some(section.lines().any(|text| {
        text.trim()
            .strip_prefix(&line)
            .is_some_and(|rest| rest.starts_with("true"))
    }))
}

/// Whether the installed launcher is a test build: debuggable (`DEBUGGABLE`
/// in its `flags=[ … ]`) or profileable (`PROFILEABLE…` in any flags line),
/// from `dumpsys package ai.rusttv.launcher`. `None` when it isn't installed
/// or no `flags=` line is found: callers treat that like a test build.
pub fn launcher_test_build(output: &str) -> Option<bool> {
    let section = launcher_section(output)?;
    let mut found = false;
    let mut test = false;
    for line in section.lines().map(str::trim) {
        let Some((name, flags)) = line.split_once("=[") else {
            continue;
        };
        if !name.ends_with("lags") {
            continue;
        }
        if name == "flags" || name == "pkgFlags" {
            found = true;
        }
        test |= flags
            .split_whitespace()
            .any(|flag| flag == "DEBUGGABLE" || flag.contains("PROFILEABLE"));
    }
    found.then_some(test)
}

/// Whether the bytes of a `content read …/export/…` are an export (its
/// header line), not a refusal (`rusttv-result 1 error <CODE> <head>`).
pub fn is_export(output: &[u8]) -> bool {
    output.starts_with(b"rusttv-export ")
}

#[cfg(test)]
mod tests;

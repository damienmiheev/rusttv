//! The safety engine for changes to a device (SECURITY.md § Change lifecycle).
//!
//! Every change goes: [`Plan`] → [`Plan::dry_run`] → [`Plan::approve`] (a
//! human typed the confirmation) → [`apply`]. `apply` checks the device is
//! still the one planned for, records the original state in a [`Journal`]
//! before touching anything, then runs one step at a time, verifying each.
//! When a step or its verification fails, the completed steps are undone in
//! reverse. [`restore`] undoes an applied plan later, from its journal.
//!
//! Steps are typed [`DeviceCommand`]s; there is no way to put a shell string
//! in a plan. Steps above R1 must have an inverse ([`DeviceCommand::inverse`]).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::adb::Adb;
use crate::apps::{PackageListError, package_names};
use crate::bench::{Sleep, parse_wakefulness};
use crate::command::{
    Component, DeviceCommand, HistorySince, InvalidApk, PackageName, RiskClass, SettingKey,
    SettingState,
};
use crate::error::AdbError;
use crate::grant::Grant;
use crate::inspect::parse_properties;
use crate::journal::{ApprovedBy, Journal, JournalStatus, JournalStep, JournalStore, StepStatus};
use crate::launcher::{parse_home_candidates, parse_resumed_activity};
use crate::perf::{parse_home_activity, parse_process_list};
use crate::privacy::parse_bound_tv_input_services;

/// How often a polled step re-checks; see [`Step::polling_for`].
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A read-only fact about the device, checked before and after steps.
///
/// Serialized like [`DeviceCommand`]: `{kind = "package_enabled", arg = "com.example"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "arg", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Check {
    /// The package is not disabled for user 0 (absent from `pm list packages -d`).
    ///
    /// Also true for a package that isn't installed; plans only use it for
    /// packages they have seen on the device.
    PackageEnabled(PackageName),
    /// The package is disabled for user 0 (listed by `pm list packages -d`).
    PackageDisabled(PackageName),
    /// The Home button opens an activity of this package.
    HomeIs(PackageName),
    /// An activity of this package is the resumed (front) activity and a
    /// process of that package is running.
    InFront(PackageName),
    /// The package is installed (listed by `pm list packages`, enabled or not).
    PackageInstalled(PackageName),
    /// The package is not installed.
    PackageNotInstalled(PackageName),
    /// An activity of this package can handle Home (`query-activities`).
    HomeCandidate(PackageName),
    /// The Home button opens exactly this activity.
    HomeActivityIs(Component),
    /// The activities that can handle Home are exactly these (any order).
    HomeCandidatesAre(Vec<Component>),
    /// The package passes the protected-package check now (SECURITY.md §
    /// Protected package policy, rules 1–3): Home doesn't open it, it can't
    /// handle Home, it provides no input method or enabled accessibility
    /// service, and it isn't on `compat/protected.toml`. Output that can't be
    /// read counts as protected.
    NotProtected(PackageName),
    /// Each of these TV input services is still bound by the system
    /// (`dumpsys tv_input`); others may be bound too.
    TvInputServicesBound(Vec<Component>),
    /// The global setting is in this state (`settings get global <key>`;
    /// `null` is unset). Output that isn't a valid value never matches.
    SettingIs {
        /// The setting.
        key: SettingKey,
        /// Its expected state.
        state: SettingState,
    },
    /// The screen is on with no screensaver (`mWakefulness=Awake` in
    /// `dumpsys power`). Output without the field (another Android) counts
    /// as awake.
    Awake,
    /// The RustTV launcher holds `WRITE_SECURE_SETTINGS` (`true`) or doesn't
    /// (`false`), as `dumpsys package ai.rusttv.launcher` says
    /// (`android.permission.WRITE_SECURE_SETTINGS: granted=true`). Output
    /// without the launcher's package never matches.
    SecureSettingsGranted(bool),
    /// The TV's history exports the entries after this one, i.e. it holds
    /// the entry (`content read …/export/<entry>` prints an export, not a
    /// refusal). For `all`: the history isn't empty.
    HistoryContinues(HistorySince),
}

impl Check {
    /// The state `command` should leave behind, if it is a state-changing command.
    fn effect_of(command: &DeviceCommand) -> Option<Self> {
        match command {
            DeviceCommand::SetSetting { key, to, .. } => Some(Self::SettingIs {
                key: *key,
                state: to.clone(),
            }),
            DeviceCommand::DisablePackage(package) => Some(Self::PackageDisabled(package.clone())),
            DeviceCommand::EnablePackage(package) => Some(Self::PackageEnabled(package.clone())),
            DeviceCommand::InstallLauncher(_) | DeviceCommand::UpgradeLauncher(_) => {
                Some(Self::PackageInstalled(PackageName::rusttv_launcher()))
            }
            DeviceCommand::UninstallLauncher => {
                Some(Self::PackageNotInstalled(PackageName::rusttv_launcher()))
            }
            DeviceCommand::GrantSecureSettings => Some(Self::SecureSettingsGranted(true)),
            DeviceCommand::RevokeSecureSettings => Some(Self::SecureSettingsGranted(false)),
            _ => None,
        }
    }

    /// Whether the fact holds on the device now. Runs only R0 commands.
    pub fn holds(&self, adb: &dyn Adb, serial: &str) -> Result<bool, AdbError> {
        match self {
            Self::PackageEnabled(package) | Self::PackageDisabled(package) => {
                let output = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;
                let disabled = package_names(&output)?.contains(package.as_str());
                Ok(disabled == matches!(self, Self::PackageDisabled(_)))
            }
            Self::HomeIs(package) => {
                let output = adb.run(serial, &DeviceCommand::HomeActivity)?;
                Ok(parse_home_activity(&output)
                    .is_some_and(|(home, _)| home.trim() == package.as_str()))
            }
            Self::InFront(package) => {
                let activities = adb.run(serial, &DeviceCommand::Activities)?;
                let in_front = parse_resumed_activity(&activities)
                    .and_then(|component| component.split_once('/'))
                    .is_some_and(|(front, _)| front == package.as_str());
                if !in_front {
                    return Ok(false);
                }
                let processes = adb.run(serial, &DeviceCommand::ProcessList)?;
                Ok(parse_process_list(&processes)
                    .iter()
                    .any(|process| process.name == package.as_str()))
            }
            Self::PackageInstalled(package) | Self::PackageNotInstalled(package) => {
                let output = adb.run(serial, &DeviceCommand::ListPackages)?;
                let installed = package_names(&output)?.contains(package.as_str());
                Ok(installed == matches!(self, Self::PackageInstalled(_)))
            }
            Self::HomeCandidate(package) => {
                let output = adb.run(serial, &DeviceCommand::HomeCandidates)?;
                Ok(parse_home_candidates(&output)
                    .iter()
                    .any(|candidate| candidate.package() == package.as_str()))
            }
            Self::HomeActivityIs(component) => {
                let output = adb.run(serial, &DeviceCommand::HomeActivity)?;
                Ok(
                    parse_home_activity(&output).is_some_and(|(package, activity)| {
                        format!("{package}/{activity}") == component.as_str()
                    }),
                )
            }
            Self::HomeCandidatesAre(expected) => {
                let output = adb.run(serial, &DeviceCommand::HomeCandidates)?;
                let mut now: Vec<String> = parse_home_candidates(&output)
                    .into_iter()
                    .map(|candidate| candidate.component)
                    .collect();
                let mut expected: Vec<&str> = expected.iter().map(Component::as_str).collect();
                now.sort_unstable();
                now.dedup();
                expected.sort_unstable();
                expected.dedup();
                Ok(now == expected)
            }
            Self::NotProtected(package) => {
                Ok(crate::privacy::protection_now(adb, serial, package.as_str())?.is_none())
            }
            Self::TvInputServicesBound(expected) => {
                let output = adb.run(serial, &DeviceCommand::TvInputs)?;
                let bound = parse_bound_tv_input_services(&output);
                Ok(expected.iter().all(|service| bound.contains(service)))
            }
            Self::Awake => {
                let output = adb.run(serial, &DeviceCommand::PowerState)?;
                Ok(parse_wakefulness(&output).is_none_or(|state| state == "Awake"))
            }
            Self::SettingIs { key, state } => {
                let output = adb.run(serial, &DeviceCommand::GetSetting(*key))?;
                Ok(SettingState::from_output(&output).is_ok_and(|now| now == *state))
            }
            Self::SecureSettingsGranted(granted) => {
                let output = adb.run(
                    serial,
                    &DeviceCommand::PackageInfo(PackageName::rusttv_launcher()),
                )?;
                Ok(crate::history::launcher_grant(&output) == Some(*granted))
            }
            Self::HistoryContinues(since) => {
                let output =
                    adb.run_binary(serial, &DeviceCommand::HistoryExport(since.clone()), &[])?;
                Ok(crate::history::is_export(&output))
            }
        }
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackageEnabled(package) => write!(f, "{package} is enabled"),
            Self::PackageDisabled(package) => write!(f, "{package} is disabled"),
            Self::HomeIs(package) => write!(f, "Home opens {package}"),
            Self::InFront(package) => write!(f, "{package} is in front and running"),
            Self::PackageInstalled(package) => write!(f, "{package} is installed"),
            Self::PackageNotInstalled(package) => write!(f, "{package} is not installed"),
            Self::HomeCandidate(package) => write!(f, "{package} can handle Home"),
            Self::HomeActivityIs(component) => write!(f, "Home opens {component}"),
            Self::HomeCandidatesAre(components) => {
                let names: Vec<&str> = components.iter().map(Component::as_str).collect();
                write!(f, "the Home candidates are still {}", names.join(", "))
            }
            Self::NotProtected(package) => {
                write!(f, "{package} passes the protected-package check")
            }
            Self::TvInputServicesBound(services) => {
                let names: Vec<&str> = services.iter().map(Component::as_str).collect();
                write!(
                    f,
                    "the TV input services are still bound: {}",
                    names.join(", ")
                )
            }
            Self::SettingIs { key, state } => write!(f, "{key} is {state}"),
            Self::Awake => f.write_str("the screen is on, without a screensaver"),
            Self::SecureSettingsGranted(true) => {
                f.write_str("the RustTV launcher holds WRITE_SECURE_SETTINGS")
            }
            Self::SecureSettingsGranted(false) => {
                f.write_str("the RustTV launcher doesn't hold WRITE_SECURE_SETTINGS")
            }
            Self::HistoryContinues(since) => match since.as_entry() {
                Some(entry) => write!(f, "the TV's history holds entry {entry}"),
                None => f.write_str("the TV's history isn't empty"),
            },
        }
    }
}

/// One step of a plan: a command and how to tell it worked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    command: DeviceCommand,
    extra_checks: Vec<Check>,
    poll: Option<Duration>,
    requirements: Vec<Check>,
}

impl Step {
    /// A step running `command`. It is verified by the command's own effect
    /// (e.g. "package is disabled") and then by `extra_checks`, in order.
    pub fn new(command: DeviceCommand, extra_checks: Vec<Check>) -> Self {
        Self {
            command,
            extra_checks,
            poll: None,
            requirements: Vec::new(),
        }
    }

    /// Facts that must still hold right before the command runs; if one
    /// doesn't, the step isn't run and the plan rolls back. For checks that
    /// can change between planning and applying (the protected-package check).
    pub fn requiring(mut self, requirements: Vec<Check>) -> Self {
        self.requirements = requirements;
        self
    }

    /// Facts checked right before the command runs.
    pub fn requirements(&self) -> &[Check] {
        &self.requirements
    }

    /// Re-checks the verification every 500 ms for up to `timeout` before
    /// calling the step failed, for effects that take a moment (an app
    /// coming to the front).
    pub fn polling_for(mut self, timeout: Duration) -> Self {
        self.poll = Some(timeout);
        self
    }

    /// How long the verification may be retried, if at all.
    pub fn poll(&self) -> Option<Duration> {
        self.poll
    }

    /// Whether undoing needs a command. Transient (R0/R1) steps without an
    /// inverse need none: they changed nothing persistent.
    fn needs_undo(command: &DeviceCommand) -> bool {
        command.risk() > RiskClass::Transient || command.inverse().is_some()
    }

    /// The command this step runs.
    pub fn command(&self) -> &DeviceCommand {
        &self.command
    }

    /// Risk class of the command.
    pub fn risk(&self) -> RiskClass {
        self.command.risk()
    }

    /// Facts that must hold after the command ran, in the order they are checked.
    pub fn verification(&self) -> Vec<Check> {
        Check::effect_of(&self.command)
            .into_iter()
            .chain(self.extra_checks.iter().cloned())
            .collect()
    }

    /// The command that undoes this step, if any.
    pub fn undo(&self) -> Option<DeviceCommand> {
        self.command.inverse()
    }

    /// The state the undo command brings back. It must hold before the step
    /// runs, and is recorded in the journal as the original state.
    pub fn original_state(&self) -> Option<Check> {
        self.undo().as_ref().and_then(Check::effect_of)
    }
}

/// A checked list of steps for one device. Build with [`Plan::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    name: String,
    serial: String,
    fingerprint: String,
    confirmation: String,
    steps: Vec<Step>,
    notes: Vec<String>,
}

impl Plan {
    /// Builds a plan for the device with `serial` and build `fingerprint`
    /// (`ro.build.fingerprint`). `confirmation` is what a human must type to
    /// approve it. Fails if there are no steps, or a step above R1 has no
    /// inverse, unless it is [irreversible by design](DeviceCommand::is_irreversible)
    /// and the last step: then nothing after it can fail and need it undone.
    pub fn new(
        name: &str,
        serial: &str,
        fingerprint: &str,
        confirmation: &str,
        steps: Vec<Step>,
        notes: Vec<String>,
    ) -> Result<Self, PlanError> {
        if steps.is_empty() {
            return Err(
                PlanErrorKind::InvalidPlan("a plan needs at least one step".to_owned()).into(),
            );
        }
        if confirmation.trim().is_empty() {
            return Err(PlanErrorKind::InvalidPlan("empty confirmation phrase".to_owned()).into());
        }
        let last = steps.len() - 1;
        if let Some((_, step)) = steps.iter().enumerate().find(|(index, step)| {
            step.risk() > RiskClass::Transient
                && step.undo().is_none()
                && !(*index == last && step.command().is_irreversible())
        }) {
            return Err(PlanErrorKind::InvalidPlan(format!(
                "`{}` is {} but can't be undone",
                adb_command(serial, step.command()),
                step.risk()
            ))
            .into());
        }
        Ok(Self {
            name: name.to_owned(),
            serial: serial.to_owned(),
            fingerprint: fingerprint.to_owned(),
            confirmation: confirmation.trim().to_owned(),
            steps,
            notes,
        })
    }

    /// The plan with `notes` added after its own.
    pub(crate) fn with_notes(mut self, notes: impl IntoIterator<Item = String>) -> Self {
        self.notes.extend(notes);
        self
    }

    /// Short name of the kind of plan, e.g. `home-takeover`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// adb serial of the target device.
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// What a human must type to approve this plan.
    pub fn confirmation(&self) -> &str {
        &self.confirmation
    }

    /// The steps, in the order they run.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The highest risk class of any step.
    pub fn max_risk(&self) -> RiskClass {
        self.steps
            .iter()
            .map(Step::risk)
            .max()
            .unwrap_or(RiskClass::Read)
    }

    /// What would run, at what risk, and what gets recorded. Changes nothing.
    pub fn dry_run(&self) -> DryRun {
        DryRun {
            plan: self.name.clone(),
            serial: self.serial.clone(),
            fingerprint: self.fingerprint.clone(),
            max_risk: self.max_risk().code(),
            confirmation: self.confirmation.clone(),
            steps: self
                .steps
                .iter()
                .map(|step| DryRunStep {
                    risk: step.risk().code(),
                    command: adb_command(&self.serial, step.command()),
                    records: step.original_state().map(|check| check.to_string()),
                    verify: step
                        .verification()
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    undo: step.undo().map(|undo| adb_command(&self.serial, &undo)),
                    poll_ms: step
                        .poll()
                        .map(|poll| u64::try_from(poll.as_millis()).unwrap_or(u64::MAX)),
                })
                .collect(),
            notes: self.notes.clone(),
        }
    }

    /// Approves this plan with the phrase a human typed.
    ///
    /// Only the CLI calls this, with text read from an interactive terminal.
    /// Agents and MCP tools must never call it (SECURITY.md § AI agents): they
    /// may build and show plans, not approve them.
    pub fn approve(&self, typed: &str) -> Result<Approval, PlanError> {
        if typed.trim() == self.confirmation {
            Ok(Approval {
                plan: self.clone(),
                by: ApprovedBy::Terminal,
            })
        } else {
            Err(PlanError::not_approved(&format!(
                "the confirmation `{}` was not typed; nothing changed",
                self.confirmation
            )))
        }
    }

    /// Approves this plan by a time-limited delegation (SECURITY.md §
    /// Delegated approval), if `grant` is valid at `now_unix_ms` and is for
    /// this plan's device (serial), firmware (`ro.build.fingerprint`, which
    /// [`apply`] checks again on the live device) and kind of plan.
    ///
    /// Only the CLI calls this, and only with a grant a person created with
    /// `rusttv approve` at an interactive terminal (or [`Grant::for_this_run`]
    /// after typing `upgrade`). Agents and MCP tools must never call it, nor
    /// create or edit the grant file.
    pub fn approve_by_grant(&self, grant: &Grant, now_unix_ms: u64) -> Result<Approval, PlanError> {
        match grant.refuses(&self.name, &self.serial, &self.fingerprint, now_unix_ms) {
            None => Ok(Approval {
                plan: self.clone(),
                by: grant.approved_by(),
            }),
            Some(reason) => Err(PlanError::not_approved(&format!(
                "the delegation doesn't approve this plan: {reason}; nothing changed"
            ))),
        }
    }

    /// An approval of this plan by `by`, for [`crate::approve_by_tv`] after
    /// it checked the plan is exactly what was approved on the TV.
    pub(crate) fn approval_by(&self, by: ApprovedBy) -> Approval {
        Approval {
            plan: self.clone(),
            by,
        }
    }

    /// The same plan with another confirmation word (e.g. `upgrade` for an
    /// install plan run by `rusttv launcher upgrade`).
    pub fn with_confirmation(mut self, confirmation: &str) -> Result<Self, PlanError> {
        if confirmation.trim().is_empty() {
            return Err(PlanErrorKind::InvalidPlan("empty confirmation phrase".to_owned()).into());
        }
        confirmation.trim().clone_into(&mut self.confirmation);
        Ok(self)
    }

    /// `ro.build.fingerprint` of the device the plan was made for.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

/// Proof that a human approved one specific [`Plan`], directly or by a
/// delegation. Only [`Plan::approve`] and [`Plan::approve_by_grant`] create
/// it, and it is valid for that plan only.
#[derive(Debug)]
pub struct Approval {
    plan: Plan,
    by: ApprovedBy,
}

impl Approval {
    fn check(&self, plan: &Plan) -> Result<(), PlanError> {
        if self.plan == *plan {
            Ok(())
        } else {
            Err(PlanError::not_approved(
                "the approval is for a different plan; nothing changed",
            ))
        }
    }
}

/// Dry-run view of a plan. Serialized in `--dry-run --json` output (CLI.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRun {
    /// Kind of plan, e.g. `home-takeover`.
    pub plan: String,
    /// Target device.
    pub serial: String,
    /// `ro.build.fingerprint` of the target when the plan was made.
    pub fingerprint: String,
    /// Highest risk class, `R0`..`R3`.
    pub max_risk: &'static str,
    /// What a human must type to approve the plan.
    pub confirmation: String,
    /// The steps in order.
    pub steps: Vec<DryRunStep>,
    /// Warnings and recovery hints for the human.
    pub notes: Vec<String>,
}

/// One step of a [`DryRun`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunStep {
    /// Risk class, `R0`..`R3`.
    pub risk: &'static str,
    /// The exact adb command.
    pub command: String,
    /// Original state that is checked and written to the journal first.
    pub records: Option<String>,
    /// Facts checked after the command; any failure rolls the plan back.
    pub verify: Vec<String>,
    /// The command that undoes the step.
    pub undo: Option<String>,
    /// How long the checks are retried before the step counts as failed.
    pub poll_ms: Option<u64>,
}

impl fmt::Display for DryRun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Plan:     {} on {}", self.plan, self.serial)?;
        writeln!(f, "Firmware: {}", self.fingerprint)?;
        writeln!(f, "Risk:     {}", self.max_risk)?;
        for (number, step) in self.steps.iter().enumerate() {
            writeln!(f, "\n{}. [{}] {}", number + 1, step.risk, step.command)?;
            if let Some(records) = &step.records {
                writeln!(f, "   records first: {records}")?;
            }
            if !step.verify.is_empty() {
                let within = step
                    .poll_ms
                    .map(|ms| format!(" (within {} s)", ms.div_ceil(1000)))
                    .unwrap_or_default();
                writeln!(f, "   then checks:   {}{within}", step.verify.join("; "))?;
            }
            if let Some(undo) = &step.undo {
                writeln!(f, "   undo:          {undo}")?;
            }
        }
        if !self.notes.is_empty() {
            writeln!(f)?;
            for note in &self.notes {
                writeln!(f, "- {note}")?;
            }
        }
        Ok(())
    }
}

/// Applies an approved plan step by step; see the module docs.
///
/// `sleep` waits between re-checks of polled steps; tests pass a no-op.
/// Returns the final journal (status `applied`). On failure the device is
/// rolled back, and the error says whether that worked.
pub fn apply(
    adb: &dyn Adb,
    plan: &Plan,
    approval: &Approval,
    store: &mut dyn JournalStore,
    sleep: Sleep<'_>,
) -> Result<Journal, PlanError> {
    approval.check(plan)?;
    let serial = plan.serial.as_str();

    let fingerprint = device_fingerprint(adb, serial)?;
    if fingerprint != plan.fingerprint {
        return Err(PlanErrorKind::Precondition(format!(
            "the device now reports firmware `{fingerprint}`, not `{}`; make a new plan",
            plan.fingerprint
        ))
        .into());
    }
    for step in &plan.steps {
        if let Some(original) = step.original_state()
            && !original.holds(adb, serial)?
        {
            return Err(PlanErrorKind::Precondition(format!(
                "expected `{original}` before `{}`",
                adb_command(serial, step.command())
            ))
            .into());
        }
    }

    let mut journal = Journal {
        schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
        plan: plan.name.clone(),
        serial: plan.serial.clone(),
        fingerprint: plan.fingerprint.clone(),
        started_at_unix_ms: now_unix_ms(),
        status: JournalStatus::Applying,
        approved_by: Some(approval.by.clone()),
        restore_approved_by: None,
        steps: plan
            .steps
            .iter()
            .map(|step| JournalStep {
                adb: adb_command(serial, step.command()),
                command: step.command().clone(),
                original_state: step.original_state(),
                status: StepStatus::Pending,
            })
            .collect(),
    };
    // Nothing has changed yet, so a journal that can't be written just stops here.
    store.save(&journal).map_err(PlanError::journal)?;

    for (index, step) in plan.steps.iter().enumerate() {
        if let Some(reason) = first_failure(adb, serial, step.requirements(), "no longer true") {
            let reason = format!("before step {}: {reason}", index + 1);
            return Err(rollback(adb, &mut journal, store, reason));
        }
        journal.steps[index].status = StepStatus::Started;
        if let Err(error) = store.save(&journal) {
            let reason = format!("could not write the journal: {error}");
            return Err(rollback(adb, &mut journal, store, reason));
        }
        if let Some(reason) = run_and_verify(adb, serial, step, sleep) {
            let reason = format!("step {}: {reason}", index + 1);
            return Err(rollback(adb, &mut journal, store, reason));
        }
        journal.steps[index].status = StepStatus::Done;
        if let Err(error) = store.save(&journal) {
            let reason = format!("could not write the journal: {error}");
            return Err(rollback(adb, &mut journal, store, reason));
        }
    }

    journal.status = JournalStatus::Applied;
    if let Err(error) = store.save(&journal) {
        let reason = format!("could not write the journal: {error}");
        return Err(rollback(adb, &mut journal, store, reason));
    }
    Ok(journal)
}

/// The plan that undoes `journal`: the inverse of every step that started,
/// in reverse order, for the device `serial` with build `fingerprint`.
///
/// The journal is a file on disk and so untrusted input: callers must still
/// check that the steps are what they expect (see `takeover_restore_plan`).
pub fn restore_plan(journal: &Journal, serial: &str, fingerprint: &str) -> Result<Plan, PlanError> {
    if !journal.status.is_restorable() {
        return Err(PlanErrorKind::Precondition(format!(
            "this journal is already {}; nothing to restore",
            journal.status
        ))
        .into());
    }
    let steps: Vec<Step> = journal
        .steps
        .iter()
        .rev()
        .filter(|step| step.status.may_have_run() && Step::needs_undo(&step.command))
        .map(|step| {
            step.command
                .inverse()
                .map(|undo| Step::new(undo, Vec::new()))
                .ok_or_else(|| {
                    PlanError::from(PlanErrorKind::InvalidPlan(format!(
                        "`{}` in the journal can't be undone",
                        step.adb
                    )))
                })
        })
        .collect::<Result<_, _>>()?;
    if steps.is_empty() {
        return Err(PlanErrorKind::Precondition(
            "no step of this journal ran; nothing to restore".to_owned(),
        )
        .into());
    }
    let mut notes = Vec::new();
    if fingerprint != journal.fingerprint {
        notes.push(format!(
            "The journal was recorded on firmware `{}`; the device now reports `{fingerprint}`.",
            journal.fingerprint
        ));
    }
    Plan::new(
        &format!("{}-restore", journal.plan),
        serial,
        fingerprint,
        "restore",
        steps,
        notes,
    )
}

/// Undoes the steps of `journal` in reverse, as planned by [`restore_plan`],
/// and updates the journal. Restoring is never rolled back: a failed undo is
/// reported with the exact command to run by hand.
///
/// A setting changed since the plan ran (neither the value the step set nor
/// the one it found) is left as is, never overwritten (OPTIMIZER.md § 4,
/// owner decision 3): its step is marked undone and named in the returned
/// notes.
pub fn restore(
    adb: &dyn Adb,
    plan: &Plan,
    approval: &Approval,
    journal: &mut Journal,
    store: &mut dyn JournalStore,
) -> Result<Vec<String>, PlanError> {
    approval.check(plan)?;
    let expected = restore_plan(journal, &plan.serial, &plan.fingerprint)?;
    if expected.steps != plan.steps {
        return Err(PlanErrorKind::InvalidPlan(
            "the restore plan doesn't match its journal".to_owned(),
        )
        .into());
    }
    journal.restore_approved_by = Some(approval.by.clone());
    let (mut problems, left_as_is) = undo_steps(adb, &plan.serial, journal, store, true);
    journal.status = if problems.is_empty() {
        JournalStatus::Restored
    } else {
        JournalStatus::RestoreFailed
    };
    if let Err(error) = store.save(journal) {
        problems.push(format!("could not write the journal: {error}"));
    }
    if problems.is_empty() {
        Ok(left_as_is)
    } else {
        Err(PlanErrorKind::RestoreFailed(problems).into())
    }
}

/// Runs a step and its checks, re-checking polled steps; returns why it
/// failed, if it did.
fn run_and_verify(adb: &dyn Adb, serial: &str, step: &Step, sleep: Sleep<'_>) -> Option<String> {
    if let Err(error) = crate::adb::run_step(adb, serial, step.command()) {
        return Some(format!(
            "`{}` failed: {error}",
            adb_command(serial, step.command())
        ));
    }
    let checks = step.verification();
    let retries = step.poll().map_or(0, |timeout| {
        timeout.as_millis().div_ceil(POLL_INTERVAL.as_millis())
    });
    let mut failure = first_failure(adb, serial, &checks, "verification failed: expected");
    for _ in 0..retries {
        if failure.is_none() {
            break;
        }
        sleep(POLL_INTERVAL);
        failure = first_failure(adb, serial, &checks, "verification failed: expected");
    }
    match (failure, step.poll()) {
        (Some(reason), Some(timeout)) => Some(format!("{reason} within {} s", timeout.as_secs())),
        (failure, _) => failure,
    }
}

/// The first of `checks` that doesn't hold, as `<failed> `check``.
fn first_failure(adb: &dyn Adb, serial: &str, checks: &[Check], failed: &str) -> Option<String> {
    checks
        .iter()
        .find_map(|check| match check.holds(adb, serial) {
            Ok(true) => None,
            Ok(false) => Some(format!("{failed} `{check}`")),
            Err(error) => Some(format!("could not verify `{check}`: {error}")),
        })
}

/// Rolls back after a failure and returns the error to report.
fn rollback(
    adb: &dyn Adb,
    journal: &mut Journal,
    store: &mut dyn JournalStore,
    reason: String,
) -> PlanError {
    let serial = journal.serial.clone();
    // Nothing can have changed a setting in between: a value other than the
    // two expected is the failed step's own doing, so it is undone.
    let (mut problems, _) = undo_steps(adb, &serial, journal, store, false);
    journal.status = if problems.is_empty() {
        JournalStatus::RolledBack
    } else {
        JournalStatus::RollbackFailed
    };
    if let Err(error) = store.save(journal) {
        problems.push(format!("could not write the journal: {error}"));
    }
    if problems.is_empty() {
        PlanErrorKind::RolledBack(reason).into()
    } else {
        PlanErrorKind::RollbackFailed { reason, problems }.into()
    }
}

/// Undoes every step that may have run, last first, verifying each undo.
/// An undo that fails while the device already is in the original state
/// (e.g. Android refused both the step and its undo) still counts as undone. Keeps going after a failure;
/// returns what went wrong, with the command to run by hand, and (with
/// `skip_drifted`) the settings left as is because they changed since.
fn undo_steps(
    adb: &dyn Adb,
    serial: &str,
    journal: &mut Journal,
    store: &mut dyn JournalStore,
    skip_drifted: bool,
) -> (Vec<String>, Vec<String>) {
    let mut problems = Vec::new();
    let mut left_as_is = Vec::new();
    for index in (0..journal.steps.len()).rev() {
        let step = &journal.steps[index];
        if !step.status.may_have_run() || !Step::needs_undo(&step.command) {
            continue;
        }
        if skip_drifted && let Some(note) = drifted_setting(adb, serial, &step.command) {
            left_as_is.push(note);
            journal.steps[index].status = StepStatus::Undone;
            if let Err(error) = store.save(journal) {
                problems.push(format!("could not write the journal: {error}"));
            }
            continue;
        }
        let Some(undo) = step.command.inverse() else {
            problems.push(format!("`{}` can't be undone", step.adb));
            continue;
        };
        let undo_step = Step::new(undo, Vec::new());
        let failure = run_and_verify(adb, serial, &undo_step, &|_| {}).filter(|_| {
            // The undo's goal may already hold: a step Android refused never
            // changed anything, and Android may refuse its undo the same way.
            !Check::effect_of(undo_step.command())
                .is_some_and(|original| original.holds(adb, serial).unwrap_or(false))
        });
        match failure {
            None => {
                journal.steps[index].status = StepStatus::Undone;
                if let Err(error) = store.save(journal) {
                    problems.push(format!("could not write the journal: {error}"));
                }
            }
            Some(reason) => problems.push(format!(
                "{reason}; run it by hand: {}",
                adb_command(serial, undo_step.command())
            )),
        }
    }
    (problems, left_as_is)
}

/// For a [`DeviceCommand::SetSetting`] step: a note if its setting now
/// holds neither the value the step set nor the one it found, i.e. someone
/// changed it since. `None` for other commands, and when the setting can't
/// be read or holds an invalid value: then the undo runs and its
/// verification reports any problem.
pub(crate) fn drifted_setting(
    adb: &dyn Adb,
    serial: &str,
    command: &DeviceCommand,
) -> Option<String> {
    let DeviceCommand::SetSetting { key, to, from } = command else {
        return None;
    };
    let output = adb.run(serial, &DeviceCommand::GetSetting(*key)).ok()?;
    let now = SettingState::from_output(&output).ok()?;
    (now != *to && now != *from).then(|| {
        format!(
            "Skipped {key}: it is {now} now, not the {to} RustTV set (changed since); left as is."
        )
    })
}

/// `ro.build.fingerprint` of the device; plans and journals are tied to it.
pub fn device_fingerprint(adb: &dyn Adb, serial: &str) -> Result<String, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    properties
        .get("ro.build.fingerprint")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| {
            PlanErrorKind::Precondition(
                "the device doesn't report `ro.build.fingerprint`".to_owned(),
            )
            .into()
        })
}

/// The exact command line a human would type, e.g. `adb -s tv shell pm ...`.
pub(crate) fn adb_command(serial: &str, command: &DeviceCommand) -> String {
    format!("adb -s {serial} {}", command.adb_args().join(" "))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Planning, approving, applying or restoring a change failed.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct PlanError {
    kind: PlanErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum PlanErrorKind {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error("invalid plan: {0}")]
    InvalidPlan(String),
    #[error("{0}")]
    Precondition(String),
    #[error("not approved: {0}")]
    NotApproved(String),
    #[error("journal: {0}")]
    Journal(String),
    #[error("{0}")]
    NoJournal(String),
    #[error("{0}")]
    NotTested(String),
    #[error("{0}")]
    ProfileNotTested(String),
    #[error("{0}")]
    OptimizeNotTested(String),
    #[error("{0}")]
    Protected(String),
    #[error("privacy choices: {0}")]
    InvalidChoices(String),
    #[error(transparent)]
    InvalidApk(#[from] InvalidApk),
    #[error("{0}; every completed step was undone and verified")]
    RolledBack(String),
    #[error(
        "{reason}; rolling back ALSO failed: {}. Recovery is not guaranteed; run those commands by hand, or restore from the journal (`rusttv launcher restore`, or `rusttv optimize restore` for a privacy or optimizer plan) once the device is reachable",
        problems.join("; ")
    )]
    RollbackFailed {
        reason: String,
        problems: Vec<String>,
    },
    #[error(
        "restore failed: {}. Recovery is not guaranteed; run those commands by hand",
        .0.join("; ")
    )]
    RestoreFailed(Vec<String>),
}

impl From<PackageListError> for PlanErrorKind {
    fn from(error: PackageListError) -> Self {
        Self::Adb(error.into())
    }
}

impl<E: Into<PlanErrorKind>> From<E> for PlanError {
    fn from(kind: E) -> Self {
        Self { kind: kind.into() }
    }
}

impl PlanError {
    /// Stable machine-readable code (CLI.md).
    pub fn code(&self) -> &'static str {
        match &self.kind {
            PlanErrorKind::Adb(error) => error.code(),
            PlanErrorKind::InvalidPlan(_) => "INVALID_PLAN",
            PlanErrorKind::Precondition(_) => "PRECONDITION_FAILED",
            PlanErrorKind::NotApproved(_) => "NOT_APPROVED",
            PlanErrorKind::Journal(_) => "JOURNAL_FAILED",
            PlanErrorKind::NoJournal(_) => "NO_JOURNAL",
            PlanErrorKind::NotTested(_) => "TAKEOVER_NOT_TESTED",
            PlanErrorKind::ProfileNotTested(_) => "PROFILE_NOT_TESTED",
            PlanErrorKind::OptimizeNotTested(_) => "OPTIMIZE_NOT_TESTED",
            PlanErrorKind::Protected(_) => "PACKAGE_PROTECTED",
            PlanErrorKind::InvalidChoices(_) => "INVALID_CHOICES",
            PlanErrorKind::InvalidApk(_) => "INVALID_APK",
            PlanErrorKind::RolledBack(_) => "ROLLED_BACK",
            PlanErrorKind::RollbackFailed { .. } => "ROLLBACK_FAILED",
            PlanErrorKind::RestoreFailed(_) => "RESTORE_FAILED",
        }
    }

    /// The device could not be reached before anything changed.
    pub fn is_device_unreachable(&self) -> bool {
        matches!(&self.kind, PlanErrorKind::Adb(error) if error.is_device_unreachable())
    }

    /// The plan was not approved: no interactive terminal, or the wrong confirmation.
    pub fn not_approved(reason: &str) -> Self {
        PlanErrorKind::NotApproved(reason.to_owned()).into()
    }

    /// Reading or writing a journal failed.
    pub fn journal(error: impl fmt::Display) -> Self {
        PlanErrorKind::Journal(error.to_string()).into()
    }

    pub(crate) fn precondition(reason: String) -> Self {
        PlanErrorKind::Precondition(reason).into()
    }

    /// No journal to restore from (`NO_JOURNAL`).
    pub fn no_journal(reason: String) -> Self {
        PlanErrorKind::NoJournal(reason).into()
    }

    pub(crate) fn not_tested(reason: String) -> Self {
        PlanErrorKind::NotTested(reason).into()
    }

    pub(crate) fn profile_not_tested(reason: String) -> Self {
        PlanErrorKind::ProfileNotTested(reason).into()
    }

    pub(crate) fn optimize_not_tested(reason: String) -> Self {
        PlanErrorKind::OptimizeNotTested(reason).into()
    }

    pub(crate) fn invalid_plan(reason: String) -> Self {
        PlanErrorKind::InvalidPlan(reason).into()
    }

    pub(crate) fn protected(reason: String) -> Self {
        PlanErrorKind::Protected(reason).into()
    }

    /// The adb error inside, if this is one; otherwise this error back.
    pub(crate) fn into_adb(self) -> Result<AdbError, Self> {
        match self.kind {
            PlanErrorKind::Adb(error) => Ok(error),
            kind => Err(kind.into()),
        }
    }

    /// The privacy choices file can't be read or doesn't fit the catalog.
    pub(crate) fn invalid_choices(reason: impl Into<String>) -> Self {
        PlanErrorKind::InvalidChoices(reason.into()).into()
    }

    pub(crate) fn invalid_apk(reason: String) -> Self {
        PlanErrorKind::InvalidApk(InvalidApk::new(reason)).into()
    }
}

/// One error of every kind; see `crate::error::every_adb_error`.
#[cfg(test)]
pub(crate) fn every_plan_error() -> Vec<PlanError> {
    let all: Vec<PlanError> = vec![
        PlanErrorKind::InvalidPlan(String::new()).into(),
        PlanErrorKind::Precondition(String::new()).into(),
        PlanErrorKind::NotApproved(String::new()).into(),
        PlanErrorKind::Journal(String::new()).into(),
        PlanErrorKind::NoJournal(String::new()).into(),
        PlanErrorKind::NotTested(String::new()).into(),
        PlanErrorKind::ProfileNotTested(String::new()).into(),
        PlanErrorKind::OptimizeNotTested(String::new()).into(),
        PlanErrorKind::Protected(String::new()).into(),
        PlanErrorKind::InvalidChoices(String::new()).into(),
        PlanErrorKind::InvalidApk(InvalidApk::new(String::new())).into(),
        PlanErrorKind::RolledBack(String::new()).into(),
        PlanErrorKind::RollbackFailed {
            reason: String::new(),
            problems: Vec::new(),
        }
        .into(),
        PlanErrorKind::RestoreFailed(Vec::new()).into(),
    ];
    for error in &all {
        match error.kind {
            PlanErrorKind::Adb(_)
            | PlanErrorKind::InvalidPlan(_)
            | PlanErrorKind::Precondition(_)
            | PlanErrorKind::NotApproved(_)
            | PlanErrorKind::Journal(_)
            | PlanErrorKind::NoJournal(_)
            | PlanErrorKind::NotTested(_)
            | PlanErrorKind::ProfileNotTested(_)
            | PlanErrorKind::OptimizeNotTested(_)
            | PlanErrorKind::Protected(_)
            | PlanErrorKind::InvalidChoices(_)
            | PlanErrorKind::InvalidApk(_)
            | PlanErrorKind::RolledBack(_)
            | PlanErrorKind::RollbackFailed { .. }
            | PlanErrorKind::RestoreFailed(_) => {}
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;
    use crate::journal::tests::MemoryJournal;

    const SERIAL: &str = "tv";
    const GETPROP: &str = "[ro.build.fingerprint]: [google/tv/x:14/A/1:user/release-keys]\n";

    fn package(name: &str) -> PackageName {
        PackageName::new(name).unwrap()
    }

    fn disable(name: &str) -> Step {
        Step::new(DeviceCommand::DisablePackage(package(name)), Vec::new())
    }

    fn plan(steps: Vec<Step>) -> Plan {
        Plan::new(
            "test",
            SERIAL,
            "google/tv/x:14/A/1:user/release-keys",
            "yes",
            steps,
            Vec::new(),
        )
        .unwrap()
    }

    /// `pm list packages -d` output listing `names`.
    fn disabled(names: &[&str]) -> String {
        names
            .iter()
            .map(|name| format!("package:{name}\n"))
            .collect()
    }

    #[test]
    fn steps_above_transient_must_be_reversible() {
        let force_stop = Step::new(DeviceCommand::ForceStop(package("a.b")), Vec::new());
        assert!(Plan::new("t", SERIAL, "f", "yes", vec![force_stop], Vec::new()).is_ok());
        assert_eq!(
            Plan::new("t", SERIAL, "f", "yes", Vec::new(), Vec::new())
                .unwrap_err()
                .code(),
            "INVALID_PLAN"
        );
    }

    #[test]
    fn an_irreversible_step_is_allowed_only_last() {
        let uninstall = || Step::new(DeviceCommand::UninstallLauncher, Vec::new());
        assert!(Plan::new("t", SERIAL, "f", "yes", vec![uninstall()], Vec::new()).is_ok());
        let error = Plan::new(
            "t",
            SERIAL,
            "f",
            "yes",
            vec![uninstall(), disable("a.b")],
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "INVALID_PLAN");
        assert!(error.to_string().contains("can't be undone"), "{error}");
    }

    #[test]
    fn only_the_exact_phrase_approves_and_only_its_own_plan() {
        let first = plan(vec![disable("a.b")]);
        let second = plan(vec![disable("c.d")]);
        assert_eq!(first.approve("y").unwrap_err().code(), "NOT_APPROVED");
        assert_eq!(first.approve("").unwrap_err().code(), "NOT_APPROVED");

        let approval = first.approve(" yes\n").unwrap();
        let adb = FakeAdb::new();
        let error = apply(
            &adb,
            &second,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "NOT_APPROVED");
        assert!(adb.calls().is_empty(), "nothing may run without approval");
    }

    #[test]
    fn a_grant_approves_only_a_plan_it_covers_and_the_journal_says_so() {
        use crate::grant::GrantScope;

        let fingerprint = "google/tv/x:14/A/1:user/release-keys";
        let now = 1_761_300_000_000;
        let launcher_plan = |name: &str, serial: &str| {
            Plan::new(
                name,
                serial,
                fingerprint,
                "takeover",
                vec![disable("a.b")],
                Vec::new(),
            )
            .unwrap()
        };
        let grant = Grant::new(
            SERIAL,
            fingerprint,
            GrantScope::Launcher,
            now,
            Duration::from_secs(1800),
        )
        .unwrap();

        // Expired, another device, another kind of plan: refused.
        let takeover = launcher_plan("home-takeover", SERIAL);
        let expired = takeover.approve_by_grant(&grant, now + 1_800_000);
        assert_eq!(expired.unwrap_err().code(), "NOT_APPROVED");
        let other = launcher_plan("home-takeover", "other-tv").approve_by_grant(&grant, now);
        assert_eq!(other.unwrap_err().code(), "NOT_APPROVED");
        let uninstall = launcher_plan("launcher-uninstall", SERIAL).approve_by_grant(&grant, now);
        assert_eq!(uninstall.unwrap_err().code(), "NOT_APPROVED");
        let wrong_firmware = plan(vec![disable("a.b")]);
        let other_firmware = Grant::new(
            SERIAL,
            "other",
            GrantScope::Launcher,
            now,
            Duration::from_secs(60),
        )
        .unwrap();
        let refused = launcher_plan("home-takeover", SERIAL).approve_by_grant(&other_firmware, now);
        assert!(refused.unwrap_err().to_string().contains("firmware"));

        // Valid: approves that plan only, and the journal records the grant.
        let approval = takeover.approve_by_grant(&grant, now + 1).unwrap();
        let adb = FakeAdb::new();
        let error = apply(
            &adb,
            &wrong_firmware,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "NOT_APPROVED");
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond_sequence(
                DeviceCommand::ListDisabledPackages,
                [disabled(&[]), disabled(&["a.b"])],
            )
            .respond(DeviceCommand::DisablePackage(package("a.b")), "");
        let mut store = MemoryJournal::default();
        apply(&adb, &takeover, &approval, &mut store, &|_| {}).unwrap();
        assert_eq!(
            store.last().approved_by,
            Some(ApprovedBy::Delegation {
                granted_at_unix_ms: now
            })
        );
    }

    #[test]
    fn dry_run_lists_risk_command_record_check_and_undo() {
        let text = plan(vec![disable("a.b")]).dry_run().to_string();
        for expected in [
            "Risk:     R3",
            "1. [R3] adb -s tv shell pm disable-user --user 0 a.b",
            "records first: a.b is enabled",
            "then checks:   a.b is disabled",
            "undo:          adb -s tv shell pm enable --user 0 a.b",
        ] {
            assert!(
                text.contains(expected),
                "{expected:?} missing from:\n{text}"
            );
        }
    }

    #[test]
    fn steps_run_in_order_with_the_journal_written_before_each() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond_sequence(
                DeviceCommand::ListDisabledPackages,
                [
                    disabled(&[]),
                    disabled(&[]),
                    disabled(&["a.b"]),
                    disabled(&["a.b", "c.d"]),
                ],
            )
            .respond(DeviceCommand::DisablePackage(package("a.b")), "")
            .respond(DeviceCommand::DisablePackage(package("c.d")), "");
        let plan = plan(vec![disable("a.b"), disable("c.d")]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal::default();

        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();

        assert_eq!(journal.status, JournalStatus::Applied);
        let statuses: Vec<Vec<StepStatus>> = store
            .saves
            .iter()
            .map(|saved| saved.steps.iter().map(|step| step.status).collect())
            .collect();
        use StepStatus::{Done, Pending, Started};
        assert_eq!(
            statuses,
            [
                vec![Pending, Pending],
                vec![Started, Pending],
                vec![Done, Pending],
                vec![Done, Started],
                vec![Done, Done],
                vec![Done, Done],
            ]
        );
        assert_eq!(
            store.saves[0].steps[1].original_state,
            Some(Check::PackageEnabled(package("c.d")))
        );
    }

    #[test]
    fn a_failure_mid_plan_undoes_completed_steps_in_reverse() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond_sequence(
                DeviceCommand::ListDisabledPackages,
                [
                    disabled(&[]),
                    disabled(&[]),
                    disabled(&["a.b"]),
                    disabled(&["a.b"]),
                    disabled(&[]),
                ],
            )
            .respond(DeviceCommand::DisablePackage(package("a.b")), "")
            // No response for c.d: the second step's command fails.
            .respond(DeviceCommand::EnablePackage(package("a.b")), "")
            .respond(DeviceCommand::EnablePackage(package("c.d")), "");
        let plan = plan(vec![disable("a.b"), disable("c.d")]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();

        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(error.to_string().contains("step 2"), "{error}");
        let changes: Vec<DeviceCommand> = adb
            .calls()
            .into_iter()
            .filter(|command| command.risk() > RiskClass::Transient)
            .collect();
        assert_eq!(
            changes,
            [
                DeviceCommand::DisablePackage(package("a.b")),
                DeviceCommand::DisablePackage(package("c.d")),
                // The failed step may have half-run, so it is undone too, first.
                DeviceCommand::EnablePackage(package("c.d")),
                DeviceCommand::EnablePackage(package("a.b")),
            ]
        );
        let last = store.last();
        assert_eq!(last.status, JournalStatus::RolledBack);
        assert!(
            last.steps
                .iter()
                .all(|step| step.status == StepStatus::Undone)
        );
    }

    #[test]
    fn a_failed_verification_rolls_back() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            // The command "succeeds" but the package stays enabled.
            .respond(DeviceCommand::ListDisabledPackages, disabled(&[]))
            .respond(DeviceCommand::DisablePackage(package("a.b")), "")
            .respond(DeviceCommand::EnablePackage(package("a.b")), "");
        let plan = plan(vec![disable("a.b")]);
        let approval = plan.approve("yes").unwrap();

        let error = apply(
            &adb,
            &plan,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK");
        assert!(
            error.to_string().contains("expected `a.b is disabled`"),
            "{error}"
        );
    }

    #[test]
    fn a_failed_rollback_says_so_and_names_the_manual_command() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond_sequence(
                DeviceCommand::ListDisabledPackages,
                [disabled(&[]), disabled(&["a.b"])],
            )
            .respond(DeviceCommand::DisablePackage(package("a.b")), "")
            .respond(DeviceCommand::HomeActivity, "android/.Resolver\n");
        // The extra check fails, and re-enabling has no canned response.
        let step = Step::new(
            DeviceCommand::DisablePackage(package("a.b")),
            vec![Check::HomeIs(package("ai.rusttv.launcher"))],
        );
        let plan = plan(vec![step]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLBACK_FAILED");
        let message = error.to_string();
        assert!(
            message.contains("run it by hand: adb -s tv shell pm enable --user 0 a.b"),
            "{message}"
        );
        assert!(message.contains("Recovery is not guaranteed"), "{message}");
        assert_eq!(store.last().status, JournalStatus::RollbackFailed);
        assert_eq!(store.last().steps[0].status, StepStatus::Started);
    }

    #[test]
    fn nothing_runs_when_the_original_state_is_not_what_was_planned() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::ListDisabledPackages, disabled(&["a.b"]));
        let plan = plan(vec![disable("a.b")]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(store.saves.is_empty());
        assert!(
            !adb.calls()
                .iter()
                .any(|command| command.risk() > RiskClass::Read)
        );
    }

    #[test]
    fn nothing_runs_on_different_firmware() {
        let adb = FakeAdb::new().respond(
            DeviceCommand::GetProperties,
            "[ro.build.fingerprint]: [google/tv/x:14/B/2:user/release-keys]\n",
        );
        let plan = plan(vec![disable("a.b")]);
        let approval = plan.approve("yes").unwrap();
        let error = apply(
            &adb,
            &plan,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("make a new plan"), "{error}");
    }

    #[test]
    fn nothing_runs_when_the_journal_cannot_be_written() {
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::ListDisabledPackages, disabled(&[]));
        let plan = plan(vec![disable("a.b")]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal {
            fail_from: Some(0),
            ..MemoryJournal::default()
        };
        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "JOURNAL_FAILED");
        assert!(
            !adb.calls()
                .iter()
                .any(|command| command.risk() > RiskClass::Read)
        );
    }

    #[test]
    fn restore_after_an_interruption_undoes_the_started_step() {
        // The process died after journaling "started" for step 1.
        let journal = Journal {
            schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
            plan: "test".to_owned(),
            serial: SERIAL.to_owned(),
            fingerprint: "old".to_owned(),
            started_at_unix_ms: 0,
            status: JournalStatus::Applying,
            approved_by: None,
            restore_approved_by: None,
            steps: vec![
                JournalStep {
                    adb: String::new(),
                    command: DeviceCommand::DisablePackage(package("a.b")),
                    original_state: Some(Check::PackageEnabled(package("a.b"))),
                    status: StepStatus::Started,
                },
                JournalStep {
                    adb: String::new(),
                    command: DeviceCommand::DisablePackage(package("c.d")),
                    original_state: Some(Check::PackageEnabled(package("c.d"))),
                    status: StepStatus::Pending,
                },
            ],
        };
        let plan = restore_plan(&journal, SERIAL, "new").unwrap();
        assert_eq!(plan.steps().len(), 1, "only the started step is undone");
        assert!(
            plan.dry_run().notes[0].contains("now reports `new`"),
            "a firmware change is shown to the human"
        );

        let adb = FakeAdb::new()
            .respond(DeviceCommand::EnablePackage(package("a.b")), "")
            .respond(DeviceCommand::ListDisabledPackages, disabled(&[]));
        let approval = plan.approve("restore").unwrap();
        let mut journal = journal;
        let mut store = MemoryJournal::default();
        restore(&adb, &plan, &approval, &mut journal, &mut store).unwrap();
        assert_eq!(journal.status, JournalStatus::Restored);
        assert_eq!(journal.steps[0].status, StepStatus::Undone);
        assert_eq!(journal.steps[1].status, StepStatus::Pending);

        let error = restore_plan(&journal, SERIAL, "new").unwrap_err();
        assert!(error.to_string().contains("already restored"), "{error}");
    }

    #[test]
    fn a_failed_restore_is_reported_not_rolled_back() {
        let journal_step = JournalStep {
            adb: String::new(),
            command: DeviceCommand::DisablePackage(package("a.b")),
            original_state: None,
            status: StepStatus::Done,
        };
        let mut journal = Journal {
            schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
            plan: "test".to_owned(),
            serial: SERIAL.to_owned(),
            fingerprint: "f".to_owned(),
            started_at_unix_ms: 0,
            status: JournalStatus::Applied,
            approved_by: None,
            restore_approved_by: None,
            steps: vec![journal_step],
        };
        let plan = restore_plan(&journal, SERIAL, "f").unwrap();
        let approval = plan.approve("restore").unwrap();
        // No canned response for `pm enable`: the device refuses.
        let adb = FakeAdb::new();
        let mut store = MemoryJournal::default();
        let error = restore(&adb, &plan, &approval, &mut journal, &mut store).unwrap_err();
        assert_eq!(error.code(), "RESTORE_FAILED");
        assert_eq!(journal.status, JournalStatus::RestoreFailed);
        // After the refusal, whether the package is enabled anyway (it can't
        // be read here either).
        assert_eq!(
            adb.calls(),
            [
                DeviceCommand::EnablePackage(package("a.b")),
                DeviceCommand::ListDisabledPackages
            ]
        );
    }

    #[test]
    fn a_refused_undo_of_a_step_that_changed_nothing_counts_as_undone() {
        // As on the Philips TV: Android refused the step's command, so the
        // rollback's undo is refused too, but nothing needs undoing.
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::ListDisabledPackages, disabled(&[]));
        let plan = plan(vec![disable("a.b"), disable("c.d")]);
        let approval = plan.approve("yes").unwrap();
        let mut store = MemoryJournal::default();

        // `pm disable-user a.b` has no canned response: the device refuses it.
        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(store.last().steps[0].status, StepStatus::Undone);
    }
}

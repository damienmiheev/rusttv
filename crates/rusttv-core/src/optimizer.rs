//! Optimizer profiles (R2): put allowlisted global settings to a profile's
//! values, and back (OPTIMIZER.md § 1–5).
//!
//! An optimizer profile (`compat/optimizer/*.toml`, compiled in) says which
//! settings to change; a device profile's `[[optimizer]]` entry says whether
//! it was tested there. Both directions are [`Plan`]s of
//! [`DeviceCommand::SetSetting`] steps only, run by the engine in
//! [`crate::plan`]. Each step carries the value it found, so its inverse and
//! the journal's record of the original come from the command alone.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::adb::Adb;
use crate::command::{Component, DeviceCommand, SettingKey, SettingState, SettingValue};
use crate::compat::{DeviceProfile, ProfileOptimizer, firmware_family, profile_file_name};
use crate::inspect::parse_properties;
use crate::journal::{
    Journal, JournalStatus, RestorableJournals, StepStatus, UnreadableJournal,
    find_restorable_journal, find_restorable_journal_where, restorable_journals,
};
use crate::perf::parse_home_activity;
use crate::plan::{Check, Plan, PlanError, Step, adb_command, device_fingerprint, restore_plan};
use crate::privacy::{PRIVACY, privacy_restore_plan};

/// Name of the animation profile, `compat/optimizer/reduced-animations.toml`.
pub const REDUCED_ANIMATIONS: &str = "reduced-animations";

/// Start of every optimizer plan name, `optimize-<profile>-v<version>`.
pub const OPTIMIZE_PLAN_PREFIX: &str = "optimize-";

/// Version of the optimizer profile format.
pub const OPTIMIZER_SCHEMA_VERSION: u32 = 1;

/// What a person types to approve an optimizer plan.
const CONFIRMATION: &str = "optimize";

/// Every profile in `compat/optimizer/`, compiled in so a file on disk can't
/// widen what runs. A test fails if one is missing here.
const BUNDLED_OPTIMIZER_PROFILES: &[(&str, &str)] = &[(
    "reduced-animations.toml",
    include_str!("../../../compat/optimizer/reduced-animations.toml"),
)];

/// What an optimizer profile changes (OPTIMIZER.md § 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizerProfile {
    /// [`OPTIMIZER_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Name used on the command line, e.g. `reduced-animations`.
    pub name: String,
    /// Bumped on any change to the settings or values; device entries for
    /// another version count for nothing.
    pub version: u32,
    /// One line for people.
    pub description: String,
    /// The settings and the values it writes, in plan order.
    pub settings: Vec<ProfileSetting>,
}

/// One setting of an [`OptimizerProfile`]; key and value are validated
/// while parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSetting {
    /// An allowlisted key.
    pub key: SettingKey,
    /// The value the profile writes.
    pub value: SettingValue,
}

impl OptimizerProfile {
    /// The plan and journal name, `optimize-<name>-v<version>`, so a journal
    /// says exactly which version was applied.
    pub fn plan_name(&self) -> String {
        format!("{OPTIMIZE_PLAN_PREFIX}{}-v{}", self.name, self.version)
    }
}

/// The bundled optimizer profiles from `compat/optimizer/`.
pub fn bundled_optimizer_profiles() -> Vec<OptimizerProfile> {
    // Every bundled profile parses: `bundled_optimizer_profiles_are_valid`
    // checks that.
    BUNDLED_OPTIMIZER_PROFILES
        .iter()
        .filter_map(|(_, text)| toml::from_str(text).ok())
        .collect()
}

/// The bundled optimizer profile called `name`; `INVALID_PLAN` if there is none.
pub fn bundled_optimizer_profile(name: &str) -> Result<OptimizerProfile, PlanError> {
    bundled_optimizer_profiles()
        .into_iter()
        .find(|profile| profile.name == name)
        .ok_or_else(|| PlanError::invalid_plan(format!("no bundled optimizer profile `{name}`")))
}

/// The profile name in an optimizer plan name `optimize-<name>-v<n>`.
pub fn profile_of_plan(plan: &str) -> Option<&str> {
    let (name, version) = plan.strip_prefix(OPTIMIZE_PLAN_PREFIX)?.rsplit_once("-v")?;
    (!name.is_empty() && !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()))
        .then_some(name)
}

/// Which kind of `optimize` change a restore is about: the privacy profile
/// (R3) or an optimizer profile (R2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OptimizeKind {
    /// `optimize apply privacy`, journals named `privacy`.
    Privacy,
    /// `optimize apply reduced-animations`, journals `optimize-reduced-animations-v<n>`.
    ReducedAnimations,
}

impl OptimizeKind {
    /// Every kind.
    pub const ALL: [Self; 2] = [Self::Privacy, Self::ReducedAnimations];

    /// The name on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Privacy => PRIVACY,
            Self::ReducedAnimations => REDUCED_ANIMATIONS,
        }
    }

    /// The kind of a journal's plan, if it is an `optimize` one.
    pub fn of_plan(plan: &str) -> Option<Self> {
        if plan == PRIVACY {
            return Some(Self::Privacy);
        }
        let name = profile_of_plan(plan)?;
        Self::ALL
            .into_iter()
            .find(|kind| *kind != Self::Privacy && kind.as_str() == name)
    }

    fn matches(self, plan: &str) -> bool {
        Self::of_plan(plan) == Some(self)
    }
}

impl FromStr for OptimizeKind {
    type Err = InvalidOptimizeKind;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == name)
            .ok_or_else(|| InvalidOptimizeKind(name.to_owned()))
    }
}

/// Returned when parsing an [`OptimizeKind`] fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an optimize profile; expected `privacy` or `reduced-animations`")]
pub struct InvalidOptimizeKind(String);

/// What [`find_optimize_journal`] found.
#[derive(Debug)]
pub struct OptimizeJournalSearch {
    /// The newest restorable journal of the kind asked for, or of the only
    /// kind restorable; `NO_JOURNAL` if there is none, `PRECONDITION_FAILED`
    /// if no kind was asked for and both are restorable.
    pub found: Result<(OptimizeKind, PathBuf, Journal), PlanError>,
    /// `.toml` files in the directory that could not be read.
    pub unreadable: Vec<UnreadableJournal>,
}

/// The journal `rusttv optimize restore [<kind>]` restores: the newest
/// restorable one of `kind` for the device's `fingerprint`. Without a kind,
/// the one kind that has a restorable journal; if both have one, refused
/// with the two commands to choose from.
pub fn find_optimize_journal(
    dir: &Path,
    fingerprint: &str,
    kind: Option<OptimizeKind>,
) -> OptimizeJournalSearch {
    let search = |kind: OptimizeKind| match kind {
        OptimizeKind::Privacy => find_restorable_journal(dir, PRIVACY, fingerprint),
        _ => find_restorable_journal_where(dir, kind.as_str(), fingerprint, |plan| {
            kind.matches(plan)
        }),
    };
    if let Some(kind) = kind {
        let search = search(kind);
        return OptimizeJournalSearch {
            found: search.found.map(|(path, journal)| (kind, path, journal)),
            unreadable: search.unreadable,
        };
    }
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut first_error = None;
    for kind in OptimizeKind::ALL {
        let search = search(kind);
        // Every search reads the same directory.
        unreadable = search.unreadable;
        match search.found {
            Ok((path, journal)) => found.push((kind, path, journal)),
            Err(error) if error.code() == "NO_JOURNAL" => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    let found = match (first_error, found.len()) {
        (Some(error), _) => Err(error),
        (None, 1) => Ok(found.remove(0)),
        (None, 0) => Err(PlanError::no_journal(format!(
            "no privacy journal and no optimizer journal to restore for firmware `{fingerprint}` \
             in {}; pass `--journal <file>`",
            dir.display()
        ))),
        (None, _) => {
            let listed: Vec<String> = found
                .iter()
                .map(|(kind, path, _)| {
                    format!(
                        "`rusttv optimize restore {}` ({})",
                        kind.as_str(),
                        path.display()
                    )
                })
                .collect();
            Err(PlanError::precondition(format!(
                "more than one kind of optimize change can be restored on this device; choose \
                 one: {}",
                listed.join(", ")
            )))
        }
    };
    OptimizeJournalSearch { found, unreadable }
}

/// Plans undoing `journal` with the validator of `kind`; each refuses a
/// journal of the other kind (see [`privacy_restore_plan`] and
/// [`optimizer_restore_plan`]).
pub fn optimize_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    kind: OptimizeKind,
    journal: &mut Journal,
) -> Result<Plan, PlanError> {
    match kind {
        OptimizeKind::Privacy => privacy_restore_plan(adb, serial, journal),
        OptimizeKind::ReducedAnimations => {
            optimizer_restore_plan(adb, serial, REDUCED_ANIMATIONS, journal)
        }
    }
}

/// Where an optimizer profile stands on a device (OPTIMIZER.md § 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizerState {
    /// Tested on this model and firmware family with a measured benefit
    /// (`evidence`); not applied now.
    Suggested,
    /// Tested here (`apply` allowed) but without a measured benefit, so not
    /// suggested.
    Available,
    /// A restorable journal exists and every key still has the value it set.
    Applied,
    /// A restorable journal exists but some key has another value now.
    Drifted,
    /// Applied on another firmware of this model (updated since): restore
    /// with `--journal`, then apply again.
    Stale,
    /// Not tested on this model and firmware family: `apply` refuses it.
    NotTested,
    /// Every key already has the profile's value.
    AlreadySet,
}

/// One optimizer profile in `rusttv optimize analyze`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptimizerStatus {
    /// Profile name.
    pub profile: String,
    /// Profile version.
    pub version: u32,
    /// What it does.
    pub description: String,
    /// Where it stands.
    pub status: OptimizerState,
    /// Whether this device's profile records it as tested (this version).
    pub tested: bool,
    /// The committed report of its measured benefit here, if any.
    pub evidence: Option<String>,
    /// The journal behind `applied`, `drifted` or `stale`.
    pub journal: Option<String>,
    /// Each setting: the profile's value and the device's now.
    pub settings: Vec<SettingStatus>,
}

/// One setting of an [`OptimizerStatus`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingStatus {
    /// The key.
    pub key: SettingKey,
    /// The value the profile writes.
    pub value: SettingValue,
    /// What `settings get global <key>` prints now; `None` for `null` (unset).
    pub current: Option<String>,
}

/// The device profile's entry for this version of `profile`, if any.
fn device_entry<'a>(
    device: Option<&'a DeviceProfile>,
    profile: &OptimizerProfile,
) -> Option<&'a ProfileOptimizer> {
    device?
        .optimizer
        .iter()
        .find(|entry| entry.profile == profile.name && entry.version == profile.version)
}

/// Restorable `optimize-*` journals on any firmware; a missing directory has none.
fn optimizer_journals(dir: &Path) -> Result<RestorableJournals, PlanError> {
    match restorable_journals(dir, |plan| plan.starts_with(OPTIMIZE_PLAN_PREFIX)) {
        Ok(found) => Ok(found),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(RestorableJournals::default()),
        Err(error) => Err(PlanError::journal(format!("{}: {error}", dir.display()))),
    }
}

/// Whether two fingerprints belong to the same firmware family.
fn same_family(a: &str, b: &str) -> bool {
    firmware_family(a).is_some_and(|family| Some(family) == firmware_family(b))
}

/// Each bundled optimizer profile and where it stands on the device whose
/// `getprop` is `properties`, matched to `device`. Reads the settings (R0)
/// and the restorable journals in `journals`.
pub(crate) fn optimizer_statuses(
    adb: &dyn Adb,
    serial: &str,
    properties: &HashMap<String, String>,
    device: Option<&DeviceProfile>,
    journals: &Path,
) -> Result<Vec<OptimizerStatus>, PlanError> {
    let fingerprint = properties
        .get("ro.build.fingerprint")
        .map_or("", String::as_str);
    let mut current = HashMap::new();
    for key in SettingKey::ALL {
        let output = adb.run(serial, &DeviceCommand::GetSetting(key))?;
        current.insert(key, output.trim().to_owned());
    }
    let state_of = |key: SettingKey| {
        current
            .get(&key)
            .and_then(|output| SettingState::from_output(output).ok())
    };
    let journals = optimizer_journals(journals)?.journals;

    let mut statuses = Vec::new();
    for profile in bundled_optimizer_profiles() {
        let entry = device_entry(device, &profile).filter(|entry| entry.tested);
        let ours =
            |journal: &Journal| profile_of_plan(&journal.plan) == Some(profile.name.as_str());
        let applied = journals
            .iter()
            .find(|(_, journal)| ours(journal) && journal.fingerprint == fingerprint);
        let stale = journals.iter().find(|(_, journal)| {
            ours(journal)
                && journal.fingerprint != fingerprint
                && same_family(&journal.fingerprint, fingerprint)
        });
        let (status, journal) = if let Some((path, journal)) = applied {
            let drifted = journal.steps.iter().any(|step| match &step.command {
                DeviceCommand::SetSetting { key, to, .. } => {
                    step.status.may_have_run() && state_of(*key).as_ref() != Some(to)
                }
                _ => false,
            });
            let status = if drifted {
                OptimizerState::Drifted
            } else {
                OptimizerState::Applied
            };
            (status, Some(path))
        } else if let Some((path, _)) = stale {
            (OptimizerState::Stale, Some(path))
        } else if profile.settings.iter().all(|setting| {
            state_of(setting.key) == Some(SettingState::Value(setting.value.clone()))
        }) {
            (OptimizerState::AlreadySet, None)
        } else {
            let status = match entry {
                Some(entry) if entry.evidence.is_some() => OptimizerState::Suggested,
                Some(_) => OptimizerState::Available,
                None => OptimizerState::NotTested,
            };
            (status, None)
        };
        statuses.push(OptimizerStatus {
            status,
            tested: entry.is_some(),
            evidence: entry.and_then(|entry| entry.evidence.clone()),
            journal: journal.map(|path| path.display().to_string()),
            settings: profile
                .settings
                .iter()
                .map(|setting| SettingStatus {
                    key: setting.key,
                    value: setting.value.clone(),
                    current: current
                        .get(&setting.key)
                        .filter(|output| *output != "null")
                        .cloned(),
                })
                .collect(),
            profile: profile.name,
            version: profile.version,
            description: profile.description,
        });
    }
    Ok(statuses)
}

/// What a setting does, for the dry run.
fn effect(key: SettingKey) -> &'static str {
    match key {
        SettingKey::WindowAnimationScale => "window open/close animations",
        SettingKey::TransitionAnimationScale => {
            "activity transitions: launching an app, returning Home"
        }
        SettingKey::AnimatorDurationScale => "animator-based animations in every app",
    }
}

/// Builds an optimizer plan: only [`DeviceCommand::SetSetting`] steps, none
/// writing the value it found (OPTIMIZER.md § 4, plan shape).
fn settings_plan(
    name: &str,
    serial: &str,
    fingerprint: &str,
    steps: Vec<Step>,
    notes: Vec<String>,
) -> Result<Plan, PlanError> {
    for step in &steps {
        match step.command() {
            DeviceCommand::SetSetting { key, to, from } if to == from => {
                return Err(PlanError::precondition(format!(
                    "{key} is already {to}; nothing to change"
                )));
            }
            DeviceCommand::SetSetting { .. } => {}
            other => {
                return Err(PlanError::invalid_plan(format!(
                    "optimizer plans only change settings, not `{}`",
                    adb_command(serial, other)
                )));
            }
        }
    }
    Plan::new(name, serial, fingerprint, CONFIRMATION, steps, notes)
}

/// Plans putting `profile`'s settings on the device. Read-only.
///
/// - The device's profile (one of `devices`, normally
///   [`crate::bundled_profiles`]) must record this profile and version as
///   tested, else `OPTIMIZE_NOT_TESTED`; with `allow_untested` it is planned
///   anyway and the notes say it is UNTESTED on this model.
/// - Refused (`PRECONDITION_FAILED`) while a restorable optimizer journal in
///   `journals` exists for this model and firmware family: one at a time.
/// - A current value that isn't a valid [`SettingValue`] refuses the plan:
///   it couldn't be put back. Keys already at the profile's value are
///   skipped; if all are, `PRECONDITION_FAILED`.
/// - One `SetSetting` step per other key, in profile order; after the last,
///   Home must still open the activity it opens now.
pub fn optimizer_plan(
    adb: &dyn Adb,
    serial: &str,
    profile: &OptimizerProfile,
    devices: &[DeviceProfile],
    journals: &Path,
    allow_untested: bool,
) -> Result<Plan, PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
    let fingerprint = property("ro.build.fingerprint");
    let device = devices.iter().find(|device| device.matches(&properties));
    let tested_on = match device {
        Some(device) => format!("this model's profile ({})", profile_file_name(device)),
        None => format!(
            "no profile in compat/devices/ matches this device ({} {}, `{}`, firmware `{}`)",
            property("ro.product.manufacturer"),
            property("ro.product.model"),
            property("ro.product.device"),
            fingerprint,
        ),
    };
    let tested = device_entry(device, profile).is_some_and(|entry| entry.tested);
    if !tested && !allow_untested {
        return Err(PlanError::optimize_not_tested(format!(
            "`{}` v{} is not tested on this model and firmware family ({tested_on} has no \
             `[[optimizer]]` entry with profile = \"{}\", version = {}, tested = true); untested \
             devices get read-only commands only (SECURITY.md). For the first test on a device, \
             pass --allow-untested; the plan then says it is UNTESTED",
            profile.name, profile.version, profile.name, profile.version
        )));
    }

    let RestorableJournals {
        journals: open,
        unreadable,
    } = optimizer_journals(journals)?;
    if let Some((path, journal)) = open
        .iter()
        .find(|(_, journal)| same_family(&journal.fingerprint, fingerprint))
    {
        let name = profile_of_plan(&journal.plan).unwrap_or(&journal.plan);
        return Err(PlanError::precondition(format!(
            "an optimizer change ({}, {}) may still be applied on this model; one at a time: \
             restore it first with `rusttv optimize restore {name} --journal {}`, then apply again",
            journal.plan,
            journal.status,
            path.display()
        )));
    }

    let home = adb.run(serial, &DeviceCommand::HomeActivity)?;
    let home: Component = parse_home_activity(&home)
        .ok_or_else(|| PlanError::precondition("nothing handles Home on this device".to_owned()))
        .and_then(|(package, activity)| {
            format!("{package}/{activity}")
                .parse()
                .map_err(|error| PlanError::precondition(format!("Home: {error}")))
        })?;

    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    for setting in &profile.settings {
        let output = adb.run(serial, &DeviceCommand::GetSetting(setting.key))?;
        let from = SettingState::from_output(&output).map_err(|error| {
            PlanError::precondition(format!(
                "{} holds a value RustTV can't validate, so it couldn't be put back ({error}); \
                 nothing is planned",
                setting.key
            ))
        })?;
        let to = SettingState::Value(setting.value.clone());
        if from == to {
            skipped.push(format!("Skipped {}: already {to}.", setting.key));
        } else {
            changes.push((setting.key, to, from));
        }
    }
    if changes.is_empty() {
        return Err(PlanError::precondition(format!(
            "nothing to do: every setting of `{}` already has its value",
            profile.name
        )));
    }

    let mut notes: Vec<String> = changes
        .iter()
        .map(|(key, to, from)| format!("{key}: {from} → {to} ({}).", effect(*key)))
        .collect();
    notes.extend(skipped);
    notes.extend(unreadable.iter().map(|file| {
        format!(
            "Warning: {} can't be read ({}); if it is an optimizer journal that still needs \
             restoring, restore it before applying this.",
            file.path.display(),
            file.error
        )
    }));
    notes.push(if tested {
        format!("Tested on this model and firmware family, according to {tested_on}.")
    } else {
        format!(
            "UNTESTED on this model: {tested_on} doesn't record `{}` v{} as tested; planned \
             because of --allow-untested.",
            profile.name, profile.version
        )
    });
    notes.extend([
        format!(
            "Takes effect at once, no reboot. After the last step Home must still open {home}; \
             otherwise every step is undone."
        ),
        format!(
            "Undo with `rusttv optimize restore {}`: it puts back the values found now (a key \
             unset now is deleted again) and leaves alone any key changed since.",
            profile.name
        ),
        "Without ADB: Settings > System > Developer options > Window animation scale, \
         Transition animation scale, Animator duration scale > 1x."
            .to_owned(),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
    ]);

    let last = changes.len() - 1;
    let steps = changes
        .into_iter()
        .enumerate()
        .map(|(index, (key, to, from))| {
            let checks = if index == last {
                vec![Check::HomeActivityIs(home.clone())]
            } else {
                Vec::new()
            };
            Step::new(DeviceCommand::SetSetting { key, to, from }, checks)
        })
        .collect();
    settings_plan(&profile.plan_name(), serial, fingerprint, steps, notes)
}

/// Plans undoing an optimizer plan of profile `name` recorded in `journal`.
/// Read-only on the device.
///
/// The journal is untrusted: refused unless its plan is `optimize-<name>-v<n>`
/// and every undo is a `SetSetting` (keys and values were validated when it
/// was read). A key already back at the value found before is skipped; a key
/// that is neither that nor the value RustTV set was changed since and is
/// left as is (owner decision 3). Both are marked undone in `journal` (in
/// memory; [`crate::restore`] saves it) and named in the notes. If nothing
/// is left, the journal is marked restored (in memory) and
/// `PRECONDITION_FAILED` returned.
pub fn optimizer_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    name: &str,
    journal: &mut Journal,
) -> Result<Plan, PlanError> {
    if profile_of_plan(&journal.plan) != Some(name) {
        return Err(PlanError::precondition(format!(
            "this is a `{}` journal, not an {OPTIMIZE_PLAN_PREFIX}{name}-v<n> journal",
            journal.plan
        )));
    }
    let fingerprint = device_fingerprint(adb, serial)?;
    let plan = restore_plan(journal, serial, &fingerprint)?;
    if !plan
        .steps()
        .iter()
        .all(|step| matches!(step.command(), DeviceCommand::SetSetting { .. }))
    {
        return Err(PlanError::precondition(
            "the journal asks for more than putting settings back; refusing to restore from it"
                .to_owned(),
        ));
    }

    let mut skipped = Vec::new();
    for step in &mut journal.steps {
        let DeviceCommand::SetSetting { key, to, from } = &step.command else {
            continue;
        };
        if !step.status.may_have_run() {
            continue;
        }
        let output = adb.run(serial, &DeviceCommand::GetSetting(*key))?;
        // An invalid value now is neither ours nor the original: the undo
        // puts the original back, and its verification reports a failure.
        let Ok(now) = SettingState::from_output(&output) else {
            continue;
        };
        if now == *from {
            step.status = StepStatus::Undone;
            skipped.push(format!("Skipped {key}: it is {from} again already."));
        } else if now != *to {
            step.status = StepStatus::Undone;
            skipped.push(format!(
                "Skipped {key}: it is {now} now, not the {to} RustTV set (changed since); left \
                 as is."
            ));
        }
    }
    if !journal.steps.iter().any(|step| step.status.may_have_run()) {
        // Nothing is left to undo: callers that save the journal keep the
        // next `optimize restore` from finding it again.
        journal.status = JournalStatus::Restored;
        return Err(PlanError::precondition(format!(
            "nothing to restore: {}",
            skipped.join(" ")
        )));
    }
    let plan = restore_plan(journal, serial, &fingerprint)?;
    Ok(plan.with_notes(
        skipped.into_iter().chain([
            "Right before each step the setting is read again; one changed since is left as is."
                .to_owned(),
        ]),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use crate::RiskClass;
    use crate::adb::FakeAdb;
    use crate::compat::bundled_profiles;
    use crate::error::AdbError;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{FileJournal, JournalStep, JournalStore, journal_file_name};
    use crate::plan::{apply, restore};

    const SERIAL: &str = "tv";
    const GETPROP: &str = include_str!("fixtures/philips-65oled818-api34/getprop.txt");
    const HOME: &str = include_str!("fixtures/philips-65oled818-api34/home-activity.txt");
    const NULL: &str = include_str!("fixtures/synthetic-optimizer/settings-get-null.txt");
    const ONE: &str = include_str!("fixtures/synthetic-optimizer/settings-get-1.0.txt");
    const HALF: &str = include_str!("fixtures/synthetic-optimizer/settings-get-0.5.txt");
    const INVALID: &str = include_str!("fixtures/synthetic-optimizer/settings-get-invalid.txt");

    const WINDOW: SettingKey = SettingKey::WindowAnimationScale;
    const TRANSITION: SettingKey = SettingKey::TransitionAnimationScale;
    const ANIMATOR: SettingKey = SettingKey::AnimatorDurationScale;

    fn profile() -> OptimizerProfile {
        bundled_optimizer_profiles()
            .into_iter()
            .find(|profile| profile.name == REDUCED_ANIMATIONS)
            .unwrap()
    }

    fn value(text: &str) -> SettingState {
        SettingState::Value(SettingValue::new(text).unwrap())
    }

    /// The bundled profiles, with the Philips one recording `reduced-animations` v1.
    fn tested_profiles(tested: bool, evidence: Option<&str>) -> Vec<DeviceProfile> {
        let mut profiles = bundled_profiles();
        for profile in &mut profiles {
            if profile.device.model.starts_with("Philips") {
                profile.optimizer = vec![ProfileOptimizer {
                    profile: REDUCED_ANIMATIONS.to_owned(),
                    version: 1,
                    tested,
                    evidence: evidence.map(str::to_owned),
                }];
            }
        }
        profiles
    }

    /// A fresh, empty journal directory for one test.
    struct Dir(PathBuf);

    impl Dir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("rusttv-optimizer-{}-{test}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The Philips (getprop, Home) with the three keys reading `window`,
    /// `transition`, `animator`.
    fn tv(window: &str, transition: &str, animator: &str) -> FakeAdb {
        FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::HomeActivity, HOME)
            .respond(DeviceCommand::GetSetting(WINDOW), window)
            .respond(DeviceCommand::GetSetting(TRANSITION), transition)
            .respond(DeviceCommand::GetSetting(ANIMATOR), animator)
    }

    fn set(key: SettingKey, to: SettingState, from: SettingState) -> DeviceCommand {
        DeviceCommand::SetSetting { key, to, from }
    }

    fn changes(adb: &FakeAdb) -> Vec<DeviceCommand> {
        adb.calls()
            .into_iter()
            .filter(|command| command.risk() > RiskClass::Read)
            .collect()
    }

    fn plan_on(adb: &dyn Adb, dir: &Dir) -> Result<Plan, PlanError> {
        optimizer_plan(
            adb,
            SERIAL,
            &profile(),
            &tested_profiles(true, None),
            &dir.0,
            false,
        )
    }

    fn fingerprint() -> String {
        parse_properties(GETPROP)["ro.build.fingerprint"].clone()
    }

    fn journal(plan: &str, fingerprint: &str, steps: Vec<JournalStep>) -> Journal {
        Journal {
            schema_version: crate::journal::JOURNAL_SCHEMA_VERSION,
            plan: plan.to_owned(),
            serial: SERIAL.to_owned(),
            fingerprint: fingerprint.to_owned(),
            started_at_unix_ms: 1,
            status: JournalStatus::Applied,
            steps,
        }
    }

    fn done(command: DeviceCommand) -> JournalStep {
        JournalStep {
            adb: String::new(),
            command,
            original_state: None,
            status: StepStatus::Done,
        }
    }

    /// A journal of `reduced-animations` v1 that set all three from 1.0 / unset.
    fn applied_journal() -> Journal {
        journal(
            "optimize-reduced-animations-v1",
            &fingerprint(),
            vec![
                done(set(WINDOW, value("0.5"), value("1.0"))),
                done(set(TRANSITION, value("0.5"), SettingState::Unset)),
                done(set(ANIMATOR, value("0.5"), value("1.0"))),
            ],
        )
    }

    #[test]
    fn bundled_optimizer_profiles_are_valid() {
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../compat/optimizer"
        ));
        let mut committed: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".toml"))
            .collect();
        committed.sort();
        let bundled: Vec<&str> = BUNDLED_OPTIMIZER_PROFILES
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(bundled, committed, "update BUNDLED_OPTIMIZER_PROFILES");
        for (file, text) in BUNDLED_OPTIMIZER_PROFILES {
            let profile: OptimizerProfile = toml::from_str(text).unwrap();
            assert_eq!(profile.schema_version, OPTIMIZER_SCHEMA_VERSION, "{file}");
            assert_eq!(format!("{}.toml", profile.name), *file);
            assert!(!profile.settings.is_empty(), "{file}");
            let mut keys: Vec<SettingKey> = profile.settings.iter().map(|s| s.key).collect();
            keys.dedup();
            assert_eq!(keys.len(), profile.settings.len(), "{file}: duplicate key");
            let kind: OptimizeKind = profile.name.parse().unwrap();
            assert_eq!(OptimizeKind::of_plan(&profile.plan_name()), Some(kind));
        }
        // OPTIMIZER.md § 1, owner decision 1: 0.5 for the three scales.
        let profile = profile();
        assert_eq!(profile.plan_name(), "optimize-reduced-animations-v1");
        let settings: Vec<(SettingKey, &str)> = profile
            .settings
            .iter()
            .map(|setting| (setting.key, setting.value.as_str()))
            .collect();
        assert_eq!(
            settings,
            [(WINDOW, "0.5"), (TRANSITION, "0.5"), (ANIMATOR, "0.5")]
        );
    }

    #[test]
    fn profiles_reject_unknown_keys_bad_values_and_fields() {
        let text = include_str!("../../../compat/optimizer/reduced-animations.toml");
        for bad in [
            text.replace("window_animation_scale", "adb_enabled"),
            text.replacen("\"0.5\"", "\"0.5; reboot\"", 1),
            text.replacen("\"0.5\"", "\"fast\"", 1),
            text.replace("version = 1", "version = 1\nnamespace = \"secure\""),
        ] {
            assert!(toml::from_str::<OptimizerProfile>(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn plan_names_carry_the_profile_and_version() {
        assert_eq!(
            profile_of_plan("optimize-reduced-animations-v1"),
            Some(REDUCED_ANIMATIONS)
        );
        assert_eq!(
            profile_of_plan("optimize-reduced-animations-v12"),
            Some(REDUCED_ANIMATIONS)
        );
        for plan in [
            "privacy",
            "home-takeover",
            "optimize-",
            "optimize--v1",
            "optimize-x-v",
            "optimize-x-v1a",
        ] {
            assert_eq!(profile_of_plan(plan), None, "{plan}");
        }
        assert_eq!(
            OptimizeKind::of_plan("privacy"),
            Some(OptimizeKind::Privacy)
        );
        assert_eq!(OptimizeKind::of_plan("optimize-other-v1"), None);
        assert_eq!(OptimizeKind::of_plan("optimize-privacy-v1"), None);
        assert!("bloat".parse::<OptimizeKind>().is_err());
    }

    #[test]
    fn the_plan_sets_each_key_and_checks_home_after_the_last() {
        let dir = Dir::new("plan");
        let adb = tv(ONE, NULL, ONE);
        let plan = plan_on(&adb, &dir).unwrap();

        assert_eq!(plan.name(), "optimize-reduced-animations-v1");
        assert_eq!(plan.confirmation(), "optimize");
        let commands: Vec<&DeviceCommand> = plan.steps().iter().map(Step::command).collect();
        assert_eq!(
            commands,
            [
                &set(WINDOW, value("0.5"), value("1.0")),
                &set(TRANSITION, value("0.5"), SettingState::Unset),
                &set(ANIMATOR, value("0.5"), value("1.0")),
            ]
        );
        let text = plan.dry_run().to_string();
        for expected in [
            "Risk:     R2",
            "1. [R2] adb -s tv shell settings put global window_animation_scale 0.5",
            "records first: window_animation_scale is 1.0",
            "then checks:   window_animation_scale is 0.5",
            "undo:          adb -s tv shell settings put global window_animation_scale 1.0",
            "records first: transition_animation_scale is unset",
            "undo:          adb -s tv shell settings delete global transition_animation_scale",
            "then checks:   animator_duration_scale is 0.5; Home opens com.google.android.apps.tv.launcherx/",
            "transition_animation_scale: unset → 0.5",
            "Tested on this model and firmware family, according to this model's profile (tpv-ph1m-ww-9972.toml).",
        ] {
            assert!(
                text.contains(expected),
                "{expected:?} missing from:\n{text}"
            );
        }
        assert!(changes(&adb).is_empty(), "planning changes nothing");
    }

    #[test]
    fn an_untested_device_is_refused_unless_allowed_and_then_marked() {
        let dir = Dir::new("untested");
        let adb = tv(ONE, ONE, ONE);
        let bundled = bundled_profiles();
        let error = optimizer_plan(&adb, SERIAL, &profile(), &bundled, &dir.0, false).unwrap_err();
        assert_eq!(error.code(), "OPTIMIZE_NOT_TESTED");
        assert!(error.to_string().contains("--allow-untested"), "{error}");
        // `tested = false`, or tested for another version, is no better.
        for profiles in [tested_profiles(false, None), {
            let mut profiles = tested_profiles(true, None);
            for profile in &mut profiles {
                for entry in &mut profile.optimizer {
                    entry.version = 2;
                }
            }
            profiles
        }] {
            let error =
                optimizer_plan(&adb, SERIAL, &profile(), &profiles, &dir.0, false).unwrap_err();
            assert_eq!(error.code(), "OPTIMIZE_NOT_TESTED");
        }

        let plan = optimizer_plan(&adb, SERIAL, &profile(), &bundled, &dir.0, true).unwrap();
        assert_eq!(plan.steps().len(), 3);
        assert!(
            plan.dry_run()
                .notes
                .iter()
                .any(|note| note.starts_with("UNTESTED on this model")),
            "{:?}",
            plan.dry_run().notes
        );
    }

    #[test]
    fn keys_already_set_are_skipped_and_all_set_is_refused() {
        let dir = Dir::new("already");
        let plan = plan_on(&tv(HALF, ONE, HALF), &dir).unwrap();
        assert_eq!(plan.steps().len(), 1);
        assert!(
            plan.dry_run()
                .notes
                .contains(&"Skipped window_animation_scale: already 0.5.".to_owned())
        );

        let error = plan_on(&tv(HALF, HALF, HALF), &dir).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    #[test]
    fn an_original_that_does_not_validate_refuses_the_plan() {
        let dir = Dir::new("invalid");
        let error = plan_on(&tv(ONE, INVALID, ONE), &dir).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(
            error.to_string().contains("transition_animation_scale"),
            "{error}"
        );
    }

    #[test]
    fn the_builder_refuses_package_steps_and_unchanged_values() {
        let package = crate::command::PackageName::new("com.example.app").unwrap();
        let error = settings_plan(
            "optimize-x-v1",
            SERIAL,
            "f",
            vec![Step::new(
                DeviceCommand::DisablePackage(package),
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "INVALID_PLAN");
        let error = settings_plan(
            "optimize-x-v1",
            SERIAL,
            "f",
            vec![Step::new(
                set(WINDOW, value("0.5"), value("0.5")),
                Vec::new(),
            )],
            Vec::new(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("already 0.5"), "{error}");
    }

    #[test]
    fn a_restorable_optimizer_journal_of_this_model_refuses_a_second_plan() {
        let dir = Dir::new("second");
        let adb = tv(ONE, ONE, ONE);
        // Restorable on a security update of the same firmware family.
        let mut other = applied_journal();
        other.fingerprint = fingerprint().replace("3000518", "3000600");
        let path = dir.0.join(journal_file_name(1, &other.plan));
        FileJournal::new(&path).save(&other).unwrap();
        let error = plan_on(&adb, &dir).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        let message = error.to_string();
        assert!(message.contains("one at a time"), "{message}");
        assert!(
            message.contains(&format!("--journal {}", path.display())),
            "{message}"
        );

        // Restored, or another model's: no longer in the way.
        other.status = JournalStatus::Restored;
        FileJournal::new(&path).save(&other).unwrap();
        assert!(plan_on(&adb, &dir).is_ok());
        other.status = JournalStatus::Applied;
        other.fingerprint = "google/tv/x:14/A/1:user/release-keys".to_owned();
        FileJournal::new(&path).save(&other).unwrap();
        assert!(plan_on(&adb, &dir).is_ok());
    }

    /// A TV whose settings change when written, for apply and restore.
    /// Optionally fails the `fail_write`-th write (0-based) without changing
    /// anything, or stores `clamped` whenever 0.5 is written (a vendor
    /// clamping the value).
    struct SettingsTv {
        getprop: String,
        settings: RefCell<HashMap<SettingKey, SettingState>>,
        fail_write: Option<usize>,
        clamped: Option<SettingState>,
        writes: RefCell<usize>,
        log: RefCell<Vec<DeviceCommand>>,
    }

    impl SettingsTv {
        /// Window 1.0, transition unset, animator 1.0.
        fn new() -> Self {
            Self {
                getprop: GETPROP.to_owned(),
                settings: RefCell::new(HashMap::from([
                    (WINDOW, value("1.0")),
                    (TRANSITION, SettingState::Unset),
                    (ANIMATOR, value("1.0")),
                ])),
                fail_write: None,
                clamped: None,
                writes: RefCell::new(0),
                log: RefCell::new(Vec::new()),
            }
        }

        fn state(&self) -> HashMap<SettingKey, SettingState> {
            self.settings.borrow().clone()
        }

        fn put(&self, key: SettingKey, state: SettingState) {
            self.settings.borrow_mut().insert(key, state);
        }

        fn writes(&self) -> Vec<DeviceCommand> {
            self.log
                .borrow()
                .iter()
                .filter(|command| command.risk() > RiskClass::Read)
                .cloned()
                .collect()
        }
    }

    impl Adb for SettingsTv {
        fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
            self.log.borrow_mut().push(command.clone());
            let refuse = || {
                AdbError::from_failure(serial, &command.adb_args(), "exit 1".to_owned(), "refused")
            };
            match command {
                DeviceCommand::GetProperties => Ok(self.getprop.clone()),
                DeviceCommand::HomeActivity => Ok(HOME.to_owned()),
                DeviceCommand::GetSetting(key) => Ok(match &self.settings.borrow()[key] {
                    SettingState::Unset => NULL.to_owned(),
                    SettingState::Value(value) => format!("{value}\n"),
                }),
                DeviceCommand::SetSetting { key, to, .. } => {
                    let write = *self.writes.borrow();
                    *self.writes.borrow_mut() += 1;
                    if self.fail_write == Some(write) {
                        return Err(refuse());
                    }
                    let stored = match &self.clamped {
                        Some(clamped) if *to == value("0.5") => clamped.clone(),
                        _ => to.clone(),
                    };
                    self.put(*key, stored);
                    Ok(String::new())
                }
                _ => Err(refuse()),
            }
        }

        fn devices(&self) -> Result<Vec<crate::device::DeviceEntry>, AdbError> {
            Ok(Vec::new())
        }

        fn connect(
            &self,
            address: &crate::device::DeviceAddress,
        ) -> Result<crate::device::ConnectOutcome, AdbError> {
            Err(AdbError::connect_failed(
                address.to_string(),
                "no network".to_owned(),
            ))
        }
    }

    /// Plans on `tv` and applies with `store`.
    fn apply_on(tv: &SettingsTv, store: &mut MemoryJournal) -> Result<Journal, PlanError> {
        let dir = Dir::new("apply");
        let plan = plan_on(tv, &dir).unwrap();
        let approval = plan.approve("optimize").unwrap();
        apply(tv, &plan, &approval, store, &|_| {})
    }

    fn originals() -> HashMap<SettingKey, SettingState> {
        SettingsTv::new().state()
    }

    #[test]
    fn apply_sets_and_verifies_every_key_and_journals_the_originals() {
        let tv = SettingsTv::new();
        let mut store = MemoryJournal::default();
        let journal = apply_on(&tv, &mut store).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(journal.plan, "optimize-reduced-animations-v1");
        assert_eq!(
            store.saves[0].steps[1].original_state,
            Some(Check::SettingIs {
                key: TRANSITION,
                state: SettingState::Unset
            })
        );
        assert_eq!(
            tv.writes(),
            [
                set(WINDOW, value("0.5"), value("1.0")),
                set(TRANSITION, value("0.5"), SettingState::Unset),
                set(ANIMATOR, value("0.5"), value("1.0")),
            ]
        );
        assert!(tv.state().values().all(|state| *state == value("0.5")));

        // And restore puts every original back, deleting the unset one.
        let mut journal = journal;
        let plan = optimizer_restore_plan(&tv, SERIAL, REDUCED_ANIMATIONS, &mut journal).unwrap();
        let approval = plan.approve("restore").unwrap();
        let notes = restore(&tv, &plan, &approval, &mut journal, &mut store).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(journal.status, JournalStatus::Restored);
        assert_eq!(tv.state(), originals());
        assert_eq!(
            tv.writes()[3..],
            [
                set(ANIMATOR, value("1.0"), value("0.5")),
                set(TRANSITION, SettingState::Unset, value("0.5")),
                set(WINDOW, value("1.0"), value("0.5")),
            ]
        );
    }

    #[test]
    fn adb_failing_at_any_step_rolls_every_earlier_key_back() {
        for failing in 0..3 {
            let tv = SettingsTv {
                fail_write: Some(failing),
                ..SettingsTv::new()
            };
            let mut store = MemoryJournal::default();
            let error = apply_on(&tv, &mut store).unwrap_err();
            assert_eq!(error.code(), "ROLLED_BACK", "step {failing}: {error}");
            assert!(
                error.to_string().contains(&format!("step {}", failing + 1)),
                "{error}"
            );
            assert_eq!(tv.state(), originals(), "step {failing}");
            assert_eq!(store.last().status, JournalStatus::RolledBack);
        }
    }

    #[test]
    fn a_vendor_clamping_the_value_fails_verification_and_rolls_back() {
        // The firmware stores 0.6 instead of 0.5: nothing else can have
        // changed it in between, so the rollback undoes it.
        let tv = SettingsTv {
            clamped: Some(value("0.6")),
            ..SettingsTv::new()
        };
        let error = apply_on(&tv, &mut MemoryJournal::default()).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error.to_string().contains("window_animation_scale is 0.5"),
            "{error}"
        );
        assert_eq!(tv.state(), originals());
    }

    #[test]
    fn a_journal_write_failing_before_or_after_any_step_rolls_back() {
        // Saves: 0 pending, then started/done per step, then applied.
        for fail_from in 1..=7 {
            let tv = SettingsTv::new();
            let mut store = MemoryJournal {
                fail_from: Some(fail_from),
                ..MemoryJournal::default()
            };
            let error = apply_on(&tv, &mut store).unwrap_err();
            // The rollback can't write the journal either, and says so.
            assert_eq!(error.code(), "ROLLBACK_FAILED", "save {fail_from}: {error}");
            assert!(
                error.to_string().contains("could not write the journal"),
                "{error}"
            );
            assert_eq!(tv.state(), originals(), "save {fail_from}");
        }
        // The first save failing changes nothing at all.
        let tv = SettingsTv::new();
        let mut store = MemoryJournal {
            fail_from: Some(0),
            ..MemoryJournal::default()
        };
        assert_eq!(
            apply_on(&tv, &mut store).unwrap_err().code(),
            "JOURNAL_FAILED"
        );
        assert!(tv.writes().is_empty());
    }

    #[test]
    fn a_firmware_change_between_plan_and_apply_changes_nothing() {
        let tv = SettingsTv::new();
        let dir = Dir::new("firmware");
        let plan = plan_on(&tv, &dir).unwrap();
        let approval = plan.approve("optimize").unwrap();
        let updated = SettingsTv {
            getprop: GETPROP.replace("3000518", "3000600"),
            ..SettingsTv::new()
        };
        let error = apply(
            &updated,
            &plan,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(updated.writes().is_empty());
    }

    #[test]
    fn a_key_changed_between_plan_and_apply_changes_nothing() {
        let tv = SettingsTv::new();
        let dir = Dir::new("changed");
        let plan = plan_on(&tv, &dir).unwrap();
        let approval = plan.approve("optimize").unwrap();
        tv.put(ANIMATOR, value("2.0"));
        let mut store = MemoryJournal::default();
        let error = apply(&tv, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(
            error.to_string().contains("animator_duration_scale is 1.0"),
            "{error}"
        );
        assert!(tv.writes().is_empty());
        assert!(store.saves.is_empty());
    }

    #[test]
    fn a_process_that_died_mid_apply_is_restored_from_started_and_done_steps() {
        // Window was set and verified; transition "started" but never ran.
        let tv = SettingsTv::new();
        tv.put(WINDOW, value("0.5"));
        let mut journal = applied_journal();
        journal.status = JournalStatus::Applying;
        journal.steps[1].status = StepStatus::Started;
        journal.steps[2].status = StepStatus::Pending;
        let plan = optimizer_restore_plan(&tv, SERIAL, REDUCED_ANIMATIONS, &mut journal).unwrap();
        // Transition is already back at its original (unset): skipped.
        let commands: Vec<&DeviceCommand> = plan.steps().iter().map(Step::command).collect();
        assert_eq!(commands, [&set(WINDOW, value("1.0"), value("0.5"))]);
        let approval = plan.approve("restore").unwrap();
        let mut store = MemoryJournal::default();
        restore(&tv, &plan, &approval, &mut journal, &mut store).unwrap();
        assert_eq!(journal.status, JournalStatus::Restored);
        assert_eq!(tv.state(), originals());
    }

    #[test]
    fn restore_leaves_keys_changed_since_as_they_are() {
        // Window was changed to 0.75 since; transition is back to unset.
        let tv = SettingsTv::new();
        tv.put(WINDOW, value("0.75"));
        tv.put(ANIMATOR, value("0.5"));
        let mut journal = applied_journal();
        let plan = optimizer_restore_plan(&tv, SERIAL, REDUCED_ANIMATIONS, &mut journal).unwrap();
        let commands: Vec<&DeviceCommand> = plan.steps().iter().map(Step::command).collect();
        assert_eq!(commands, [&set(ANIMATOR, value("1.0"), value("0.5"))]);
        let notes = plan.dry_run().notes;
        assert!(
            notes.contains(
                &"Skipped window_animation_scale: it is 0.75 now, not the 0.5 RustTV set (changed since); left as is."
                    .to_owned()
            ),
            "{notes:?}"
        );
        assert!(
            notes.contains(
                &"Skipped transition_animation_scale: it is unset again already.".to_owned()
            ),
            "{notes:?}"
        );
        assert_eq!(journal.steps[0].status, StepStatus::Undone);
        let approval = plan.approve("restore").unwrap();
        restore(
            &tv,
            &plan,
            &approval,
            &mut journal,
            &mut MemoryJournal::default(),
        )
        .unwrap();
        assert_eq!(tv.state()[&WINDOW], value("0.75"), "never overwritten");
        assert_eq!(tv.state()[&ANIMATOR], value("1.0"));
    }

    #[test]
    fn a_key_changed_between_restore_plan_and_restore_is_left_as_is() {
        let tv = SettingsTv::new();
        for key in SettingKey::ALL {
            tv.put(key, value("0.5"));
        }
        let mut journal = applied_journal();
        let plan = optimizer_restore_plan(&tv, SERIAL, REDUCED_ANIMATIONS, &mut journal).unwrap();
        assert_eq!(plan.steps().len(), 3);
        // Someone sets animator to 2.0 before the restore runs.
        tv.put(ANIMATOR, value("2.0"));
        let approval = plan.approve("restore").unwrap();
        let mut store = MemoryJournal::default();
        let notes = restore(&tv, &plan, &approval, &mut journal, &mut store).unwrap();
        assert_eq!(
            notes,
            [
                "Skipped animator_duration_scale: it is 2.0 now, not the 0.5 RustTV set (changed since); left as is."
            ]
        );
        assert_eq!(tv.state()[&ANIMATOR], value("2.0"));
        assert_eq!(tv.state()[&WINDOW], value("1.0"));
        assert_eq!(tv.state()[&TRANSITION], SettingState::Unset);
        assert_eq!(journal.status, JournalStatus::Restored);
        assert_eq!(store.last().steps[2].status, StepStatus::Undone);
    }
    #[test]
    fn restore_with_every_key_back_or_changed_has_nothing_to_do() {
        let mut journal = applied_journal();
        let error = optimizer_restore_plan(
            &tv(ONE, "0.75\n", ONE),
            SERIAL,
            REDUCED_ANIMATIONS,
            &mut journal,
        )
        .unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("nothing to restore"), "{error}");
        assert_eq!(journal.status, JournalStatus::Restored);
    }

    #[test]
    fn restore_refuses_other_kinds_and_journals_that_change_more_than_settings() {
        let adb = tv(HALF, HALF, HALF);
        let package = crate::command::PackageName::new("com.example.app").unwrap();
        let mut privacy = journal(
            PRIVACY,
            &fingerprint(),
            vec![done(DeviceCommand::DisablePackage(package.clone()))],
        );
        let error =
            optimizer_restore_plan(&adb, SERIAL, REDUCED_ANIMATIONS, &mut privacy).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        // And the privacy validator refuses an optimizer journal.
        let error = privacy_restore_plan(&adb, SERIAL, &mut applied_journal()).unwrap_err();
        assert!(
            error.to_string().contains("not a privacy journal"),
            "{error}"
        );

        let mut mixed = applied_journal();
        mixed
            .steps
            .push(done(DeviceCommand::DisablePackage(package)));
        let error =
            optimizer_restore_plan(&adb, SERIAL, REDUCED_ANIMATIONS, &mut mixed).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("more than putting settings back"),
            "{error}"
        );
    }

    #[test]
    fn a_tampered_journal_does_not_load() {
        let dir = Dir::new("tampered");
        let path = dir
            .0
            .join(journal_file_name(1, "optimize-reduced-animations-v1"));
        FileJournal::new(&path).save(&applied_journal()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("set_setting"), "{text}");
        assert!(crate::journal::load_journal(&path).is_ok());
        for bad in [
            text.replace("animator_duration_scale", "adb_enabled"),
            text.replacen("value = \"0.5\"", "value = \"0.5; reboot\"", 1),
        ] {
            fs::write(&path, bad).unwrap();
            assert_eq!(
                crate::journal::load_journal(&path).unwrap_err().code(),
                "JOURNAL_FAILED"
            );
        }
    }

    #[test]
    fn the_journal_to_restore_is_chosen_by_kind() {
        let dir = Dir::new("find");
        let fp = fingerprint();
        let search = find_optimize_journal(&dir.0, &fp, None);
        let error = search.found.unwrap_err();
        assert_eq!(error.code(), "NO_JOURNAL");
        assert!(error.to_string().contains("no privacy journal"), "{error}");

        let optimizer = dir
            .0
            .join(journal_file_name(2, "optimize-reduced-animations-v1"));
        FileJournal::new(&optimizer)
            .save(&applied_journal())
            .unwrap();
        let (kind, path, _) = find_optimize_journal(&dir.0, &fp, None).found.unwrap();
        assert_eq!(
            (kind, path),
            (OptimizeKind::ReducedAnimations, optimizer.clone())
        );
        assert_eq!(
            find_optimize_journal(&dir.0, &fp, Some(OptimizeKind::Privacy))
                .found
                .unwrap_err()
                .code(),
            "NO_JOURNAL"
        );

        let package = crate::command::PackageName::new("com.example.app").unwrap();
        let privacy = dir.0.join(journal_file_name(1, PRIVACY));
        FileJournal::new(&privacy)
            .save(&journal(
                PRIVACY,
                &fp,
                vec![done(DeviceCommand::DisablePackage(package))],
            ))
            .unwrap();
        let error = find_optimize_journal(&dir.0, &fp, None).found.unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        let message = error.to_string();
        assert!(
            message.contains("`rusttv optimize restore privacy`"),
            "{message}"
        );
        assert!(
            message.contains("`rusttv optimize restore reduced-animations`"),
            "{message}"
        );
        let (kind, path, _) = find_optimize_journal(&dir.0, &fp, Some(OptimizeKind::Privacy))
            .found
            .unwrap();
        assert_eq!((kind, path), (OptimizeKind::Privacy, privacy));
    }

    #[test]
    fn statuses_follow_journals_values_and_testing() {
        let dir = Dir::new("statuses");
        let properties = parse_properties(GETPROP);
        let status = |adb: &FakeAdb, devices: &[DeviceProfile]| {
            let device = devices.iter().find(|device| device.matches(&properties));
            optimizer_statuses(adb, SERIAL, &properties, device, &dir.0).unwrap()[0].clone()
        };
        let untested = bundled_profiles();
        let first = status(&tv(ONE, NULL, ONE), &untested);
        assert_eq!(first.status, OptimizerState::NotTested);
        assert_eq!(first.settings[0].current.as_deref(), Some("1.0"));
        assert_eq!(first.settings[1].current, None);
        assert_eq!(first.settings[2].value.as_str(), "0.5");
        assert_eq!(
            status(&tv(ONE, ONE, ONE), &tested_profiles(true, None)).status,
            OptimizerState::Available
        );
        assert_eq!(
            status(
                &tv(ONE, ONE, ONE),
                &tested_profiles(true, Some("x/report.md"))
            )
            .status,
            OptimizerState::Suggested
        );
        assert_eq!(
            status(&tv(HALF, HALF, HALF), &untested).status,
            OptimizerState::AlreadySet
        );

        let path = dir
            .0
            .join(journal_file_name(1, "optimize-reduced-animations-v1"));
        FileJournal::new(&path).save(&applied_journal()).unwrap();
        let applied = status(&tv(HALF, HALF, HALF), &untested);
        assert_eq!(applied.status, OptimizerState::Applied);
        assert_eq!(applied.journal, Some(path.display().to_string()));
        assert_eq!(
            status(&tv(HALF, ONE, HALF), &untested).status,
            OptimizerState::Drifted
        );

        let mut stale = applied_journal();
        stale.fingerprint = fingerprint().replace("3000518", "3000400");
        FileJournal::new(&path).save(&stale).unwrap();
        assert_eq!(
            status(&tv(HALF, HALF, HALF), &untested).status,
            OptimizerState::Stale
        );
    }
}

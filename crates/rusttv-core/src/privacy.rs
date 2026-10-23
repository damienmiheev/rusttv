//! Privacy profiles (R3): disable, never uninstall, the catalog packages a
//! person's choices select, and undo it (OPTIMIZER.md § Generic design,
//! SECURITY.md § Protected package policy).
//!
//! Both directions are [`Plan`]s run by the safety engine in [`crate::plan`].
//! Every package is checked against the protected-package policy first:
//! rules 1–2 live on the device ([`LiveState`]), rule 3 against the deny-list
//! `compat/protected.toml`, compiled into the binary.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::adb::Adb;
use crate::apps::package_names;
use crate::catalog::{CatalogEntry, LoadedChoices};
use crate::command::{Component, DeviceCommand, PackageName};
use crate::compat::{DeviceProfile, PrivacyCategory, TestedPackage, profile_file_name};
use crate::error::AdbError;
use crate::inspect::parse_properties;
use crate::journal::{Journal, JournalStatus, StepStatus};
use crate::launcher::{HomeCandidate, launcher_status};
use crate::optimizer::{OptimizerStatus, optimizer_statuses};
use crate::plan::{Check, Plan, PlanError, Step, device_fingerprint, restore_plan};

/// Plan name of a privacy profile, also used in journal file names.
pub const PRIVACY: &str = "privacy";

/// The static deny-list (SECURITY.md rule 3). Compiled in, so a file on disk
/// can't shrink it.
const PROTECTED_TOML: &str = include_str!("../../../compat/protected.toml");

/// Version of `compat/protected.toml` this build understands.
const PROTECTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedList {
    schema_version: u32,
    packages: Vec<ProtectedEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedEntry {
    pattern: String,
    #[serde(rename = "match")]
    kind: MatchKind,
    reason: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MatchKind {
    Exact,
    Prefix,
    Contains,
}

impl ProtectedEntry {
    fn matches(&self, package: &str) -> bool {
        match self.kind {
            MatchKind::Exact => package == self.pattern,
            MatchKind::Prefix => package.starts_with(&self.pattern),
            MatchKind::Contains => package.contains(&self.pattern),
        }
    }
}

/// Why `package` is on the static deny-list `compat/protected.toml`, if it
/// is. Fails closed: if the compiled-in list can't be read, every package is
/// protected (a test checks it reads).
pub fn statically_protected(package: &str) -> Option<String> {
    match toml::from_str::<ProtectedList>(PROTECTED_TOML) {
        Ok(list) if list.schema_version == PROTECTED_SCHEMA_VERSION => list
            .packages
            .iter()
            .find(|entry| entry.matches(package))
            .map(|entry| {
                format!(
                    "on the protected list (compat/protected.toml): {}",
                    entry.reason
                )
            }),
        _ => Some(
            "compat/protected.toml could not be read, so every package counts as protected"
                .to_owned(),
        ),
    }
}

/// What the device says now about protected packages (SECURITY.md rules
/// 1–2). Read fresh for every plan, never cached.
#[derive(Debug)]
struct LiveState {
    /// `package/activity` the Home button opens.
    home: Option<String>,
    candidates: Vec<HomeCandidate>,
    input_methods: HashSet<String>,
    accessibility_services: HashSet<String>,
}

impl LiveState {
    /// Reads Home, Home candidates, input methods and enabled accessibility
    /// services. R0 only. Output that can't be read fails the read: a
    /// protected-package check must never pass on a misread.
    fn read(adb: &dyn Adb, serial: &str) -> Result<Self, PlanError> {
        let status = launcher_status(adb, serial)?;
        let input_methods = parse_input_methods(&adb.run(serial, &DeviceCommand::InputMethods)?)
            .map_err(|line| {
                PlanError::precondition(format!(
                    "can't read the input methods (`ime list -a -s` printed `{line}`), so the \
                     protected-package check can't run; nothing changed"
                ))
            })?;
        let accessibility_services = parse_accessibility_services(
            &adb.run(serial, &DeviceCommand::EnabledAccessibilityServices)?,
        )
        .map_err(|value| {
            PlanError::precondition(format!(
                "can't read the enabled accessibility services (`{value}`), so the \
                 protected-package check can't run; nothing changed"
            ))
        })?;
        Ok(Self {
            home: status.home,
            candidates: status.candidates,
            input_methods,
            accessibility_services,
        })
    }

    /// Why `package` must not be disabled, if it must not (rules 1–3).
    fn protection(&self, package: &str) -> Option<String> {
        let home_package = self
            .home
            .as_deref()
            .and_then(|home| home.split_once('/'))
            .map(|(home, _)| home);
        if home_package == Some(package) {
            Some("the Home button opens it".to_owned())
        } else if self
            .candidates
            .iter()
            .any(|candidate| candidate.package() == package)
        {
            Some("it can handle Home".to_owned())
        } else if self.input_methods.contains(package) {
            Some("it provides an input method (keyboard)".to_owned())
        } else if self.accessibility_services.contains(package) {
            Some("it provides an enabled accessibility service".to_owned())
        } else {
            statically_protected(package)
        }
    }
}

/// Why `package` must not be disabled right now, if it must not (rules 1–3),
/// read fresh from the device. Output that can't be read counts as a reason:
/// the check must never pass on a misread. Used by [`Check::NotProtected`]
/// right before each step of a privacy plan.
pub(crate) fn protection_now(
    adb: &dyn Adb,
    serial: &str,
    package: &str,
) -> Result<Option<String>, AdbError> {
    match LiveState::read(adb, serial) {
        Ok(live) => Ok(live.protection(package)),
        Err(error) => match error.into_adb() {
            Ok(adb_error) => Err(adb_error),
            Err(error) => Ok(Some(error.to_string())),
        },
    }
}

/// TV input services the system has bound, from `dumpsys tv_input`: the
/// `ComponentInfo{package/class}` entries of its `serviceStateMap` whose block
/// says `bound: true`. Empty on a device without TV inputs; entries whose
/// class isn't a plain Java name (e.g. `Outer$Inner`) are left out.
pub fn parse_bound_tv_input_services(output: &str) -> Vec<Component> {
    let mut bound = Vec::new();
    let mut in_map = false;
    let mut current: Option<Component> = None;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("serviceStateMap:") {
            in_map = true;
            continue;
        }
        if !in_map || trimmed.is_empty() {
            continue;
        }
        // Entries are indented by 4 spaces, their fields by 6; anything less
        // starts the next section.
        if !line.starts_with("    ") {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("ComponentInfo{") {
            current = rest
                .split_once('}')
                .and_then(|(component, _)| component.parse().ok());
        } else if trimmed == "bound: true"
            && let Some(component) = current.take()
        {
            bound.push(component);
        }
    }
    bound
}

/// Packages in `ime list -a -s` output: one `package/class` id per line.
/// Returns the first line that isn't one (e.g. an error message).
fn parse_input_methods(output: &str) -> Result<HashSet<String>, String> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| component_package(line).ok_or_else(|| line.to_owned()))
        .collect()
}

/// Packages in `settings get secure enabled_accessibility_services`:
/// `null` (unset), empty, or `:`-separated `package/class` components.
/// Returns the output if it is something else.
fn parse_accessibility_services(output: &str) -> Result<HashSet<String>, String> {
    let value = output.trim();
    if value.is_empty() || value == "null" {
        return Ok(HashSet::new());
    }
    value
        .split(':')
        .filter(|component| !component.is_empty())
        .map(|component| component_package(component).ok_or_else(|| value.to_owned()))
        .collect()
}

/// The package of a `package/class` component, if `text` is one.
fn component_package(text: &str) -> Option<String> {
    let (package, class) = text.split_once('/')?;
    (!class.is_empty() && !class.contains(char::is_whitespace))
        .then(|| PackageName::new(package).ok())
        .flatten()
        .map(String::from)
}

/// What a privacy plan or analysis reads besides the device: the catalog,
/// the device profiles and the person's choices.
#[derive(Debug, Clone)]
pub struct PrivacyData {
    /// Normally [`crate::bundled_catalog`].
    pub catalog: Vec<CatalogEntry>,
    /// Normally [`crate::bundled_profiles`].
    pub profiles: Vec<DeviceProfile>,
    /// Normally [`crate::load_choices`].
    pub choices: LoadedChoices,
}

impl PrivacyData {
    /// The bundled catalog and profiles with these choices.
    pub fn bundled(choices: LoadedChoices) -> Self {
        Self {
            catalog: crate::bundled_catalog(),
            profiles: crate::bundled_profiles(),
            choices,
        }
    }
}

/// `rusttv optimize analyze`: catalog packages on the device and what the
/// choices would do with them. Read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptimizeAnalysis {
    /// Target device.
    pub serial: String,
    /// File name of the bundled compat profile matching the device, if any.
    pub profile: Option<String>,
    /// The choices file read, or `None` for the defaults.
    pub choices: Option<String>,
    /// The categories the choices apply.
    pub categories: Vec<PrivacyCategory>,
    /// Every catalog package installed on the device and every `extra` of
    /// the choices, in plan order.
    pub packages: Vec<PrivacyPackageStatus>,
    /// Every bundled optimizer profile (R2 settings) and where it stands on
    /// the device, with the current values of its settings.
    pub optimizer: Vec<OptimizerStatus>,
}

/// One package and what RustTV knows and would do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivacyPackageStatus {
    /// Package name.
    pub package: String,
    /// In the catalog, or an `extra` of the choices.
    pub source: PackageSource,
    /// Its catalog category; `None` for an `extra`.
    pub category: Option<PrivacyCategory>,
    /// Whether its category is on by default (telemetry, ads, extras).
    pub default: bool,
    /// What it does; `None` for an `extra`.
    pub what: Option<String>,
    /// What stops working while it is disabled; `None` for an `extra`.
    pub side_effects: Option<String>,
    /// Whether it was tested on this model and firmware family.
    pub tested: bool,
    /// Date it was tested on this model, if it was.
    pub checked: Option<String>,
    /// Whether the choices select it (`optimize apply privacy` would plan it
    /// if it is enabled, and tested or `--allow-untested` is given).
    pub chosen: bool,
    /// Installed and enabled, installed and disabled, or not installed.
    pub state: PackageState,
    /// Why RustTV refuses to disable it, if it does (protected-package policy).
    pub protected: Option<String>,
}

/// Where a package in a privacy plan comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageSource {
    /// The catalog, `compat/packages/`.
    Catalog,
    /// `extra` in the person's choices: nothing is known about it.
    Extra,
}

/// Whether a package is installed and enabled for user 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageState {
    /// Not installed.
    Absent,
    /// Installed and enabled.
    Enabled,
    /// Installed and disabled.
    Disabled,
}

/// The bundled profile matching the device, with the device's properties.
fn matching_profile<'a>(
    adb: &dyn Adb,
    serial: &str,
    profiles: &'a [DeviceProfile],
) -> Result<(HashMap<String, String>, Option<&'a DeviceProfile>), PlanError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let profile = profiles.iter().find(|profile| profile.matches(&properties));
    Ok((properties, profile))
}

/// Installed and disabled packages now.
struct PackageStates {
    installed: HashSet<String>,
    disabled: HashSet<String>,
}

impl PackageStates {
    fn read(adb: &dyn Adb, serial: &str) -> Result<Self, PlanError> {
        let installed = adb.run(serial, &DeviceCommand::ListPackages)?;
        let disabled = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;
        let owned = |names: HashSet<&str>| names.into_iter().map(str::to_owned).collect();
        Ok(Self {
            installed: owned(package_names(&installed)?),
            disabled: owned(package_names(&disabled)?),
        })
    }

    fn of(&self, package: &str) -> PackageState {
        if !self.installed.contains(package) {
            PackageState::Absent
        } else if self.disabled.contains(package) {
            PackageState::Disabled
        } else {
            PackageState::Enabled
        }
    }
}

/// A catalog entry or an `extra`, with what the device profile and the
/// choices say about it.
struct Candidate<'a> {
    package: &'a PackageName,
    entry: Option<&'a CatalogEntry>,
    /// Date tested on this model, if it was.
    checked: Option<&'a str>,
    chosen: bool,
    /// Plan order: category, then tested order, then catalog order.
    order: (usize, usize, usize),
}

impl Candidate<'_> {
    fn label(&self) -> String {
        match self.entry {
            Some(entry) => format!("{} ({})", self.package, entry.category),
            None => format!("{} (extra, not in the catalog)", self.package),
        }
    }
}

/// Every catalog entry and every `extra`, in plan order, marked chosen or
/// not. `only` narrows the chosen categories (CLI `--only`); with it, `also`
/// counts only within those categories and `extra` not at all.
fn candidates<'a>(
    data: &'a PrivacyData,
    profile: Option<&'a DeviceProfile>,
    only: &[PrivacyCategory],
) -> Vec<Candidate<'a>> {
    let choices = &data.choices.choices;
    let tested: &[TestedPackage] = profile
        .and_then(|profile| profile.privacy.as_ref())
        .map_or(&[], |privacy| privacy.tested.as_slice());
    let allowed = |category: PrivacyCategory| only.is_empty() || only.contains(&category);
    let kept = |package: &PackageName| choices.keep.contains(package);
    let category_index = |category: PrivacyCategory| {
        PrivacyCategory::ALL
            .iter()
            .position(|c| *c == category)
            .unwrap_or(PrivacyCategory::ALL.len())
    };

    let mut list: Vec<Candidate<'a>> = data
        .catalog
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let tested_at = tested.iter().position(|t| t.package == entry.package);
            let by_category = choices.categories.contains(&entry.category);
            let by_also = choices.also.contains(&entry.package);
            Candidate {
                package: &entry.package,
                entry: Some(entry),
                checked: tested_at.map(|at| tested[at].checked.as_str()),
                chosen: !kept(&entry.package)
                    && (by_category || by_also)
                    && allowed(entry.category),
                order: (
                    category_index(entry.category),
                    tested_at.unwrap_or(usize::MAX),
                    index,
                ),
            }
        })
        .collect();
    list.sort_by_key(|candidate| candidate.order);
    list.extend(
        choices
            .extra
            .iter()
            .enumerate()
            .map(|(index, package)| Candidate {
                package,
                entry: None,
                checked: None,
                chosen: !kept(package) && only.is_empty(),
                order: (PrivacyCategory::ALL.len(), usize::MAX, index),
            }),
    );
    list
}

/// Lists the catalog packages installed on the device and the `extra`
/// packages of the choices, with what the choices would do, and where each
/// optimizer profile stands (reading the restorable journals in `journals`).
/// Read-only (R0).
pub fn optimize_analyze(
    adb: &dyn Adb,
    serial: &str,
    data: &PrivacyData,
    journals: &Path,
) -> Result<OptimizeAnalysis, PlanError> {
    data.choices.choices.validate(&data.catalog)?;
    let (properties, profile) = matching_profile(adb, serial, &data.profiles)?;
    let live = LiveState::read(adb, serial)?;
    let states = PackageStates::read(adb, serial)?;
    let packages = candidates(data, profile, &[])
        .into_iter()
        .filter_map(|candidate| {
            let name = candidate.package.as_str();
            let state = states.of(name);
            if candidate.entry.is_some() && state == PackageState::Absent {
                return None;
            }
            Some(PrivacyPackageStatus {
                package: name.to_owned(),
                source: if candidate.entry.is_some() {
                    PackageSource::Catalog
                } else {
                    PackageSource::Extra
                },
                category: candidate.entry.map(|entry| entry.category),
                default: candidate
                    .entry
                    .is_some_and(|entry| entry.category.is_default()),
                what: candidate.entry.map(|entry| entry.what.clone()),
                side_effects: candidate.entry.map(|entry| entry.side_effects.clone()),
                tested: candidate.checked.is_some(),
                checked: candidate.checked.map(str::to_owned),
                chosen: candidate.chosen,
                state,
                protected: live.protection(name),
            })
        })
        .collect();
    Ok(OptimizeAnalysis {
        serial: serial.to_owned(),
        profile: profile.map(profile_file_name),
        choices: data
            .choices
            .path
            .as_ref()
            .map(|path| path.display().to_string()),
        categories: data.choices.choices.categories.clone(),
        packages,
        optimizer: optimizer_statuses(adb, serial, &properties, profile, journals)?,
    })
}

/// Plans disabling the packages the choices select that are installed and
/// enabled. Read-only.
///
/// - Selected: catalog entries in the chosen categories (narrowed by `only`)
///   or in `also`, plus `extra` (only without `only`), minus `keep`.
/// - A package counts as tested only if the device's profile lists it under
///   `[[privacy.tested]]`. Untested ones are planned only with
///   `allow_untested`, and the notes mark each "UNTESTED on this model". With
///   nothing tested or allowed left, `PROFILE_NOT_TESTED`.
/// - Every selected package is checked against the protected-package policy
///   (Home, Home candidates, input methods, enabled accessibility services,
///   `compat/protected.toml`); if any is protected the whole plan is refused
///   with `PACKAGE_PROTECTED`.
/// - Packages already disabled are skipped and named in the notes; catalog
///   packages not installed are only counted (named if in `also` or `extra`).
///   Each other package gets one `pm disable-user` step.
/// - After the last step Home must open the same activity as before and the
///   Home candidates must be unchanged; otherwise the plan rolls back.
pub fn privacy_plan(
    adb: &dyn Adb,
    serial: &str,
    data: &PrivacyData,
    only: &[PrivacyCategory],
    allow_untested: bool,
) -> Result<Plan, PlanError> {
    data.choices.choices.validate(&data.catalog)?;
    let (properties, profile) = matching_profile(adb, serial, &data.profiles)?;
    let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
    let selected: Vec<Candidate<'_>> = candidates(data, profile, only)
        .into_iter()
        .filter(|candidate| candidate.chosen)
        .collect();
    if selected.is_empty() {
        let categories: Vec<&str> = data
            .choices
            .choices
            .categories
            .iter()
            .filter(|category| only.is_empty() || only.contains(category))
            .map(|category| category.as_str())
            .collect();
        return Err(PlanError::precondition(format!(
            "your choices select no package (categories: {}); opt-in categories (casting, \
             smart-home, voice, streaming-apps) are chosen only in the choices file",
            if categories.is_empty() {
                "none".to_owned()
            } else {
                categories.join(", ")
            }
        )));
    }

    let live = LiveState::read(adb, serial)?;
    let refused: Vec<String> = selected
        .iter()
        .filter_map(|candidate| {
            live.protection(candidate.package.as_str())
                .map(|why| format!("`{}`: {why}", candidate.package))
        })
        .collect();
    if !refused.is_empty() {
        return Err(PlanError::protected(format!(
            "your choices select protected packages, so nothing is planned (add them to `keep` \
             in your choices file, or remove them from `extra`): {}",
            refused.join("; ")
        )));
    }

    let home = live
        .home
        .as_deref()
        .ok_or_else(|| PlanError::precondition("nothing handles Home on this device".to_owned()))?;
    let home: Component = home
        .parse()
        .map_err(|error| PlanError::precondition(format!("Home: {error}")))?;
    let home_candidates: Vec<Component> = live
        .candidates
        .iter()
        .map(|candidate| candidate.component.parse())
        .collect::<Result<_, _>>()
        .map_err(|error| PlanError::precondition(format!("Home candidate: {error}")))?;

    let states = PackageStates::read(adb, serial)?;
    let mut notes = Vec::new();
    let mut skipped = Vec::new();
    // Catalog packages of the chosen categories that aren't installed: most
    // of the catalog on any one device, so only counted.
    let mut absent = 0;
    let mut already_disabled = Vec::new();
    let named = |name: &str| {
        let choices = &data.choices.choices;
        choices
            .also
            .iter()
            .chain(&choices.extra)
            .any(|package| package.as_str() == name)
    };
    let mut untested_left_out = Vec::new();
    let mut packages = Vec::new();
    let mut tested_count = 0;
    for candidate in &selected {
        let name = candidate.package.as_str();
        match states.of(name) {
            PackageState::Absent if named(name) => {
                skipped.push(format!("Skipped {name}: not installed."));
            }
            PackageState::Absent => absent += 1,
            PackageState::Disabled => {
                already_disabled.push(name);
                skipped.push(format!("Skipped {name}: already disabled."));
            }
            PackageState::Enabled if candidate.checked.is_none() && !allow_untested => {
                untested_left_out.push(name.to_owned());
            }
            PackageState::Enabled => {
                notes.push(plan_note(candidate));
                tested_count += usize::from(candidate.checked.is_some());
                packages.push(candidate.package.clone());
            }
        }
    }
    let profile_name = profile.map(profile_file_name);
    let tested_on = match &profile_name {
        Some(name) => format!("this model's profile ({name})"),
        None => format!(
            "no profile: none in compat/devices/ matches this device ({} {}, `{}`, firmware `{}`)",
            property("ro.product.manufacturer"),
            property("ro.product.model"),
            property("ro.product.device"),
            property("ro.build.fingerprint"),
        ),
    };
    if absent > 0 {
        skipped.push(format!(
            "Skipped {absent} catalog package{} of the chosen categories: not installed on this \
             device.",
            if absent == 1 { "" } else { "s" }
        ));
    }
    if packages.is_empty() {
        if !untested_left_out.is_empty() {
            return Err(PlanError::profile_not_tested(format!(
                "none of the chosen packages that are enabled on this device is tested on this \
                 model ({tested_on}). Untested: {}. Pass --allow-untested to plan them anyway, \
                 each marked UNTESTED in the dry run",
                untested_left_out.join(", ")
            )));
        }
        let mut parts = Vec::new();
        if !already_disabled.is_empty() {
            parts.push(format!("already disabled: {}", already_disabled.join(", ")));
        }
        if absent > 0 {
            parts.push(format!("{absent} catalog packages not installed"));
        }
        parts.extend(
            skipped
                .iter()
                .filter(|note| note.ends_with(": not installed."))
                .map(|note| {
                    note.trim_start_matches("Skipped ")
                        .trim_end_matches('.')
                        .to_owned()
                }),
        );
        return Err(PlanError::precondition(format!(
            "nothing to do: every chosen package is already disabled or not installed ({})",
            parts.join("; ")
        )));
    }
    notes.extend(skipped);
    if !untested_left_out.is_empty() {
        notes.push(format!(
            "Not planned, UNTESTED on this model (pass --allow-untested to include them): {}.",
            untested_left_out.join(", ")
        ));
    }
    // TV input services bound now, except those of packages this plan
    // disables (e.g. an AirPlay receiver that is also a TV input).
    let tv_inputs: Vec<Component> =
        parse_bound_tv_input_services(&adb.run(serial, &DeviceCommand::TvInputs)?)
            .into_iter()
            .filter(|service| !packages.contains(&service.package()))
            .collect();
    notes.extend([
        "Checked just now: none of these packages handles Home, provides an input method or \
         an enabled accessibility service, or is on compat/protected.toml. The same check runs \
         again right before each step; if it fails, the steps done so far are undone."
            .to_owned(),
        format!(
            "After the last step Home must still open {home}, the Home candidates must be \
             unchanged{}; otherwise every step is undone.",
            if tv_inputs.is_empty() {
                String::new()
            } else {
                format!(
                    " and the {} TV input services bound now must still be bound",
                    tv_inputs.len()
                )
            }
        ),
        "Disables, never uninstalls. Undo with `rusttv optimize restore`.".to_owned(),
        "Without ADB: Settings > Apps > See all apps > Show system apps > <app> > Enable."
            .to_owned(),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
        format!(
            "Tested on this model and firmware family, according to {tested_on}: {tested_count} \
             of the {} planned packages.",
            packages.len()
        ),
    ]);

    let last = packages.len() - 1;
    let steps = packages
        .into_iter()
        .enumerate()
        .map(|(index, package)| {
            let mut checks = Vec::new();
            if index == last {
                checks.push(Check::HomeActivityIs(home.clone()));
                checks.push(Check::HomeCandidatesAre(home_candidates.clone()));
                if !tv_inputs.is_empty() {
                    checks.push(Check::TvInputServicesBound(tv_inputs.clone()));
                }
            }
            Step::new(DeviceCommand::DisablePackage(package.clone()), checks)
                .requiring(vec![Check::NotProtected(package)])
        })
        .collect();
    let fingerprint = property("ro.build.fingerprint");
    Plan::new(PRIVACY, serial, fingerprint, PRIVACY, steps, notes)
}

/// The dry-run note for one planned package.
fn plan_note(candidate: &Candidate<'_>) -> String {
    let label = candidate.label();
    match (candidate.entry, candidate.checked) {
        (Some(entry), Some(checked)) => format!(
            "{label}: {}. Side effects: {}. Tested on this model ({checked}).",
            entry.what, entry.side_effects
        ),
        (Some(entry), None) => format!(
            "{label}: UNTESTED on this model. {}. Side effects: {}.",
            entry.what, entry.side_effects
        ),
        (None, _) => format!(
            "{label}: UNTESTED on this model; RustTV knows nothing about what it does or what \
             stops working without it."
        ),
    }
}

/// Plans undoing a privacy plan recorded in `journal`. Read-only on the
/// device.
///
/// The journal is a file anyone could edit, so the plan is refused unless it
/// is a privacy journal whose every step only re-enables a package. Packages
/// that are enabled already (re-enabled since) are skipped: their steps are
/// marked undone in `journal` (in memory; [`crate::restore`] saves it), and
/// the plan's notes name them. If every package is enabled already, the
/// journal is marked restored (in memory) and `PRECONDITION_FAILED` returned.
pub fn privacy_restore_plan(
    adb: &dyn Adb,
    serial: &str,
    journal: &mut Journal,
) -> Result<Plan, PlanError> {
    if journal.plan != PRIVACY {
        return Err(PlanError::precondition(format!(
            "this is a `{}` journal, not a {PRIVACY} journal",
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

    let disabled = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;
    let disabled = package_names(&disabled)?;
    let mut skipped = Vec::new();
    for step in &mut journal.steps {
        if let DeviceCommand::DisablePackage(package) = &step.command
            && step.status.may_have_run()
            && !disabled.contains(package.as_str())
        {
            step.status = StepStatus::Undone;
            skipped.push(package.to_string());
        }
    }
    if skipped.is_empty() {
        return Ok(plan);
    }
    if !journal.steps.iter().any(|step| step.status.may_have_run()) {
        // Nothing is left to undo, so the journal is done: callers that save
        // it keep the next `optimize restore` from finding it again.
        journal.status = JournalStatus::Restored;
        return Err(PlanError::precondition(format!(
            "nothing to restore: every package this journal disabled is enabled now ({})",
            skipped.join(", ")
        )));
    }
    let plan = restore_plan(journal, serial, &fingerprint)?;
    Ok(plan.with_notes(skipped.into_iter().map(|package| {
        format!("Skipped {package}: it is enabled now (re-enabled since); left as is.")
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RiskClass;
    use crate::adb::FakeAdb;
    use crate::catalog::PrivacyChoices;
    use crate::command::SettingKey;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{JOURNAL_SCHEMA_VERSION, JournalStep};
    use crate::optimizer::OptimizerState;
    use crate::plan::{apply, restore};

    const SERIAL: &str = "tv";

    macro_rules! philips {
        ($file:literal) => {
            include_str!(concat!("fixtures/philips-65oled818-api34/", $file)).to_owned()
        };
    }
    macro_rules! synthetic {
        ($file:literal) => {
            include_str!(concat!("fixtures/synthetic-privacy/", $file)).to_owned()
        };
    }

    const CANDEEBUG: &str = "org.droidtv.candeebug";
    const GEL: &str = "org.droidtv.gel";
    const ADSERVICES: &str = "com.android.adservices.api";
    const ODP: &str = "com.android.ondevicepersonalization.services";
    const FEDERATED: &str = "com.android.federatedcompute.services";
    const PRINTSPOOLER: &str = "com.android.printspooler";
    /// Telemetry in the catalog, installed in the fixture (not on the real TV),
    /// not tested on the Philips.
    const AUTOLOG: &str = "org.droidtv.usagelogger";
    /// Opt-in (casting), installed in the fixture, not tested on the Philips.
    const MEDIASHELL: &str = "com.google.android.apps.mediashell";
    /// Opt-in (voice), installed in the fixture, not tested on the Philips.
    const ALEXA: &str = "org.droidtv.amazonalexa";

    const TELEMETRY: [&str; 5] = [
        CANDEEBUG,
        GEL,
        "com.google.android.partnersetup",
        "com.google.android.feedback",
        "com.android.tv.feedbackconsent",
    ];
    const ADS: [&str; 3] = [ADSERVICES, ODP, FEDERATED];
    const EXTRAS: [&str; 4] = [
        "com.google.android.play.games",
        "com.google.android.syncadapters.calendar",
        PRINTSPOOLER,
        "com.google.android.onetimeinitializer",
    ];

    fn package(name: &str) -> PackageName {
        PackageName::new(name).unwrap()
    }

    fn packages(names: &[&str]) -> Vec<PackageName> {
        names.iter().map(|name| package(name)).collect()
    }

    /// `pm list packages -d` output listing `names`.
    fn disabled(names: &[&str]) -> String {
        names
            .iter()
            .map(|name| format!("package:{name}\n"))
            .collect()
    }

    /// The bundled catalog and profiles with these choices (no file).
    fn data(choices: PrivacyChoices) -> PrivacyData {
        PrivacyData::bundled(LoadedChoices {
            path: None,
            choices,
        })
    }

    /// The bundled data with the default choices.
    fn defaults() -> PrivacyData {
        data(PrivacyChoices::default())
    }

    /// The Philips TV with the catalog packages of the fixture installed and
    /// enabled, RustTV installed, no accessibility service on. `disabled` is
    /// the output of `pm list packages -d` on each call.
    fn tv(disabled: Vec<String>) -> FakeAdb {
        FakeAdb::new()
            .respond(DeviceCommand::GetProperties, philips!("getprop.txt"))
            .respond(DeviceCommand::HomeActivity, philips!("home-activity.txt"))
            .respond(
                DeviceCommand::HomeCandidates,
                philips!("home-candidates-rusttv-installed.txt"),
            )
            .respond(DeviceCommand::InputMethods, synthetic!("ime-list.txt"))
            .respond(
                DeviceCommand::EnabledAccessibilityServices,
                synthetic!("accessibility-none.txt"),
            )
            .respond(DeviceCommand::ListPackages, synthetic!("packages-f.txt"))
            .respond(DeviceCommand::TvInputs, synthetic!("tv-input.txt"))
            .respond(
                DeviceCommand::GetSetting(SettingKey::WindowAnimationScale),
                "null\n",
            )
            .respond(
                DeviceCommand::GetSetting(SettingKey::TransitionAnimationScale),
                "null\n",
            )
            .respond(
                DeviceCommand::GetSetting(SettingKey::AnimatorDurationScale),
                "null\n",
            )
            .respond_sequence(DeviceCommand::ListDisabledPackages, disabled)
    }

    /// A journal directory that doesn't exist: no journals.
    fn no_journals() -> std::path::PathBuf {
        std::env::temp_dir().join("rusttv-privacy-tests-no-journals")
    }

    /// [`tv`] with one more package installed.
    fn tv_with(extra: &str, disabled: Vec<String>) -> FakeAdb {
        tv(disabled).respond(
            DeviceCommand::ListPackages,
            format!(
                "{}package:/data/app/x/base.apk={extra}\n",
                synthetic!("packages-f.txt")
            ),
        )
    }

    fn with_toggles(mut adb: FakeAdb, names: &[&str]) -> FakeAdb {
        for name in names {
            adb = adb
                .respond(DeviceCommand::DisablePackage(package(name)), "")
                .respond(DeviceCommand::EnablePackage(package(name)), "");
        }
        adb
    }

    fn changes(adb: &FakeAdb) -> Vec<DeviceCommand> {
        adb.calls()
            .into_iter()
            .filter(|command| command.risk() > RiskClass::Read)
            .collect()
    }

    fn planned(plan: &Plan) -> Vec<String> {
        plan.steps()
            .iter()
            .map(|step| match step.command() {
                DeviceCommand::DisablePackage(package) => package.to_string(),
                other => panic!("unexpected step {other:?}"),
            })
            .collect()
    }

    fn note<'a>(notes: &'a [String], start: &str) -> &'a str {
        notes
            .iter()
            .find(|note| note.starts_with(start))
            .unwrap_or_else(|| panic!("no note starting with {start}: {notes:?}"))
    }

    #[test]
    fn the_static_list_reads_and_covers_the_required_packages() {
        let list: ProtectedList = toml::from_str(PROTECTED_TOML).unwrap();
        assert_eq!(list.schema_version, PROTECTED_SCHEMA_VERSION);
        for name in [
            "android",
            "com.android.systemui",
            "com.android.settings",
            "com.android.tv.settings",
            "com.android.packageinstaller",
            "com.google.android.packageinstaller",
            "com.android.networkstack",
            "com.android.networkstack.tethering",
            "com.android.bluetooth",
            "com.google.android.gms",
            "com.android.vending",
            "com.google.android.tungsten.setupwraith",
            "com.google.android.apps.tv.launcherx",
            "com.android.providers.tv",
            "com.mediatek.tvinput",
            "com.droidlogic.tvinput",
            "com.google.android.katniss",
            "com.google.android.inputmethod.latin",
            "com.mediatek.tv.service",
            "org.droidtv",
            "com.android.shell",
            "ai.rusttv.launcher",
        ] {
            assert!(statically_protected(name).is_some(), "{name}");
        }
        // `org.droidtv` is exact: the TV service, not every TP Vision app.
        assert_eq!(statically_protected(CANDEEBUG), None);
        assert_eq!(statically_protected("com.google.android.gmsx"), None);
    }

    #[test]
    fn input_method_and_accessibility_output_is_parsed_or_refused() {
        let imes = parse_input_methods(
            "com.google.android.inputmethod.latin/com.android.inputmethod.latin.LatinIME\n\
             com.example.keyboard/.Ime\n\n",
        )
        .unwrap();
        assert!(imes.contains("com.example.keyboard"));
        assert_eq!(imes.len(), 2);
        assert_eq!(parse_input_methods("").unwrap().len(), 0);
        for bad in [
            "Error: unknown command",
            "cmd: Can't find service: input_method",
        ] {
            assert_eq!(parse_input_methods(bad).unwrap_err(), bad);
        }

        assert!(parse_accessibility_services("null\n").unwrap().is_empty());
        assert!(parse_accessibility_services("").unwrap().is_empty());
        let services =
            parse_accessibility_services("com.example.reader/.Reader:com.b.c/com.b.c.Svc\n")
                .unwrap();
        assert!(services.contains("com.example.reader") && services.contains("com.b.c"));
        assert!(parse_accessibility_services("Exception occurred").is_err());
    }

    #[test]
    fn default_categories_plan_the_tested_packages_in_order() {
        use PrivacyCategory::{Ads, Extras, Telemetry};
        let all: Vec<&str> = TELEMETRY
            .iter()
            .chain(&ADS)
            .chain(&EXTRAS)
            .copied()
            .collect();
        for (only, expected) in [
            (vec![], all.clone()),
            (vec![Telemetry, Ads, Extras], all.clone()),
            (vec![Telemetry], TELEMETRY.to_vec()),
            (vec![Ads], ADS.to_vec()),
            (vec![Extras], EXTRAS.to_vec()),
            // Category order, then tested order, whatever order `only` has.
            (
                vec![Ads, Telemetry],
                TELEMETRY.iter().chain(&ADS).copied().collect(),
            ),
        ] {
            let adb = tv(vec![disabled(&[])]);
            let plan = privacy_plan(&adb, SERIAL, &defaults(), &only, false).unwrap();
            assert_eq!(planned(&plan), expected, "{only:?}");
            assert!(changes(&adb).is_empty(), "planning is read-only");
        }
    }

    #[test]
    fn untested_packages_are_left_out_and_named_without_the_flag() {
        let adb = tv(vec![disabled(&[])]);
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[], false).unwrap();
        assert!(!planned(&plan).contains(&AUTOLOG.to_owned()));
        let notes = plan.dry_run().notes;
        let left_out = note(&notes, "Not planned, UNTESTED on this model");
        assert!(left_out.contains(AUTOLOG), "{left_out}");
        // Opt-in packages aren't chosen at all, so not mentioned.
        assert!(!left_out.contains(MEDIASHELL) && !left_out.contains(ALEXA));
        assert!(
            note(&notes, "Tested on this model").contains("12 of the 12 planned"),
            "{notes:?}"
        );
    }

    #[test]
    fn allow_untested_plans_them_marked_untested() {
        let adb = tv(vec![disabled(&[])]);
        let plan = privacy_plan(
            &adb,
            SERIAL,
            &defaults(),
            &[PrivacyCategory::Telemetry],
            true,
        )
        .unwrap();
        let mut expected = TELEMETRY.to_vec();
        expected.push(AUTOLOG);
        assert_eq!(planned(&plan), expected);
        let notes = plan.dry_run().notes;
        assert!(
            note(&notes, AUTOLOG)
                .starts_with(&format!("{AUTOLOG} (telemetry): UNTESTED on this model. ")),
            "{notes:?}"
        );
        assert!(
            note(&notes, CANDEEBUG).ends_with("Tested on this model (2026-10-23)."),
            "{notes:?}"
        );
        assert!(note(&notes, "Tested on this model").contains("5 of the 6 planned"));
    }

    #[test]
    fn opt_in_categories_are_planned_only_when_chosen() {
        // Not by default, and `--only` can't add them: it only narrows.
        let error = privacy_plan(
            &tv(vec![disabled(&[])]),
            SERIAL,
            &defaults(),
            &[PrivacyCategory::Casting],
            true,
        )
        .unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("select no package"), "{error}");

        // Chosen in the choices file: planned, but untested on the Philips.
        let casting = data(PrivacyChoices {
            categories: vec![PrivacyCategory::Casting],
            ..PrivacyChoices::default()
        });
        let error =
            privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &casting, &[], false).unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        let message = error.to_string();
        assert!(
            message.contains(MEDIASHELL) && message.contains("--allow-untested"),
            "{message}"
        );
        let plan = privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &casting, &[], true).unwrap();
        assert_eq!(planned(&plan), [MEDIASHELL]);
        let notes = plan.dry_run().notes;
        assert!(note(&notes, MEDIASHELL).contains("(casting): UNTESTED on this model"));
    }

    #[test]
    fn keep_also_and_extra_change_what_is_chosen() {
        let choices = PrivacyChoices {
            categories: vec![PrivacyCategory::Telemetry],
            keep: packages(&[CANDEEBUG]),
            also: packages(&[ALEXA, PRINTSPOOLER]),
            extra: packages(&["com.example.preinstalled"]),
        };
        let adb = tv_with("com.example.preinstalled", vec![disabled(&[])]);
        let plan = privacy_plan(&adb, SERIAL, &data(choices.clone()), &[], true).unwrap();
        let mut expected: Vec<&str> = TELEMETRY[1..].to_vec();
        expected.extend([AUTOLOG, PRINTSPOOLER, ALEXA, "com.example.preinstalled"]);
        assert_eq!(planned(&plan), expected);
        let notes = plan.dry_run().notes;
        assert!(
            note(&notes, "com.example.preinstalled").contains(
                "(extra, not in the catalog): UNTESTED on this model; RustTV knows nothing"
            ),
            "{notes:?}"
        );

        // `keep` wins over `also` and `extra`.
        let kept = PrivacyChoices {
            keep: packages(&[CANDEEBUG, ALEXA, "com.example.preinstalled"]),
            ..choices.clone()
        };
        let adb = tv_with("com.example.preinstalled", vec![disabled(&[])]);
        let plan = privacy_plan(&adb, SERIAL, &data(kept), &[], true).unwrap();
        assert!(
            !planned(&plan).iter().any(|name| name == ALEXA
                || name == CANDEEBUG
                || name == "com.example.preinstalled")
        );

        // `--only` narrows: `also` only within those categories, `extra` not at all.
        let adb = tv_with("com.example.preinstalled", vec![disabled(&[])]);
        let plan = privacy_plan(
            &adb,
            SERIAL,
            &data(choices),
            &[PrivacyCategory::Telemetry, PrivacyCategory::Extras],
            true,
        )
        .unwrap();
        let mut expected: Vec<&str> = TELEMETRY[1..].to_vec();
        expected.extend([AUTOLOG, PRINTSPOOLER]);
        assert_eq!(planned(&plan), expected);
    }

    #[test]
    fn invalid_choices_are_refused_before_reading_the_device() {
        let choices = PrivacyChoices {
            also: packages(&["com.example.unknown"]),
            ..PrivacyChoices::default()
        };
        let adb = FakeAdb::new();
        let error = privacy_plan(&adb, SERIAL, &data(choices.clone()), &[], false).unwrap_err();
        assert_eq!(error.code(), "INVALID_CHOICES");
        let error = optimize_analyze(&adb, SERIAL, &data(choices), &no_journals()).unwrap_err();
        assert_eq!(error.code(), "INVALID_CHOICES");
        assert!(adb.calls().is_empty());
    }

    #[test]
    fn dry_run_shows_what_and_side_effects_and_checks_home_after_the_last_step() {
        let adb = tv(vec![disabled(&[])]);
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap();
        let dry_run = plan.dry_run();
        assert_eq!(dry_run.plan, PRIVACY);
        assert_eq!(dry_run.max_risk, "R3");
        assert_eq!(dry_run.confirmation, "privacy");
        assert_eq!(
            dry_run.steps[0].command,
            format!("adb -s {SERIAL} shell pm disable-user --user 0 {ADSERVICES}")
        );
        assert_eq!(
            dry_run.steps[0].records.as_deref(),
            Some(format!("{ADSERVICES} is enabled").as_str())
        );
        assert_eq!(
            dry_run.steps[0].verify,
            [format!("{ADSERVICES} is disabled")]
        );
        let last = &dry_run.steps[2].verify;
        assert_eq!(last[0], format!("{FEDERATED} is disabled"));
        assert_eq!(
            last[1],
            "Home opens com.google.android.apps.tv.launcherx/.home.HomeActivity"
        );
        assert!(
            last[2].starts_with("the Home candidates are still "),
            "{last:?}"
        );
        let ads_note = note(&dry_run.notes, ADSERVICES);
        assert!(ads_note.contains("(ads)"), "{ads_note}");
        assert!(
            ads_note.contains("told they are unavailable"),
            "side effects shown: {ads_note}"
        );
    }

    #[test]
    fn nothing_tested_or_allowed_is_refused_as_not_tested() {
        // A profile that lists no tested package.
        let mut untested = defaults();
        for profile in &mut untested.profiles {
            profile.privacy = None;
        }
        let error =
            privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &untested, &[], false).unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        assert!(error.to_string().contains("--allow-untested"), "{error}");
        // With the flag every package is planned, all marked untested.
        let plan = privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &untested, &[], true).unwrap();
        assert_eq!(planned(&plan).len(), 13);
        assert!(note(&plan.dry_run().notes, "Tested on this model").contains("0 of the 13"));

        // No profile for the device at all.
        let adb = tv(vec![disabled(&[])]).respond(
            DeviceCommand::GetProperties,
            include_str!("fixtures/synthetic-home-takeover/getprop.txt"),
        );
        let error = privacy_plan(&adb, SERIAL, &defaults(), &[], false).unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        assert!(error.to_string().contains("no profile"), "{error}");
    }

    #[test]
    fn a_protected_package_among_the_chosen_refuses_the_whole_plan() {
        let cases: Vec<(&str, FakeAdb, &str)> = vec![
            (
                "com.example.keyboard",
                tv(vec![disabled(&[])]).respond(
                    DeviceCommand::InputMethods,
                    format!("{}com.example.keyboard/.Ime\n", synthetic!("ime-list.txt")),
                ),
                "input method",
            ),
            (
                "com.example.reader",
                tv(vec![disabled(&[])]).respond(
                    DeviceCommand::EnabledAccessibilityServices,
                    "com.example.reader/com.example.reader.ReaderService\n",
                ),
                "accessibility service",
            ),
            (
                "com.example.home",
                tv(vec![disabled(&[])]).respond(
                    DeviceCommand::HomeCandidates,
                    format!(
                        "{}  Activity #4:\n    priority=0 preferredOrder=0 match=0x108000\n    com.example.home/.Home\n",
                        philips!("home-candidates-rusttv-installed.txt")
                    ),
                ),
                "can handle Home",
            ),
            (
                "com.google.android.apps.tv.launcherx",
                tv(vec![disabled(&[])]),
                "the Home button opens it",
            ),
            ("com.google.android.gms", tv(vec![disabled(&[])]), "Google Play services"),
            (
                "com.android.networkstack.tethering",
                tv(vec![disabled(&[])]),
                "network stack",
            ),
            ("com.mediatek.tvinput", tv(vec![disabled(&[])]), "TV input"),
        ];
        for (extra, adb, why) in cases {
            // Through `extra`, even with --allow-untested.
            let choices = PrivacyChoices {
                extra: packages(&[extra]),
                ..PrivacyChoices::default()
            };
            let error = privacy_plan(&adb, SERIAL, &data(choices), &[], true).unwrap_err();
            assert_eq!(error.code(), "PACKAGE_PROTECTED", "{extra}: {error}");
            let message = error.to_string();
            assert!(
                message.contains(extra) && message.contains(why) && message.contains("`keep`"),
                "{message}"
            );
            assert!(changes(&adb).is_empty());
        }
    }

    #[test]
    fn unreadable_input_methods_refuse_the_plan() {
        let adb = tv(vec![disabled(&[])]).respond(
            DeviceCommand::InputMethods,
            "cmd: Can't find service: input_method\n",
        );
        let error = privacy_plan(&adb, SERIAL, &defaults(), &[], false).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("input methods"), "{error}");
    }

    #[test]
    fn absent_and_already_disabled_packages_are_skipped_and_named() {
        let adb = tv(vec![disabled(&[GEL])]).respond(
            DeviceCommand::ListPackages,
            synthetic!("packages-f.txt").replace(PRINTSPOOLER, "com.example.unrelated"),
        );
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[], false).unwrap();
        let steps = planned(&plan);
        assert_eq!(steps.len(), 10);
        assert!(!steps.iter().any(|step| step == GEL || step == PRINTSPOOLER));
        let notes = plan.dry_run().notes;
        assert!(
            notes.contains(&format!("Skipped {GEL}: already disabled.")),
            "{notes:?}"
        );
        // Absent catalog packages are only counted.
        assert!(
            !notes.iter().any(|note| note.contains(PRINTSPOOLER)),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|note| note.starts_with("Skipped ")
                && note.ends_with(
                    "catalog packages of the chosen categories: not installed on \
                                   this device."
                )),
            "{notes:?}"
        );

        // A package the person named is still named.
        let named = data(PrivacyChoices {
            extra: packages(&["com.example.missing"]),
            ..PrivacyChoices::default()
        });
        let plan = privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &named, &[], true).unwrap();
        let notes = plan.dry_run().notes;
        assert!(
            notes.contains(&"Skipped com.example.missing: not installed.".to_owned()),
            "{notes:?}"
        );

        // Nothing left to disable.
        let adb = tv(vec![disabled(&ADS)]);
        let error =
            privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    #[test]
    fn apply_then_restore_skips_a_package_re_enabled_since() {
        let adb = with_toggles(
            tv(vec![
                // Planning, then the original state of each step.
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                // After each step.
                disabled(&[ADSERVICES]),
                disabled(&[ADSERVICES, ODP]),
                disabled(&ADS),
                // Restore planning: the person re-enabled ODP in Settings.
                disabled(&[ADSERVICES, FEDERATED]),
                // After each re-enable.
                disabled(&[ADSERVICES]),
                disabled(&[]),
            ]),
            &ADS,
        );
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap();
        let approval = plan.approve("privacy").unwrap();
        let mut store = MemoryJournal::default();
        let journal = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap();
        assert_eq!(journal.status, JournalStatus::Applied);
        assert_eq!(journal.plan, PRIVACY);

        let mut journal = store.last().clone();
        let plan = privacy_restore_plan(&adb, SERIAL, &mut journal).unwrap();
        assert_eq!(plan.name(), "privacy-restore");
        assert_eq!(
            plan.steps().iter().map(Step::command).collect::<Vec<_>>(),
            [
                &DeviceCommand::EnablePackage(package(FEDERATED)),
                &DeviceCommand::EnablePackage(package(ADSERVICES)),
            ]
        );
        note(
            &plan.dry_run().notes,
            &format!("Skipped {ODP}: it is enabled now"),
        );
        let approval = plan.approve("restore").unwrap();
        restore(&adb, &plan, &approval, &mut journal, &mut store).unwrap();
        assert_eq!(store.last().status, JournalStatus::Restored);
        assert!(
            store
                .last()
                .steps
                .iter()
                .all(|step| step.status == StepStatus::Undone)
        );
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(package(ADSERVICES)),
                DeviceCommand::DisablePackage(package(ODP)),
                DeviceCommand::DisablePackage(package(FEDERATED)),
                DeviceCommand::EnablePackage(package(FEDERATED)),
                DeviceCommand::EnablePackage(package(ADSERVICES)),
            ]
        );
    }

    #[test]
    fn a_journal_written_by_the_previous_version_still_restores() {
        // The exact format `optimize apply privacy` wrote before the catalog
        // existed (plan `privacy`, one disable step per package).
        let text = format!(
            "schema_version = 1\nplan = \"privacy\"\nserial = \"{SERIAL}\"\n\
             fingerprint = \"Philips/PH1M_WW_9972/PH1M_WW_9972:14/UKN2.241117.001/3000518:user/release-keys\"\n\
             started_at_unix_ms = 1761220000000\nstatus = \"applied\"\n\n\
             [[steps]]\nadb = \"adb -s {SERIAL} shell pm disable-user --user 0 {CANDEEBUG}\"\n\
             command = {{ kind = \"disable_package\", arg = \"{CANDEEBUG}\" }}\n\
             original_state = {{ kind = \"package_enabled\", arg = \"{CANDEEBUG}\" }}\n\
             status = \"done\"\n"
        );
        let mut journal: Journal = toml::from_str(&text).unwrap();
        let adb = tv(vec![disabled(&[CANDEEBUG])]);
        let plan = privacy_restore_plan(&adb, SERIAL, &mut journal).unwrap();
        assert_eq!(
            plan.steps().iter().map(Step::command).collect::<Vec<_>>(),
            [&DeviceCommand::EnablePackage(package(CANDEEBUG))]
        );
    }

    #[test]
    fn home_changing_after_the_last_step_rolls_everything_back() {
        let adb = with_toggles(
            tv(vec![
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[ADSERVICES]),
                disabled(&[ADSERVICES, ODP]),
                disabled(&ADS),
                // After each undo, last first.
                disabled(&[ADSERVICES, ODP]),
                disabled(&[ADSERVICES]),
                disabled(&[]),
            ]),
            &ADS,
        )
        // Home opens the stock launcher when planning, then setupwraith.
        .respond_sequence(
            DeviceCommand::HomeActivity,
            [
                philips!("home-activity.txt"),
                "com.google.android.tungsten.setupwraith/.RecoveryActivity\n".to_owned(),
            ],
        );
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap();
        let approval = plan.approve("privacy").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error
                .to_string()
                .contains("Home opens com.google.android.apps.tv.launcherx/.home.HomeActivity"),
            "{error}"
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(package(ADSERVICES)),
                DeviceCommand::DisablePackage(package(ODP)),
                DeviceCommand::DisablePackage(package(FEDERATED)),
                DeviceCommand::EnablePackage(package(FEDERATED)),
                DeviceCommand::EnablePackage(package(ODP)),
                DeviceCommand::EnablePackage(package(ADSERVICES)),
            ]
        );
    }

    #[test]
    fn a_changed_set_of_home_candidates_rolls_back() {
        let adb = with_toggles(
            tv(vec![
                disabled(&[]),
                disabled(&[]),
                disabled(&[PRINTSPOOLER]),
                disabled(&[]),
            ]),
            &[PRINTSPOOLER],
        )
        .respond_sequence(
            DeviceCommand::HomeCandidates,
            [
                philips!("home-candidates-rusttv-installed.txt"),
                philips!("home-candidates.txt"),
            ],
        );
        let only_printing = data(PrivacyChoices {
            categories: vec![PrivacyCategory::Extras],
            keep: packages(&[
                "com.google.android.play.games",
                "com.google.android.syncadapters.calendar",
                "com.google.android.onetimeinitializer",
            ]),
            ..PrivacyChoices::default()
        });
        let plan = privacy_plan(&adb, SERIAL, &only_printing, &[], false).unwrap();
        assert_eq!(planned(&plan), [PRINTSPOOLER]);
        let approval = plan.approve("privacy").unwrap();
        let error = apply(
            &adb,
            &plan,
            &approval,
            &mut MemoryJournal::default(),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(error.to_string().contains("Home candidates"), "{error}");
    }

    fn journal(plan: &str, steps: Vec<JournalStep>) -> Journal {
        Journal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            plan: plan.to_owned(),
            serial: SERIAL.to_owned(),
            fingerprint: "x".to_owned(),
            started_at_unix_ms: 0,
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

    #[test]
    fn restore_refuses_a_journal_that_would_disable_something_or_isnt_privacy() {
        // A tampered journal: "undoing" an enable would disable Settings.
        let mut tampered = journal(
            PRIVACY,
            vec![done(DeviceCommand::EnablePackage(package(
                "com.android.tv.settings",
            )))],
        );
        let error =
            privacy_restore_plan(&tv(vec![disabled(&[])]), SERIAL, &mut tampered).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("refusing"), "{error}");

        let mut takeover = journal(
            crate::HOME_TAKEOVER,
            vec![done(DeviceCommand::DisablePackage(package(GEL)))],
        );
        let error =
            privacy_restore_plan(&tv(vec![disabled(&[GEL])]), SERIAL, &mut takeover).unwrap_err();
        assert!(
            error.to_string().contains("not a privacy journal"),
            "{error}"
        );
    }

    #[test]
    fn restore_with_every_package_enabled_again_has_nothing_to_do() {
        let mut journal = journal(
            PRIVACY,
            vec![done(DeviceCommand::DisablePackage(package(GEL)))],
        );
        let error =
            privacy_restore_plan(&tv(vec![disabled(&[])]), SERIAL, &mut journal).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("nothing to restore"), "{error}");
        // Marked restored, so a saved journal isn't found again.
        assert_eq!(journal.status, JournalStatus::Restored);
        assert!(!journal.status.is_restorable());
    }

    #[test]
    fn analyze_lists_present_catalog_packages_and_extras() {
        let choices = PrivacyChoices {
            keep: packages(&[CANDEEBUG]),
            extra: packages(&["com.example.preinstalled", "com.android.vending"]),
            ..PrivacyChoices::default()
        };
        let adb = tv(vec![disabled(&[GEL])]).respond(
            DeviceCommand::ListPackages,
            format!(
                "{}package:/data/app/x/base.apk=com.example.preinstalled\n",
                synthetic!("packages-f.txt").replace(PRINTSPOOLER, "com.example.unrelated")
            ),
        );
        let analysis = optimize_analyze(&adb, SERIAL, &data(choices), &no_journals()).unwrap();
        assert_eq!(analysis.profile.as_deref(), Some("tpv-ph1m-ww-9972.toml"));
        assert_eq!(analysis.choices, None);
        assert_eq!(analysis.categories, PrivacyCategory::DEFAULT);
        let names: Vec<&str> = analysis
            .packages
            .iter()
            .map(|entry| entry.package.as_str())
            .collect();
        // Present catalog packages in plan order, then the extras (absent too).
        assert_eq!(names.len(), 14 + 2, "{names:?}");
        assert_eq!(names[0], CANDEEBUG);
        assert_eq!(
            &names[names.len() - 2..],
            ["com.example.preinstalled", "com.android.vending"]
        );
        assert!(
            !names.contains(&PRINTSPOOLER),
            "absent catalog packages aren't listed"
        );
        let entry = |name: &str| {
            analysis
                .packages
                .iter()
                .find(|entry| entry.package == name)
                .unwrap()
        };
        assert_eq!(entry(GEL).state, PackageState::Disabled);
        assert!(entry(GEL).tested && entry(GEL).chosen && entry(GEL).default);
        assert_eq!(entry(GEL).checked.as_deref(), Some("2026-10-23"));
        assert!(!entry(CANDEEBUG).chosen, "kept");
        assert!(!entry(AUTOLOG).tested && entry(AUTOLOG).chosen);
        let cast = entry(MEDIASHELL);
        assert_eq!(cast.category, Some(PrivacyCategory::Casting));
        assert!(!cast.default && !cast.chosen && !cast.tested);
        let extra = entry("com.example.preinstalled");
        assert_eq!(extra.source, PackageSource::Extra);
        assert_eq!((extra.category, extra.what.as_deref()), (None, None));
        assert!(extra.chosen && !extra.tested);
        assert_eq!(extra.state, PackageState::Enabled);
        let vending = entry("com.android.vending");
        assert!(vending.protected.as_deref().unwrap().contains("Play Store"));
        assert!(changes(&adb).is_empty());

        // Another device: no profile, nothing tested, same catalog.
        let adb = tv(vec![disabled(&[])]).respond(
            DeviceCommand::GetProperties,
            include_str!("fixtures/synthetic-home-takeover/getprop.txt"),
        );
        let analysis = optimize_analyze(&adb, SERIAL, &defaults(), &no_journals()).unwrap();
        assert_eq!(analysis.profile, None);
        assert!(analysis.packages.iter().all(|entry| !entry.tested));
        assert_eq!(analysis.packages.len(), 15);
        // The optimizer profiles are reported next to the packages.
        let optimizer: Vec<(&str, OptimizerState)> = analysis
            .optimizer
            .iter()
            .map(|status| (status.profile.as_str(), status.status))
            .collect();
        assert_eq!(
            optimizer,
            [(crate::REDUCED_ANIMATIONS, OptimizerState::NotTested)]
        );
    }

    #[test]
    fn bound_tv_input_services_are_read_from_the_service_map() {
        let bound: Vec<String> = parse_bound_tv_input_services(&synthetic!("tv-input.txt"))
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            bound,
            [
                "com.mediatek.tis/com.mediatek.tis.HdmiInputService",
                "com.mediatek.dtv.tvinput.dvbtuner/com.mediatek.dtv.tvinput.dvbtuner.DvbTvInputService",
                "com.mediatek.AirplayAPK/com.mediatek.AirplayAPK.AirPlayTvInputService",
            ]
        );
        // A device without TV inputs.
        assert!(parse_bound_tv_input_services("Can't find service: tv_input\n").is_empty());
    }

    #[test]
    fn a_package_protected_since_planning_stops_apply_before_its_step() {
        // ODP becomes an enabled accessibility service after the plan is made.
        let adb = with_toggles(
            tv(vec![
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[ADSERVICES]),
                // After undoing step 1.
                disabled(&[]),
            ]),
            &ADS,
        )
        .respond_sequence(
            DeviceCommand::EnabledAccessibilityServices,
            [
                synthetic!("accessibility-none.txt").to_owned(),
                synthetic!("accessibility-none.txt").to_owned(),
                format!("{ODP}/.Reader\n"),
            ],
        );
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap();
        assert_eq!(
            plan.steps()[1].requirements(),
            [Check::NotProtected(package(ODP))]
        );
        let approval = plan.approve("privacy").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error.to_string().contains(&format!(
                "before step 2: no longer true `{ODP} passes the protected-package check`"
            )),
            "{error}"
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(
            changes(&adb),
            [
                DeviceCommand::DisablePackage(package(ADSERVICES)),
                DeviceCommand::EnablePackage(package(ADSERVICES)),
            ]
        );
    }

    #[test]
    fn a_tv_input_service_lost_after_the_last_step_rolls_back() {
        let adb = with_toggles(
            tv(vec![
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[ADSERVICES]),
                disabled(&[ADSERVICES, ODP]),
                disabled(&ADS),
                disabled(&[ADSERVICES, ODP]),
                disabled(&[ADSERVICES]),
                disabled(&[]),
            ]),
            &ADS,
        )
        // The HDMI input service is no longer bound after the last step.
        .respond_sequence(
            DeviceCommand::TvInputs,
            [
                synthetic!("tv-input.txt").to_owned(),
                synthetic!("tv-input.txt").replacen("bound: true", "bound: false", 1),
            ],
        );
        let plan = privacy_plan(&adb, SERIAL, &defaults(), &[PrivacyCategory::Ads], false).unwrap();
        assert!(
            note(&plan.dry_run().notes, "After the last step")
                .contains("the 3 TV input services bound now must still be bound"),
            "{:?}",
            plan.dry_run().notes
        );
        let approval = plan.approve("privacy").unwrap();
        let mut store = MemoryJournal::default();

        let error = apply(&adb, &plan, &approval, &mut store, &|_| {}).unwrap_err();
        assert_eq!(error.code(), "ROLLED_BACK", "{error}");
        assert!(
            error
                .to_string()
                .contains("the TV input services are still bound"),
            "{error}"
        );
        assert_eq!(store.last().status, JournalStatus::RolledBack);
        assert_eq!(
            changes(&adb).len(),
            6,
            "three disabled, three enabled again"
        );
    }

    #[test]
    fn a_tv_input_of_a_planned_package_is_not_required_afterwards() {
        let adb = tv(vec![disabled(&[])]).respond(
            DeviceCommand::ListPackages,
            format!(
                "{}package:/system/app/x/x.apk=com.mediatek.AirplayAPK\n",
                synthetic!("packages-f.txt")
            ),
        );
        let casting = data(PrivacyChoices {
            categories: vec![PrivacyCategory::Casting],
            ..PrivacyChoices::default()
        });
        let plan = privacy_plan(&adb, SERIAL, &casting, &[], true).unwrap();
        let last = plan.steps().last().unwrap().verification();
        let Some(Check::TvInputServicesBound(services)) = last.last() else {
            panic!("{last:?}");
        };
        let services: Vec<&str> = services.iter().map(Component::as_str).collect();
        assert_eq!(
            services,
            [
                "com.mediatek.tis/com.mediatek.tis.HdmiInputService",
                "com.mediatek.dtv.tvinput.dvbtuner/com.mediatek.dtv.tvinput.dvbtuner.DvbTvInputService",
            ]
        );
    }
}

//! Privacy profiles (R3): disable, never uninstall, the packages a device's
//! compat profile lists as telemetry, ads or extras, and undo it
//! (OPTIMIZER.md § Privacy profiles, SECURITY.md § Protected package policy).
//!
//! Both directions are [`Plan`]s run by the safety engine in [`crate::plan`].
//! Every package is checked against the protected-package policy first:
//! rules 1–2 live on the device ([`LiveState`]), rule 3 against the deny-list
//! `compat/protected.toml`, compiled into the binary.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::adb::Adb;
use crate::apps::package_names;
use crate::command::{Component, DeviceCommand, PackageName};
use crate::compat::{DeviceProfile, PrivacyCategory, ProfilePrivacy, profile_file_name};
use crate::inspect::parse_properties;
use crate::journal::{Journal, StepStatus};
use crate::launcher::{HomeCandidate, launcher_status};
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

/// `rusttv optimize analyze`: what the device's privacy profile would do. Read-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptimizeAnalysis {
    /// Target device.
    pub serial: String,
    /// File name of the bundled compat profile matching the device, if any.
    pub profile: Option<String>,
    /// The profile's `[privacy]` section with each package's state now;
    /// `None` if no profile matches or it has none.
    pub privacy: Option<PrivacyAnalysis>,
}

/// A privacy profile checked against the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivacyAnalysis {
    /// Whether the profile is tested on this model; `apply` is refused otherwise.
    pub tested: bool,
    /// Every package of the profile, in profile order.
    pub packages: Vec<PrivacyPackageStatus>,
}

/// One package of a privacy profile and its state on the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivacyPackageStatus {
    /// Package name.
    pub package: String,
    /// `telemetry`, `ads` or `extras`.
    pub category: PrivacyCategory,
    /// What it does.
    pub what: String,
    /// What stops working while it is disabled.
    pub side_effects: String,
    /// Date it was disabled and lived with on this model, or empty.
    pub checked: String,
    /// Installed and enabled, installed and disabled, or not installed.
    pub state: PackageState,
    /// Why RustTV refuses to disable it, if it does (protected-package policy).
    pub protected: Option<String>,
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

/// The bundled profile matching the device, with its file name.
fn matching_profile<'a>(
    adb: &dyn Adb,
    serial: &str,
    profiles: &'a [DeviceProfile],
) -> Result<
    (
        std::collections::HashMap<String, String>,
        Option<&'a DeviceProfile>,
    ),
    PlanError,
> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let profile = profiles.iter().find(|profile| profile.matches(&properties));
    Ok((properties, profile))
}

/// Installed and disabled packages now.
fn package_states(
    adb: &dyn Adb,
    serial: &str,
) -> Result<(HashSet<String>, HashSet<String>), PlanError> {
    let installed = adb.run(serial, &DeviceCommand::ListPackages)?;
    let disabled = adb.run(serial, &DeviceCommand::ListDisabledPackages)?;
    let owned = |names: HashSet<&str>| names.into_iter().map(str::to_owned).collect();
    Ok((
        owned(package_names(&installed)?),
        owned(package_names(&disabled)?),
    ))
}

/// Checks the device against its privacy profile. Read-only (R0).
pub fn optimize_analyze(
    adb: &dyn Adb,
    serial: &str,
    profiles: &[DeviceProfile],
) -> Result<OptimizeAnalysis, PlanError> {
    let (_, profile) = matching_profile(adb, serial, profiles)?;
    let Some(profile) = profile else {
        return Ok(OptimizeAnalysis {
            serial: serial.to_owned(),
            profile: None,
            privacy: None,
        });
    };
    let privacy = match &profile.privacy {
        Some(privacy) => Some(analyze_privacy(adb, serial, privacy)?),
        None => None,
    };
    Ok(OptimizeAnalysis {
        serial: serial.to_owned(),
        profile: Some(profile_file_name(profile)),
        privacy,
    })
}

fn analyze_privacy(
    adb: &dyn Adb,
    serial: &str,
    privacy: &ProfilePrivacy,
) -> Result<PrivacyAnalysis, PlanError> {
    let live = LiveState::read(adb, serial)?;
    let (installed, disabled) = package_states(adb, serial)?;
    let packages = privacy
        .packages
        .iter()
        .map(|entry| {
            let name = entry.package.as_str();
            let state = if !installed.contains(name) {
                PackageState::Absent
            } else if disabled.contains(name) {
                PackageState::Disabled
            } else {
                PackageState::Enabled
            };
            PrivacyPackageStatus {
                package: name.to_owned(),
                category: entry.category,
                what: entry.what.clone(),
                side_effects: entry.side_effects.clone(),
                checked: entry.checked.clone(),
                state,
                protected: live.protection(name),
            }
        })
        .collect();
    Ok(PrivacyAnalysis {
        tested: privacy.tested,
        packages,
    })
}

/// Plans disabling the packages of the device's privacy profile in
/// `categories` (all if empty). Read-only.
///
/// - One of `profiles` (normally [`crate::bundled_profiles`]) must match the
///   device's model and firmware family and have a `[privacy]` section with
///   `tested = true`; otherwise `PROFILE_NOT_TESTED`.
/// - Every selected package is checked against the protected-package policy
///   (Home, Home candidates, input methods, enabled accessibility services,
///   `compat/protected.toml`); if any is protected the whole plan is refused
///   with `PACKAGE_PROTECTED`.
/// - Packages not installed or already disabled are skipped and named in the
///   notes. Each other package gets one `pm disable-user` step.
/// - After the last step Home must open the same activity as before and the
///   Home candidates must be unchanged; otherwise the plan rolls back.
pub fn privacy_plan(
    adb: &dyn Adb,
    serial: &str,
    profiles: &[DeviceProfile],
    categories: &[PrivacyCategory],
) -> Result<Plan, PlanError> {
    let (properties, profile) = matching_profile(adb, serial, profiles)?;
    let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
    let Some(profile) = profile else {
        return Err(PlanError::profile_not_tested(format!(
            "no compatibility profile in compat/devices/ matches this device ({} {}, `{}`, \
             firmware `{}`); privacy profiles are only offered on devices where they were tested",
            property("ro.product.manufacturer"),
            property("ro.product.model"),
            property("ro.product.device"),
            property("ro.build.fingerprint"),
        )));
    };
    let profile_name = profile_file_name(profile);
    let Some(privacy) = &profile.privacy else {
        return Err(PlanError::profile_not_tested(format!(
            "the compatibility profile {profile_name} has no privacy profile (`[privacy]`)"
        )));
    };
    if !privacy.tested {
        return Err(PlanError::profile_not_tested(format!(
            "the privacy profile in {profile_name} isn't tested on this model yet; `[privacy] \
             tested = true` is set only after apply, a reboot, normal use and restore all \
             worked on it"
        )));
    }
    let selected: Vec<_> = privacy
        .packages
        .iter()
        .filter(|entry| categories.is_empty() || categories.contains(&entry.category))
        .collect();
    if selected.is_empty() {
        return Err(PlanError::precondition(format!(
            "the privacy profile in {profile_name} lists no package in the chosen categories"
        )));
    }

    let live = LiveState::read(adb, serial)?;
    let refused: Vec<String> = selected
        .iter()
        .filter_map(|entry| {
            live.protection(entry.package.as_str())
                .map(|why| format!("`{}`: {why}", entry.package))
        })
        .collect();
    if !refused.is_empty() {
        return Err(PlanError::protected(format!(
            "the privacy profile in {profile_name} lists protected packages, so nothing is \
             planned: {}",
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
    let candidates: Vec<Component> = live
        .candidates
        .iter()
        .map(|candidate| candidate.component.parse())
        .collect::<Result<_, _>>()
        .map_err(|error| PlanError::precondition(format!("Home candidate: {error}")))?;

    let (installed, disabled) = package_states(adb, serial)?;
    let mut notes = Vec::new();
    let mut skipped = Vec::new();
    let mut packages = Vec::new();
    for entry in selected {
        let name = entry.package.as_str();
        if !installed.contains(name) {
            skipped.push(format!("Skipped {name}: not installed."));
        } else if disabled.contains(name) {
            skipped.push(format!("Skipped {name}: already disabled."));
        } else {
            notes.push(format!(
                "{name} ({}): {}. Side effects: {}.",
                entry.category, entry.what, entry.side_effects
            ));
            packages.push(entry.package.clone());
        }
    }
    if packages.is_empty() {
        return Err(PlanError::precondition(format!(
            "nothing to do: every chosen package is absent or already disabled ({})",
            skipped.join(" ")
        )));
    }
    notes.extend(skipped);
    notes.extend([
        "Checked just now: none of these packages handles Home, provides an input method or \
         an enabled accessibility service, or is on compat/protected.toml."
            .to_owned(),
        format!(
            "After the last step Home must still open {home} and the Home candidates must be \
             unchanged; otherwise every step is undone."
        ),
        "Disables, never uninstalls. Undo with `rusttv optimize restore`.".to_owned(),
        "Without ADB: Settings > Apps > See all apps > Show system apps > <app> > Enable."
            .to_owned(),
        "Recovery is never guaranteed (SECURITY.md).".to_owned(),
        format!(
            "This privacy profile was tested on this model and firmware family ({profile_name})."
        ),
    ]);

    let last = packages.len() - 1;
    let steps = packages
        .into_iter()
        .enumerate()
        .map(|(index, package)| {
            let checks = if index == last {
                vec![
                    Check::HomeActivityIs(home.clone()),
                    Check::HomeCandidatesAre(candidates.clone()),
                ]
            } else {
                Vec::new()
            };
            Step::new(DeviceCommand::DisablePackage(package), checks)
        })
        .collect();
    let fingerprint = property("ro.build.fingerprint");
    Plan::new(PRIVACY, serial, fingerprint, PRIVACY, steps, notes)
}

/// Plans undoing a privacy plan recorded in `journal`. Read-only on the
/// device.
///
/// The journal is a file anyone could edit, so the plan is refused unless it
/// is a privacy journal whose every step only re-enables a package. Packages
/// that are enabled already (re-enabled since) are skipped: their steps are
/// marked undone in `journal` (in memory; [`crate::restore`] saves it), and
/// the plan's notes name them.
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
    use crate::adb::FakeAdb;
    use crate::journal::tests::MemoryJournal;
    use crate::journal::{JOURNAL_SCHEMA_VERSION, JournalStatus, JournalStep};
    use crate::plan::{apply, restore};
    use crate::{PrivacyPackage, RiskClass};

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

    fn package(name: &str) -> PackageName {
        PackageName::new(name).unwrap()
    }

    /// `pm list packages -d` output listing `names`.
    fn disabled(names: &[&str]) -> String {
        names
            .iter()
            .map(|name| format!("package:{name}\n"))
            .collect()
    }

    /// The bundled Philips profile, with its privacy profile marked tested.
    fn tested() -> Vec<DeviceProfile> {
        let mut profiles = crate::bundled_profiles();
        for profile in &mut profiles {
            if let Some(privacy) = &mut profile.privacy {
                privacy.tested = true;
            }
        }
        profiles
    }

    /// Like [`tested`], with `extra` appended to the Philips privacy profile.
    fn tested_with(extra: &str) -> Vec<DeviceProfile> {
        let mut profiles = tested();
        for profile in &mut profiles {
            if let Some(privacy) = &mut profile.privacy {
                privacy.packages.push(PrivacyPackage {
                    package: package(extra),
                    category: PrivacyCategory::Extras,
                    what: "test".to_owned(),
                    side_effects: "none".to_owned(),
                    checked: String::new(),
                });
            }
        }
        profiles
    }

    /// The Philips TV with every profile package installed and enabled,
    /// RustTV installed, no accessibility service on. `disabled` is the
    /// output of `pm list packages -d` on each call.
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
            .respond_sequence(DeviceCommand::ListDisabledPackages, disabled)
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
    fn no_bundled_privacy_profile_lists_a_statically_protected_package() {
        for profile in crate::bundled_profiles() {
            for entry in profile.privacy.iter().flat_map(|privacy| &privacy.packages) {
                assert_eq!(
                    statically_protected(entry.package.as_str()),
                    None,
                    "{}",
                    entry.package
                );
            }
        }
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
    fn plan_disables_each_category_in_profile_order() {
        let telemetry = [
            CANDEEBUG,
            GEL,
            "com.google.android.partnersetup",
            "com.google.android.feedback",
            "com.android.tv.feedbackconsent",
        ];
        let ads = [ADSERVICES, ODP, FEDERATED];
        let extras = [
            "com.google.android.play.games",
            "com.google.android.syncadapters.calendar",
            PRINTSPOOLER,
            "com.google.android.onetimeinitializer",
        ];
        use PrivacyCategory::{Ads, Extras, Telemetry};
        let all: Vec<&str> = telemetry
            .iter()
            .chain(&ads)
            .chain(&extras)
            .copied()
            .collect();
        for (categories, expected) in [
            (vec![], all.clone()),
            (vec![Telemetry, Ads, Extras], all.clone()),
            (vec![Telemetry], telemetry.to_vec()),
            (vec![Ads], ads.to_vec()),
            (vec![Extras], extras.to_vec()),
            // Profile order, whatever order the categories are given in.
            (
                vec![Ads, Telemetry],
                telemetry.iter().chain(&ads).copied().collect(),
            ),
        ] {
            let adb = tv(vec![disabled(&[])]);
            let plan = privacy_plan(&adb, SERIAL, &tested(), &categories).unwrap();
            assert_eq!(planned(&plan), expected, "{categories:?}");
            assert!(changes(&adb).is_empty(), "planning is read-only");
        }
    }

    #[test]
    fn dry_run_shows_what_and_side_effects_and_checks_home_after_the_last_step() {
        let adb = tv(vec![disabled(&[])]);
        let plan = privacy_plan(&adb, SERIAL, &tested(), &[PrivacyCategory::Ads]).unwrap();
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
        let ads_note = dry_run
            .notes
            .iter()
            .find(|note| note.starts_with(ADSERVICES))
            .unwrap();
        assert!(ads_note.contains("(ads)"), "{ads_note}");
        assert!(
            ads_note.contains("told they are unavailable"),
            "side effects shown: {ads_note}"
        );
    }

    #[test]
    fn an_untested_or_missing_privacy_profile_is_refused() {
        // The bundled Philips profile isn't tested yet.
        let error = privacy_plan(
            &tv(vec![disabled(&[])]),
            SERIAL,
            &crate::bundled_profiles(),
            &[],
        )
        .unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        assert!(error.to_string().contains("tested = true"), "{error}");

        // A profile without `[privacy]`.
        let mut profiles = tested();
        for profile in &mut profiles {
            profile.privacy = None;
        }
        let error = privacy_plan(&tv(vec![disabled(&[])]), SERIAL, &profiles, &[]).unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        assert!(error.to_string().contains("`[privacy]`"), "{error}");

        // No profile for the device at all.
        let adb = tv(vec![disabled(&[])]).respond(
            DeviceCommand::GetProperties,
            include_str!("fixtures/synthetic-home-takeover/getprop.txt"),
        );
        let error = privacy_plan(&adb, SERIAL, &tested(), &[]).unwrap_err();
        assert_eq!(error.code(), "PROFILE_NOT_TESTED");
        assert!(
            error.to_string().contains("no compatibility profile"),
            "{error}"
        );
    }

    #[test]
    fn a_protected_package_in_the_profile_refuses_the_whole_plan() {
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
            let error = privacy_plan(&adb, SERIAL, &tested_with(extra), &[]).unwrap_err();
            assert_eq!(error.code(), "PACKAGE_PROTECTED", "{extra}: {error}");
            let message = error.to_string();
            assert!(
                message.contains(extra) && message.contains(why),
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
        let error = privacy_plan(&adb, SERIAL, &tested(), &[]).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("input methods"), "{error}");
    }

    #[test]
    fn absent_and_already_disabled_packages_are_skipped_and_named() {
        let adb = tv(vec![disabled(&[GEL])]).respond(
            DeviceCommand::ListPackages,
            synthetic!("packages-f.txt").replace(PRINTSPOOLER, "com.example.unrelated"),
        );
        let plan = privacy_plan(&adb, SERIAL, &tested(), &[]).unwrap();
        let steps = planned(&plan);
        assert_eq!(steps.len(), 10);
        assert!(!steps.iter().any(|step| step == GEL || step == PRINTSPOOLER));
        let notes = plan.dry_run().notes;
        assert!(
            notes.contains(&format!("Skipped {GEL}: already disabled.")),
            "{notes:?}"
        );
        assert!(
            notes.contains(&format!("Skipped {PRINTSPOOLER}: not installed.")),
            "{notes:?}"
        );

        // Nothing left to disable.
        let adb = tv(vec![disabled(&[ADSERVICES, ODP, FEDERATED])]);
        let error = privacy_plan(&adb, SERIAL, &tested(), &[PrivacyCategory::Ads]).unwrap_err();
        assert_eq!(error.code(), "PRECONDITION_FAILED");
        assert!(error.to_string().contains("nothing to do"), "{error}");
    }

    #[test]
    fn apply_then_restore_skips_a_package_re_enabled_since() {
        let ads = [ADSERVICES, ODP, FEDERATED];
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
                disabled(&ads),
                // Restore planning: the person re-enabled ODP in Settings.
                disabled(&[ADSERVICES, FEDERATED]),
                // After each re-enable.
                disabled(&[ADSERVICES]),
                disabled(&[]),
            ]),
            &ads,
        );
        let plan = privacy_plan(&adb, SERIAL, &tested(), &[PrivacyCategory::Ads]).unwrap();
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
        assert!(
            plan.dry_run()
                .notes
                .iter()
                .any(|note| note.starts_with(&format!("Skipped {ODP}: it is enabled now"))),
            "{:?}",
            plan.dry_run().notes
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
    fn home_changing_after_the_last_step_rolls_everything_back() {
        let ads = [ADSERVICES, ODP, FEDERATED];
        let adb = with_toggles(
            tv(vec![
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[]),
                disabled(&[ADSERVICES]),
                disabled(&[ADSERVICES, ODP]),
                disabled(&ads),
                // After each undo, last first.
                disabled(&[ADSERVICES, ODP]),
                disabled(&[ADSERVICES]),
                disabled(&[]),
            ]),
            &ads,
        )
        // Home opens the stock launcher when planning, then setupwraith.
        .respond_sequence(
            DeviceCommand::HomeActivity,
            [
                philips!("home-activity.txt"),
                "com.google.android.tungsten.setupwraith/.RecoveryActivity\n".to_owned(),
            ],
        );
        let plan = privacy_plan(&adb, SERIAL, &tested(), &[PrivacyCategory::Ads]).unwrap();
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
        let profiles = {
            let mut profiles = tested();
            for profile in &mut profiles {
                if let Some(privacy) = &mut profile.privacy {
                    privacy
                        .packages
                        .retain(|entry| entry.package.as_str() == PRINTSPOOLER);
                }
            }
            profiles
        };
        let plan = privacy_plan(&adb, SERIAL, &profiles, &[]).unwrap();
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
    }

    #[test]
    fn analyze_reports_state_and_protection_of_every_package() {
        let adb = tv(vec![disabled(&[GEL])]).respond(
            DeviceCommand::ListPackages,
            synthetic!("packages-f.txt").replace(PRINTSPOOLER, "com.example.unrelated"),
        );
        let analysis = optimize_analyze(&adb, SERIAL, &crate::bundled_profiles()).unwrap();
        assert_eq!(analysis.profile.as_deref(), Some("tpv-ph1m-ww-9972.toml"));
        let privacy = analysis.privacy.unwrap();
        assert!(!privacy.tested);
        assert_eq!(privacy.packages.len(), 12);
        let state = |name: &str| {
            privacy
                .packages
                .iter()
                .find(|entry| entry.package == name)
                .unwrap()
                .state
        };
        assert_eq!(state(CANDEEBUG), PackageState::Enabled);
        assert_eq!(state(GEL), PackageState::Disabled);
        assert_eq!(state(PRINTSPOOLER), PackageState::Absent);
        assert!(
            privacy
                .packages
                .iter()
                .all(|entry| entry.protected.is_none())
        );
        assert!(changes(&adb).is_empty());

        // Another device: nothing to report, and nothing else is read.
        let adb = FakeAdb::new().respond(
            DeviceCommand::GetProperties,
            include_str!("fixtures/synthetic-home-takeover/getprop.txt"),
        );
        let analysis = optimize_analyze(&adb, SERIAL, &crate::bundled_profiles()).unwrap();
        assert_eq!(analysis.profile, None);
        assert_eq!(analysis.privacy, None);
    }
}

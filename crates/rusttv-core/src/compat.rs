//! Device compatibility profiles: one TOML file per device and firmware
//! family in `compat/devices/`.
//!
//! These types are the profile schema: profiles are parsed with unknown
//! fields rejected, and a test loads every committed profile. Profiles never
//! contain per-unit identifiers (serial numbers, MAC addresses, accounts);
//! the build fingerprint is the same on every unit with that firmware.

use std::collections::HashMap;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::command::PackageName;
use crate::inspect::DeviceInfo;

/// Version of the profile format. Bump it when fields change meaning.
///
/// 2: optional `[launcher]` section (Home takeover, SECURITY.md).
/// 3: optional `[privacy]` section (privacy profiles, OPTIMIZER.md).
/// 4: `[privacy]` lists only which catalog packages were tested
///    (`[[privacy.tested]]`); what they are lives in `compat/packages/`.
///    Version 3 sections still load (see [`ProfilePrivacy`]).
/// 5: optional `[[optimizer]]` entries (optimizer profiles tested on the
///    device, OPTIMIZER.md § 3). Version 3 and 4 profiles still load: they
///    have none.
pub const PROFILE_SCHEMA_VERSION: u32 = 5;

/// Every profile in `compat/devices/`, compiled into the binary.
///
/// Bundled rather than read from disk, so the Home takeover gate can't be
/// satisfied by dropping a file somewhere. A test fails if a profile in
/// `compat/devices/` is missing here.
const BUNDLED_PROFILES: &[(&str, &str)] = &[
    (
        "google-emu64a.toml",
        include_str!("../../../compat/devices/google-emu64a.toml"),
    ),
    (
        "tpv-ph1m-ww-9972.toml",
        include_str!("../../../compat/devices/tpv-ph1m-ww-9972.toml"),
    ),
];

/// A device compatibility profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfile {
    /// Always [`PROFILE_SCHEMA_VERSION`] for profiles this version writes.
    pub schema_version: u32,
    /// Who the device is.
    pub device: ProfileDevice,
    /// Which firmware was tested.
    pub firmware: ProfileFirmware,
    /// What the hardware offers.
    pub hardware: ProfileHardware,
    /// Who tested it and how.
    pub tested: ProfileTested,
    /// The stock launcher and whether Home takeover was tested. Absent means
    /// "not tested": takeover is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher: Option<ProfileLauncher>,
    /// Which catalog packages were tested on this device (privacy profiles).
    /// Absent means none: every package counts as untested here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy: Option<ProfilePrivacy>,
    /// Optimizer profiles (`compat/optimizer/`) tried on this device. Empty
    /// means none: `optimize apply <profile>` is refused here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optimizer: Vec<ProfileOptimizer>,
}

/// One optimizer profile on one device and firmware family (OPTIMIZER.md
/// § 3), like `[launcher] takeover_tested`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileOptimizer {
    /// Name of the optimizer profile, e.g. `reduced-animations`.
    pub profile: String,
    /// The optimizer profile's `version` this entry is about; an entry for
    /// another version counts for nothing.
    pub version: u32,
    /// Set only after `optimize apply` and `optimize restore` both succeeded
    /// on this device and firmware family.
    pub tested: bool,
    /// Path, from the repository root, of the committed report measuring the
    /// benefit (BENCHMARK_PLAN.md). Only entries with evidence are suggested
    /// by `optimize analyze`; a test checks the file exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// Which packages of the catalog (`compat/packages/`) were tested on one
/// device and firmware family (OPTIMIZER.md § Generic design, SECURITY.md
/// § Protected package policy rule 4).
///
/// Also reads the version 3 form (`tested = true` plus `[[privacy.packages]]`
/// with descriptions): each package with a `checked` date counts as tested
/// if `tested` was true; the descriptions are dropped (the catalog has them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "PrivacySection")]
pub struct ProfilePrivacy {
    /// The tested packages, in the order they are disabled within a category.
    pub tested: Vec<TestedPackage>,
}

/// One package tested on a device: disabled, the TV rebooted and used
/// normally (Home, HDMI inputs, apps, remote buttons, Settings), and a
/// restore brought it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestedPackage {
    /// The package; validated like every package name. Must be in the catalog.
    pub package: PackageName,
    /// Date it was tested on this model, `YYYY-MM-DD`.
    pub checked: String,
}

/// The `[privacy]` section as written by schema 4, or by schema 3.
#[derive(Deserialize)]
#[serde(untagged)]
enum PrivacySection {
    Current(CurrentPrivacy),
    Legacy(LegacyPrivacy),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentPrivacy {
    #[serde(default)]
    tested: Vec<TestedPackage>,
}

/// Schema 3: a profile-level `tested` flag and a description per package.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPrivacy {
    tested: bool,
    #[serde(default)]
    packages: Vec<LegacyPackage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPackage {
    package: PackageName,
    #[serde(rename = "category")]
    _category: PrivacyCategory,
    #[serde(rename = "what")]
    _what: String,
    #[serde(rename = "side_effects")]
    _side_effects: String,
    checked: String,
}

impl From<PrivacySection> for ProfilePrivacy {
    fn from(section: PrivacySection) -> Self {
        match section {
            PrivacySection::Current(current) => Self {
                tested: current.tested,
            },
            PrivacySection::Legacy(legacy) => Self {
                tested: legacy
                    .packages
                    .into_iter()
                    .filter(|package| legacy.tested && !package.checked.is_empty())
                    .map(|package| TestedPackage {
                        package: package.package,
                        checked: package.checked,
                    })
                    .collect(),
            },
        }
    }
}

/// What a package in the catalog is for. The first three are on by default;
/// the others are features some people use, planned only when chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyCategory {
    /// Collects usage data or crash reports and sends them off the TV.
    Telemetry,
    /// Serves or personalises ads.
    Ads,
    /// Runs for nothing on a TV (demo modes, manuals, printing, ...).
    Extras,
    /// Casting and screen mirroring (Chromecast, AirPlay, Miracast, DLNA). Opt-in.
    Casting,
    /// Control by home-automation systems and phone apps. Opt-in.
    SmartHome,
    /// Voice assistants other than Google's own. Opt-in.
    Voice,
    /// Preinstalled streaming apps. Opt-in.
    StreamingApps,
}

impl PrivacyCategory {
    /// Every category, in plan order.
    pub const ALL: [Self; 7] = [
        Self::Telemetry,
        Self::Ads,
        Self::Extras,
        Self::Casting,
        Self::SmartHome,
        Self::Voice,
        Self::StreamingApps,
    ];

    /// The categories chosen when nobody chose: nobody needs these.
    pub const DEFAULT: [Self; 3] = [Self::Telemetry, Self::Ads, Self::Extras];

    /// Whether this category is on unless a person opts out.
    pub fn is_default(self) -> bool {
        Self::DEFAULT.contains(&self)
    }

    /// The name used in the catalog, choices files and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telemetry => "telemetry",
            Self::Ads => "ads",
            Self::Extras => "extras",
            Self::Casting => "casting",
            Self::SmartHome => "smart-home",
            Self::Voice => "voice",
            Self::StreamingApps => "streaming-apps",
        }
    }
}

impl fmt::Display for PrivacyCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PrivacyCategory {
    type Err = InvalidPrivacyCategory;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == name)
            .ok_or_else(|| InvalidPrivacyCategory(name.to_owned()))
    }
}

/// Returned when parsing a [`PrivacyCategory`] fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not a privacy category; expected `telemetry`, `ads`, `extras`, `casting`, \
     `smart-home`, `voice` or `streaming-apps`"
)]
pub struct InvalidPrivacyCategory(String);

/// Home takeover facts for one device and firmware family (SECURITY.md
/// § Home takeover, rule 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileLauncher {
    /// Package of the stock launcher, e.g. `com.google.android.apps.tv.launcherx`.
    pub stock_package: String,
    /// Other system packages with a Home activity of higher priority than
    /// RustTV, which the takeover disables too, e.g.
    /// `com.google.android.tungsten.setupwraith`: with the stock launcher
    /// gone, Android would open the highest of them instead of RustTV. (The
    /// adb shell may not disable a single activity of a system app.)
    #[serde(default)]
    pub disable_home_packages: Vec<String>,
    /// Set only after `rusttv launcher takeover` and `rusttv launcher restore`
    /// both succeeded on this device and firmware family.
    pub takeover_tested: bool,
}

impl DeviceProfile {
    /// Whether this profile describes the device with these system properties
    /// (`getprop`): same manufacturer, model and codename, and the same
    /// firmware family (see [`firmware_family`]).
    pub fn matches(&self, properties: &HashMap<String, String>) -> bool {
        let property = |name: &str| properties.get(name).map(String::as_str);
        let family = |fingerprint: Option<&str>| fingerprint.and_then(firmware_family);
        property("ro.product.manufacturer") == Some(self.device.manufacturer.as_str())
            && property("ro.product.model") == Some(self.device.model.as_str())
            && property("ro.product.device") == Some(self.device.codename.as_str())
            && family(property("ro.build.fingerprint")).is_some()
            && family(property("ro.build.fingerprint"))
                == family(Some(&self.firmware.build_fingerprint))
    }
}

/// The firmware family of a build fingerprint: everything except the build
/// ID and incremental, so security updates of one Android version stay in
/// the family and a new Android version doesn't.
///
/// `brand/product/device:release/id/incremental:type/tags` →
/// `brand/product/device:release:type/tags`. `None` if malformed.
pub fn firmware_family(fingerprint: &str) -> Option<String> {
    let mut parts = fingerprint.split(':');
    let (name, version, build_type) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || name.is_empty() || build_type.is_empty() {
        return None;
    }
    let release = version
        .split('/')
        .next()
        .filter(|release| !release.is_empty())?;
    Some(format!("{name}:{release}:{build_type}"))
}

/// The bundled profiles from `compat/devices/`.
pub fn bundled_profiles() -> Vec<DeviceProfile> {
    // Every bundled profile parses: `bundled_profiles_are_the_committed_profiles`
    // checks that.
    BUNDLED_PROFILES
        .iter()
        .filter_map(|(_, text)| toml::from_str(text).ok())
        .collect()
}

/// Identity of the device model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileDevice {
    /// `ro.product.manufacturer`.
    pub manufacturer: String,
    /// `ro.product.brand`.
    pub brand: String,
    /// `ro.product.model`.
    pub model: String,
    /// `ro.product.device`, the hardware codename.
    pub codename: String,
    /// What kind of device this is.
    pub kind: DeviceKind,
}

/// Kind of device, from `ro.build.characteristics`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// An Android TV / Google TV device (`tv`).
    Tv,
    /// An Android emulator (`emulator`).
    Emulator,
    /// Anything else, e.g. a phone.
    Other,
}

impl DeviceKind {
    /// Classifies `ro.build.characteristics`, a comma-separated list.
    pub fn from_characteristics(characteristics: &str) -> Self {
        let has = |wanted: &str| {
            characteristics
                .split(',')
                .any(|value| value.trim() == wanted)
        };
        if has("tv") {
            Self::Tv
        } else if has("emulator") {
            Self::Emulator
        } else {
            Self::Other
        }
    }
}

/// Firmware the profile applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileFirmware {
    /// `ro.build.version.release`.
    pub android_version: String,
    /// `ro.build.version.sdk`.
    pub api_level: u32,
    /// `ro.build.version.security_patch`.
    pub security_patch: String,
    /// `ro.build.fingerprint`.
    pub build_fingerprint: String,
}

/// Hardware characteristics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileHardware {
    /// Supported ABIs in preference order.
    pub abis: Vec<String>,
    /// `MemTotal`, rounded down to MiB.
    pub memory_total_mib: u64,
    /// Size of `/data`, rounded down to MiB.
    pub data_storage_total_mib: u64,
}

/// Test provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileTested {
    /// RustTV version that generated the profile.
    pub rusttv_version: String,
    /// Free-form notes from the contributor: what was tested, quirks seen.
    pub notes: String,
}

/// Builds a profile from an inspection. Fails with the name of the first
/// missing field: a profile must be complete to be useful.
pub fn profile_from_info(info: &DeviceInfo) -> Result<DeviceProfile, MissingField> {
    fn required<T: Clone>(value: &Option<T>, name: &'static str) -> Result<T, MissingField> {
        value.clone().ok_or(MissingField(name))
    }
    let memory = info.memory.ok_or(MissingField("memory"))?;
    let storage = info.data_storage.ok_or(MissingField("data_storage"))?;
    if info.abis.is_empty() {
        return Err(MissingField("abis"));
    }
    let characteristics = required(&info.build_characteristics, "build_characteristics")?;

    Ok(DeviceProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        device: ProfileDevice {
            manufacturer: required(&info.manufacturer, "manufacturer")?,
            brand: required(&info.brand, "brand")?,
            model: required(&info.model, "model")?,
            codename: required(&info.device, "device")?,
            kind: DeviceKind::from_characteristics(&characteristics),
        },
        firmware: ProfileFirmware {
            android_version: required(&info.android_version, "android_version")?,
            api_level: required(&info.api_level, "api_level")?,
            security_patch: required(&info.security_patch, "security_patch")?,
            build_fingerprint: required(&info.build_fingerprint, "build_fingerprint")?,
        },
        hardware: ProfileHardware {
            abis: info.abis.clone(),
            memory_total_mib: memory.total_kib / 1024,
            data_storage_total_mib: storage.total_kib / 1024,
        },
        tested: ProfileTested {
            rusttv_version: crate::VERSION.to_owned(),
            notes: String::new(),
        },
        launcher: None,
        privacy: None,
        optimizer: Vec::new(),
    })
}

/// A device didn't report something a profile needs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the device did not report `{0}`, which a compatibility profile needs")]
pub struct MissingField(&'static str);

/// Suggested file name for a profile: `<manufacturer>-<codename>.toml`,
/// lowercase, with anything but ASCII letters and digits replaced by `-`.
pub fn profile_file_name(profile: &DeviceProfile) -> String {
    let slug = |text: &str| {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
    };
    format!(
        "{}-{}.toml",
        slug(&profile.device.manufacturer),
        slug(&profile.device.codename)
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::adb::FakeAdb;
    use crate::command::DeviceCommand;
    use crate::inspect::inspect;

    fn emulator_info() -> DeviceInfo {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                include_str!("fixtures/emulator-pixel3a-api34/getprop.txt"),
            )
            .respond(
                DeviceCommand::MemInfo,
                include_str!("fixtures/emulator-pixel3a-api34/meminfo.txt"),
            )
            .respond(
                DeviceCommand::DataStorage,
                include_str!("fixtures/emulator-pixel3a-api34/df-data.txt"),
            );
        inspect(&adb, "emulator-5554").unwrap()
    }

    fn compat_devices_dir() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../compat/devices"))
    }

    #[test]
    fn every_committed_profile_is_valid() {
        let mut checked = 0;
        for entry in fs::read_dir(compat_devices_dir()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            let profile: DeviceProfile =
                toml::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));

            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(profile.schema_version, PROFILE_SCHEMA_VERSION, "{name}");
            assert_eq!(
                profile_file_name(&profile),
                name,
                "file name must match the profile"
            );
            assert!(!profile.hardware.abis.is_empty(), "{name}: no ABIs");
            checked += 1;
        }
        assert!(checked > 0, "no profiles found in compat/devices");
    }

    #[test]
    fn committed_emulator_profile_matches_the_generator() {
        // `tested` records provenance (hand-written notes, the RustTV version
        // that generated the file), so compare everything else.
        let mut generated = profile_from_info(&emulator_info()).unwrap();
        let path = compat_devices_dir().join(profile_file_name(&generated));
        let committed: DeviceProfile = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        generated.tested.clone_from(&committed.tested);
        assert_eq!(generated, committed);
    }

    fn philips_info() -> DeviceInfo {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                include_str!("fixtures/philips-65oled818-api34/getprop.txt"),
            )
            .respond(
                DeviceCommand::MemInfo,
                include_str!("fixtures/philips-65oled818-api34/meminfo.txt"),
            )
            .respond(
                DeviceCommand::DataStorage,
                include_str!("fixtures/philips-65oled818-api34/df-data.txt"),
            );
        inspect(&adb, "tv").unwrap()
    }

    #[test]
    fn committed_philips_profile_matches_the_generator() {
        let mut generated = profile_from_info(&philips_info()).unwrap();
        let path = compat_devices_dir().join(profile_file_name(&generated));
        let committed: DeviceProfile = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        generated.tested.clone_from(&committed.tested);
        // Written by hand from what was seen on the device, like `tested`.
        generated.launcher.clone_from(&committed.launcher);
        generated.privacy.clone_from(&committed.privacy);
        generated.optimizer.clone_from(&committed.optimizer);
        assert_eq!(generated, committed);
    }

    #[test]
    fn optimizer_entries_name_bundled_profiles_and_existing_evidence() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let optimizer = crate::optimizer::bundled_optimizer_profiles();
        for profile in bundled_profiles() {
            for entry in &profile.optimizer {
                assert!(
                    optimizer.iter().any(|known| known.name == entry.profile),
                    "{}: unknown optimizer profile {}",
                    profile.device.model,
                    entry.profile
                );
                if let Some(evidence) = &entry.evidence {
                    assert!(
                        repo.join(evidence).is_file(),
                        "{}: evidence {evidence} is not a committed file",
                        profile.device.model
                    );
                }
            }
        }
    }

    #[test]
    fn optimizer_entries_parse_and_older_profiles_still_load() {
        let entry = "[[optimizer]]\nprofile = \"reduced-animations\"\nversion = 1\ntested = true\n";
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            optimizer: Vec<ProfileOptimizer>,
        }
        let parsed: Wrapper = toml::from_str(entry).unwrap();
        assert_eq!(parsed.optimizer[0].evidence, None);
        assert!(toml::from_str::<Wrapper>(&format!("{entry}measured = true\n")).is_err());

        // A schema 4 profile (no `[[optimizer]]`) loads with none.
        let mut text = toml::to_string(&profile_from_info(&emulator_info()).unwrap()).unwrap();
        text = text.replace("schema_version = 5", "schema_version = 4");
        let profile: DeviceProfile = toml::from_str(&text).unwrap();
        assert_eq!(profile.schema_version, 4);
        assert!(profile.optimizer.is_empty());
    }

    #[test]
    fn the_philips_profile_records_reduced_animations_as_tested() {
        // Measured A–B–A on the owner's TV on 2026-10-23.
        let properties = crate::inspect::parse_properties(include_str!(
            "fixtures/philips-65oled818-api34/getprop.txt"
        ));
        let profile = bundled_profiles()
            .into_iter()
            .find(|profile| profile.matches(&properties))
            .unwrap();
        assert!(
            profile
                .optimizer
                .iter()
                .any(|entry| entry.profile == "reduced-animations"
                    && entry.version == 1
                    && entry.tested
                    && entry.evidence.is_some())
        );
    }

    #[test]
    fn the_philips_profile_matches_its_capture_and_records_a_tested_takeover() {
        let properties = crate::inspect::parse_properties(include_str!(
            "fixtures/philips-65oled818-api34/getprop.txt"
        ));
        let matching: Vec<String> = bundled_profiles()
            .iter()
            .filter(|profile| profile.matches(&properties))
            .map(profile_file_name)
            .collect();
        assert_eq!(matching, ["tpv-ph1m-ww-9972.toml"]);
        let profile = bundled_profiles()
            .into_iter()
            .find(|profile| profile.matches(&properties))
            .unwrap();
        // Takeover and restore were tested on it on 2026-10-23, with
        // setupwraith disabled too (its Home activity outranks RustTV).
        let launcher = profile.launcher.unwrap();
        assert!(launcher.takeover_tested);
        assert_eq!(
            launcher.disable_home_packages,
            ["com.google.android.tungsten.setupwraith"]
        );
    }

    #[test]
    fn the_philips_privacy_profile_lists_its_tested_packages() {
        let properties = crate::inspect::parse_properties(include_str!(
            "fixtures/philips-65oled818-api34/getprop.txt"
        ));
        let privacy = bundled_profiles()
            .into_iter()
            .find(|profile| profile.matches(&properties))
            .and_then(|profile| profile.privacy)
            .unwrap();
        // Applied, used, rebooted and restored on the owner's TV on 2026-10-23.
        let tested: Vec<&str> = privacy
            .tested
            .iter()
            .map(|entry| entry.package.as_str())
            .collect();
        assert_eq!(
            tested,
            [
                "org.droidtv.candeebug",
                "org.droidtv.gel",
                "com.google.android.partnersetup",
                "com.google.android.feedback",
                "com.android.tv.feedbackconsent",
                "com.android.adservices.api",
                "com.android.ondevicepersonalization.services",
                "com.android.federatedcompute.services",
                "com.google.android.play.games",
                "com.google.android.syncadapters.calendar",
                "com.android.printspooler",
                "com.google.android.onetimeinitializer",
                // Second round, the same day.
                "org.droidtv.tv.frameworklogger",
                "org.droidtv.customautologservice",
                "tv.samba.ssm",
                "android.autoinstalls.config.google.gtvpai",
                "com.android.cts.ctsshim",
                "com.android.cts.priv.ctsshim",
                "org.droidtv.demome",
                "org.droidtv.control4app",
                "org.droidtv.control4UcService",
                "org.droidtv.amazonalexa",
            ]
        );
        assert!(
            privacy
                .tested
                .iter()
                .all(|entry| entry.checked == "2026-10-23")
        );
    }

    #[test]
    fn tested_entries_reject_bad_packages_and_unknown_fields() {
        let entry = |package: &str| {
            format!("[[tested]]\npackage = \"{package}\"\nchecked = \"2026-10-23\"\n")
        };
        assert!(toml::from_str::<ProfilePrivacy>(&entry("com.example")).is_ok());
        assert!(toml::from_str::<ProfilePrivacy>(&entry("com.x; reboot")).is_err());
        let mut text = entry("com.example");
        text.push_str("category = \"ads\"\n");
        assert!(toml::from_str::<ProfilePrivacy>(&text).is_err());
        assert_eq!("extras".parse(), Ok(PrivacyCategory::Extras));
        assert_eq!("smart-home".parse(), Ok(PrivacyCategory::SmartHome));
        assert!("Ads".parse::<PrivacyCategory>().is_err());
        assert!("smart_home".parse::<PrivacyCategory>().is_err());
    }

    #[test]
    fn a_schema_3_privacy_section_still_loads() {
        let legacy = |tested: bool| {
            format!(
                "tested = {tested}\n\
                 [[packages]]\npackage = \"com.example.a\"\ncategory = \"ads\"\n\
                 what = \"x\"\nside_effects = \"x\"\nchecked = \"2026-10-23\"\n\
                 [[packages]]\npackage = \"com.example.b\"\ncategory = \"extras\"\n\
                 what = \"x\"\nside_effects = \"x\"\nchecked = \"\"\n"
            )
        };
        let privacy: ProfilePrivacy = toml::from_str(&legacy(true)).unwrap();
        assert_eq!(
            privacy.tested,
            [TestedPackage {
                package: PackageName::new("com.example.a").unwrap(),
                checked: "2026-10-23".to_owned(),
            }]
        );
        let privacy: ProfilePrivacy = toml::from_str(&legacy(false)).unwrap();
        assert!(privacy.tested.is_empty());
        let bad = legacy(true).replace("\"ads\"", "\"bloat\"");
        assert!(toml::from_str::<ProfilePrivacy>(&bad).is_err());
    }

    #[test]
    fn bundled_profiles_are_the_committed_profiles() {
        let mut committed: Vec<String> = fs::read_dir(compat_devices_dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".toml"))
            .collect();
        committed.sort();
        let mut bundled: Vec<String> = BUNDLED_PROFILES
            .iter()
            .map(|(name, _)| (*name).to_owned())
            .collect();
        bundled.sort();
        assert_eq!(bundled, committed, "update BUNDLED_PROFILES");
        assert_eq!(bundled_profiles().len(), BUNDLED_PROFILES.len());
    }

    #[test]
    fn emulator_profiles_are_not_takeover_tested() {
        // Owner decision: a takeover tested on an emulator says nothing about a TV.
        for profile in bundled_profiles() {
            if profile.device.kind == DeviceKind::Emulator
                || profile.device.model.starts_with("sdk_")
            {
                assert!(
                    !profile
                        .launcher
                        .is_some_and(|launcher| launcher.takeover_tested),
                    "{}",
                    profile.device.model
                );
            }
        }
    }

    #[test]
    fn firmware_family_drops_only_build_id_and_incremental() {
        let family = |fingerprint| firmware_family(fingerprint).unwrap();
        let base = family("google/tv/x:14/UTT3.240625.001/12345678:user/release-keys");
        assert_eq!(base, "google/tv/x:14:user/release-keys");
        assert_eq!(
            family("google/tv/x:14/UTT3.250101.002/99999999:user/release-keys"),
            base,
            "a security update stays in the family"
        );
        assert_ne!(family("google/tv/x:15/A/1:user/release-keys"), base);
        assert_ne!(family("google/tv/x:14/A/1:userdebug/dev-keys"), base);
        for bad in [
            "",
            "no-colons",
            "a:b",
            "a:b:c:d",
            ":14/A/1:user/k",
            "a:/A/1:user/k",
        ] {
            assert_eq!(firmware_family(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn profiles_match_by_model_and_firmware_family() {
        let profile = profile_from_info(&emulator_info()).unwrap();
        let mut properties: HashMap<String, String> = [
            ("ro.product.manufacturer", "Google"),
            ("ro.product.model", "sdk_gphone64_arm64"),
            ("ro.product.device", "emu64a"),
            (
                "ro.build.fingerprint",
                "google/sdk_gphone64_arm64/emu64a:14/UE1A.240101.001/1:userdebug/dev-keys",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert!(profile.matches(&properties));

        properties.insert("ro.product.model".to_owned(), "other".to_owned());
        assert!(!profile.matches(&properties));
        properties.insert(
            "ro.product.model".to_owned(),
            "sdk_gphone64_arm64".to_owned(),
        );
        properties.insert(
            "ro.build.fingerprint".to_owned(),
            "google/sdk_gphone64_arm64/emu64a:15/A/1:userdebug/dev-keys".to_owned(),
        );
        assert!(!profile.matches(&properties));
        properties.remove("ro.build.fingerprint");
        assert!(!profile.matches(&properties));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let mut text = toml::to_string(&profile_from_info(&emulator_info()).unwrap()).unwrap();
        text.push_str("\n[device_extra]\nserial = \"ABC123\"\n");
        assert!(toml::from_str::<DeviceProfile>(&text).is_err());
    }

    #[test]
    fn incomplete_inspection_cannot_become_a_profile() {
        let mut info = emulator_info();
        info.build_fingerprint = None;
        assert_eq!(
            profile_from_info(&info),
            Err(MissingField("build_fingerprint"))
        );
    }

    #[test]
    fn device_kind_from_characteristics() {
        assert_eq!(DeviceKind::from_characteristics("tv"), DeviceKind::Tv);
        assert_eq!(
            DeviceKind::from_characteristics("nosdcard,tv"),
            DeviceKind::Tv
        );
        assert_eq!(
            DeviceKind::from_characteristics("emulator"),
            DeviceKind::Emulator
        );
        assert_eq!(
            DeviceKind::from_characteristics("nosdcard"),
            DeviceKind::Other
        );
        assert_eq!(DeviceKind::from_characteristics("tvish"), DeviceKind::Other);
    }
}

#[cfg(test)]
mod odd_input {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::adb_output;

    const PART: &str = "[A-Za-z0-9_.-]{1,12}";

    proptest! {
        #[test]
        fn firmware_family_never_panics(fingerprint in adb_output()) {
            if let Some(family) = firmware_family(&fingerprint) {
                prop_assert_eq!(family.matches(':').count(), 2);
            }
        }

        #[test]
        fn family_ignores_build_id_and_incremental(
            (brand, product, device, release) in (PART, PART, PART, PART),
            (id_a, incremental_a, id_b, incremental_b) in (PART, PART, PART, PART),
            build_type in "(user|userdebug|eng)/(release-keys|dev-keys|test-keys)",
        ) {
            let fingerprint = |id: &str, incremental: &str| {
                format!("{brand}/{product}/{device}:{release}/{id}/{incremental}:{build_type}")
            };
            let family = firmware_family(&fingerprint(&id_a, &incremental_a));
            prop_assert_eq!(family.clone(), Some(format!("{brand}/{product}/{device}:{release}:{build_type}")));
            prop_assert_eq!(firmware_family(&fingerprint(&id_b, &incremental_b)), family);
        }
    }

    #[test]
    fn malformed_fingerprints_have_no_family() {
        for fingerprint in [
            "",
            ":",
            "::",
            "a:b",
            "a:/x:",
            ":14/X/1:user/release-keys",
            "a:b:c:d",
        ] {
            assert_eq!(firmware_family(fingerprint), None, "{fingerprint}");
        }
    }
}

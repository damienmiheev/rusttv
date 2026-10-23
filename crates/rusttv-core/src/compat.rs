//! Device compatibility profiles: one TOML file per device and firmware
//! family in `compat/devices/`.
//!
//! These types are the profile schema: profiles are parsed with unknown
//! fields rejected, and a test loads every committed profile. Profiles never
//! contain per-unit identifiers (serial numbers, MAC addresses, accounts);
//! the build fingerprint is the same on every unit with that firmware.

use serde::{Deserialize, Serialize};

use crate::inspect::DeviceInfo;

/// Version of the profile format. Bump it when fields change meaning.
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

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

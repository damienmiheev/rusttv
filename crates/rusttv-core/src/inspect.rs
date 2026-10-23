//! `rusttv device inspect`: who a device is and what it has.
//!
//! Every field is optional or may be empty: vendors omit or rename
//! properties, and inspection must still work (capability-driven, not
//! brand-driven). Parsers are pure functions over captured `adb` output.

use std::collections::HashMap;

use serde::Serialize;

use crate::adb::Adb;
use crate::command::DeviceCommand;
use crate::error::AdbError;

/// Identity, build and resources of a device. Serialized as the `data` of
/// `rusttv device inspect --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    /// adb serial the information was read from.
    pub serial: String,
    /// `ro.product.manufacturer`, e.g. `Philips`.
    pub manufacturer: Option<String>,
    /// `ro.product.brand`.
    pub brand: Option<String>,
    /// `ro.product.model`.
    pub model: Option<String>,
    /// `ro.product.device`, the hardware codename.
    pub device: Option<String>,
    /// `ro.build.version.release`, e.g. `12`.
    pub android_version: Option<String>,
    /// `ro.build.version.sdk`, the API level, e.g. `31`.
    pub api_level: Option<u32>,
    /// `ro.build.version.security_patch`, e.g. `2025-05-01`.
    pub security_patch: Option<String>,
    /// `ro.build.fingerprint`; identifies the exact firmware build.
    pub build_fingerprint: Option<String>,
    /// `ro.build.characteristics`; contains `tv` on Android TV builds.
    pub build_characteristics: Option<String>,
    /// Supported ABIs in preference order, from `ro.product.cpu.abilist`.
    pub abis: Vec<String>,
    /// From `/proc/meminfo`.
    pub memory: Option<MemoryInfo>,
    /// The `/data` partition, where apps and their data live.
    pub data_storage: Option<StorageInfo>,
}

/// System memory in KiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct MemoryInfo {
    /// `MemTotal`: RAM available to the kernel.
    pub total_kib: u64,
    /// `MemAvailable`: estimate of memory available for new apps.
    pub available_kib: Option<u64>,
}

/// Partition size in KiB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StorageInfo {
    /// Partition size.
    pub total_kib: u64,
    /// Free space usable by apps.
    pub available_kib: u64,
}

/// Reads identity, build and resource information from a device.
///
/// Runs three read-only commands. A failing command is an error; missing or
/// unparsable values in successful output become `None`.
pub fn inspect(adb: &dyn Adb, serial: &str) -> Result<DeviceInfo, AdbError> {
    let properties = parse_properties(&adb.run(serial, &DeviceCommand::GetProperties)?);
    let memory = parse_meminfo(&adb.run(serial, &DeviceCommand::MemInfo)?);
    let data_storage = parse_df(&adb.run(serial, &DeviceCommand::DataStorage)?);

    let property = |key: &str| properties.get(key).filter(|v| !v.is_empty()).cloned();
    Ok(DeviceInfo {
        serial: serial.to_owned(),
        manufacturer: property("ro.product.manufacturer"),
        brand: property("ro.product.brand"),
        model: property("ro.product.model"),
        device: property("ro.product.device"),
        android_version: property("ro.build.version.release"),
        api_level: property("ro.build.version.sdk").and_then(|v| v.parse().ok()),
        security_patch: property("ro.build.version.security_patch"),
        build_fingerprint: property("ro.build.fingerprint"),
        build_characteristics: property("ro.build.characteristics"),
        abis: property("ro.product.cpu.abilist")
            .map(|list| {
                list.split(',')
                    .map(str::trim)
                    .filter(|abi| !abi.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        memory,
        data_storage,
    })
}

/// Parses `getprop` output: one `[key]: [value]` per line.
///
/// Lines that don't match (rare multi-line values) are skipped. Whitespace
/// around a line is ignored: a stray `\r` (from `\r\r\n` line endings) or
/// trailing spaces would otherwise drop the property.
pub fn parse_properties(output: &str) -> HashMap<String, String> {
    output
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once("]: [")?;
            let key = key.strip_prefix('[')?;
            let value = value.strip_suffix(']')?;
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// Parses `/proc/meminfo`, e.g. `MemTotal:        2026264 kB`.
pub fn parse_meminfo(output: &str) -> Option<MemoryInfo> {
    let field = |name: &str| {
        output.lines().find_map(|line| {
            let value = line.strip_prefix(name)?.strip_prefix(':')?;
            value.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()
        })
    };
    Some(MemoryInfo {
        total_kib: field("MemTotal")?,
        available_kib: field("MemAvailable"),
    })
}

/// Parses `df -k <path>` output from toybox (Android 6+):
///
/// ```text
/// Filesystem       1K-blocks    Used Available Use% Mounted on
/// /dev/block/dm-39   6082144 1125980   4813952  19% /data/user/0
/// ```
pub fn parse_df(output: &str) -> Option<StorageInfo> {
    // Use the last data row; counting columns from the end tolerates a
    // filesystem name containing spaces.
    let row = output
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .last()?;
    let columns: Vec<&str> = row.split_whitespace().collect();
    let [.., total, _used, available, _percent, _mount] = columns.as_slice() else {
        return None;
    };
    Some(StorageInfo {
        total_kib: total.parse().ok()?,
        available_kib: available.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;

    const GETPROP: &str = include_str!("fixtures/emulator-pixel3a-api34/getprop.txt");
    const MEMINFO: &str = include_str!("fixtures/emulator-pixel3a-api34/meminfo.txt");
    const DF_DATA: &str = include_str!("fixtures/emulator-pixel3a-api34/df-data.txt");

    fn emulator() -> FakeAdb {
        FakeAdb::new()
            .respond(DeviceCommand::GetProperties, GETPROP)
            .respond(DeviceCommand::MemInfo, MEMINFO)
            .respond(DeviceCommand::DataStorage, DF_DATA)
    }

    #[test]
    fn inspects_a_real_emulator_capture() {
        let info = inspect(&emulator(), "emulator-5554").unwrap();

        assert_eq!(info.serial, "emulator-5554");
        assert_eq!(info.manufacturer.as_deref(), Some("Google"));
        assert_eq!(info.brand.as_deref(), Some("google"));
        assert_eq!(info.model.as_deref(), Some("sdk_gphone64_arm64"));
        assert_eq!(info.device.as_deref(), Some("emu64a"));
        assert_eq!(info.android_version.as_deref(), Some("14"));
        assert_eq!(info.api_level, Some(34));
        assert_eq!(info.security_patch.as_deref(), Some("2023-09-05"));
        assert_eq!(
            info.build_fingerprint.as_deref(),
            Some(
                "google/sdk_gphone64_arm64/emu64a:14/UE1A.230829.036.A2/11596452:userdebug/dev-keys"
            )
        );
        assert_eq!(info.build_characteristics.as_deref(), Some("emulator"));
        assert_eq!(info.abis, ["arm64-v8a"]);
        assert_eq!(
            info.memory,
            Some(MemoryInfo {
                total_kib: 2_026_264,
                available_kib: Some(1_007_788),
            })
        );
        assert_eq!(
            info.data_storage,
            Some(StorageInfo {
                total_kib: 6_082_144,
                available_kib: 4_813_952,
            })
        );
    }

    #[test]
    fn inspects_the_philips_capture_as_32_bit() {
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
        let info = inspect(&adb, "tv").unwrap();
        assert_eq!(info.manufacturer.as_deref(), Some("TPV"));
        assert_eq!(info.model.as_deref(), Some("Philips Google TV TA1"));
        assert_eq!(info.device.as_deref(), Some("PH1M_WW_9972"));
        assert_eq!(info.api_level, Some(34));
        assert_eq!(info.abis, ["armeabi-v7a", "armeabi"]);
        assert_eq!(
            info.memory,
            Some(MemoryInfo {
                total_kib: 2_852_964,
                available_kib: Some(1_017_240),
            })
        );
        assert_eq!(
            info.data_storage,
            Some(StorageInfo {
                total_kib: 5_184_564,
                available_kib: 1_990_952,
            })
        );
    }

    #[test]
    fn missing_properties_become_none_instead_of_failing() {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                "[ro.product.model]: [Some TV]\n",
            )
            .respond(DeviceCommand::MemInfo, "")
            .respond(DeviceCommand::DataStorage, "");

        let info = inspect(&adb, "tv").unwrap();
        assert_eq!(info.model.as_deref(), Some("Some TV"));
        assert_eq!(info.manufacturer, None);
        assert_eq!(info.api_level, None);
        assert!(info.abis.is_empty());
        assert_eq!(info.memory, None);
        assert_eq!(info.data_storage, None);
    }

    #[test]
    fn a_failing_command_fails_the_inspection() {
        let adb = FakeAdb::new().respond(DeviceCommand::GetProperties, GETPROP);
        assert!(inspect(&adb, "emulator-5554").is_err());
    }

    #[test]
    fn multi_abi_list_keeps_preference_order() {
        let properties = "[ro.product.cpu.abilist]: [arm64-v8a,armeabi-v7a,armeabi]\n";
        let adb = FakeAdb::new()
            .respond(DeviceCommand::GetProperties, properties)
            .respond(DeviceCommand::MemInfo, MEMINFO)
            .respond(DeviceCommand::DataStorage, DF_DATA);
        let info = inspect(&adb, "tv").unwrap();
        assert_eq!(info.abis, ["arm64-v8a", "armeabi-v7a", "armeabi"]);
    }

    #[test]
    fn property_values_may_contain_brackets_and_colons() {
        let properties =
            parse_properties("[ro.build.fingerprint]: [a/b:14/X:user/release-keys]\n[x]: []\n");
        assert_eq!(
            properties.get("ro.build.fingerprint").map(String::as_str),
            Some("a/b:14/X:user/release-keys")
        );
        assert_eq!(properties.get("x").map(String::as_str), Some(""));
    }

    #[test]
    fn df_tolerates_wrapped_or_garbage_output() {
        assert_eq!(
            parse_df("Filesystem 1K-blocks Used Available Use% Mounted on\n"),
            None
        );
        assert_eq!(parse_df("df: /data: Permission denied\n"), None);
    }
}

#[cfg(test)]
mod odd_input {
    use proptest::prelude::*;

    use super::*;
    use crate::adb::FakeAdb;
    use crate::test_support::{adb_output, line_ending};

    proptest! {
        #[test]
        fn parsers_never_panic(output in adb_output()) {
            parse_properties(&output);
            parse_meminfo(&output);
            parse_df(&output);
        }

        #[test]
        fn getprop_lines_parse_back_to_their_parts(
            entries in prop::collection::hash_map("[a-z][a-z0-9._]{0,30}", "[^\r\n]{0,40}", 0..20),
            ending in line_ending(),
            trailing in " {0,3}",
        ) {
            let output: String = entries
                .iter()
                .map(|(key, value)| format!("[{key}]: [{value}]{trailing}{ending}"))
                .collect();
            prop_assert_eq!(parse_properties(&output), entries);
        }

        #[test]
        fn meminfo_parses_back_to_its_values(
            total in any::<u64>(),
            available in any::<u64>(),
            padding in " {1,12}",
            ending in line_ending(),
        ) {
            let output = format!(
                "MemTotal:{padding}{total} kB{ending}MemFree:{padding}1 kB{ending}MemAvailable:{padding}{available} kB{ending}"
            );
            prop_assert_eq!(
                parse_meminfo(&output),
                Some(MemoryInfo { total_kib: total, available_kib: Some(available) })
            );
        }

        #[test]
        fn df_parses_back_to_its_values(
            total in any::<u64>(),
            used in any::<u64>(),
            available in any::<u64>(),
            ending in line_ending(),
        ) {
            let output = format!(
                "Filesystem 1K-blocks Used Available Use% Mounted on{ending}/dev/block/dm-39 {total} {used} {available} 19% /data{ending}"
            );
            prop_assert_eq!(
                parse_df(&output),
                Some(StorageInfo { total_kib: total, available_kib: available })
            );
        }
    }

    #[test]
    fn empty_output_parses_to_nothing() {
        assert!(parse_properties("").is_empty());
        assert_eq!(parse_meminfo(""), None);
        assert_eq!(parse_df(""), None);
    }

    #[test]
    fn getprop_with_a_bare_carriage_return_keeps_the_property() {
        // `\r\r\n` (both the device pty and the host converted) or a last
        // line cut after its `\r`.
        let properties =
            parse_properties("[ro.product.model]: [TV]\r\r\n[ro.build.version.sdk]: [34]\r");
        assert_eq!(
            properties.get("ro.product.model").map(String::as_str),
            Some("TV")
        );
        assert_eq!(
            properties.get("ro.build.version.sdk").map(String::as_str),
            Some("34")
        );
    }

    #[test]
    fn getprop_with_warnings_and_a_truncated_last_line() {
        let properties =
            parse_properties("WARNING: linker: libfoo.so has text relocations\n[a]: [1]\n[b]: [2");
        assert_eq!(properties.len(), 1);
        assert_eq!(properties.get("a").map(String::as_str), Some("1"));
    }

    #[test]
    fn abi_list_with_spaces_or_a_trailing_comma_has_no_blank_entries() {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                "[ro.product.cpu.abilist]: [arm64-v8a, armeabi-v7a,]\n",
            )
            .respond(DeviceCommand::MemInfo, "")
            .respond(DeviceCommand::DataStorage, "");
        let info = inspect(&adb, "tv").unwrap();
        assert_eq!(info.abis, ["arm64-v8a", "armeabi-v7a"]);
    }

    #[test]
    fn api_level_out_of_range_is_none() {
        let adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                "[ro.build.version.sdk]: [99999999999]\n",
            )
            .respond(DeviceCommand::MemInfo, "")
            .respond(DeviceCommand::DataStorage, "");
        assert_eq!(inspect(&adb, "tv").unwrap().api_level, None);
    }

    #[test]
    fn meminfo_with_crlf_and_overflowing_or_foreign_values() {
        assert_eq!(
            parse_meminfo("MemTotal:  2026264 kB\r\nMemAvailable:  1 kB\r\n"),
            Some(MemoryInfo {
                total_kib: 2_026_264,
                available_kib: Some(1)
            })
        );
        // 2^64 kB does not fit: no value rather than a wrapped one.
        assert_eq!(parse_meminfo("MemTotal: 18446744073709551616 kB\n"), None);
        // Another unit is not silently read as kB.
        assert_eq!(parse_meminfo("MemTotal: 2026 MB\n"), None);
        assert_eq!(
            parse_meminfo("MemTotal: 10 kB\nMemAvailable: lots\n"),
            Some(MemoryInfo {
                total_kib: 10,
                available_kib: None
            })
        );
    }

    #[test]
    fn df_with_warnings_crlf_and_extra_lines() {
        let output = "WARNING: something\r\nFilesystem 1K-blocks Used Available Use% Mounted on\r\n/dev/block/dm-39   6082144 1125980   4813952  19% /data   \r\n\r\n";
        assert_eq!(
            parse_df(output),
            Some(StorageInfo {
                total_kib: 6_082_144,
                available_kib: 4_813_952
            })
        );
    }

    #[test]
    fn df_truncated_row_or_overflow_is_none() {
        let header = "Filesystem 1K-blocks Used Available Use% Mounted on\n";
        assert_eq!(
            parse_df(&format!("{header}/dev/block/dm-39 6082144 1125980 48")),
            None
        );
        assert_eq!(
            parse_df(&format!(
                "{header}/dev/x 18446744073709551616 1 1 1% /data\n"
            )),
            None
        );
    }

    #[test]
    fn df_wrapped_filesystem_name_reads_the_value_line() {
        // busybox puts a long filesystem name on its own line.
        let output = "Filesystem 1K-blocks Used Available Use% Mounted on\n/dev/block/platform/soc/very-long-name\n  100 40 60 40% /data\n";
        assert_eq!(
            parse_df(output),
            Some(StorageInfo {
                total_kib: 100,
                available_kib: 60
            })
        );
    }
}

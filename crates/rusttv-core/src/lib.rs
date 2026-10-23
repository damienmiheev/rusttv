//! Shared core for RustTV.
//!
//! All device access goes through this crate: build a [`DeviceCommand`] and
//! run it with an [`Adb`] implementation. See ARCHITECTURE.md for the modules
//! still to come (`device`, `packages`, `perf`, ...).

mod adb;
mod command;
mod device;
mod error;
mod inspect;

#[cfg(any(test, feature = "test-util"))]
pub use adb::FakeAdb;
pub use adb::{Adb, SystemAdb};
pub use command::{DeviceCommand, InvalidPackageName, PackageName, RiskClass};
pub use device::{
    ConnectOutcome, DEFAULT_ADB_PORT, DeviceAddress, DeviceEntry, DeviceSelectionError,
    DeviceState, InvalidDeviceAddress, parse_connect_output, parse_device_list, select_device,
};
pub use error::AdbError;
pub use inspect::{
    DeviceInfo, MemoryInfo, StorageInfo, inspect, parse_df, parse_meminfo, parse_properties,
};

/// Version of the core crate, shown by `rusttv --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

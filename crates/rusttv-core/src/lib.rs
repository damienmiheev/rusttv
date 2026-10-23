//! Shared core for RustTV.
//!
//! All device access goes through this crate: build a [`DeviceCommand`] and
//! run it with an [`Adb`] implementation. See ARCHITECTURE.md for the modules
//! still to come (`device`, `packages`, `perf`, ...).

mod adb;
mod command;
mod error;

#[cfg(any(test, feature = "test-util"))]
pub use adb::FakeAdb;
pub use adb::{Adb, SystemAdb};
pub use command::{DeviceCommand, InvalidPackageName, PackageName, RiskClass};
pub use error::AdbError;

/// Version of the core crate, shown by `rusttv --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

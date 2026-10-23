//! Shared core for RustTV.
//!
//! All device access goes through this crate. See ARCHITECTURE.md for the
//! planned modules (`adb`, `command`, `device`, ...), added in later steps.

/// Version of the core crate, shown by `rusttv --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

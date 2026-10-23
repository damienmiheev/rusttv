//! Errors from talking to a device over `adb`.

use std::io;

use crate::apps::PackageListError;

/// Every stable error code the core can produce, from [`AdbError::code`] and
/// [`crate::DeviceSelectionError::code`] and [`crate::BenchmarkError::code`] and
/// [`crate::PlanError::code`]. Documented in CLI.md; a test keeps
/// this list, the code and the docs in sync. Codes are never renamed.
pub const ERROR_CODES: &[&str] = &[
    "ADB_NOT_FOUND",
    "ADB_SPAWN_FAILED",
    "NO_DEVICE",
    "MULTIPLE_DEVICES",
    "DEVICE_NOT_FOUND",
    "DEVICE_UNAUTHORIZED",
    "DEVICE_OFFLINE",
    "DEVICE_NOT_READY",
    "CONNECT_FAILED",
    "COMMAND_FAILED",
    "INVALID_OUTPUT",
    "PACKAGE_LIST_FAILED",
    "LAUNCH_FAILED",
    "INVALID_PLAN",
    "PRECONDITION_FAILED",
    "NOT_APPROVED",
    "JOURNAL_FAILED",
    "NO_JOURNAL",
    "TAKEOVER_NOT_TESTED",
    "PROFILE_NOT_TESTED",
    "OPTIMIZE_NOT_TESTED",
    "PACKAGE_PROTECTED",
    "FEATURE_NOT_SUPPORTED",
    "INVALID_CHOICES",
    "INVALID_APK",
    "ROLLED_BACK",
    "ROLLBACK_FAILED",
    "RESTORE_FAILED",
];

/// Running a command through `adb` failed.
///
/// Follows M-ERRORS-CANONICAL-STRUCTS (rust-guidelines skill): the failure
/// kind is private; callers use [`AdbError::code`] and the `is_*` helpers.
/// We skip the recommended `Backtrace`: these errors are expected runtime
/// conditions (TV off, cable unplugged), not bugs, and are shown to users.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AdbError {
    kind: ErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum ErrorKind {
    #[error(
        "`adb` was not found; install Android platform-tools and make sure `adb` is on your PATH"
    )]
    AdbNotFound,
    #[error("could not run `adb`")]
    Spawn(#[source] io::Error),
    #[error("device `{0}` not found; check `adb devices` and the TV's network address")]
    DeviceNotFound(String),
    #[error("device `{0}` is unauthorized; accept the debugging prompt on the TV")]
    DeviceUnauthorized(String),
    #[error("device `{0}` is offline; reconnect it or restart network debugging on the TV")]
    DeviceOffline(String),
    #[error("`adb {args}` failed with {status}: {stderr}")]
    CommandFailed {
        args: String,
        status: String,
        stderr: String,
    },
    #[error("could not connect to `{address}`: {message}")]
    ConnectFailed { address: String, message: String },
    #[error("`adb {0}` printed output that is not valid UTF-8")]
    InvalidOutput(String),
    #[error(transparent)]
    PackageList(PackageListError),
}

impl From<PackageListError> for AdbError {
    fn from(error: PackageListError) -> Self {
        Self {
            kind: ErrorKind::PackageList(error),
        }
    }
}

impl AdbError {
    /// Stable machine-readable code, used in `--json` output. Never renamed.
    pub fn code(&self) -> &'static str {
        match self.kind {
            ErrorKind::AdbNotFound => "ADB_NOT_FOUND",
            ErrorKind::Spawn(_) => "ADB_SPAWN_FAILED",
            ErrorKind::DeviceNotFound(_) => "DEVICE_NOT_FOUND",
            ErrorKind::DeviceUnauthorized(_) => "DEVICE_UNAUTHORIZED",
            ErrorKind::DeviceOffline(_) => "DEVICE_OFFLINE",
            ErrorKind::CommandFailed { .. } => "COMMAND_FAILED",
            ErrorKind::ConnectFailed { .. } => "CONNECT_FAILED",
            ErrorKind::InvalidOutput(_) => "INVALID_OUTPUT",
            ErrorKind::PackageList(_) => "PACKAGE_LIST_FAILED",
        }
    }

    /// The device could not be reached (missing, unauthorized or offline).
    pub fn is_device_unreachable(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::DeviceNotFound(_)
                | ErrorKind::DeviceUnauthorized(_)
                | ErrorKind::DeviceOffline(_)
                | ErrorKind::ConnectFailed { .. }
        )
    }

    pub(crate) fn spawn(error: io::Error) -> Self {
        let kind = if error.kind() == io::ErrorKind::NotFound {
            ErrorKind::AdbNotFound
        } else {
            ErrorKind::Spawn(error)
        };
        Self { kind }
    }

    pub(crate) fn connect_failed(address: String, message: String) -> Self {
        Self {
            kind: ErrorKind::ConnectFailed { address, message },
        }
    }

    /// A host-side command (`adb devices`, ...) failed; no device is involved.
    pub(crate) fn host_failure(args: &[&str], status: String, stderr: &str) -> Self {
        Self {
            kind: ErrorKind::CommandFailed {
                args: args.join(" "),
                status,
                stderr: stderr.trim().to_owned(),
            },
        }
    }

    pub(crate) fn invalid_output(args: &[&str]) -> Self {
        Self {
            kind: ErrorKind::InvalidOutput(args.join(" ")),
        }
    }

    /// Classifies a failed `adb` run from its stderr.
    ///
    /// `status` describes how the process ended, e.g. "exit code 1".
    pub(crate) fn from_failure(serial: &str, args: &[&str], status: String, stderr: &str) -> Self {
        // Messages printed by the adb client (verified against adb 1.0.41).
        let kind = if stderr.contains("not found") && stderr.contains(serial) {
            ErrorKind::DeviceNotFound(serial.to_owned())
        } else if stderr.contains("device unauthorized") {
            ErrorKind::DeviceUnauthorized(serial.to_owned())
        } else if stderr.contains("device offline") {
            ErrorKind::DeviceOffline(serial.to_owned())
        } else {
            ErrorKind::CommandFailed {
                args: args.join(" "),
                status,
                stderr: stderr.trim().to_owned(),
            }
        };
        Self { kind }
    }
}

/// One error of every kind. The `match` stops compiling when a kind is
/// added, so new codes can't skip the `ERROR_CODES` check.
#[cfg(test)]
pub(crate) fn every_adb_error() -> Vec<AdbError> {
    let all = vec![
        AdbError::spawn(io::Error::from(io::ErrorKind::NotFound)),
        AdbError::spawn(io::Error::from(io::ErrorKind::PermissionDenied)),
        AdbError::from_failure("tv", &[], String::new(), "adb: device 'tv' not found"),
        AdbError::from_failure("tv", &[], String::new(), "adb: device unauthorized."),
        AdbError::from_failure("tv", &[], String::new(), "adb: device offline"),
        AdbError::host_failure(&["devices"], String::new(), "boom"),
        AdbError::connect_failed("tv:5555".to_owned(), "refused".to_owned()),
        AdbError::invalid_output(&["shell"]),
        AdbError::from(crate::apps::parse_package_list("Error: boom").unwrap_err()),
    ];
    for error in &all {
        match error.kind {
            ErrorKind::AdbNotFound
            | ErrorKind::Spawn(_)
            | ErrorKind::DeviceNotFound(_)
            | ErrorKind::DeviceUnauthorized(_)
            | ErrorKind::DeviceOffline(_)
            | ErrorKind::CommandFailed { .. }
            | ErrorKind::ConnectFailed { .. }
            | ErrorKind::InvalidOutput(_)
            | ErrorKind::PackageList(_) => {}
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(stderr: &str) -> AdbError {
        AdbError::from_failure(
            "192.168.1.20:5555",
            &["shell", "getprop"],
            "exit code 1".to_owned(),
            stderr,
        )
    }

    #[test]
    fn missing_device_is_unreachable() {
        // Captured from adb 1.0.41.
        let error = classify("adb: device '192.168.1.20:5555' not found\n");
        assert_eq!(error.code(), "DEVICE_NOT_FOUND");
        assert!(error.is_device_unreachable());
    }

    #[test]
    fn unauthorized_device_is_unreachable() {
        // Text from the adb client source; not yet captured from a real device.
        let error = classify(
            "adb: device unauthorized.\n\
             This adb server's $ADB_VENDOR_KEYS is not set\n\
             Try 'adb kill-server' if that seems wrong.\n\
             Otherwise check for a confirmation dialog on your device.\n",
        );
        assert_eq!(error.code(), "DEVICE_UNAUTHORIZED");
        assert!(error.is_device_unreachable());
    }

    #[test]
    fn offline_device_is_unreachable() {
        let error = classify("adb: device offline\n");
        assert_eq!(error.code(), "DEVICE_OFFLINE");
        assert!(error.is_device_unreachable());
    }

    #[test]
    fn a_failing_command_keeps_its_stderr() {
        // A device-side failure that merely mentions "not found" must not be
        // mistaken for a missing device.
        let error = classify("/system/bin/sh: getprop: not found\n");
        assert_eq!(error.code(), "COMMAND_FAILED");
        assert!(!error.is_device_unreachable());
        let message = error.to_string();
        assert!(message.contains("adb shell getprop"), "{message}");
        assert!(message.contains("getprop: not found"), "{message}");
    }

    #[test]
    fn missing_adb_binary_has_its_own_code() {
        let error = AdbError::spawn(io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(error.code(), "ADB_NOT_FOUND");
        assert!(!error.is_device_unreachable());
    }
}

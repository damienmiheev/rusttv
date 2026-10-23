//! Access to devices through the Android Debug Bridge.
//!
//! [`Adb`] is the seam that keeps the rest of the core testable
//! (M-MOCKABLE-SYSCALLS): production code uses [`SystemAdb`], tests use
//! `FakeAdb` from the `test-util` feature.

use std::ffi::OsString;
use std::process::Command;

use crate::command::DeviceCommand;
use crate::error::AdbError;

/// Runs typed commands on a device.
///
/// There is intentionally no method that takes a free-form shell string.
pub trait Adb {
    /// Runs `command` on the device with the given `adb` serial and returns its stdout.
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError>;
}

/// Runs commands with the `adb` program from Android platform-tools.
#[derive(Debug, Clone)]
pub struct SystemAdb {
    program: OsString,
}

impl SystemAdb {
    /// Uses the `adb` found on `PATH`.
    pub fn new() -> Self {
        Self::with_program("adb")
    }

    /// Uses a specific `adb` executable.
    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// Runs `command` on the device with the given `adb` serial and returns its stdout.
    pub fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
        let args = command.adb_args();
        let output = Command::new(&self.program)
            .arg("-s")
            .arg(serial)
            .args(&args)
            .output()
            .map_err(AdbError::spawn)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdbError::from_failure(
                serial,
                &args,
                output.status.to_string(),
                &stderr,
            ));
        }
        String::from_utf8(output.stdout).map_err(|_| AdbError::invalid_output(&args))
    }
}

impl Default for SystemAdb {
    fn default() -> Self {
        Self::new()
    }
}

impl Adb for SystemAdb {
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
        Self::run(self, serial, command)
    }
}

#[cfg(any(test, feature = "test-util"))]
pub use fake::FakeAdb;

#[cfg(any(test, feature = "test-util"))]
mod fake {
    use std::collections::HashMap;

    use super::Adb;
    use crate::command::DeviceCommand;
    use crate::error::AdbError;

    /// An [`Adb`] that returns canned output instead of touching a device.
    ///
    /// Commands without a canned response fail with `COMMAND_FAILED`, so a
    /// test notices when code runs a command it didn't expect.
    #[derive(Debug, Default)]
    pub struct FakeAdb {
        responses: HashMap<DeviceCommand, String>,
    }

    impl FakeAdb {
        /// Creates a fake with no canned responses.
        pub fn new() -> Self {
            Self::default()
        }

        /// Makes `command` return `stdout` on every call.
        pub fn respond(mut self, command: DeviceCommand, stdout: impl Into<String>) -> Self {
            self.responses.insert(command, stdout.into());
            self
        }

        /// Returns the canned stdout for `command`.
        pub fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
            self.responses.get(command).cloned().ok_or_else(|| {
                AdbError::from_failure(
                    serial,
                    &command.adb_args(),
                    "no canned response".to_owned(),
                    "FakeAdb has no response for this command",
                )
            })
        }
    }

    impl Adb for FakeAdb {
        fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
            Self::run(self, serial, command)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_adb_program_reports_adb_not_found() {
        let adb = SystemAdb::with_program("/nonexistent/rusttv-test/adb");
        let error = adb
            .run("emulator-5554", &DeviceCommand::GetProperties)
            .unwrap_err();
        assert_eq!(error.code(), "ADB_NOT_FOUND");
    }

    /// Code written against the trait works with any implementation.
    fn total_memory_line(adb: &dyn Adb, serial: &str) -> Result<String, AdbError> {
        let meminfo = adb.run(serial, &DeviceCommand::MemInfo)?;
        Ok(meminfo.lines().next().unwrap_or_default().to_owned())
    }

    #[test]
    fn fake_adb_serves_canned_output_through_the_trait() {
        let adb = FakeAdb::new().respond(DeviceCommand::MemInfo, "MemTotal: 1999876 kB\n");
        assert_eq!(
            total_memory_line(&adb, "emulator-5554").unwrap(),
            "MemTotal: 1999876 kB"
        );
    }

    #[test]
    fn fake_adb_rejects_unexpected_commands() {
        let adb = FakeAdb::new().respond(DeviceCommand::MemInfo, "MemTotal: 1 kB\n");
        let error = adb
            .run("emulator-5554", &DeviceCommand::ListPackages)
            .unwrap_err();
        assert_eq!(error.code(), "COMMAND_FAILED");
    }
}

//! Access to devices through the Android Debug Bridge.
//!
//! [`Adb`] is the seam that keeps the rest of the core testable
//! (M-MOCKABLE-SYSCALLS): production code uses [`SystemAdb`], tests use
//! `FakeAdb` from the `test-util` feature.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::command::DeviceCommand;
use crate::device::{self, ConnectOutcome, DeviceAddress, DeviceEntry};
use crate::error::AdbError;

/// Talks to devices through the local `adb` server.
///
/// There is intentionally no method that takes a free-form shell string.
pub trait Adb {
    /// Runs `command` on the device with the given `adb` serial and returns its stdout.
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError>;

    /// Runs `command` with `stdin` as its input and returns its stdout as
    /// bytes, for the commands that move bytes rather than text
    /// ([`DeviceCommand::is_binary`]: the history door's `exec-out` and
    /// `exec-in`). The argument list is still the command's fixed one.
    fn run_binary(
        &self,
        serial: &str,
        command: &DeviceCommand,
        stdin: &[u8],
    ) -> Result<Vec<u8>, AdbError>;

    /// Lists devices known to the local `adb` server (`adb devices -l`).
    fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError>;

    /// Connects to a device over the network (`adb connect`).
    ///
    /// Only changes state on this computer. The TV may then ask the user to
    /// authorize the computer; until they do, the device is `unauthorized`.
    fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError>;
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
        if command.stdin().is_none_or(|stdin| !stdin.is_empty()) {
            // Its input would be lost: such commands go through `run_binary`.
            return Err(AdbError::from_failure(
                serial,
                &args,
                "not run".to_owned(),
                "this command reads its input on stdin; RustTV runs it with run_binary",
            ));
        }
        let mut adb = Command::new(&self.program);
        adb.args(["-s", serial].iter().chain(&args));
        let output = match command.timeout() {
            None => adb.output().map_err(AdbError::spawn)?,
            Some(timeout) => output_within(adb, timeout)
                .map_err(AdbError::spawn)?
                .ok_or_else(|| {
                    AdbError::from_failure(
                        serial,
                        &args,
                        format!("timed out after {} s", timeout.as_secs()),
                        "",
                    )
                })?,
        };

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

    /// Runs `command` with `stdin` as its input and returns its stdout as
    /// bytes (`adb -s <serial> exec-out …` / `exec-in …`, a fixed argument
    /// list; no shell on this computer).
    pub fn run_binary(
        &self,
        serial: &str,
        command: &DeviceCommand,
        stdin: &[u8],
    ) -> Result<Vec<u8>, AdbError> {
        let args = command.adb_args();
        let mut child = Command::new(&self.program)
            .args(["-s", serial].iter().chain(&args))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(AdbError::spawn)?;
        // Written from another thread while stdout is read here, so neither
        // side can block the other on a full pipe.
        let writer = child.stdin.take().map(|mut pipe| {
            let input = stdin.to_vec();
            thread::spawn(move || pipe.write_all(&input))
        });
        let output = child.wait_with_output().map_err(AdbError::spawn)?;
        let written = writer.map_or(Ok(()), |writer| {
            writer
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("the stdin writer stopped")))
        });
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdbError::from_failure(
                serial,
                &args,
                output.status.to_string(),
                &stderr,
            ));
        }
        if let Err(error) = written {
            return Err(AdbError::from_failure(
                serial,
                &args,
                "stdin not written".to_owned(),
                &error.to_string(),
            ));
        }
        Ok(output.stdout)
    }

    /// Lists devices known to the local `adb` server (`adb devices -l`).
    pub fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError> {
        let args = ["devices", "-l"];
        let stdout = self.exec_host(&args)?;
        Ok(device::parse_device_list(&stdout))
    }

    /// Connects to a device over the network (`adb connect`).
    pub fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError> {
        let address_arg = address.to_string();
        let stdout = self.exec_host(&["connect", &address_arg])?;
        device::parse_connect_output(&stdout)
            .map_err(|message| AdbError::connect_failed(address_arg, message))
    }

    fn exec(&self, args: &[&str]) -> Result<Output, AdbError> {
        Command::new(&self.program)
            .args(args)
            .output()
            .map_err(AdbError::spawn)
    }

    /// Runs a host-side command (not addressed to a device) and returns stdout.
    fn exec_host(&self, args: &[&str]) -> Result<String, AdbError> {
        let output = self.exec(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AdbError::host_failure(
                args,
                output.status.to_string(),
                &stderr,
            ));
        }
        String::from_utf8(output.stdout).map_err(|_| AdbError::invalid_output(args))
    }
}

/// Runs `command` like [`Command::output`], but kills it and returns `None`
/// if it is still running after `timeout`.
///
/// Its output is read only after it exits, so it must fit in the pipe buffer
/// (64 KiB on macOS and Linux) or the command blocks until the timeout. Only
/// `am start -W`, which prints a few lines, uses this.
fn output_within(mut command: Command, timeout: Duration) -> io::Result<Option<Output>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            // It may exit between the check and the kill; either way it's gone.
            let _ = child.kill();
            child.wait()?;
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().map(Some)
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

    fn run_binary(
        &self,
        serial: &str,
        command: &DeviceCommand,
        stdin: &[u8],
    ) -> Result<Vec<u8>, AdbError> {
        Self::run_binary(self, serial, command, stdin)
    }

    fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError> {
        Self::devices(self)
    }

    fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError> {
        Self::connect(self, address)
    }
}

/// Runs `command` the way it needs: binary commands with their stdin through
/// [`Adb::run_binary`] (stdout discarded), the rest through [`Adb::run`].
/// Used by the plan engine, whose steps only need to know they ran.
pub(crate) fn run_step(
    adb: &dyn Adb,
    serial: &str,
    command: &DeviceCommand,
) -> Result<(), AdbError> {
    if !command.is_binary() {
        return adb.run(serial, command).map(drop);
    }
    let stdin = command.stdin().ok_or_else(|| {
        AdbError::from_failure(
            serial,
            &command.adb_args(),
            "not run".to_owned(),
            "the payload was read back from a journal, which keeps only its digest",
        )
    })?;
    adb.run_binary(serial, command, stdin).map(drop)
}

#[cfg(any(test, feature = "test-util"))]
pub use fake::FakeAdb;

#[cfg(any(test, feature = "test-util"))]
mod fake {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::Adb;
    use crate::command::DeviceCommand;
    use crate::device::{ConnectOutcome, DeviceAddress, DeviceEntry};
    use crate::error::AdbError;

    /// An [`Adb`] that returns canned output instead of touching a device.
    ///
    /// Commands without a canned response fail with `COMMAND_FAILED`, so a
    /// test notices when code runs a command it didn't expect.
    #[derive(Debug, Default)]
    pub struct FakeAdb {
        /// Responses per command, served in order; the last one repeats.
        responses: HashMap<DeviceCommand, Vec<String>>,
        /// How many times each command has run.
        calls: RefCell<HashMap<DeviceCommand, usize>>,
        /// Every command run, in order, including ones without a response.
        log: RefCell<Vec<DeviceCommand>>,
        /// Binary responses per command ([`Adb::run_binary`]), served like
        /// `responses`.
        binary: HashMap<DeviceCommand, Vec<Vec<u8>>>,
        /// What each `run_binary` call got on stdin, in order.
        inputs: RefCell<Vec<Vec<u8>>>,
        devices: Vec<DeviceEntry>,
    }

    impl FakeAdb {
        /// Creates a fake with no canned responses.
        pub fn new() -> Self {
            Self::default()
        }

        /// Makes `command` return `stdout` on every call.
        pub fn respond(self, command: DeviceCommand, stdout: impl Into<String>) -> Self {
            self.respond_sequence(command, [stdout.into()])
        }

        /// Makes successive calls of `command` return these outputs in order;
        /// after the last one, it keeps returning the last.
        pub fn respond_sequence(
            mut self,
            command: DeviceCommand,
            outputs: impl IntoIterator<Item = String>,
        ) -> Self {
            self.responses
                .insert(command, outputs.into_iter().collect());
            self
        }

        /// Adds a device to the list. `connect` to an address equal to its
        /// serial reports `AlreadyConnected`; any other address fails.
        pub fn with_device(mut self, device: DeviceEntry) -> Self {
            self.devices.push(device);
            self
        }

        /// Makes successive `run_binary` calls of `command` return these
        /// outputs in order; after the last one, it keeps returning the last.
        pub fn respond_binary(
            mut self,
            command: DeviceCommand,
            outputs: impl IntoIterator<Item = Vec<u8>>,
        ) -> Self {
            self.binary.insert(command, outputs.into_iter().collect());
            self
        }

        /// Every command run so far, in order.
        pub fn calls(&self) -> Vec<DeviceCommand> {
            self.log.borrow().clone()
        }

        /// What each `run_binary` call got on stdin, in order.
        pub fn inputs(&self) -> Vec<Vec<u8>> {
            self.inputs.borrow().clone()
        }

        /// Returns the canned binary stdout for `command`.
        pub fn run_binary(
            &self,
            serial: &str,
            command: &DeviceCommand,
            stdin: &[u8],
        ) -> Result<Vec<u8>, AdbError> {
            self.log.borrow_mut().push(command.clone());
            self.inputs.borrow_mut().push(stdin.to_vec());
            let Some(outputs) = self
                .binary
                .get(command)
                .filter(|outputs| !outputs.is_empty())
            else {
                return Err(AdbError::from_failure(
                    serial,
                    &command.adb_args(),
                    "no canned response".to_owned(),
                    "FakeAdb has no binary response for this command",
                ));
            };
            let mut calls = self.calls.borrow_mut();
            let call = calls.entry(command.clone()).or_default();
            let output = outputs[(*call).min(outputs.len() - 1)].clone();
            *call += 1;
            Ok(output)
        }

        /// Returns the canned stdout for `command`.
        pub fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
            self.log.borrow_mut().push(command.clone());
            let outputs = self
                .responses
                .get(command)
                .filter(|outputs| !outputs.is_empty());
            let Some(outputs) = outputs else {
                return Err(AdbError::from_failure(
                    serial,
                    &command.adb_args(),
                    "no canned response".to_owned(),
                    "FakeAdb has no response for this command",
                ));
            };
            let mut calls = self.calls.borrow_mut();
            let call = calls.entry(command.clone()).or_default();
            let output = outputs[(*call).min(outputs.len() - 1)].clone();
            *call += 1;
            Ok(output)
        }

        /// Returns the devices added with [`FakeAdb::with_device`].
        pub fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError> {
            Ok(self.devices.clone())
        }

        /// Succeeds only for addresses of devices added with [`FakeAdb::with_device`].
        pub fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError> {
            let address = address.to_string();
            if self.devices.iter().any(|device| device.serial == address) {
                Ok(ConnectOutcome::AlreadyConnected)
            } else {
                Err(AdbError::connect_failed(
                    address,
                    "FakeAdb has no device at this address".to_owned(),
                ))
            }
        }
    }

    impl Adb for FakeAdb {
        fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError> {
            Self::run(self, serial, command)
        }

        fn run_binary(
            &self,
            serial: &str,
            command: &DeviceCommand,
            stdin: &[u8],
        ) -> Result<Vec<u8>, AdbError> {
            Self::run_binary(self, serial, command, stdin)
        }

        fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError> {
            Self::devices(self)
        }

        fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError> {
            Self::connect(self, address)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_past_its_timeout_is_killed() {
        let started = Instant::now();
        let mut sleeper = Command::new("sleep");
        sleeper.arg("10");
        let output = output_within(sleeper, Duration::from_millis(200)).unwrap();
        assert!(output.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));

        let mut echo = Command::new("echo");
        echo.arg("Status: ok");
        let output = output_within(echo, Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "Status: ok\n");
    }

    #[test]
    fn binary_commands_get_their_stdin_and_return_raw_stdout() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rusttv-adb-binary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("adb");
        // Echoes stdin, then its arguments, like a device answering.
        std::fs::write(&program, "#!/bin/sh\ncat\nprintf '%s|' \"$@\"\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = SystemAdb::with_program(&program);
        let inbox = DeviceCommand::HistoryInbox {
            nonce: crate::command::HistoryNonce::new("0123456789abcdef").unwrap(),
            payload: crate::command::InboxPayload::new(vec![0, 255, 10]),
        };
        let out = adb
            .run_binary("tv", &inbox, inbox.stdin().unwrap())
            .unwrap();
        assert_eq!(&out[..3], &[0, 255, 10]);
        assert_eq!(
            String::from_utf8_lossy(&out[3..]),
            "-s|tv|exec-in|content|write|--uri|content://ai.rusttv.launcher.history/inbox/0123456789abcdef|"
        );
        // Its input would be lost through `run`, so `run` refuses it.
        assert!(adb.run("tv", &inbox).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    fn fake_adb_serves_a_sequence_then_repeats_the_last() {
        let adb = FakeAdb::new().respond_sequence(
            DeviceCommand::MemInfo,
            ["first".to_owned(), "second".to_owned()],
        );
        let outputs: Vec<String> = (0..3)
            .map(|_| adb.run("tv", &DeviceCommand::MemInfo).unwrap())
            .collect();
        assert_eq!(outputs, ["first", "second", "second"]);
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

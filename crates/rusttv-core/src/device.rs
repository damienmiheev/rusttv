//! Devices as seen by the local `adb` server: addresses, list entries and
//! connection results.
//!
//! The parsers are pure functions over `adb` output (M-IMPL-IO, "sans IO"),
//! so they are tested with captured text instead of real devices.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;

/// Port Android TVs listen on for network debugging unless configured otherwise.
pub const DEFAULT_ADB_PORT: u16 = 5555;

/// A network address for `adb connect`: a hostname or IPv4 address and a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAddress {
    host: String,
    port: u16,
}

impl DeviceAddress {
    /// The host part.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port part.
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl FromStr for DeviceAddress {
    type Err = InvalidDeviceAddress;

    /// Parses `host` or `host:port`; the port defaults to [`DEFAULT_ADB_PORT`].
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidDeviceAddress(input.to_owned());
        let (host, port) = match input.rsplit_once(':') {
            Some((host, port)) => (host, port.parse::<u16>().map_err(|_| invalid())?),
            None => (input, DEFAULT_ADB_PORT),
        };
        // Hostname/IPv4 characters only. Also rules out leading `-`, which
        // `adb` would read as an option. IPv6 is not supported yet.
        let host_ok = host
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        if !host_ok || port == 0 {
            return Err(invalid());
        }
        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }
}

impl fmt::Display for DeviceAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

/// Returned when parsing a [`DeviceAddress`] fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not a valid device address; expected `host` or `host:port`, e.g. 192.168.1.20:5555"
)]
pub struct InvalidDeviceAddress(String);

/// Connection state of a device, as reported by `adb devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    /// Connected and authorized; commands can run.
    Device,
    /// The device has not accepted this computer's debugging key yet.
    Unauthorized,
    /// Known to adb but not responding.
    Offline,
    /// Any other state adb reports (`recovery`, `no permissions`, ...).
    Other(String),
}

impl fmt::Display for DeviceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Device => "device",
            Self::Unauthorized => "unauthorized",
            Self::Offline => "offline",
            Self::Other(state) => state,
        })
    }
}

/// One line of `adb devices -l`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceEntry {
    /// The serial used to address the device, e.g. `192.168.1.20:5555`.
    pub serial: String,
    /// Connection state.
    pub state: DeviceState,
    /// Model name with spaces replaced by `_`, when adb knows it.
    pub model: Option<String>,
    /// Product name, when adb knows it.
    pub product: Option<String>,
    /// Device codename, when adb knows it.
    pub device: Option<String>,
}

/// Parses the stdout of `adb devices -l`.
pub fn parse_device_list(output: &str) -> Vec<DeviceEntry> {
    output
        .lines()
        .filter(|line| !line.starts_with("List of devices") && !line.starts_with('*'))
        .filter_map(parse_device_line)
        .collect()
}

fn parse_device_line(line: &str) -> Option<DeviceEntry> {
    let mut tokens = line.split_whitespace();
    let serial = tokens.next()?.to_owned();
    let rest: Vec<&str> = tokens.collect();
    let state = match rest.as_slice() {
        ["device", ..] => DeviceState::Device,
        ["unauthorized", ..] => DeviceState::Unauthorized,
        ["offline", ..] => DeviceState::Offline,
        // e.g. "no permissions (user in plugdev group; ...); see [http://...]"
        ["no", "permissions", ..] => DeviceState::Other("no permissions".to_owned()),
        [state, ..] => DeviceState::Other((*state).to_owned()),
        [] => return None,
    };
    // Remaining `key:value` tokens; keys are lowercase words like `model`.
    let property = |key: &str| {
        rest.iter().find_map(|token| {
            token
                .strip_prefix(key)
                .and_then(|value| value.strip_prefix(':'))
                .map(str::to_owned)
        })
    };
    Some(DeviceEntry {
        serial,
        state,
        model: property("model"),
        product: property("product"),
        device: property("device"),
    })
}

/// Successful result of `adb connect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectOutcome {
    /// A new connection was made.
    Connected,
    /// adb was already connected to this address.
    AlreadyConnected,
}

/// Interprets the stdout of `adb connect`.
///
/// `adb connect` exits with status 0 even when it fails (adb 1.0.41) and
/// reports the result as text on stdout, so the text is all we can go by.
/// Returns the failure message when the connection failed.
pub fn parse_connect_output(output: &str) -> Result<ConnectOutcome, String> {
    let output = output.trim();
    if output.starts_with("already connected to") {
        Ok(ConnectOutcome::AlreadyConnected)
    } else if output.starts_with("connected to") {
        Ok(ConnectOutcome::Connected)
    } else {
        Err(output.to_owned())
    }
}

/// Picks the device a command should run on.
///
/// With `requested`, that serial must be listed and ready. Without it, there
/// must be exactly one device, so commands never guess between several TVs.
pub fn select_device<'a>(
    devices: &'a [DeviceEntry],
    requested: Option<&str>,
) -> Result<&'a DeviceEntry, DeviceSelectionError> {
    let device = match requested {
        Some(serial) => devices
            .iter()
            .find(|device| device.serial == serial)
            .ok_or_else(|| SelectionKind::NotFound(serial.to_owned()))?,
        None => match devices {
            [] => return Err(SelectionKind::NoDevices.into()),
            [only] => only,
            _ => {
                let serials = devices.iter().map(|d| d.serial.clone()).collect();
                return Err(SelectionKind::Ambiguous(serials).into());
            }
        },
    };
    if device.state == DeviceState::Device {
        Ok(device)
    } else {
        Err(SelectionKind::NotReady {
            serial: device.serial.clone(),
            state: device.state.clone(),
        }
        .into())
    }
}

/// No single ready device could be chosen. See [`select_device`].
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct DeviceSelectionError {
    kind: SelectionKind,
}

#[derive(Debug, thiserror::Error)]
enum SelectionKind {
    #[error("no devices found; connect one with `rusttv device connect <address>`")]
    NoDevices,
    #[error("device `{0}` not found; see `rusttv device list`")]
    NotFound(String),
    #[error("several devices are connected ({}); choose one with `--serial`", .0.join(", "))]
    Ambiguous(Vec<String>),
    #[error("device `{serial}` is {state}{hint}", hint = state_hint(state))]
    NotReady { serial: String, state: DeviceState },
}

fn state_hint(state: &DeviceState) -> &'static str {
    match state {
        DeviceState::Unauthorized => "; accept the debugging prompt on the TV",
        DeviceState::Offline => "; reconnect it or restart network debugging on the TV",
        _ => "",
    }
}

impl From<SelectionKind> for DeviceSelectionError {
    fn from(kind: SelectionKind) -> Self {
        Self { kind }
    }
}

impl DeviceSelectionError {
    /// Stable machine-readable code; shared with [`crate::AdbError`] where the meaning is the same.
    pub fn code(&self) -> &'static str {
        match &self.kind {
            SelectionKind::NoDevices => "NO_DEVICE",
            SelectionKind::NotFound(_) => "DEVICE_NOT_FOUND",
            SelectionKind::Ambiguous(_) => "MULTIPLE_DEVICES",
            SelectionKind::NotReady { state, .. } => match state {
                DeviceState::Unauthorized => "DEVICE_UNAUTHORIZED",
                DeviceState::Offline => "DEVICE_OFFLINE",
                _ => "DEVICE_NOT_READY",
            },
        }
    }

    /// Several devices matched and the user must pick one (a usage problem,
    /// not an unreachable device).
    pub fn is_ambiguous(&self) -> bool {
        matches!(self.kind, SelectionKind::Ambiguous(_))
    }
}

/// One error of every kind; see `crate::error::every_adb_error`.
#[cfg(test)]
pub(crate) fn every_selection_error() -> Vec<DeviceSelectionError> {
    let not_ready = |state| SelectionKind::NotReady {
        serial: "tv".to_owned(),
        state,
    };
    let all: Vec<DeviceSelectionError> = vec![
        SelectionKind::NoDevices.into(),
        SelectionKind::NotFound("tv".to_owned()).into(),
        SelectionKind::Ambiguous(vec!["a".to_owned(), "b".to_owned()]).into(),
        not_ready(DeviceState::Unauthorized).into(),
        not_ready(DeviceState::Offline).into(),
        not_ready(DeviceState::Other("recovery".to_owned())).into(),
    ];
    for error in &all {
        match error.kind {
            SelectionKind::NoDevices
            | SelectionKind::NotFound(_)
            | SelectionKind::Ambiguous(_)
            | SelectionKind::NotReady { .. } => {}
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(serial: &str, state: DeviceState) -> DeviceEntry {
        DeviceEntry {
            serial: serial.to_owned(),
            state,
            model: None,
            product: None,
            device: None,
        }
    }

    #[test]
    fn the_only_ready_device_is_selected_automatically() {
        let devices = [entry("emulator-5554", DeviceState::Device)];
        assert_eq!(
            select_device(&devices, None).unwrap().serial,
            "emulator-5554"
        );
    }

    #[test]
    fn several_devices_require_an_explicit_serial() {
        let devices = [
            entry("emulator-5554", DeviceState::Device),
            entry("192.168.1.20:5555", DeviceState::Device),
        ];
        let error = select_device(&devices, None).unwrap_err();
        assert_eq!(error.code(), "MULTIPLE_DEVICES");
        assert!(error.is_ambiguous());
        assert!(error.to_string().contains("192.168.1.20:5555"));

        let chosen = select_device(&devices, Some("192.168.1.20:5555")).unwrap();
        assert_eq!(chosen.serial, "192.168.1.20:5555");
    }

    #[test]
    fn selection_errors_explain_what_to_do() {
        assert_eq!(select_device(&[], None).unwrap_err().code(), "NO_DEVICE");

        let devices = [entry("192.168.1.20:5555", DeviceState::Unauthorized)];
        let error = select_device(&devices, None).unwrap_err();
        assert_eq!(error.code(), "DEVICE_UNAUTHORIZED");
        assert!(error.to_string().contains("accept the debugging prompt"));

        let error = select_device(&devices, Some("10.0.0.9:5555")).unwrap_err();
        assert_eq!(error.code(), "DEVICE_NOT_FOUND");
    }

    #[test]
    fn real_emulator_device_line() {
        let devices = parse_device_list(include_str!(
            "fixtures/emulator-pixel3a-api34/devices-l.txt"
        ));
        assert_eq!(
            devices,
            [DeviceEntry {
                serial: "emulator-5554".to_owned(),
                state: DeviceState::Device,
                model: Some("sdk_gphone64_arm64".to_owned()),
                product: Some("sdk_gphone64_arm64".to_owned()),
                device: Some("emu64a".to_owned()),
            }]
        );
    }

    #[test]
    fn address_without_port_uses_the_adb_default() {
        let address: DeviceAddress = "192.168.1.20".parse().unwrap();
        assert_eq!(address.to_string(), "192.168.1.20:5555");
    }

    #[test]
    fn address_keeps_an_explicit_port_and_hostnames() {
        let address: DeviceAddress = "living-room-tv.local:5556".parse().unwrap();
        assert_eq!(address.host(), "living-room-tv.local");
        assert_eq!(address.port(), 5556);
    }

    #[test]
    fn malformed_addresses_are_rejected() {
        for input in [
            "",
            ":5555",
            "-s",
            "--help:5555",
            "192.168.1.20:",
            "192.168.1.20:0",
            "192.168.1.20:70000",
            "192.168.1.20:abc",
            "tv local",
            "tv;reboot",
            "[::1]:5555",
        ] {
            assert!(input.parse::<DeviceAddress>().is_err(), "{input:?}");
        }
    }

    #[test]
    fn empty_device_list() {
        // Captured from adb 1.0.41 with no devices attached.
        assert!(parse_device_list("List of devices attached\n\n").is_empty());
    }

    #[test]
    fn device_list_with_mixed_states() {
        // Format of `adb devices -l` (adb 1.0.41). Replace with a capture from
        // the Philips TV in step 4.
        let output = "\
* daemon not running; starting now at tcp:5037
* daemon started successfully
List of devices attached
emulator-5554          device product:sdk_google_atv_x86 model:Android_TV_on_x86 device:generic_x86 transport_id:1
192.168.1.20:5555      unauthorized transport_id:2
0123456789ABCDEF       no permissions (user in plugdev group; are your udev rules wrong?); see [http://developer.android.com/tools/device.html] usb:1-1 transport_id:3
192.168.1.21:5555      offline transport_id:4

";
        let devices = parse_device_list(output);
        assert_eq!(devices.len(), 4);

        assert_eq!(devices[0].serial, "emulator-5554");
        assert_eq!(devices[0].state, DeviceState::Device);
        assert_eq!(devices[0].model.as_deref(), Some("Android_TV_on_x86"));
        assert_eq!(devices[0].product.as_deref(), Some("sdk_google_atv_x86"));
        assert_eq!(devices[0].device.as_deref(), Some("generic_x86"));

        assert_eq!(devices[1].state, DeviceState::Unauthorized);
        assert_eq!(devices[1].model, None);

        assert_eq!(
            devices[2].state,
            DeviceState::Other("no permissions".to_owned())
        );
        assert_eq!(devices[2].model, None);

        assert_eq!(devices[3].state, DeviceState::Offline);
    }

    #[test]
    fn connect_success_messages() {
        assert_eq!(
            parse_connect_output("connected to 192.168.1.20:5555\n"),
            Ok(ConnectOutcome::Connected)
        );
        assert_eq!(
            parse_connect_output("already connected to 192.168.1.20:5555\n"),
            Ok(ConnectOutcome::AlreadyConnected)
        );
    }

    #[test]
    fn connect_failure_messages() {
        // Both captured from adb 1.0.41; each exited with status 0.
        for output in [
            "failed to connect to '127.0.0.1:5': Connection refused\n",
            "failed to resolve host: 'no-such-host.invalid': nodename nor servname provided, or not known\n",
        ] {
            assert_eq!(parse_connect_output(output), Err(output.trim().to_owned()));
        }
    }
}

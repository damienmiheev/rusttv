mod apps;
mod compat;
mod output;
mod performance;

use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use rusttv_core::{
    Adb, AdbError, ConnectOutcome, DeviceAddress, DeviceEntry, DeviceInfo, DeviceSelectionError,
    DeviceState, SystemAdb,
};
use serde::Serialize;

/// Rootless toolkit for stock Android TVs.
#[derive(Debug, Parser)]
#[command(name = "rusttv", version = rusttv_core::VERSION, about)]
struct Cli {
    /// Print machine-readable JSON on stdout instead of text.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Find and connect to devices.
    #[command(subcommand)]
    Device(DeviceAction),
    /// Inspect installed apps.
    #[command(subcommand)]
    Apps(AppsAction),
    /// Measure device load.
    #[command(subcommand)]
    Performance(PerformanceAction),
    /// Device compatibility profiles.
    #[command(subcommand)]
    Compat(CompatAction),
}

#[derive(Debug, Subcommand)]
enum CompatAction {
    /// Print a compatibility profile (TOML) for a device, ready to add to `compat/devices/`. Read-only.
    Profile {
        #[command(flatten)]
        target: Target,
    },
}

#[derive(Debug, Subcommand)]
enum PerformanceAction {
    /// CPU, memory, launcher memory and the biggest processes right now. Read-only.
    Snapshot {
        #[command(flatten)]
        target: Target,
        /// How long to sample CPU usage, in milliseconds.
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(100..=60_000))]
        sample_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum AppsAction {
    /// List installed packages: system or third-party, enabled or disabled. Read-only.
    List {
        #[command(flatten)]
        target: Target,
    },
}

/// Which device a command runs on.
#[derive(Debug, Args)]
struct Target {
    /// adb serial of the device (see `rusttv device list`); needed when several are connected.
    #[arg(long)]
    serial: Option<String>,
}

impl Target {
    /// Resolves to the serial of a single ready device.
    fn resolve(&self, adb: &dyn Adb) -> Result<String, CliError> {
        let devices = adb.devices()?;
        let device = rusttv_core::select_device(&devices, self.serial.as_deref())?;
        Ok(device.serial.clone())
    }
}

#[derive(Debug, Subcommand)]
enum DeviceAction {
    /// List devices known to adb.
    List,
    /// Connect to a TV over the network. Turn on network debugging on the TV first.
    Connect {
        /// TV address: `host` or `host:port` (default port 5555).
        address: DeviceAddress,
    },
    /// Show manufacturer, model, Android version, CPU, memory and storage. Read-only.
    Inspect {
        #[command(flatten)]
        target: Target,
    },
}

/// Exit codes documented in ARCHITECTURE.md; clap itself exits with 2 on usage errors.
const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_DEVICE_UNREACHABLE: u8 = 3;

/// Everything a command can fail with.
#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error(transparent)]
    Selection(#[from] DeviceSelectionError),
    #[error(transparent)]
    IncompleteProfile(#[from] rusttv_core::MissingField),
}

impl CliError {
    fn code(&self) -> &'static str {
        match self {
            Self::Adb(error) => error.code(),
            Self::Selection(error) => error.code(),
            Self::IncompleteProfile(_) => "INCOMPLETE_PROFILE",
        }
    }

    fn exit_code(&self) -> u8 {
        match self {
            Self::Adb(error) if error.is_device_unreachable() => EXIT_DEVICE_UNREACHABLE,
            Self::Adb(_) => EXIT_FAILURE,
            Self::Selection(error) if error.is_ambiguous() => EXIT_USAGE,
            Self::Selection(_) => EXIT_DEVICE_UNREACHABLE,
            Self::IncompleteProfile(_) => EXIT_FAILURE,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // RUSTTV_ADB points to a specific adb executable; otherwise `adb` from PATH.
    let adb = std::env::var_os("RUSTTV_ADB").map_or_else(SystemAdb::new, SystemAdb::with_program);

    let result = match &cli.command {
        Command::Device(DeviceAction::List) => list(&adb, cli.json),
        Command::Device(DeviceAction::Connect { address }) => connect(&adb, address, cli.json),
        Command::Device(DeviceAction::Inspect { target }) => inspect(&adb, target, cli.json),
        Command::Apps(AppsAction::List { target }) => apps::list(&adb, target, cli.json),
        Command::Performance(PerformanceAction::Snapshot { target, sample_ms }) => {
            performance::snapshot(&adb, target, *sample_ms, cli.json)
        }
        Command::Compat(CompatAction::Profile { target }) => {
            compat::profile(&adb, target, cli.json)
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::print_error(cli.json, error.code(), &error);
            ExitCode::from(error.exit_code())
        }
    }
}

fn list(adb: &dyn Adb, json: bool) -> Result<(), CliError> {
    let devices = adb.devices()?;
    if json {
        output::print_json(&devices);
    } else if devices.is_empty() {
        println!("No devices. Connect one with `rusttv device connect <address>`.");
    } else {
        print_device_table(&devices);
    }
    Ok(())
}

fn print_device_table(devices: &[DeviceEntry]) {
    let serial_width = devices
        .iter()
        .map(|device| device.serial.len())
        .max()
        .unwrap_or_default()
        .max("SERIAL".len());
    println!("{:serial_width$}  {:12}  MODEL", "SERIAL", "STATE");
    for device in devices {
        let model = device.model.as_deref().unwrap_or("-");
        let state = device.state.to_string();
        println!("{:serial_width$}  {state:12}  {model}", device.serial);
    }
}

#[derive(Debug, Serialize)]
struct ConnectReport {
    address: String,
    outcome: ConnectOutcome,
    /// State after connecting, if adb lists the device; `unauthorized` until
    /// the user accepts the prompt on the TV.
    state: Option<DeviceState>,
}

fn connect(adb: &dyn Adb, address: &DeviceAddress, json: bool) -> Result<(), CliError> {
    let outcome = adb.connect(address)?;
    let address = address.to_string();
    // Best effort: a failing `adb devices` shouldn't turn a successful connect into an error.
    let state = adb.devices().ok().and_then(|devices| {
        devices
            .into_iter()
            .find(|device| device.serial == address)
            .map(|device| device.state)
    });

    if json {
        output::print_json(&ConnectReport {
            address,
            outcome,
            state,
        });
        return Ok(());
    }
    match outcome {
        ConnectOutcome::Connected => println!("Connected to {address}."),
        ConnectOutcome::AlreadyConnected => println!("Already connected to {address}."),
    }
    match state {
        Some(DeviceState::Device) => println!("The device is ready."),
        Some(DeviceState::Unauthorized) => println!(
            "The TV is asking whether to allow debugging from this computer. \
             Accept the prompt on the TV, then run `rusttv device list`."
        ),
        Some(other) => println!("The device is {other}."),
        None => {}
    }
    Ok(())
}

fn inspect(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let info = rusttv_core::inspect(adb, &serial)?;
    if json {
        output::print_json(&info);
    } else {
        print_device_info(&info);
    }
    Ok(())
}

fn print_device_info(info: &DeviceInfo) {
    let or_unknown = |value: &Option<String>| value.clone().unwrap_or_else(|| "unknown".to_owned());
    let model = match (&info.model, &info.device) {
        (Some(model), Some(codename)) => format!("{model} ({codename})"),
        (model, _) => or_unknown(model),
    };
    let mut android = or_unknown(&info.android_version);
    if let Some(api) = info.api_level {
        android.push_str(&format!(" (API {api})"));
    }
    if let Some(patch) = &info.security_patch {
        android.push_str(&format!(", security patch {patch}"));
    }
    let memory = info.memory.map_or_else(
        || "unknown".to_owned(),
        |memory| match memory.available_kib {
            Some(available) => format!(
                "{} total, {} available",
                output::gib(memory.total_kib),
                output::gib(available)
            ),
            None => format!("{} total", output::gib(memory.total_kib)),
        },
    );
    let storage = info.data_storage.map_or_else(
        || "unknown".to_owned(),
        |storage| {
            format!(
                "{} total, {} free",
                output::gib(storage.total_kib),
                output::gib(storage.available_kib)
            )
        },
    );
    let abis = if info.abis.is_empty() {
        "unknown".to_owned()
    } else {
        info.abis.join(", ")
    };

    for (label, value) in [
        ("Device", info.serial.clone()),
        ("Manufacturer", or_unknown(&info.manufacturer)),
        ("Brand", or_unknown(&info.brand)),
        ("Model", model),
        ("Android", android),
        ("Build", or_unknown(&info.build_fingerprint)),
        ("Characteristics", or_unknown(&info.build_characteristics)),
        ("CPU ABIs", abis),
        ("Memory", memory),
        ("Storage (/data)", storage),
    ] {
        println!("{label:16} {value}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

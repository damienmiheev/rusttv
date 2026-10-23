mod output;

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use rusttv_core::{
    Adb, AdbError, ConnectOutcome, DeviceAddress, DeviceEntry, DeviceState, SystemAdb,
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
}

/// Exit codes documented in ARCHITECTURE.md; clap itself exits with 2 on usage errors.
const EXIT_FAILURE: u8 = 1;
const EXIT_DEVICE_UNREACHABLE: u8 = 3;

fn main() -> ExitCode {
    let cli = Cli::parse();
    // RUSTTV_ADB points to a specific adb executable; otherwise `adb` from PATH.
    let adb = std::env::var_os("RUSTTV_ADB").map_or_else(SystemAdb::new, SystemAdb::with_program);

    let result = match &cli.command {
        Command::Device(DeviceAction::List) => list(&adb, cli.json),
        Command::Device(DeviceAction::Connect { address }) => connect(&adb, address, cli.json),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            output::print_error(cli.json, error.code(), &error);
            if error.is_device_unreachable() {
                ExitCode::from(EXIT_DEVICE_UNREACHABLE)
            } else {
                ExitCode::from(EXIT_FAILURE)
            }
        }
    }
}

fn list(adb: &dyn Adb, json: bool) -> Result<(), AdbError> {
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

fn connect(adb: &dyn Adb, address: &DeviceAddress, json: bool) -> Result<(), AdbError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}

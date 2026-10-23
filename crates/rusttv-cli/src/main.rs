mod apps;
mod compat;
mod launcher;
mod optimize;
mod output;
mod performance;
mod tools;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use rusttv_core::{
    Adb, AdbError, BenchmarkError, BenchmarkOptions, ConnectOutcome, DeviceAddress, DeviceEntry,
    DeviceInfo, DeviceSelectionError, DeviceState, PlanError, SystemAdb,
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
    /// The home screen (launcher) on a device.
    #[command(subcommand)]
    Launcher(LauncherAction),
    /// Turn off telemetry, ads and other packages you choose, shorten
    /// animations, and undo it.
    #[command(subcommand)]
    Optimize(OptimizeAction),
    /// Experimental: turn on features a TV ships installed but disabled,
    /// until it restarts (TOOLS.md).
    #[command(subcommand)]
    Tools(ToolsAction),
}

#[derive(Debug, Subcommand)]
enum ToolsAction {
    /// MediaTek's AirPlay receiver, on models whose maker ships it disabled.
    /// Experimental; not certified by Apple for these models.
    #[command(subcommand)]
    Airplay(AirplayAction),
}

#[derive(Debug, Subcommand)]
enum AirplayAction {
    /// Whether the tool runs on this device, the AirPlay switch, whether the
    /// vendor service runs, its recent log lines and listening ports. Read-only.
    Status {
        #[command(flatten)]
        target: Target,
        /// Also look for AirPlay receivers on the network from this computer
        /// (`dns-sd` on macOS, `avahi-browse` on Linux; skipped if neither).
        #[arg(long)]
        check_bonjour: bool,
    },
    /// Turn AirPlay on until the TV restarts: set the vendor switch to 1 and
    /// start the vendor service through RustTV's launcher. Shows the plan and
    /// asks you to type a confirmation; rolls back on failure.
    Enable {
        #[command(flatten)]
        target: Target,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Undo `enable` from its journal: put the switch back and stop the
    /// service. Shows the plan and asks you to type a confirmation.
    Disable {
        #[command(flatten)]
        target: Target,
        /// Journal file to restore from; by default the newest `enable`
        /// journal for this device's firmware.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum OptimizeAction {
    /// List the catalog packages on this device and what your choices would
    /// do with them: state, category, tested on this model or not. Also where
    /// each optimizer profile stands, with the current values of its
    /// settings. Read-only.
    Analyze {
        #[command(flatten)]
        target: Target,
        /// Choices file instead of `<config dir>/privacy.toml`.
        #[arg(long)]
        choices: Option<PathBuf>,
    },
    /// Apply a profile. Shows the plan and asks you to type a confirmation;
    /// rolls back on failure. Undo with `optimize restore`.
    #[command(subcommand)]
    Apply(ApplyProfile),
    /// Undo `optimize apply`: re-enable the packages a privacy journal
    /// disabled, or put back the settings an optimizer journal changed
    /// (leaving any changed since as they are). Shows the plan and asks you
    /// to type a confirmation.
    Restore {
        /// Which change to undo: `privacy` or `reduced-animations`. Needed
        /// only when both can be restored on this device.
        profile: Option<rusttv_core::OptimizeKind>,
        #[command(flatten)]
        target: Target,
        /// Journal file to restore from; by default the newest restorable
        /// journal of that kind for this device's firmware.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ApplyProfile {
    /// Disable (never uninstall) the catalog packages your choices select
    /// (by default telemetry, ads and extras) that are installed and enabled.
    /// Only packages tested on this model, unless --allow-untested.
    Privacy {
        #[command(flatten)]
        target: Target,
        /// Narrow the categories of your choices to these, comma-separated:
        /// telemetry, ads, extras, casting, smart-home, voice, streaming-apps.
        #[arg(long, value_delimiter = ',')]
        only: Vec<rusttv_core::PrivacyCategory>,
        /// Choices file instead of `<config dir>/privacy.toml`.
        #[arg(long)]
        choices: Option<PathBuf>,
        /// Also plan packages not tested on this model; the plan marks each
        /// one UNTESTED.
        #[arg(long)]
        allow_untested: bool,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Halve window, transition and animator animation durations (global
    /// settings at 0.5). Only on a model where the profile was tested,
    /// unless --allow-untested.
    ReducedAnimations {
        #[command(flatten)]
        target: Target,
        /// Plan it although this model's profile doesn't record it as tested
        /// (for the first test on a device); the plan says it is UNTESTED.
        #[arg(long)]
        allow_untested: bool,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
enum LauncherAction {
    /// Show which launcher the Home button opens and which others could. Read-only.
    Status {
        #[command(flatten)]
        target: Target,
    },
    /// Make RustTV the Home screen by disabling (never uninstalling) the stock
    /// launcher. Shows the plan and asks you to type a confirmation; rolls back
    /// on failure. Undo with `launcher restore`.
    Takeover {
        #[command(flatten)]
        target: Target,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Install the RustTV launcher, or upgrade it, from the APK for the
    /// device's primary ABI (or --apk). Shows the plan and asks you to type a
    /// confirmation; a fresh install is uninstalled again if its checks fail.
    Install {
        #[command(flatten)]
        target: Target,
        /// APK to install instead of
        /// `android/app/build/outputs/apk/release/app-<abi>-release.apk`
        /// (relative to the current directory) from `android/build.sh`.
        #[arg(long)]
        apk: Option<PathBuf>,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Uninstall the RustTV launcher and its data. Refused while a Home
    /// takeover is active: run `launcher restore` first. Shows the plan and
    /// asks you to type a confirmation.
    Uninstall {
        #[command(flatten)]
        target: Target,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Undo `launcher takeover`: re-enable the stock launcher, using the
    /// takeover's journal. Shows the plan and asks you to type a confirmation.
    Restore {
        #[command(flatten)]
        target: Target,
        /// Journal file to restore from; by default the newest takeover journal
        /// for this device's firmware.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Without a journal (lost, or the takeover ran on another computer):
        /// re-enable the packages this device's profile says a takeover
        /// disables, where they are disabled now.
        #[arg(long, conflicts_with = "journal")]
        from_profile: bool,
        /// Only show the plan; change nothing.
        #[arg(long)]
        dry_run: bool,
    },
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
    /// Cold start, resume, idle memory and CPU of one or more apps, repeated
    /// and interleaved (BENCHMARK_PLAN.md). Stops and starts the apps; changes no settings.
    Benchmark {
        #[command(flatten)]
        target: Target,
        /// Activity to measure, `package/.Activity`. Repeat to compare apps;
        /// resume times need at least two.
        #[arg(long = "app", required = true)]
        apps: Vec<rusttv_core::Component>,
        /// Counted runs per app and measurement.
        #[arg(long, default_value_t = BenchmarkOptions::default().runs, value_parser = clap::value_parser!(u32).range(1..=1000))]
        runs: u32,
        /// Discarded warm-up runs per app.
        #[arg(long, default_value_t = BenchmarkOptions::default().warmup)]
        warmup: u32,
        /// Pause after stopping or switching apps, in milliseconds.
        #[arg(long, default_value_t = BenchmarkOptions::default().pause_ms)]
        pause_ms: u64,
        /// How long an app idles in front before memory is read after the
        /// resume series, in milliseconds.
        #[arg(long, default_value_t = BenchmarkOptions::default().settle_ms)]
        settle_ms: u64,
        /// How long an app idles after a fresh cold start before memory is
        /// read, in milliseconds; long enough to get past the start-up peak.
        #[arg(long, default_value_t = BenchmarkOptions::default().fresh_settle_ms)]
        fresh_settle_ms: u64,
        /// Window for idle CPU usage, in milliseconds (0 skips it).
        #[arg(long, default_value_t = BenchmarkOptions::default().cpu_window_ms)]
        cpu_window_ms: u64,
        /// Idle memory/CPU samples per app.
        #[arg(long, default_value_t = BenchmarkOptions::default().idle_samples)]
        idle_samples: u32,
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
    #[error(transparent)]
    Benchmark(#[from] BenchmarkError),
    #[error(transparent)]
    Plan(#[from] PlanError),
}

impl CliError {
    fn code(&self) -> &'static str {
        match self {
            Self::Adb(error) => error.code(),
            Self::Selection(error) => error.code(),
            Self::IncompleteProfile(_) => "INCOMPLETE_PROFILE",
            Self::Benchmark(error) => error.code(),
            Self::Plan(error) => error.code(),
        }
    }

    fn exit_code(&self) -> u8 {
        match self {
            Self::Adb(error) if error.is_device_unreachable() => EXIT_DEVICE_UNREACHABLE,
            Self::Adb(_) => EXIT_FAILURE,
            Self::Selection(error) if error.is_ambiguous() => EXIT_USAGE,
            Self::Selection(_) => EXIT_DEVICE_UNREACHABLE,
            Self::IncompleteProfile(_) => EXIT_FAILURE,
            Self::Benchmark(error) if error.is_device_unreachable() => EXIT_DEVICE_UNREACHABLE,
            Self::Benchmark(_) => EXIT_FAILURE,
            Self::Plan(error) if error.is_device_unreachable() => EXIT_DEVICE_UNREACHABLE,
            Self::Plan(_) => EXIT_FAILURE,
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
        Command::Launcher(LauncherAction::Status { target }) => {
            launcher::status(&adb, target, cli.json)
        }
        Command::Launcher(LauncherAction::Takeover { target, dry_run }) => {
            launcher::takeover(&adb, target, *dry_run, cli.json)
        }
        Command::Launcher(LauncherAction::Install {
            target,
            apk,
            dry_run,
        }) => launcher::install(&adb, target, apk.as_deref(), *dry_run, cli.json),
        Command::Launcher(LauncherAction::Uninstall { target, dry_run }) => {
            launcher::uninstall(&adb, target, *dry_run, cli.json)
        }
        Command::Launcher(LauncherAction::Restore {
            target,
            journal,
            from_profile,
            dry_run,
        }) => launcher::restore(
            &adb,
            target,
            journal.as_deref(),
            *from_profile,
            *dry_run,
            cli.json,
        ),
        Command::Optimize(OptimizeAction::Analyze { target, choices }) => {
            optimize::analyze(&adb, target, choices.as_deref(), cli.json)
        }
        Command::Optimize(OptimizeAction::Apply(ApplyProfile::Privacy {
            target,
            only,
            choices,
            allow_untested,
            dry_run,
        })) => optimize::apply_privacy(
            &adb,
            target,
            &optimize::PrivacyOptions {
                only,
                choices: choices.as_deref(),
                allow_untested: *allow_untested,
            },
            *dry_run,
            cli.json,
        ),
        Command::Optimize(OptimizeAction::Apply(ApplyProfile::ReducedAnimations {
            target,
            allow_untested,
            dry_run,
        })) => {
            optimize::apply_reduced_animations(&adb, target, *allow_untested, *dry_run, cli.json)
        }
        Command::Optimize(OptimizeAction::Restore {
            profile,
            target,
            journal,
            dry_run,
        }) => optimize::restore(
            &adb,
            target,
            *profile,
            journal.as_deref(),
            *dry_run,
            cli.json,
        ),
        Command::Tools(ToolsAction::Airplay(AirplayAction::Status {
            target,
            check_bonjour,
        })) => tools::airplay_status(&adb, target, *check_bonjour, cli.json),
        Command::Tools(ToolsAction::Airplay(AirplayAction::Enable { target, dry_run })) => {
            tools::airplay_enable(&adb, target, *dry_run, cli.json)
        }
        Command::Tools(ToolsAction::Airplay(AirplayAction::Disable {
            target,
            journal,
            dry_run,
        })) => tools::airplay_disable(&adb, target, journal.as_deref(), *dry_run, cli.json),
        Command::Performance(PerformanceAction::Snapshot { target, sample_ms }) => {
            performance::snapshot(&adb, target, *sample_ms, cli.json)
        }
        Command::Performance(PerformanceAction::Benchmark {
            target,
            apps,
            runs,
            warmup,
            pause_ms,
            settle_ms,
            fresh_settle_ms,
            cpu_window_ms,
            idle_samples,
        }) => {
            let options = BenchmarkOptions {
                runs: *runs,
                warmup: *warmup,
                pause_ms: *pause_ms,
                settle_ms: *settle_ms,
                fresh_settle_ms: *fresh_settle_ms,
                cpu_window_ms: *cpu_window_ms,
                idle_samples: *idle_samples,
            };
            performance::benchmark(&adb, target, apps, options, cli.json)
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

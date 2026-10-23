//! `rusttv performance benchmark`: repeated, comparable launcher measurements
//! following BENCHMARK_PLAN.md.
//!
//! Measures, per app (an activity component):
//! - **cold start**: `am force-stop`, confirm the process is gone, then
//!   `am start -W`; only runs Android reports as `COLD` count.
//! - **resume**: bringing the app back to the front while it's still in
//!   memory (what pressing Home feels like); only `HOT` runs count. Needs at
//!   least two apps, which take turns being in front.
//! - **idle memory and CPU**: after the app has been in front and idle.
//!
//! Apps are interleaved (A, B, A, B, ...) so drift and heat affect each
//! equally. Stops and starts apps (R1, transient); changes no settings.
//!
//! Nobody touches the remote during a run, so the TV's screensaver can start.
//! Before every start the TV is checked and, if needed, woken like a key
//! press would; an idle sample taken while the screensaver was on is dropped.

use std::time::Duration;

use serde::Serialize;

use crate::adb::Adb;
use crate::command::{Component, DeviceCommand};
use crate::error::AdbError;
use crate::inspect::inspect;
use crate::perf::{parse_app_memory, parse_process_list};

/// How a benchmark runs. Defaults follow BENCHMARK_PLAN.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct BenchmarkOptions {
    /// Counted runs per app and measurement.
    pub runs: u32,
    /// Discarded runs per app before counting starts.
    pub warmup: u32,
    /// Pause after stopping or switching apps, in milliseconds.
    pub pause_ms: u64,
    /// How long an app sits idle in front before its memory is read.
    pub settle_ms: u64,
    /// Window over which idle CPU usage is measured.
    pub cpu_window_ms: u64,
    /// Idle memory/CPU samples per app.
    pub idle_samples: u32,
}

impl Default for BenchmarkOptions {
    fn default() -> Self {
        Self {
            runs: 30,
            warmup: 3,
            pause_ms: 2_000,
            settle_ms: 10_000,
            cpu_window_ms: 5_000,
            idle_samples: 3,
        }
    }
}

/// The full result. Serialized as the `data` of `rusttv performance benchmark --json`;
/// keep it with the raw runs (BENCHMARK_PLAN.md: preserve raw results).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BenchmarkReport {
    /// RustTV version that ran the benchmark.
    pub rusttv_version: String,
    /// The device and firmware the numbers belong to.
    pub device: BenchmarkDevice,
    /// Settings used.
    pub options: BenchmarkOptions,
    /// One summary per app, in the order given.
    pub apps: Vec<AppResult>,
    /// Every launch, including warm-up and discarded ones.
    pub runs: Vec<LaunchRun>,
}

/// Where the benchmark ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchmarkDevice {
    /// adb serial.
    pub serial: String,
    /// `ro.product.manufacturer`.
    pub manufacturer: Option<String>,
    /// `ro.product.model`.
    pub model: Option<String>,
    /// `ro.build.version.sdk`.
    pub api_level: Option<u32>,
    /// `ro.build.fingerprint`.
    pub build_fingerprint: Option<String>,
}

/// Summary for one app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppResult {
    /// The activity that was started.
    pub component: String,
    /// `TotalTime` of counted cold starts, in milliseconds.
    pub cold_start_ms: Option<Stats>,
    /// `TotalTime` of counted resumes, in milliseconds.
    pub resume_ms: Option<Stats>,
    /// Proportional set size while idle in front, in KiB.
    pub idle_pss_kib: Option<Stats>,
    /// CPU used while idle in front, % of one core, all of the app's processes.
    pub idle_cpu_percent: Option<Stats>,
    /// Runs left out of the statistics (wrong launch state, process didn't stop).
    pub discarded_runs: u32,
}

/// Summary statistics. Percentiles use the nearest-rank method.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stats {
    /// Number of values.
    pub n: usize,
    /// Smallest value.
    pub min: f64,
    /// 10th percentile.
    pub p10: f64,
    /// Median (50th percentile).
    pub median: f64,
    /// 90th percentile.
    pub p90: f64,
    /// Largest value.
    pub max: f64,
    /// Interquartile range (p75 - p25): the spread of the middle half.
    pub iqr: f64,
}

impl Stats {
    /// Summarizes `values`; `None` when there are none.
    pub fn from_values(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let rank = |percent: f64| {
            // Nearest rank: the smallest value with at least `percent`% of
            // the data at or below it.
            let index = (percent / 100.0 * sorted.len() as f64).ceil() as usize;
            sorted[index.clamp(1, sorted.len()) - 1]
        };
        Some(Self {
            n: sorted.len(),
            min: sorted[0],
            p10: rank(10.0),
            median: rank(50.0),
            p90: rank(90.0),
            max: sorted[sorted.len() - 1],
            iqr: rank(75.0) - rank(25.0),
        })
    }
}

/// What a launch run measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchKind {
    /// Process started from scratch.
    Cold,
    /// Existing activity brought back to the front.
    Resume,
}

/// One `am start -W`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchRun {
    /// The activity started.
    pub component: String,
    /// What this run was meant to measure.
    pub kind: LaunchKind,
    /// 0-based round; rounds below `options.warmup` are warm-up.
    pub round: u32,
    /// Android's `LaunchState` (`COLD`, `WARM`, `HOT`, `UNKNOWN (0)`), if reported.
    pub launch_state: Option<String>,
    /// Android's `TotalTime`, in milliseconds.
    pub total_time_ms: Option<u32>,
    /// Android's `WaitTime`, in milliseconds.
    pub wait_time_ms: Option<u32>,
    /// Whether the run is in the statistics.
    pub counted: bool,
    /// Why it isn't, when it isn't and wasn't warm-up.
    pub discarded_because: Option<String>,
}

/// Parsed `am start -W` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmStart {
    /// `LaunchState`, e.g. `COLD`.
    pub launch_state: Option<String>,
    /// `TotalTime` in milliseconds.
    pub total_time_ms: Option<u32>,
    /// `WaitTime` in milliseconds.
    pub wait_time_ms: Option<u32>,
}

/// Parses `am start -W` output.
///
/// `am start` exits with status 0 even when it fails ("Error type 3 ...
/// does not exist"), so success is `Status: ok` in the output. Returns the
/// error text otherwise.
pub fn parse_am_start(output: &str) -> Result<AmStart, String> {
    let field = |name: &str| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(name)?
                .strip_prefix(':')
                .map(str::trim)
        })
    };
    if field("Status") != Some("ok") {
        // `Error type 3` comes first; the `Error: ...` line says what's wrong.
        let error = output
            .lines()
            .find(|line| line.starts_with("Error:"))
            .or_else(|| output.lines().find(|line| line.starts_with("Error")))
            .unwrap_or("no `Status: ok` in `am start` output");
        return Err(error.trim().to_owned());
    }
    Ok(AmStart {
        launch_state: field("LaunchState").map(str::to_owned),
        total_time_ms: field("TotalTime").and_then(|value| value.parse().ok()),
        wait_time_ms: field("WaitTime").and_then(|value| value.parse().ok()),
    })
}

/// Parses toybox `TIME+` (`m:ss.cc`, or `h:mm:ss.cc` for long runners) into seconds.
///
/// Parts must be plain decimals: `f64::from_str` would also accept `inf`,
/// `NaN`, `1e3` and `+5`.
pub fn parse_cpu_time(text: &str) -> Option<f64> {
    let is_decimal = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
    };
    text.split(':')
        .try_fold(0.0, |total, part| {
            Some(total * 60.0 + part.parse::<f64>().ok().filter(|_| is_decimal(part))?)
        })
        .filter(|seconds: &f64| seconds.is_finite())
}

/// Total CPU seconds of an app's processes (`package` and `package:*`) in
/// `ps -A -o PID,TIME+,NAME` output.
pub fn app_cpu_seconds(ps_output: &str, package: &str) -> f64 {
    ps_output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let _pid = columns.next()?;
            let time = parse_cpu_time(columns.next()?)?;
            let name = columns.next()?;
            let belongs = name == package
                || name
                    .strip_prefix(package)
                    .is_some_and(|rest| rest.starts_with(':'));
            belongs.then_some(time)
        })
        .sum()
}

/// A benchmark couldn't run.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct BenchmarkError {
    kind: BenchmarkErrorKind,
}

#[derive(Debug, thiserror::Error)]
enum BenchmarkErrorKind {
    #[error(transparent)]
    Adb(#[from] AdbError),
    #[error("`am start` could not start `{component}`: {message}")]
    LaunchFailed { component: String, message: String },
}

impl<E: Into<BenchmarkErrorKind>> From<E> for BenchmarkError {
    fn from(kind: E) -> Self {
        Self { kind: kind.into() }
    }
}

impl BenchmarkError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match &self.kind {
            BenchmarkErrorKind::Adb(error) => error.code(),
            BenchmarkErrorKind::LaunchFailed { .. } => "LAUNCH_FAILED",
        }
    }

    /// The device could not be reached.
    pub fn is_device_unreachable(&self) -> bool {
        match &self.kind {
            BenchmarkErrorKind::Adb(error) => error.is_device_unreachable(),
            BenchmarkErrorKind::LaunchFailed { .. } => false,
        }
    }
}

/// How the runner waits; tests pass a no-op.
pub type Sleep<'a> = &'a dyn Fn(Duration);

/// Receives a line of progress for people (e.g. "cold 3/33 ...").
pub type Progress<'a> = &'a mut dyn FnMut(&str);

/// Runs the benchmark. See the module docs for what is measured.
pub fn run_benchmark(
    adb: &dyn Adb,
    serial: &str,
    apps: &[Component],
    options: BenchmarkOptions,
    sleep: Sleep<'_>,
    progress: Progress<'_>,
) -> Result<BenchmarkReport, BenchmarkError> {
    let info = inspect(adb, serial)?;
    let pause = Duration::from_millis(options.pause_ms);
    let rounds = options.warmup + options.runs;
    let mut runs = Vec::new();

    for round in 0..rounds {
        for app in apps {
            wake_if_needed(adb, serial, sleep, pause, progress)?;
            adb.run(serial, &DeviceCommand::ForceStop(app.package()))?;
            sleep(pause);
            let stopped = !is_running(adb, serial, app)?;
            let mut run = launch(adb, serial, app, LaunchKind::Cold, round, options)?;
            if !stopped && round >= options.warmup {
                run.counted = false;
                run.discarded_because =
                    Some("process was still running after force-stop".to_owned());
            }
            progress(&describe(&run, rounds));
            runs.push(run);
        }
    }

    // Resume: with every app in memory, each start brings back an app that
    // another one had covered.
    if apps.len() >= 2 {
        for round in 0..rounds {
            for app in apps {
                wake_if_needed(adb, serial, sleep, pause, progress)?;
                let run = launch(adb, serial, app, LaunchKind::Resume, round, options)?;
                progress(&describe(&run, rounds));
                runs.push(run);
                sleep(pause);
            }
        }
    }

    let mut idle: Vec<(Vec<f64>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); apps.len()];
    for sample in 0..options.idle_samples {
        for (app, (pss, cpu)) in apps.iter().zip(&mut idle) {
            wake_if_needed(adb, serial, sleep, pause, progress)?;
            let started = adb.run(serial, &DeviceCommand::StartActivity(app.clone()))?;
            parse_am_start(&started).map_err(|message| launch_failed(app, message))?;
            sleep(Duration::from_millis(options.settle_ms));
            let memory = adb.run(serial, &DeviceCommand::PackageMemInfo(app.package()))?;
            let before = adb.run(serial, &DeviceCommand::ProcessCpuTimes)?;
            sleep(Duration::from_millis(options.cpu_window_ms));
            let after = adb.run(serial, &DeviceCommand::ProcessCpuTimes)?;
            let package = app.package();
            let used = app_cpu_seconds(&after, package.as_str())
                - app_cpu_seconds(&before, package.as_str());
            let label = format!("idle {}/{} {app}", sample + 1, options.idle_samples);
            // Checked once, after the window: the screensaver doesn't stop by
            // itself, so if it is off now it was off for the whole sample.
            if !is_awake(adb, serial)? {
                progress(&format!("{label} (discarded: screensaver came on)"));
                continue;
            }
            if let Some((pss_kib, _)) = parse_app_memory(&memory) {
                pss.push(pss_kib as f64);
            }
            if options.cpu_window_ms > 0 && used >= 0.0 {
                cpu.push(used * 100_000.0 / options.cpu_window_ms as f64);
            }
            progress(&label);
        }
    }

    let results = apps
        .iter()
        .zip(idle)
        .map(|(app, (pss, cpu))| {
            let component = app.to_string();
            let times = |kind: LaunchKind| {
                let values: Vec<f64> = runs
                    .iter()
                    .filter(|run| run.component == component && run.kind == kind && run.counted)
                    .filter_map(|run| run.total_time_ms.map(f64::from))
                    .collect();
                Stats::from_values(&values)
            };
            let discarded = runs
                .iter()
                .filter(|run| run.component == component && run.discarded_because.is_some())
                .count();
            AppResult {
                cold_start_ms: times(LaunchKind::Cold),
                resume_ms: times(LaunchKind::Resume),
                idle_pss_kib: Stats::from_values(&pss),
                idle_cpu_percent: Stats::from_values(&cpu),
                discarded_runs: u32::try_from(discarded).unwrap_or(u32::MAX),
                component,
            }
        })
        .collect();

    Ok(BenchmarkReport {
        rusttv_version: crate::VERSION.to_owned(),
        device: BenchmarkDevice {
            serial: serial.to_owned(),
            manufacturer: info.manufacturer,
            model: info.model,
            api_level: info.api_level,
            build_fingerprint: info.build_fingerprint,
        },
        options,
        apps: results,
        runs,
    })
}

fn launch(
    adb: &dyn Adb,
    serial: &str,
    app: &Component,
    kind: LaunchKind,
    round: u32,
    options: BenchmarkOptions,
) -> Result<LaunchRun, BenchmarkError> {
    let output = adb.run(serial, &DeviceCommand::StartActivity(app.clone()))?;
    let parsed = parse_am_start(&output).map_err(|message| launch_failed(app, message))?;
    let expected = match kind {
        LaunchKind::Cold => "COLD",
        LaunchKind::Resume => "HOT",
    };
    let warmup = round < options.warmup;
    let discarded_because = match parsed.launch_state.as_deref() {
        _ if warmup => None,
        Some(state) if state == expected => None,
        Some(state) => Some(format!("launch state was {state}, expected {expected}")),
        None => Some("Android reported no launch state".to_owned()),
    };
    Ok(LaunchRun {
        component: app.to_string(),
        kind,
        round,
        counted: !warmup && discarded_because.is_none() && parsed.total_time_ms.is_some(),
        launch_state: parsed.launch_state,
        total_time_ms: parsed.total_time_ms,
        wait_time_ms: parsed.wait_time_ms,
        discarded_because,
    })
}

/// `mWakefulness` from `dumpsys power`: `Awake`, `Dreaming` (screensaver),
/// `Dozing` or `Asleep`.
pub fn parse_wakefulness(output: &str) -> Option<&str> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("mWakefulness="))
        .map(str::trim)
}

/// Whether the screen is on with no screensaver. Output without the field
/// (a different Android) counts as awake: the start timeout still applies.
fn is_awake(adb: &dyn Adb, serial: &str) -> Result<bool, AdbError> {
    let output = adb.run(serial, &DeviceCommand::PowerState)?;
    Ok(parse_wakefulness(&output).is_none_or(|state| state == "Awake"))
}

/// Ends the screensaver (or wakes the screen) before an app start: `am start
/// -W` waits for a first frame that never comes while it covers the app.
fn wake_if_needed(
    adb: &dyn Adb,
    serial: &str,
    sleep: Sleep<'_>,
    pause: Duration,
    progress: Progress<'_>,
) -> Result<(), AdbError> {
    if !is_awake(adb, serial)? {
        progress("screensaver or sleep: waking the TV");
        adb.run(serial, &DeviceCommand::WakeUp)?;
        sleep(pause);
    }
    Ok(())
}

fn launch_failed(app: &Component, message: String) -> BenchmarkError {
    BenchmarkErrorKind::LaunchFailed {
        component: app.to_string(),
        message,
    }
    .into()
}

fn is_running(adb: &dyn Adb, serial: &str, app: &Component) -> Result<bool, AdbError> {
    let package = app.package();
    let processes = parse_process_list(&adb.run(serial, &DeviceCommand::ProcessList)?);
    Ok(processes
        .iter()
        .any(|process| process.name == package.as_str()))
}

fn describe(run: &LaunchRun, rounds: u32) -> String {
    let kind = match run.kind {
        LaunchKind::Cold => "cold",
        LaunchKind::Resume => "resume",
    };
    let time = run
        .total_time_ms
        .map_or_else(|| "-".to_owned(), |ms| format!("{ms} ms"));
    let note = match (&run.discarded_because, run.counted) {
        (Some(reason), _) => format!(" (discarded: {reason})"),
        (None, false) => " (warm-up)".to_owned(),
        (None, true) => String::new(),
    };
    format!(
        "{kind} {}/{rounds} {}: {time}{note}",
        run.round + 1,
        run.component
    )
}

/// One error of every kind; see `crate::error::every_adb_error`.
#[cfg(test)]
pub(crate) fn every_benchmark_error() -> Vec<BenchmarkError> {
    let all: Vec<BenchmarkError> = vec![
        BenchmarkErrorKind::LaunchFailed {
            component: "a.b/.C".to_owned(),
            message: "Error type 3".to_owned(),
        }
        .into(),
    ];
    for error in &all {
        match error.kind {
            BenchmarkErrorKind::Adb(_) | BenchmarkErrorKind::LaunchFailed { .. } => {}
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adb::FakeAdb;
    use crate::command::PackageName;

    macro_rules! capture {
        ($file:literal) => {
            include_str!(concat!("fixtures/google-tv-emulator-api34/", $file))
        };
    }

    const SPIKE: &str = "ai.rusttv.spike/.LauncherActivity";
    const STOCK: &str = "com.google.android.apps.tv.launcherx/.home.HomeActivity";

    fn options() -> BenchmarkOptions {
        BenchmarkOptions {
            runs: 3,
            warmup: 1,
            idle_samples: 2,
            cpu_window_ms: 1_000,
            ..BenchmarkOptions::default()
        }
    }

    fn no_sleep(_: Duration) {}

    const AWAKE: &str = include_str!("fixtures/philips-65oled818-api34/dumpsys-power-head.txt");

    fn dreaming() -> String {
        AWAKE.replace("mWakefulness=Awake", "mWakefulness=Dreaming")
    }

    /// A device where `apps` cold-start (4 rounds), then resume HOT.
    fn device(apps: &[&str], processes_after_stop: &str) -> FakeAdb {
        let rounds = 4;
        let mut adb = FakeAdb::new()
            .respond(
                DeviceCommand::GetProperties,
                "[ro.product.model]: [Test TV]\n[ro.build.version.sdk]: [34]\n",
            )
            .respond(DeviceCommand::MemInfo, "")
            .respond(DeviceCommand::DataStorage, "")
            .respond(DeviceCommand::ProcessList, processes_after_stop)
            .respond(DeviceCommand::PowerState, AWAKE)
            // Each idle sample reads CPU times before and after its window:
            // 0.05 s used every time.
            .respond_sequence(
                DeviceCommand::ProcessCpuTimes,
                std::iter::repeat_n(
                    [
                        "  PID     TIME+ NAME\n 100   0:01.00 ai.rusttv.spike\n".to_owned(),
                        "  PID     TIME+ NAME\n 100   0:01.05 ai.rusttv.spike\n".to_owned(),
                    ],
                    16,
                )
                .flatten(),
            );
        for app in apps {
            let component: Component = app.parse().unwrap();
            let starts =
                std::iter::repeat_n(capture!("am-start-cold.txt").to_owned(), rounds).chain(
                    std::iter::repeat_n(capture!("am-start-hot.txt").to_owned(), 64),
                );
            adb = adb
                .respond(DeviceCommand::ForceStop(component.package()), "")
                .respond(
                    DeviceCommand::PackageMemInfo(component.package()),
                    include_str!("fixtures/emulator-pixel3a-api34/meminfo-launcher.txt"),
                )
                .respond_sequence(DeviceCommand::StartActivity(component), starts);
        }
        adb
    }

    fn run(adb: &FakeAdb, apps: &[&str]) -> BenchmarkReport {
        let apps: Vec<Component> = apps.iter().map(|app| app.parse().unwrap()).collect();
        run_benchmark(adb, "tv", &apps, options(), &no_sleep, &mut |_| {}).unwrap()
    }

    #[test]
    fn am_start_cold_hot_and_already_in_front() {
        let cold = parse_am_start(capture!("am-start-cold.txt")).unwrap();
        assert_eq!(cold.launch_state.as_deref(), Some("COLD"));
        assert_eq!(
            (cold.total_time_ms, cold.wait_time_ms),
            (Some(281), Some(287))
        );

        let hot = parse_am_start(capture!("am-start-hot.txt")).unwrap();
        assert_eq!(hot.launch_state.as_deref(), Some("HOT"));

        let top = parse_am_start(capture!("am-start-already-top.txt")).unwrap();
        assert_eq!(top.launch_state.as_deref(), Some("UNKNOWN (0)"));
    }

    #[test]
    fn am_start_error_is_an_error_even_though_am_exits_0() {
        let error = parse_am_start(capture!("am-start-error.txt")).unwrap_err();
        assert!(error.contains("does not exist"), "{error}");
    }

    #[test]
    fn wakefulness_from_dumpsys_power() {
        assert_eq!(parse_wakefulness(AWAKE), Some("Awake"));
        assert_eq!(parse_wakefulness(&dreaming()), Some("Dreaming"));
        assert_eq!(parse_wakefulness("POWER MANAGER (dumpsys power)\n"), None);
    }

    #[test]
    fn the_screensaver_is_ended_before_a_start() {
        // Screensaver on at the first check, then awake for the rest.
        let adb = device(&[SPIKE, STOCK], "  PID    RSS NAME\n")
            .respond_sequence(DeviceCommand::PowerState, [dreaming(), AWAKE.to_owned()])
            .respond(DeviceCommand::WakeUp, "");
        let mut lines = Vec::new();
        let apps: Vec<Component> = [SPIKE, STOCK]
            .iter()
            .map(|app| app.parse().unwrap())
            .collect();
        let report = run_benchmark(&adb, "tv", &apps, options(), &no_sleep, &mut |line| {
            lines.push(line.to_owned());
        })
        .unwrap();

        let calls = adb.calls();
        let woke = calls.iter().position(|call| *call == DeviceCommand::WakeUp);
        let first_stop = calls
            .iter()
            .position(|call| matches!(call, DeviceCommand::ForceStop(_)));
        assert!(
            woke.unwrap() < first_stop.unwrap(),
            "woken before the first start"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == DeviceCommand::WakeUp)
                .count(),
            1
        );
        assert_eq!(lines[0], "screensaver or sleep: waking the TV");
        assert_eq!(report.apps[0].cold_start_ms.unwrap().n, 3);
    }

    #[test]
    fn an_idle_sample_taken_under_the_screensaver_is_dropped() {
        // 2 apps x 4 rounds x (cold + resume) checks are awake, then each idle
        // sample checks before its start and after its window: the
        // screensaver comes on during the very first idle window only.
        let before_idle = 2 * 4 * 2;
        let states = std::iter::repeat_n(AWAKE.to_owned(), before_idle + 1)
            .chain([dreaming()])
            .chain([AWAKE.to_owned()]);
        let adb = device(&[SPIKE, STOCK], "  PID    RSS NAME\n")
            .respond_sequence(DeviceCommand::PowerState, states)
            .respond(DeviceCommand::WakeUp, "");
        let report = run(&adb, &[SPIKE, STOCK]);

        assert_eq!(
            report.apps[0].idle_pss_kib.unwrap().n,
            1,
            "one of two dropped"
        );
        assert_eq!(report.apps[0].idle_cpu_percent.unwrap().n, 1);
        assert_eq!(report.apps[1].idle_pss_kib.unwrap().n, 2);
    }

    #[test]
    fn cpu_time_formats() {
        assert_eq!(parse_cpu_time("0:01.79"), Some(1.79));
        assert_eq!(parse_cpu_time("2:03.50"), Some(123.5));
        assert_eq!(parse_cpu_time("1:00:00.00"), Some(3600.0));
        assert_eq!(parse_cpu_time("TIME+"), None);
    }

    #[test]
    fn app_cpu_includes_subprocesses_but_not_similar_names() {
        let ps =
            "  PID     TIME+ NAME\n 1 0:01.00 com.a\n 2 0:00.50 com.a:service\n 3 0:09.00 com.ab\n";
        assert_eq!(app_cpu_seconds(ps, "com.a"), 1.5);
        // Captured after launcherx was force-stopped: only its
        // `:coreservices` process (0:00.08) is left.
        let captured = app_cpu_seconds(
            capture!("ps-time.txt"),
            "com.google.android.apps.tv.launcherx",
        );
        assert!((captured - 0.08).abs() < 1e-9, "{captured}");
    }

    #[test]
    fn stats_use_nearest_rank() {
        let values: Vec<f64> = (1..=10).map(f64::from).collect();
        let stats = Stats::from_values(&values).unwrap();
        assert_eq!((stats.n, stats.min, stats.max), (10, 1.0, 10.0));
        assert_eq!((stats.p10, stats.median, stats.p90), (1.0, 5.0, 9.0));
        assert_eq!(stats.iqr, 8.0 - 3.0);
        assert_eq!(Stats::from_values(&[]), None);
        assert_eq!(Stats::from_values(&[7.0]).unwrap().median, 7.0);
    }

    #[test]
    fn two_apps_get_cold_resume_and_idle_numbers() {
        let report = run(
            &device(&[SPIKE, STOCK], "  PID    RSS NAME\n"),
            &[SPIKE, STOCK],
        );

        assert_eq!(report.device.model.as_deref(), Some("Test TV"));
        assert_eq!(report.apps.len(), 2);
        let spike = &report.apps[0];
        let cold = spike.cold_start_ms.unwrap();
        assert_eq!((cold.n, cold.median), (3, 281.0), "warm-up run excluded");
        let resume = spike.resume_ms.unwrap();
        assert_eq!((resume.n, resume.median), (3, 33.0));
        assert_eq!(spike.idle_pss_kib.unwrap().median, 44_081.0);
        // 0.05 s of CPU in a 1 s window.
        assert!((spike.idle_cpu_percent.unwrap().median - 5.0).abs() < 1e-9);
        assert_eq!(spike.discarded_runs, 0);
        // 2 apps x 4 rounds x (cold + resume).
        assert_eq!(report.runs.len(), 16);
    }

    #[test]
    fn apps_are_interleaved() {
        let report = run(
            &device(&[SPIKE, STOCK], "  PID    RSS NAME\n"),
            &[SPIKE, STOCK],
        );
        let order: Vec<&str> = report
            .runs
            .iter()
            .take(4)
            .map(|run| run.component.as_str())
            .collect();
        assert_eq!(order, [SPIKE, STOCK, SPIKE, STOCK]);
    }

    #[test]
    fn one_app_has_no_resume_measurement() {
        let report = run(&device(&[SPIKE], "  PID    RSS NAME\n"), &[SPIKE]);
        assert_eq!(report.apps[0].resume_ms, None);
        assert_eq!(report.apps[0].cold_start_ms.unwrap().n, 3);
    }

    #[test]
    fn a_process_that_survives_force_stop_is_not_a_cold_start() {
        // E.g. the system restarts the current Home app right away.
        let still_running = "  PID    RSS NAME\n 2620 158000 ai.rusttv.spike\n";
        let report = run(&device(&[SPIKE], still_running), &[SPIKE]);
        assert_eq!(report.apps[0].cold_start_ms, None);
        assert_eq!(report.apps[0].discarded_runs, 3);
        assert!(
            report.runs.iter().all(|run| !run.counted),
            "no cold run may count"
        );
    }

    #[test]
    fn wrong_launch_state_is_discarded_not_counted() {
        // Every "cold" start reports HOT: nothing may count as cold.
        let component: Component = SPIKE.parse().unwrap();
        let adb = device(&[], "  PID    RSS NAME\n")
            .respond(DeviceCommand::ForceStop(component.package()), "")
            .respond(DeviceCommand::PackageMemInfo(component.package()), "")
            .respond(
                DeviceCommand::StartActivity(component),
                capture!("am-start-hot.txt"),
            );
        let report = run(&adb, &[SPIKE]);
        assert_eq!(report.apps[0].cold_start_ms, None);
        assert!(
            report.runs[1]
                .discarded_because
                .as_deref()
                .is_some_and(|reason| reason.contains("HOT"))
        );
    }

    #[test]
    fn an_app_that_cannot_start_stops_the_benchmark() {
        let component: Component = "ai.rusttv.spike/.Nope".parse().unwrap();
        let adb = device(&[], "  PID    RSS NAME\n")
            .respond(
                DeviceCommand::ForceStop(PackageName::new("ai.rusttv.spike").unwrap()),
                "",
            )
            .respond(
                DeviceCommand::StartActivity(component.clone()),
                capture!("am-start-error.txt"),
            );
        let error =
            run_benchmark(&adb, "tv", &[component], options(), &no_sleep, &mut |_| {}).unwrap_err();
        assert_eq!(error.code(), "LAUNCH_FAILED");
    }
}

#[cfg(test)]
mod odd_input {
    use proptest::prelude::*;

    use super::*;
    use crate::test_support::{adb_output, line_ending};

    proptest! {
        #[test]
        fn parsers_never_panic(output in adb_output(), package in "[a-z.]{0,12}") {
            let _ = parse_am_start(&output);
            if let Some(seconds) = parse_cpu_time(&output) {
                prop_assert!(seconds.is_finite() && seconds >= 0.0);
            }
            prop_assert!(app_cpu_seconds(&output, &package) >= 0.0);
        }

        #[test]
        fn am_start_parses_back_to_its_fields(
            state in "(COLD|WARM|HOT|UNKNOWN \\(0\\))",
            total in any::<u32>(),
            wait in any::<u32>(),
            ending in line_ending(),
        ) {
            let output = format!(
                "Starting: Intent {{ cmp=a.b/.C }}{ending}Status: ok{ending}LaunchState: {state}{ending}Activity: a.b/.C{ending}TotalTime: {total}{ending}WaitTime: {wait}{ending}Complete{ending}"
            );
            prop_assert_eq!(
                parse_am_start(&output),
                Ok(AmStart { launch_state: Some(state), total_time_ms: Some(total), wait_time_ms: Some(wait) })
            );
        }

        #[test]
        fn cpu_time_parses_back_to_seconds(hours in 0..1000_u32, minutes in 0..60_u32, centis in 0..6000_u32) {
            let seconds = format!("{}.{:02}", centis / 100, centis % 100);
            let with_hours = format!("{hours}:{minutes:02}:{seconds:0>5}");
            let expected = f64::from(hours * 3600 + minutes * 60) + f64::from(centis) / 100.0;
            let parsed = parse_cpu_time(&with_hours).unwrap();
            prop_assert!((parsed - expected).abs() < 1e-6, "{with_hours}: {parsed} != {expected}");
        }

        #[test]
        fn app_cpu_seconds_sums_only_the_apps_processes(
            own in prop::collection::vec(0..100_000_u32, 0..5),
            others in prop::collection::vec(0..100_000_u32, 0..5),
            ending in line_ending(),
        ) {
            let row = |pid: usize, centis: &u32, name: &str| {
                format!("{pid:>5} {}:{:02}.{:02} {name}{ending}", centis / 6000, centis / 100 % 60, centis % 100)
            };
            let output: String = std::iter::once(format!("  PID     TIME+ NAME{ending}"))
                .chain(own.iter().enumerate().map(|(i, c)| row(i, c, if i % 2 == 0 { "a.b" } else { "a.b:remote" })))
                .chain(others.iter().enumerate().map(|(i, c)| row(i, c, if i % 2 == 0 { "a.bc" } else { "x.a.b" })))
                .collect();
            let expected = f64::from(own.iter().sum::<u32>()) / 100.0;
            prop_assert!((app_cpu_seconds(&output, "a.b") - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn empty_output_is_an_error_or_nothing() {
        assert!(parse_am_start("").is_err());
        assert_eq!(parse_cpu_time(""), None);
        assert!(app_cpu_seconds("", "a.b").abs() < f64::EPSILON);
    }

    #[test]
    fn am_start_with_crlf_and_overflowing_times() {
        let output =
            "Status: ok\r\nLaunchState: COLD\r\nTotalTime: 4294967296\r\nWaitTime: 12 ms\r\n";
        assert_eq!(
            parse_am_start(output),
            Ok(AmStart {
                launch_state: Some("COLD".to_owned()),
                total_time_ms: None,
                wait_time_ms: None
            })
        );
    }

    #[test]
    fn am_start_truncated_before_status_is_an_error() {
        assert!(parse_am_start("Starting: Intent { cmp=a.b/.C }\nStat").is_err());
        assert!(parse_am_start("Status: okay\n").is_err());
    }

    #[test]
    fn cpu_time_rejects_non_decimal_numbers() {
        // `f64::from_str` accepts these; toybox never prints them. `inf` made
        // an app's CPU time infinite.
        for text in [
            "inf", "NaN", "1:inf", "1e3", "0:1e2", "+5", "1:-0.5", "", "1:", ":5",
        ] {
            assert_eq!(parse_cpu_time(text), None, "{text}");
        }
        // Digits too long for f64 overflow to infinity.
        assert_eq!(parse_cpu_time(&"9".repeat(400)), None);
    }

    #[test]
    fn ps_time_with_infinite_row_does_not_poison_the_sum() {
        let output = "  PID     TIME+ NAME\n    1   0:01.50 a.b\n    2   inf a.b:x\n";
        assert!((app_cpu_seconds(output, "a.b") - 1.5).abs() < 1e-9);
    }
}

//! `rusttv performance ...` commands.

use std::time::Duration;

use rusttv_core::{Adb, BenchmarkOptions, BenchmarkReport, Component, PerformanceSnapshot, Stats};

use crate::{CliError, Target, output};

/// `rusttv performance snapshot`.
pub fn snapshot(
    adb: &dyn Adb,
    target: &Target,
    sample_ms: u64,
    json: bool,
) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let snapshot = rusttv_core::snapshot(adb, &serial, Duration::from_millis(sample_ms))?;
    if json {
        output::print_json(&snapshot);
    } else {
        print_summary(&snapshot);
    }
    Ok(())
}

fn print_summary(snapshot: &PerformanceSnapshot) {
    let unknown = || "unknown".to_owned();
    let cpu = snapshot.cpu.map_or_else(unknown, |cpu| {
        format!("{:.1}% busy over {} ms", cpu.busy_percent, cpu.sample_ms)
    });
    let memory = snapshot
        .memory
        .map_or_else(unknown, |memory| match memory.available_kib {
            Some(available) => format!(
                "{} total, {} available",
                output::gib(memory.total_kib),
                output::gib(available)
            ),
            None => format!("{} total", output::gib(memory.total_kib)),
        });
    let launcher = snapshot.home_app.as_ref().map_or_else(unknown, |home| {
        let usage = match (home.pss_kib, home.rss_kib) {
            (Some(pss), Some(rss)) => format!("{} PSS, {} RSS", output::mib(pss), output::mib(rss)),
            (Some(pss), None) => format!("{} PSS", output::mib(pss)),
            _ => "not running".to_owned(),
        };
        format!("{} ({usage})", home.package)
    });

    println!("{:10} {}", "Device", snapshot.serial);
    println!(
        "{:10} {}",
        "Build",
        snapshot.build_fingerprint.clone().unwrap_or_else(unknown)
    );
    println!("{:10} {cpu}", "CPU");
    println!("{:10} {memory}", "Memory");
    println!("{:10} {launcher}", "Launcher");
    println!("\nTop processes by resident memory:");
    for process in &snapshot.top_processes {
        println!(
            "  {:>10}  {} (pid {})",
            output::mib(process.rss_kib),
            process.name,
            process.pid
        );
    }
}

/// `rusttv performance benchmark`. Progress goes to stderr; results to stdout.
pub fn benchmark(
    adb: &dyn Adb,
    target: &Target,
    apps: &[Component],
    options: BenchmarkOptions,
    json: bool,
) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    output::print_note(&format!(
        "Benchmarking {} app(s) on {serial}: this stops and starts them repeatedly \
         and changes no settings. Leave the TV alone until it finishes.",
        apps.len()
    ));
    let mut progress = |line: &str| output::print_note(line);
    let report = rusttv_core::run_benchmark(
        adb,
        &serial,
        apps,
        options,
        &std::thread::sleep,
        &mut progress,
    )?;
    if json {
        output::print_json(&report);
    } else {
        print_report(&report);
    }
    Ok(())
}

fn print_report(report: &BenchmarkReport) {
    let device = &report.device;
    println!(
        "{} {} (API {}), build {}",
        device.manufacturer.as_deref().unwrap_or("unknown"),
        device.model.as_deref().unwrap_or("unknown"),
        device
            .api_level
            .map_or_else(|| "?".to_owned(), |api| api.to_string()),
        device.build_fingerprint.as_deref().unwrap_or("unknown"),
    );
    let stats = |value: Option<Stats>, unit: &str, scale: f64| {
        value.map_or_else(
            || "not measured".to_owned(),
            |s| {
                format!(
                    "median {:.1}{unit}  (p10 {:.1}, p90 {:.1}, IQR {:.1}, n={})",
                    s.median / scale,
                    s.p10 / scale,
                    s.p90 / scale,
                    s.iqr / scale,
                    s.n
                )
            },
        )
    };
    for app in &report.apps {
        println!("\n{}", app.component);
        println!(
            "  {:12} {}",
            "cold start",
            stats(app.cold_start_ms, " ms", 1.0)
        );
        println!("  {:12} {}", "resume", stats(app.resume_ms, " ms", 1.0));
        println!(
            "  {:12} {}",
            "idle PSS",
            stats(app.idle_pss_kib, " MiB", 1024.0)
        );
        println!(
            "  {:12} {}",
            "idle CPU",
            stats(app.idle_cpu_percent, "%", 1.0)
        );
        if app.discarded_runs > 0 {
            println!(
                "  {} run(s) discarded; see --json for reasons",
                app.discarded_runs
            );
        }
    }
    println!(
        "\nCompare only results from the same device and build. Keep the raw runs: rerun with --json > file.json."
    );
}

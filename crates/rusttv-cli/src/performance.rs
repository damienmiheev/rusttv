//! `rusttv performance ...` commands.

use std::time::Duration;

use rusttv_core::{Adb, PerformanceSnapshot};

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

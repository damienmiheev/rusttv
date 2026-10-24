//! `rusttv tools ...`: experimental tools (TOOLS.md).

use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusttv_core::{AIRPLAY_CAVEAT, AIRPLAY_TOOL, Adb, AirplayStatus, BonjourCheck, FileJournal};

use crate::launcher::{
    Approver, ChangeReport, approve_and_apply, confirm, note_journal, require_terminal, show_plan,
};
use crate::{CliError, Target, output};

/// A host browse tool: program, fixed arguments, output parser.
type BrowseTool = (
    &'static str,
    &'static [&'static str],
    fn(&str) -> Vec<String>,
);

/// How long `--check-bonjour` browses the network.
const BONJOUR_BROWSE: Duration = Duration::from_secs(5);

/// `rusttv tools airplay status`.
pub fn airplay_status(
    adb: &dyn Adb,
    target: &Target,
    check_bonjour: bool,
    json: bool,
) -> Result<(), CliError> {
    output::print_note(AIRPLAY_CAVEAT);
    let serial = target.resolve(adb)?;
    let mut status = rusttv_core::airplay_status(
        adb,
        &serial,
        &rusttv_core::bundled_profiles(),
        &rusttv_core::journals_dir()?,
    )?;
    if check_bonjour {
        status.bonjour = Some(browse_airplay());
    }
    if json {
        output::print_json(&status);
    } else {
        print_status(&status);
    }
    Ok(())
}

fn print_status(status: &AirplayStatus) {
    let or_unset = |value: &Option<String>| value.clone().unwrap_or_else(|| "unset".to_owned());
    let yes_no = |value: bool| if value { "yes" } else { "no" };
    match &status.reason {
        None => println!(
            "Supported: yes ({})",
            status.profile.as_deref().unwrap_or("-")
        ),
        Some(reason) => println!("Supported: no: {reason}"),
    }
    for package in &status.packages {
        println!(
            "  {:36} {}",
            package.package,
            if package.installed {
                "installed"
            } else {
                "missing"
            }
        );
    }
    println!(
        "RustTV launcher (trigger activity): {}",
        if status.launcher_installed {
            "installed"
        } else {
            "not installed"
        }
    );
    if status.packages.iter().any(|package| !package.installed) {
        return;
    }
    println!(
        "setup_airplay_enable: {} (0 off, 1 on, 7 hidden by the TV maker)",
        or_unset(&status.setup_airplay_enable)
    );
    println!(
        "show_airplay_setting_ui: {}",
        or_unset(&status.show_airplay_setting_ui)
    );
    if let Some(running) = status.service_running {
        println!("BootupService running: {}", yes_no(running));
    }
    match &status.listening_ports {
        Some(ports) => {
            let ports: Vec<String> = ports.iter().map(u16::to_string).collect();
            println!("Listening TCP ports: {}", ports.join(", "));
        }
        None => println!("Listening TCP ports: unknown (`ss` is missing or failed)"),
    }
    match &status.log {
        Some(lines) if lines.is_empty() => println!("Log: no AirPlay lines"),
        Some(lines) => {
            println!("Log (newest last):");
            for line in lines {
                println!("  {line}");
            }
        }
        None => println!("Log: can't be read"),
    }
    if let Some(journal) = &status.journal {
        println!("Enabled by RustTV, journal: {journal} (undo: `rusttv tools airplay disable`)");
    }
    if let Some(bonjour) = &status.bonjour {
        match (&bonjour.tool, bonjour.instances.is_empty()) {
            (None, _) => {}
            (Some(tool), true) => println!("Bonjour ({tool}): no AirPlay receiver seen"),
            (Some(tool), false) => {
                println!("Bonjour ({tool}): {}", bonjour.instances.join(", "));
            }
        }
        if let Some(note) = &bonjour.note {
            println!("  {note}");
        }
    }
}

/// `rusttv tools airplay enable`.
pub fn airplay_enable(
    adb: &dyn Adb,
    target: &Target,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    // The plan's notes start with the caveat, so it isn't printed separately.
    let plan = rusttv_core::airplay_plan(
        adb,
        &serial,
        &rusttv_core::bundled_profiles(),
        &rusttv_core::journals_dir()?,
    )?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, &Approver::Terminal, AIRPLAY_TOOL, json)?;
    if !json {
        println!("\nDone: the AirPlay service runs; AirPlay is on until the TV restarts.");
        println!("Journal: {}", path.display());
        println!(
            "Check with `rusttv tools airplay status --check-bonjour`; undo with `rusttv tools \
             airplay disable`."
        );
    }
    Ok(())
}

/// `rusttv tools airplay disable`.
pub fn airplay_disable(
    adb: &dyn Adb,
    target: &Target,
    journal: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    output::print_note(AIRPLAY_CAVEAT);
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let (path, mut journal) = match journal {
        Some(path) => (path.to_owned(), rusttv_core::load_journal(path)?),
        None => {
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            let search =
                rusttv_core::find_airplay_journal(&rusttv_core::journals_dir()?, &fingerprint);
            for file in &search.unreadable {
                output::print_note(&format!(
                    "warning: skipped {}, which can't be read. If it is an AirPlay `enable` that \
                     still needs restoring, fix it and pass `--journal {0}`, or restart the TV. \
                     Reason: {}",
                    file.path.display(),
                    file.error
                ));
            }
            search.found?
        }
    };
    let plan = rusttv_core::airplay_restore_plan(adb, &serial, &journal)?;
    if !json {
        println!("Journal: {}\n", path.display());
    }
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;
    let mut store = FileJournal::new(&path);
    let left_as_is = rusttv_core::restore(adb, &plan, &approval, &mut journal, &mut store)
        .inspect_err(|_| note_journal(&path))?;
    for note in &left_as_is {
        output::print_note(note);
    }
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    } else {
        println!("\nDone: the AirPlay switch is back and the service is stopped.");
    }
    Ok(())
}

/// Browses `_airplay._tcp` with `dns-sd` (macOS) or `avahi-browse` (Linux)
/// for [`BONJOUR_BROWSE`]. Fixed arguments, no shell. Skipped with a note
/// when neither is installed.
fn browse_airplay() -> BonjourCheck {
    let tools: [BrowseTool; 2] = [
        (
            "dns-sd",
            &["-B", "_airplay._tcp", "local."],
            rusttv_core::parse_dns_sd_browse,
        ),
        (
            "avahi-browse",
            &["-t", "-p", "_airplay._tcp"],
            rusttv_core::parse_avahi_browse,
        ),
    ];
    for (tool, args, parse) in tools {
        let child = Command::new(tool)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let child = match child {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return BonjourCheck {
                    tool: Some(tool.to_owned()),
                    instances: Vec::new(),
                    note: Some(format!("could not run {tool}: {error}")),
                };
            }
        };
        return match output_after(child, BONJOUR_BROWSE) {
            Ok(text) => BonjourCheck {
                tool: Some(tool.to_owned()),
                instances: parse(&text),
                note: Some(format!(
                    "browsed for {} s; a receiver can take a few seconds to appear",
                    BONJOUR_BROWSE.as_secs()
                )),
            },
            Err(error) => BonjourCheck {
                tool: Some(tool.to_owned()),
                instances: Vec::new(),
                note: Some(format!("{tool} failed: {error}")),
            },
        };
    }
    BonjourCheck {
        tool: None,
        instances: Vec::new(),
        note: Some("skipped: neither dns-sd (macOS) nor avahi-browse (Linux) is installed".into()),
    }
}

/// Stdout of `child` once it exits, or after `limit` (then it is killed:
/// `dns-sd -B` never exits on its own).
fn output_after(mut child: Child, limit: Duration) -> io::Result<String> {
    let deadline = Instant::now() + limit;
    while child.try_wait()?.is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    // It may exit between the check and the kill; either way it's gone.
    let _ = child.kill();
    child.wait()?;
    let mut text = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_string(&mut text)?;
    }
    Ok(text)
}

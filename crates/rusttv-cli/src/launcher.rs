//! `rusttv launcher ...` commands.

use std::env;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttv_core::{
    Adb, Approval, FileJournal, HOME_TAKEOVER, JournalStatus, LauncherStatus, Plan, PlanError,
    journal_file_name,
};
use serde::Serialize;

use crate::{CliError, Target, output};

/// `rusttv launcher status`.
pub fn status(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let status = rusttv_core::launcher_status(adb, &serial)?;
    if json {
        output::print_json(&status);
    } else {
        print_status(&status);
    }
    Ok(())
}

fn print_status(status: &LauncherStatus) {
    println!(
        "Home button opens: {}",
        status.home.as_deref().unwrap_or("nothing")
    );
    let rusttv = match (status.rusttv_installed, status.rusttv_is_home) {
        (_, true) => "Home",
        (true, false) => "installed, not Home",
        (false, false) => "not installed",
    };
    println!("RustTV launcher:   {rusttv}");

    if status.candidates.is_empty() {
        return;
    }
    println!("\nPRIORITY  HOME ACTIVITY");
    for candidate in &status.candidates {
        println!("{:>8}  {}", candidate.priority, candidate.component);
    }
    let top = status.candidates[0].priority;
    if status.rusttv_installed
        && !status.rusttv_is_home
        && status
            .candidates
            .iter()
            .any(|candidate| candidate.priority > 0 && candidate.priority == top)
    {
        println!("\nA launcher with a higher priority always wins the Home button on this device.");
    }
}

/// Result of `launcher takeover` / `launcher restore` in `--json` output.
#[derive(Debug, Serialize)]
struct ChangeReport {
    /// The journal file recording the change.
    journal: String,
    status: JournalStatus,
}

/// `rusttv launcher takeover`.
pub fn takeover(adb: &dyn Adb, target: &Target, dry_run: bool, json: bool) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::takeover_plan(adb, &serial)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;

    let path = journals_dir()?.join(journal_file_name(now_unix_ms(), HOME_TAKEOVER));
    let mut store = FileJournal::new(&path);
    let journal = rusttv_core::apply(adb, &plan, &approval, &mut store)
        .inspect_err(|_| note_journal(&path))?;
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    } else {
        println!("\nDone: Home now opens the RustTV launcher.");
        println!("Journal: {}", path.display());
        println!("Undo with `rusttv launcher restore`.");
    }
    Ok(())
}

/// `rusttv launcher restore`.
pub fn restore(
    adb: &dyn Adb,
    target: &Target,
    journal: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let (path, mut journal) = match journal {
        Some(path) => (path.to_owned(), rusttv_core::load_journal(path)?),
        None => {
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            rusttv_core::find_restorable_journal(&journals_dir()?, HOME_TAKEOVER, &fingerprint)?
        }
    };
    let plan = rusttv_core::takeover_restore_plan(adb, &serial, &journal)?;
    if !json {
        println!("Journal: {}\n", path.display());
    }
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;

    let mut store = FileJournal::new(&path);
    rusttv_core::restore(adb, &plan, &approval, &mut journal, &mut store)
        .inspect_err(|_| note_journal(&path))?;
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    } else {
        println!("\nDone: the stock launcher is enabled again.");
        // Best effort: the restore itself already succeeded and was verified.
        if let Ok(status) = rusttv_core::launcher_status(adb, &serial) {
            println!(
                "Home button opens: {}",
                status.home.as_deref().unwrap_or("nothing")
            );
        }
    }
    Ok(())
}

/// Shows the plan: as the result with `--dry-run`, otherwise before the prompt.
/// With `--json`, only a dry run goes to stdout, which carries one JSON object.
fn show_plan(plan: &Plan, dry_run: bool, json: bool) {
    let dry = plan.dry_run();
    match (json, dry_run) {
        (true, true) => output::print_json(&dry),
        (true, false) => output::print_note(&dry.to_string()),
        (false, _) => print!("{dry}"),
    }
}

/// Approval comes only from a person typing at a terminal (SECURITY.md):
/// there is deliberately no `--yes` flag.
fn require_terminal() -> Result<(), CliError> {
    if io::stdin().is_terminal() {
        Ok(())
    } else {
        Err(PlanError::not_approved(
            "this changes the device and needs a person to approve it at an interactive \
             terminal; nothing changed (use --dry-run to only see the plan)",
        )
        .into())
    }
}

fn confirm(plan: &Plan) -> Result<Approval, CliError> {
    output::print_note(&format!(
        "\nType `{}` and press Enter to apply this plan; anything else cancels:",
        plan.confirmation()
    ));
    let mut typed = String::new();
    io::stdin().read_line(&mut typed).map_err(|error| {
        PlanError::not_approved(&format!("could not read the confirmation: {error}"))
    })?;
    Ok(plan.approve(&typed)?)
}

fn note_journal(path: &Path) {
    if path.exists() {
        output::print_note(&format!("Journal: {}", path.display()));
    }
}

/// `<state dir>/journals`. The state dir is `RUSTTV_STATE_DIR` if set, else
/// `~/Library/Application Support/rusttv` on macOS and
/// `$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`) elsewhere.
fn journals_dir() -> Result<PathBuf, CliError> {
    let state = if let Some(dir) = env::var_os("RUSTTV_STATE_DIR") {
        PathBuf::from(dir)
    } else {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(|| PlanError::journal("HOME is not set; set RUSTTV_STATE_DIR"))?;
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/rusttv")
        } else {
            env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|dir| dir.is_absolute())
                .unwrap_or_else(|| home.join(".local/state"))
                .join("rusttv")
        }
    };
    Ok(state.join("journals"))
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

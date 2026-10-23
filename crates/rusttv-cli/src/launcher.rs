//! `rusttv launcher ...` commands.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttv_core::{
    Adb, Approval, FileJournal, HOME_TAKEOVER, JournalStatus, LAUNCHER_INSTALL, LAUNCHER_UNINSTALL,
    LauncherStatus, Plan, PlanError, journal_file_name,
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

/// Result of `launcher takeover` / `install` / `uninstall` / `restore` and
/// `optimize apply` / `restore` in `--json` output.
#[derive(Debug, Serialize)]
pub(crate) struct ChangeReport {
    /// The journal file recording the change.
    pub(crate) journal: String,
    pub(crate) status: JournalStatus,
}

/// `rusttv launcher takeover`.
pub fn takeover(adb: &dyn Adb, target: &Target, dry_run: bool, json: bool) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::takeover_plan(adb, &serial, &rusttv_core::bundled_profiles())?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, HOME_TAKEOVER, json)?;
    if !json {
        println!("\nDone: Home now opens the RustTV launcher.");
        println!("Journal: {}", path.display());
        println!("Undo with `rusttv launcher restore`.");
    }
    Ok(())
}

/// `rusttv launcher install`.
pub fn install(
    adb: &dyn Adb,
    target: &Target,
    apk: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::install_plan(adb, &serial, apk, &rusttv_core::journals_dir()?)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, LAUNCHER_INSTALL, json)?;
    if !json {
        println!("\nDone: the RustTV launcher is installed.");
        println!("Journal: {}", path.display());
        println!("Remove it with `rusttv launcher uninstall`.");
    }
    Ok(())
}

/// `rusttv launcher uninstall`.
pub fn uninstall(
    adb: &dyn Adb,
    target: &Target,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::uninstall_plan(adb, &serial, &rusttv_core::journals_dir()?)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, LAUNCHER_UNINSTALL, json)?;
    if !json {
        println!("\nDone: the RustTV launcher is uninstalled.");
        println!("Journal: {}", path.display());
    }
    Ok(())
}

/// Asks for the typed confirmation, then applies `plan` with a new journal
/// named after `name`. With `--json`, prints the [`ChangeReport`].
pub(crate) fn approve_and_apply(
    adb: &dyn Adb,
    plan: &Plan,
    name: &str,
    json: bool,
) -> Result<PathBuf, CliError> {
    let approval = confirm(plan)?;
    let path = rusttv_core::journals_dir()?.join(journal_file_name(now_unix_ms(), name));
    let mut store = FileJournal::new(&path);
    let journal = rusttv_core::apply(adb, plan, &approval, &mut store, &std::thread::sleep)
        .inspect_err(|_| note_journal(&path))?;
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    }
    Ok(path)
}

/// `rusttv launcher restore`.
pub fn restore(
    adb: &dyn Adb,
    target: &Target,
    journal: Option<&Path>,
    from_profile: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let (path, mut journal) = match journal {
        Some(path) => (path.to_owned(), rusttv_core::load_journal(path)?),
        // Saved as a new journal when the restore runs.
        None if from_profile => (
            rusttv_core::journals_dir()?.join(journal_file_name(
                now_unix_ms(),
                &format!("{HOME_TAKEOVER}-from-profile"),
            )),
            rusttv_core::takeover_journal_from_profile(
                adb,
                &serial,
                &rusttv_core::bundled_profiles(),
            )?,
        ),
        None => {
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            let search = rusttv_core::find_restorable_journal(
                &rusttv_core::journals_dir()?,
                HOME_TAKEOVER,
                &fingerprint,
            );
            for file in &search.unreadable {
                output::print_note(&format!(
                    "warning: skipped {}, which can't be read. If it is a takeover that still \
                     needs restoring, fix it and pass `--journal {0}`, or use `--from-profile`. \
                     Reason: {}",
                    file.path.display(),
                    file.error
                ));
            }
            search.found?
        }
    };
    let plan = rusttv_core::takeover_restore_plan(adb, &serial, &journal)?;
    if !json {
        if from_profile {
            println!(
                "No journal: rebuilt from the device profile; saved as {}\n",
                path.display()
            );
        } else {
            println!("Journal: {}\n", path.display());
        }
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
pub(crate) fn show_plan(plan: &Plan, dry_run: bool, json: bool) {
    let dry = plan.dry_run();
    match (json, dry_run) {
        (true, true) => output::print_json(&dry),
        (true, false) => output::print_note(&dry.to_string()),
        (false, _) => print!("{dry}"),
    }
}

/// Approval comes only from a person typing at a terminal (SECURITY.md):
/// there is deliberately no `--yes` flag.
pub(crate) fn require_terminal() -> Result<(), CliError> {
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

pub(crate) fn confirm(plan: &Plan) -> Result<Approval, CliError> {
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

pub(crate) fn note_journal(path: &Path) {
    if path.exists() {
        output::print_note(&format!("Journal: {}", path.display()));
    }
}

pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

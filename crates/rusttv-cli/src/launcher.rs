//! `rusttv launcher ...` commands.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttv_core::{
    Adb, Approval, DeviceCommand, DryRun, FileJournal, Grant, GrantScope, HOME_TAKEOVER, Journal,
    JournalStatus, LAUNCHER_INSTALL, LAUNCHER_UNINSTALL, LauncherStatus, Plan, PlanError,
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
    let approver = (!dry_run)
        .then(|| approver(GrantScope::Launcher))
        .transpose()?;
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::takeover_plan(adb, &serial, &rusttv_core::bundled_profiles())?;
    show_plan(&plan, dry_run, json);
    let Some(approver) = approver else {
        return Ok(()); // --dry-run
    };
    let path = approve_and_apply(adb, &plan, &approver, HOME_TAKEOVER, json)?;
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
    let approver = (!dry_run)
        .then(|| approver(GrantScope::Launcher))
        .transpose()?;
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::install_plan(adb, &serial, apk, &rusttv_core::journals_dir()?)?;
    show_plan(&plan, dry_run, json);
    let Some(approver) = approver else {
        return Ok(()); // --dry-run
    };
    let path = approve_and_apply(adb, &plan, &approver, LAUNCHER_INSTALL, json)?;
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
    let path = approve_and_apply(adb, &plan, &Approver::Terminal, LAUNCHER_UNINSTALL, json)?;
    if !json {
        println!("\nDone: the RustTV launcher is uninstalled.");
        println!("Journal: {}", path.display());
    }
    Ok(())
}

/// Gets the approval from `approver` (asks for the typed confirmation at a
/// terminal), then applies `plan` with a new journal named after `name`.
/// With `--json`, prints the [`ChangeReport`].
pub(crate) fn approve_and_apply(
    adb: &dyn Adb,
    plan: &Plan,
    approver: &Approver,
    name: &str,
    json: bool,
) -> Result<PathBuf, CliError> {
    let approval = approver.approve(plan)?;
    let (path, journal) = apply_with_journal(adb, plan, &approval, name)?;
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    }
    Ok(path)
}

/// Applies the approved `plan` with a new journal named after `name`.
fn apply_with_journal(
    adb: &dyn Adb,
    plan: &Plan,
    approval: &Approval,
    name: &str,
) -> Result<(PathBuf, Journal), CliError> {
    let path = rusttv_core::journals_dir()?.join(journal_file_name(now_unix_ms(), name));
    let mut store = FileJournal::new(&path);
    let journal = rusttv_core::apply(adb, plan, approval, &mut store, &std::thread::sleep)
        .inspect_err(|_| note_journal(&path))?;
    Ok((path, journal))
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
    let approver = (!dry_run)
        .then(|| approver(GrantScope::Launcher))
        .transpose()?;
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
    let Some(approver) = approver else {
        return Ok(()); // --dry-run
    };
    let approval = approver.approve(&plan)?;
    restore_journal(adb, &plan, &approval, &path, &mut journal)?;
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

/// Restores `journal` (at `path`) with the approved restore `plan`.
fn restore_journal(
    adb: &dyn Adb,
    plan: &Plan,
    approval: &Approval,
    path: &Path,
    journal: &mut Journal,
) -> Result<(), CliError> {
    let mut store = FileJournal::new(path);
    rusttv_core::restore(adb, plan, approval, journal, &mut store)
        .inspect_err(|_| note_journal(path))?;
    Ok(())
}

/// What `launcher upgrade` will do, in `--dry-run --json` output.
#[derive(Debug, Serialize)]
struct LauncherUpgrade {
    /// Whether a Home takeover is active, so the upgrade has three stages.
    takeover_active: bool,
    /// The word a person types once for every stage.
    confirmation: &'static str,
    stages: Vec<UpgradeStage>,
}

/// One stage of `launcher upgrade`.
#[derive(Debug, Serialize)]
struct UpgradeStage {
    /// `restore`, `install` or `takeover`.
    stage: &'static str,
    /// The stage's plan, or `None` for a stage planned only when it runs,
    /// against the device as the stage before left it.
    plan: Option<DryRun>,
    description: String,
}

/// One applied stage of `launcher upgrade`, in `--json` output.
#[derive(Debug, Serialize)]
struct StageReport {
    stage: &'static str,
    journal: String,
    status: JournalStatus,
}

/// `rusttv launcher upgrade`: `launcher install` confirmed with `upgrade`;
/// while a Home takeover is active, `launcher restore`, then that install,
/// then `launcher takeover`, each its own plan and journal, approved by one
/// confirmation (owner decision 2026-10-24, SECURITY.md).
pub fn upgrade(
    adb: &dyn Adb,
    target: &Target,
    apk: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    const WORD: &str = "upgrade";
    let approver = (!dry_run)
        .then(|| approver(GrantScope::Launcher))
        .transpose()?;
    let serial = target.resolve(adb)?;
    let journals = rusttv_core::journals_dir()?;
    let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
    let search = rusttv_core::find_restorable_journal(&journals, HOME_TAKEOVER, &fingerprint);
    let (journal_path, mut journal) = match search.found {
        Ok(found) => found,
        Err(error) if error.code() == "NO_JOURNAL" => {
            // No takeover: exactly `launcher install`, also its refusals.
            let plan =
                rusttv_core::install_plan(adb, &serial, apk, &journals)?.with_confirmation(WORD)?;
            if json && dry_run {
                output::print_json(&LauncherUpgrade {
                    takeover_active: false,
                    confirmation: WORD,
                    stages: vec![UpgradeStage {
                        stage: "install",
                        plan: Some(plan.dry_run()),
                        description: "Install or upgrade the RustTV launcher, as `rusttv \
                                      launcher install`."
                            .to_owned(),
                    }],
                });
            } else {
                show_plan(&plan, dry_run, json);
            }
            let Some(approver) = approver else {
                return Ok(()); // --dry-run
            };
            let approval = approver.approve(&plan)?;
            let (path, journal) = apply_with_journal(adb, &plan, &approval, LAUNCHER_INSTALL)?;
            if json {
                output::print_json(&[StageReport {
                    stage: "install",
                    journal: path.display().to_string(),
                    status: journal.status,
                }]);
            } else {
                println!("\nDone: the RustTV launcher is installed.");
                println!("Journal: {}", path.display());
            }
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    // An unreadable file may be another takeover journal; the install stage
    // would refuse after the restore, so refuse now (SECURITY.md rule 6).
    if let Some(file) = search.unreadable.first() {
        return Err(PlanError::journal(format!(
            "can't read journal file {}, which may be an unrestored Home takeover: {}. Check it, \
             restore it if needed, then move it out of {}; nothing changed",
            file.path.display(),
            file.error,
            journals.display()
        ))
        .into());
    }

    let restore = rusttv_core::takeover_restore_plan(adb, &serial, &journal)?;
    let apk_path = rusttv_core::install_apk(adb, &serial, apk)?;
    let disabled: Vec<&str> = journal
        .steps
        .iter()
        .filter(|step| step.status.may_have_run())
        .filter_map(|step| match &step.command {
            DeviceCommand::DisablePackage(package) => Some(package.as_str()),
            _ => None,
        })
        .collect();
    let install_text = format!(
        "Install {} as an upgrade of the RustTV launcher (`adb install -r`, keeps its data; the \
         APK's version isn't read), planned after the restore as `rusttv launcher install`.",
        apk_path.as_str()
    );
    let takeover_text = format!(
        "Take over Home again, planned after the install as `rusttv launcher takeover`: disable \
         {} (what the journal recorded, if the device's profile still says so), wake the TV, \
         press HOME and check RustTV is in front and running; rolled back otherwise.",
        disabled.join(", ")
    );
    if json && dry_run {
        output::print_json(&LauncherUpgrade {
            takeover_active: true,
            confirmation: WORD,
            stages: vec![
                UpgradeStage {
                    stage: "restore",
                    plan: Some(restore.dry_run()),
                    description: format!(
                        "Undo the Home takeover of {}, as `rusttv launcher restore`.",
                        journal_path.display()
                    ),
                },
                UpgradeStage {
                    stage: "install",
                    plan: None,
                    description: install_text,
                },
                UpgradeStage {
                    stage: "takeover",
                    plan: None,
                    description: takeover_text,
                },
            ],
        });
        return Ok(());
    }
    let explanation = format!(
        "\nA Home takeover is active (journal {}), so the upgrade runs in three stages, each its \
         own plan with its own journal, rollback and checks; it stops at the first that fails.\n\
         1. The restore above.\n2. {install_text}\n3. {takeover_text}\n",
        journal_path.display()
    );
    show_plan(&restore, dry_run, json);
    if json {
        output::print_note(&explanation);
    } else {
        print!("{explanation}");
    }
    let Some(approver) = approver else {
        return Ok(()); // --dry-run
    };
    let approver = match approver {
        Approver::Terminal => {
            let typed = read_confirmation(WORD, "run all three stages")?;
            if typed.trim() != WORD {
                return Err(PlanError::not_approved(&format!(
                    "the confirmation `{WORD}` was not typed; nothing changed"
                ))
                .into());
            }
            Approver::ThisRun(Grant::for_this_run(
                &serial,
                &fingerprint,
                GrantScope::Launcher,
                now_unix_ms(),
            )?)
        }
        other => other,
    };

    let mut reports = Vec::new();
    let approval = approver.approve(&restore)?;
    restore_journal(adb, &restore, &approval, &journal_path, &mut journal).map_err(|error| {
        error.in_stage(
            "Stage 1 of 3 (restore) failed: nothing was installed and the RustTV launcher is \
             still installed, so Home opens it or the stock launcher. Finish with `rusttv \
             launcher restore`, then run `rusttv launcher upgrade` again",
        )
    })?;
    reports.push(StageReport {
        stage: "restore",
        journal: journal_path.display().to_string(),
        status: journal.status,
    });
    if !json {
        println!("\nStage 1 of 3 done: the stock launcher is enabled again.\n");
    }

    let install = (|| {
        let plan = rusttv_core::install_plan(adb, &serial, apk, &journals)?;
        show_plan(&plan, false, json);
        let approval = approver.approve(&plan)?;
        apply_with_journal(adb, &plan, &approval, LAUNCHER_INSTALL)
    })()
    .map_err(|error| {
        error.in_stage(
            "Stage 2 of 3 (install) failed after the restore: Home opens the stock launcher, and \
             the RustTV launcher is still installed (the old version, unless the error says the \
             new one installed but failed its checks). Run `rusttv launcher takeover` to make \
             RustTV Home again",
        )
    })?;
    reports.push(StageReport {
        stage: "install",
        journal: install.0.display().to_string(),
        status: install.1.status,
    });
    if !json {
        println!("\nStage 2 of 3 done: the RustTV launcher is upgraded.\n");
    }

    let takeover = (|| {
        let plan = rusttv_core::takeover_plan(adb, &serial, &rusttv_core::bundled_profiles())?;
        show_plan(&plan, false, json);
        let approval = approver.approve(&plan)?;
        apply_with_journal(adb, &plan, &approval, HOME_TAKEOVER)
    })()
    .map_err(|error| {
        error.in_stage(
            "Stage 3 of 3 (takeover) failed: the RustTV launcher is upgraded, and Home opens the \
             stock launcher unless the error says rolling back failed. Run `rusttv launcher \
             takeover` again",
        )
    })?;
    reports.push(StageReport {
        stage: "takeover",
        journal: takeover.0.display().to_string(),
        status: takeover.1.status,
    });
    if json {
        output::print_json(&reports);
    } else {
        println!("\nDone: the RustTV launcher is upgraded and Home opens it again.");
        for report in &reports {
            println!("Journal ({}): {}", report.stage, report.journal);
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
    let typed = read_confirmation(plan.confirmation(), "apply this plan")?;
    Ok(plan.approve(&typed)?)
}

/// Asks a person to type `word` to `what` (e.g. "apply this plan") and
/// returns what they typed.
pub(crate) fn read_confirmation(word: &str, what: &str) -> Result<String, CliError> {
    output::print_note(&format!(
        "\nType `{word}` and press Enter to {what}; anything else cancels:"
    ));
    let mut typed = String::new();
    io::stdin().read_line(&mut typed).map_err(|error| {
        PlanError::not_approved(&format!("could not read the confirmation: {error}"))
    })?;
    Ok(typed)
}

/// Who approves the plans of a command that a delegation may cover
/// (SECURITY.md § Delegated approval).
#[derive(Debug)]
pub(crate) enum Approver {
    /// A person at the interactive terminal types each plan's confirmation.
    Terminal,
    /// The grant `rusttv approve` stored, for a run without a terminal.
    Delegation(Grant),
    /// The one-shot grant for the stages of one `launcher upgrade`.
    ThisRun(Grant),
}

impl Approver {
    /// Approves `plan`: by its typed confirmation at the terminal, or by the
    /// grant, which must cover it (noted on stderr for a stored delegation).
    pub(crate) fn approve(&self, plan: &Plan) -> Result<Approval, CliError> {
        match self {
            Self::Terminal => confirm(plan),
            Self::Delegation(grant) => {
                let approval = plan.approve_by_grant(grant, now_unix_ms())?;
                output::print_note(&format!(
                    "Approved by the delegation granted at {}, valid until {}.",
                    clock(grant.granted_at_unix_ms()),
                    clock(grant.expires_at_unix_ms())
                ));
                Ok(approval)
            }
            Self::ThisRun(grant) => Ok(plan.approve_by_grant(grant, now_unix_ms())?),
        }
    }
}

/// Like [`require_terminal`], for commands in `scope`: with an interactive
/// terminal a person approves as usual; without one, a valid stored grant
/// for `scope` may (checked again against each plan). Otherwise
/// `NOT_APPROVED` before contacting the device.
pub(crate) fn approver(scope: GrantScope) -> Result<Approver, CliError> {
    if io::stdin().is_terminal() {
        return Ok(Approver::Terminal);
    }
    let path = rusttv_core::grant_path()?;
    let reason = match rusttv_core::load_grant(&path) {
        Ok(Some(grant)) => match grant.invalid_at(now_unix_ms()) {
            None if grant.scope() == scope => return Ok(Approver::Delegation(grant)),
            None => format!("the delegation's scope is `{}`", grant.scope()),
            Some(reason) => format!(
                "the delegation granted at {} isn't valid: {reason}",
                clock(grant.granted_at_unix_ms())
            ),
        },
        Ok(None) => "there is no delegation".to_owned(),
        Err(error) => error.to_string(),
    };
    Err(PlanError::not_approved(&format!(
        "this changes the device and needs a person to approve it at an interactive terminal, \
         or a delegation a person grants there first with `rusttv approve --for 30m --only \
         {scope}`; {reason}; nothing changed (use --dry-run to only see the plan)"
    ))
    .into())
}

/// `HH:MM UTC` of a host-clock time.
pub(crate) fn clock(unix_ms: u64) -> String {
    let minutes = unix_ms / 60_000 % (24 * 60);
    format!("{:02}:{:02} UTC", minutes / 60, minutes % 60)
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

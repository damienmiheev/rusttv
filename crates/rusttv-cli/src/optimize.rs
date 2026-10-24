//! `rusttv optimize ...` commands: privacy and optimizer profiles (OPTIMIZER.md).

use std::path::Path;

use rusttv_core::{
    Adb, FileJournal, JournalStatus, JournalStore, OptimizeAnalysis, OptimizeKind, OptimizerState,
    OptimizerStatus, PRIVACY, PackageState, PrivacyCategory, PrivacyData, REDUCED_ANIMATIONS,
};

use crate::launcher::{
    Approver, ChangeReport, approve_and_apply, confirm, require_terminal, restore_journal,
    show_plan,
};
use crate::{CliError, Target, output};

/// `rusttv optimize analyze`.
pub fn analyze(
    adb: &dyn Adb,
    target: &Target,
    choices: Option<&Path>,
    json: bool,
) -> Result<(), CliError> {
    let data = PrivacyData::bundled(rusttv_core::load_choices(choices)?);
    let serial = target.resolve(adb)?;
    let journals = rusttv_core::journals_dir()?;
    let analysis = rusttv_core::optimize_analyze(adb, &serial, &data, &journals)?;
    if json {
        output::print_json(&analysis);
    } else {
        print_analysis(&analysis);
        print_optimizer(&analysis.optimizer);
    }
    Ok(())
}

fn print_optimizer(statuses: &[OptimizerStatus]) {
    println!("\nOptimizer profiles (settings, R2):");
    for status in statuses {
        let state = match status.status {
            OptimizerState::Suggested => "suggested",
            OptimizerState::Available => "tested here, no measured benefit (not suggested)",
            OptimizerState::Applied => "applied",
            OptimizerState::Drifted => "applied, but changed since (drifted)",
            OptimizerState::Stale => "applied on older firmware (stale): restore, then apply",
            OptimizerState::NotTested => "not tested on this model (apply refuses it)",
            OptimizerState::AlreadySet => "already set",
            OptimizerState::Unknown => "unknown: a setting can't be read",
        };
        println!(
            "  {} v{}: {state}. {}",
            status.profile, status.version, status.description
        );
        if let Some(evidence) = &status.evidence {
            println!("    evidence: {evidence}");
        }
        if let Some(journal) = &status.journal {
            println!("    journal: {journal}");
        }
        for setting in &status.settings {
            let now = match (&setting.current, &setting.read_error) {
                (_, Some(_)) => "unknown",
                (Some(current), None) => current.as_str(),
                (None, None) => "unset",
            };
            println!(
                "    {:28} now {now:7} profile {}",
                setting.key.as_str(),
                setting.value
            );
            if let Some(error) = &setting.read_error {
                println!("      can't read it: {error}");
            }
        }
    }
}

fn print_analysis(analysis: &OptimizeAnalysis) {
    match &analysis.profile {
        Some(profile) => println!("Profile: {profile}"),
        None => println!(
            "No compatibility profile matches this device: no package is tested on it \
             (`optimize apply privacy --allow-untested` plans untested ones)."
        ),
    }
    let categories: Vec<&str> = analysis
        .categories
        .iter()
        .map(|category| category.as_str())
        .collect();
    match &analysis.choices {
        Some(path) => println!("Choices: {path}"),
        None => println!("Choices: none (no choices file), the defaults"),
    }
    println!("Categories: {}", categories.join(", "));
    if analysis.packages.is_empty() {
        println!("\nNo catalog package is installed on this device.");
        return;
    }
    println!(
        "\n{:14} {:8} {:8} {:10} {:6} PACKAGE",
        "CATEGORY", "", "STATE", "TESTED", "CHOSEN"
    );
    for entry in &analysis.packages {
        let state = match entry.state {
            PackageState::Absent => "absent",
            PackageState::Enabled => "enabled",
            PackageState::Disabled => "disabled",
        };
        let (category, kind) = match entry.category {
            Some(category) if category.is_default() => (category.as_str(), "default"),
            Some(category) => (category.as_str(), "opt-in"),
            None => ("extra", ""),
        };
        let tested = entry.checked.as_deref().unwrap_or("no");
        let chosen = if entry.chosen { "yes" } else { "no" };
        println!(
            "{category:14} {kind:8} {state:8} {tested:10} {chosen:6} {}",
            entry.package
        );
        let indent = "";
        match (&entry.what, &entry.side_effects) {
            (Some(what), Some(side_effects)) => {
                println!("{indent:4}{what}");
                println!("{indent:4}side effects: {side_effects}");
            }
            _ => println!("{indent:4}not in the catalog: nothing is known about it"),
        }
        if let Some(why) = &entry.protected {
            println!("{indent:4}PROTECTED, never disabled: {why}");
        }
    }
}

/// Options of `rusttv optimize apply privacy`.
#[derive(Debug)]
pub struct PrivacyOptions<'a> {
    /// `--only`.
    pub only: &'a [PrivacyCategory],
    /// `--choices`.
    pub choices: Option<&'a Path>,
    /// `--allow-untested`.
    pub allow_untested: bool,
}

/// `rusttv optimize apply privacy`.
pub fn apply_privacy(
    adb: &dyn Adb,
    target: &Target,
    options: &PrivacyOptions<'_>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let data = PrivacyData::bundled(rusttv_core::load_choices(options.choices)?);
    let serial = target.resolve(adb)?;
    let plan =
        rusttv_core::privacy_plan(adb, &serial, &data, options.only, options.allow_untested)?;
    rusttv_core::refuse_overlapping_journals(&plan, &rusttv_core::journals_dir()?)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, &Approver::Terminal, PRIVACY, json)?;
    if !json {
        println!("\nDone: the packages are disabled.");
        println!("Journal: {}", path.display());
        println!("Undo with `rusttv optimize restore privacy`.");
    }
    Ok(())
}

/// `rusttv optimize apply reduced-animations`.
pub fn apply_reduced_animations(
    adb: &dyn Adb,
    target: &Target,
    allow_untested: bool,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let profile = rusttv_core::bundled_optimizer_profile(REDUCED_ANIMATIONS)?;
    let plan = rusttv_core::optimizer_plan(
        adb,
        &serial,
        &profile,
        &rusttv_core::bundled_profiles(),
        &rusttv_core::journals_dir()?,
        allow_untested,
    )?;
    rusttv_core::refuse_overlapping_journals(&plan, &rusttv_core::journals_dir()?)?;
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, &Approver::Terminal, plan.name(), json)?;
    if !json {
        println!("\nDone: the settings are changed.");
        println!("Journal: {}", path.display());
        println!("Undo with `rusttv optimize restore {REDUCED_ANIMATIONS}`.");
    }
    Ok(())
}

/// `rusttv optimize restore [privacy|reduced-animations]`.
pub fn restore(
    adb: &dyn Adb,
    target: &Target,
    kind: Option<OptimizeKind>,
    journal: Option<&Path>,
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let (kind, path, mut journal) = match journal {
        Some(path) => {
            let journal = rusttv_core::load_journal(path)?;
            // Without a name the journal's plan says which kind it is; any
            // other journal goes to the privacy validator, which refuses it.
            let kind = kind
                .or_else(|| OptimizeKind::of_plan(&journal.plan))
                .unwrap_or(OptimizeKind::Privacy);
            (kind, path.to_owned(), journal)
        }
        None => {
            let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
            let search = rusttv_core::find_optimize_journal(
                &rusttv_core::journals_dir()?,
                &fingerprint,
                kind,
            );
            for file in &search.unreadable {
                output::print_note(&format!(
                    "warning: skipped {}, which can't be read. If it is a privacy or optimizer \
                     plan that still needs restoring, fix it and pass `--journal {0}`. Reason: {}",
                    file.path.display(),
                    file.error
                ));
            }
            search.found?
        }
    };
    let plan = match rusttv_core::optimize_restore_plan(adb, &serial, kind, &mut journal) {
        Ok(plan) => plan,
        Err(error) => {
            // Nothing is left to undo (every package enabled, every setting
            // back or changed since): record that, so the next restore finds
            // the journal before this one. Only a file on this computer
            // changes, never the device.
            if journal.status == JournalStatus::Restored && !dry_run {
                FileJournal::new(&path)
                    .save(&journal)
                    .map_err(rusttv_core::PlanError::journal)?;
                output::print_note(&format!(
                    "Marked {} restored: nothing in it is left to undo.",
                    path.display()
                ));
            }
            return Err(error.into());
        }
    };
    if !json {
        println!("Journal: {}\n", path.display());
    }
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let approval = confirm(&plan)?;

    let left_as_is = restore_journal(adb, &plan, &approval, &path, &mut journal)?;
    for note in &left_as_is {
        output::print_note(note);
    }
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    } else {
        match kind {
            OptimizeKind::Privacy => println!("\nDone: the packages are enabled again."),
            OptimizeKind::ReducedAnimations => println!("\nDone: the settings are put back."),
        }
    }
    Ok(())
}

//! `rusttv optimize ...` commands: privacy profiles (OPTIMIZER.md).

use std::path::Path;

use rusttv_core::{
    Adb, FileJournal, JournalStatus, JournalStore, OptimizeAnalysis, PRIVACY, PackageState,
    PrivacyCategory, PrivacyData,
};

use crate::launcher::{
    ChangeReport, approve_and_apply, confirm, note_journal, require_terminal, show_plan,
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
    let analysis = rusttv_core::optimize_analyze(adb, &serial, &data)?;
    if json {
        output::print_json(&analysis);
    } else {
        print_analysis(&analysis);
    }
    Ok(())
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
    show_plan(&plan, dry_run, json);
    if dry_run {
        return Ok(());
    }
    let path = approve_and_apply(adb, &plan, PRIVACY, json)?;
    if !json {
        println!("\nDone: the packages are disabled.");
        println!("Journal: {}", path.display());
        println!("Undo with `rusttv optimize restore`.");
    }
    Ok(())
}

/// `rusttv optimize restore`.
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
            let search = rusttv_core::find_restorable_journal(
                &rusttv_core::journals_dir()?,
                PRIVACY,
                &fingerprint,
            );
            for file in &search.unreadable {
                output::print_note(&format!(
                    "warning: skipped {}, which can't be read. If it is a privacy plan that \
                     still needs restoring, fix it and pass `--journal {0}`. Reason: {}",
                    file.path.display(),
                    file.error
                ));
            }
            search.found?
        }
    };
    let plan = match rusttv_core::privacy_restore_plan(adb, &serial, &mut journal) {
        Ok(plan) => plan,
        Err(error) => {
            // Every package is enabled already: record that, so the next
            // restore finds the journal before this one. Only a file on this
            // computer changes, never the device.
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

    let mut store = FileJournal::new(&path);
    rusttv_core::restore(adb, &plan, &approval, &mut journal, &mut store)
        .inspect_err(|_| note_journal(&path))?;
    if json {
        output::print_json(&ChangeReport {
            journal: path.display().to_string(),
            status: journal.status,
        });
    } else {
        println!("\nDone: the packages are enabled again.");
    }
    Ok(())
}

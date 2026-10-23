//! `rusttv optimize ...` commands: privacy profiles (OPTIMIZER.md).

use std::path::Path;

use rusttv_core::{Adb, FileJournal, OptimizeAnalysis, PRIVACY, PackageState, PrivacyCategory};

use crate::launcher::{
    ChangeReport, approve_and_apply, confirm, note_journal, require_terminal, show_plan,
};
use crate::{CliError, Target, output};

/// `rusttv optimize analyze`.
pub fn analyze(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let analysis = rusttv_core::optimize_analyze(adb, &serial, &rusttv_core::bundled_profiles())?;
    if json {
        output::print_json(&analysis);
    } else {
        print_analysis(&analysis);
    }
    Ok(())
}

fn print_analysis(analysis: &OptimizeAnalysis) {
    let Some(profile) = &analysis.profile else {
        println!("No compatibility profile matches this device: nothing to suggest.");
        return;
    };
    println!("Profile: {profile}");
    let Some(privacy) = &analysis.privacy else {
        println!("It has no privacy profile.");
        return;
    };
    if privacy.tested {
        println!("Privacy profile: tested on this model.");
    } else {
        println!(
            "Privacy profile: NOT tested on this model yet; `optimize apply privacy` is refused."
        );
    }
    println!("\n{:10} {:9} PACKAGE", "CATEGORY", "STATE");
    for entry in &privacy.packages {
        let state = match entry.state {
            PackageState::Absent => "absent",
            PackageState::Enabled => "enabled",
            PackageState::Disabled => "disabled",
        };
        println!("{:10} {state:9} {}", entry.category.as_str(), entry.package);
        println!("{:21}{}", "", entry.what);
        println!("{:21}side effects: {}", "", entry.side_effects);
        if let Some(why) = &entry.protected {
            println!("{:21}PROTECTED, never disabled: {why}", "");
        }
    }
}

/// `rusttv optimize apply privacy`.
pub fn apply_privacy(
    adb: &dyn Adb,
    target: &Target,
    only: &[PrivacyCategory],
    dry_run: bool,
    json: bool,
) -> Result<(), CliError> {
    if !dry_run {
        require_terminal()?;
    }
    let serial = target.resolve(adb)?;
    let plan = rusttv_core::privacy_plan(adb, &serial, &rusttv_core::bundled_profiles(), only)?;
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
    let plan = rusttv_core::privacy_restore_plan(adb, &serial, &mut journal)?;
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

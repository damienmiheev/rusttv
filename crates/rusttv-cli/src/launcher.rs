//! `rusttv launcher ...` commands.

use rusttv_core::{Adb, LauncherStatus};

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

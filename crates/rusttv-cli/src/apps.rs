//! `rusttv apps ...` commands.

use rusttv_core::{Adb, AppInfo, AppKind};

use crate::{CliError, Target, output};

/// `rusttv apps list`.
pub fn list(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let apps = rusttv_core::list_apps(adb, &serial)?;
    if json {
        output::print_json(&apps);
    } else {
        print_table(&apps);
    }
    Ok(())
}

/// Longer package names overflow their own row instead of widening the table.
const MAX_PACKAGE_COLUMN: usize = 48;

fn print_table(apps: &[AppInfo]) {
    let width = apps
        .iter()
        .map(|app| app.package.len())
        .max()
        .unwrap_or_default()
        .clamp("PACKAGE".len(), MAX_PACKAGE_COLUMN);
    println!("{:width$}  {:11}  STATE", "PACKAGE", "TYPE");
    for app in apps {
        let kind = match app.kind {
            AppKind::System => "system",
            AppKind::ThirdParty => "third-party",
        };
        let state = if app.enabled { "enabled" } else { "disabled" };
        println!("{:width$}  {kind:11}  {state}", app.package);
    }

    let third_party = apps
        .iter()
        .filter(|app| app.kind == AppKind::ThirdParty)
        .count();
    let disabled = apps.iter().filter(|app| !app.enabled).count();
    println!(
        "\n{} packages: {} system, {third_party} third-party; {disabled} disabled",
        apps.len(),
        apps.len() - third_party,
    );
}

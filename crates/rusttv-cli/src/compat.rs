//! `rusttv compat ...` commands.

use rusttv_core::Adb;
use serde::Serialize;

use crate::{CliError, Target, output};

#[derive(Debug, Serialize)]
struct ProfileReport {
    /// Suggested path inside the repository.
    path: String,
    profile: rusttv_core::DeviceProfile,
}

/// `rusttv compat profile`: prints TOML on stdout (redirect it to the file),
/// and the suggested file name on stderr.
pub fn profile(adb: &dyn Adb, target: &Target, json: bool) -> Result<(), CliError> {
    let serial = target.resolve(adb)?;
    let info = rusttv_core::inspect(adb, &serial)?;
    let profile = rusttv_core::profile_from_info(&info)?;
    let path = format!(
        "compat/devices/{}",
        rusttv_core::profile_file_name(&profile)
    );

    if json {
        output::print_json(&ProfileReport { path, profile });
        return Ok(());
    }
    // Serializing these plain structs to TOML can't fail; fall back to JSON
    // rather than print nothing if it ever does.
    match toml::to_string(&profile) {
        Ok(text) => print!("{text}"),
        Err(_) => output::print_json(&profile),
    }
    output::print_note(&format!(
        "Save as {path}, fill in `tested.notes`, and open a pull request."
    ));
    Ok(())
}

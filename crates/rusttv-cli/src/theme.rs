//! `rusttv theme install <file>`: hands one theme package to the TV's
//! staging slot (SECURITY.md "Packages: staging and install", risk class
//! RS). Nothing is installed: the package waits, inert, until a person
//! reviews and approves it on the TV.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Subcommand;
use rusttv_core::{Adb, DeviceCommand, InboxPayload, PackageName, StageAnnounce};
use rusttv_history::stage::{STAGE_ERRORS, StageResult};
use rusttv_panels::{Capabilities, Kind, Version, read_package};
use serde::Serialize;

use crate::history::Failure;
use crate::{CliError, Target, output};

#[derive(Debug, Subcommand)]
pub(crate) enum ThemeAction {
    /// Send a theme package to the TV, where it waits for you to review and
    /// approve it in Settings. Nothing is installed until you do.
    Install {
        /// The theme package (a .zip, at most 4 MiB).
        file: PathBuf,
        #[command(flatten)]
        target: Target,
    },
}

pub(crate) fn run(adb: &dyn Adb, action: &ThemeAction, json: bool) -> Result<(), CliError> {
    match action {
        ThemeAction::Install { file, target } => install(adb, file, target, json),
    }
}

/// What `theme install` prints with `--json`.
#[derive(Debug, Serialize)]
struct Staged {
    id: String,
    version: String,
    size: u64,
    sha256: String,
}

fn install(adb: &dyn Adb, file: &Path, target: &Target, json: bool) -> Result<(), CliError> {
    let bytes = read_file(file)?;
    let serial = target.resolve(adb)?;
    let launcher = PackageName::new(rusttv_core::RUSTTV_LAUNCHER)
        .map_err(|error| Failure::new("THEME_INVALID", error.to_string()))?;
    let dumpsys = adb.run(&serial, &DeviceCommand::PackageInfo(launcher))?;
    // A debuggable or profileable launcher can't install: the shell could
    // write its slot and store (SECURITY.md rule 9).
    match rusttv_core::launcher_test_build(&dumpsys) {
        Some(false) => {}
        Some(true) => {
            return Err(Failure::new(
                "PRECONDITION_FAILED",
                "the launcher on this TV is a test build, which can't install packages".to_owned(),
            )
            .into());
        }
        None => {
            return Err(Failure::new(
                "PRECONDITION_FAILED",
                "the RustTV launcher isn't installed on this TV".to_owned(),
            )
            .into());
        }
    }
    // Checked as the TV's launcher will check it.
    let version = rusttv_core::parse_version_name(&dumpsys)
        .and_then(|name| Version::parse(name).ok())
        .ok_or_else(|| {
            Failure::new(
                "PRECONDITION_FAILED",
                "can't read the launcher's version on this TV".to_owned(),
            )
        })?;
    let contents = read_package(&bytes, &Capabilities::new(version, Vec::<String>::new()))
        .map_err(|error| Failure::new("THEME_INVALID", format!("{}: {error}", file.display())))?;
    let meta = &contents.package.meta;
    if meta.kind != Kind::Theme {
        return Err(Failure::new(
            "THEME_INVALID",
            format!(
                "{} is a {}, not a theme",
                file.display(),
                meta.kind.as_str()
            ),
        )
        .into());
    }
    let sha256: String = rusttv_core::sha256(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if !json {
        println!(
            "Hash {}: compare it with the one the TV shows.",
            &sha256[..12]
        );
    }
    stage(adb, &serial, bytes, size, &sha256)?;
    journal(&serial, size, &sha256);
    let staged = Staged {
        id: meta.id.as_str().to_owned(),
        version: meta.version.to_string(),
        size,
        sha256,
    };
    if json {
        output::print_json(&staged);
    } else {
        println!(
            "Sent {} {} to the TV. It waits in Settings there; nothing is installed until you approve it.",
            staged.id, staged.version
        );
    }
    Ok(())
}

/// A regular file of at most 4 MiB.
fn read_file(file: &Path) -> Result<Vec<u8>, CliError> {
    let invalid = |message: String| CliError::from(Failure::new("THEME_INVALID", message));
    let meta =
        fs::metadata(file).map_err(|error| invalid(format!("{}: {error}", file.display())))?;
    if !meta.is_file() {
        return Err(invalid(format!("{} is not a file", file.display())));
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|handle| handle.take(StageAnnounce::MAX + 1).read_to_end(&mut bytes))
        .map_err(|error| invalid(format!("{}: {error}", file.display())))?;
    if bytes.len() as u64 > StageAnnounce::MAX {
        return Err(invalid(format!("{} is larger than 4 MiB", file.display())));
    }
    Ok(bytes)
}

/// The three staging door calls: begin, write, end.
fn stage(
    adb: &dyn Adb,
    serial: &str,
    bytes: Vec<u8>,
    size: u64,
    sha256: &str,
) -> Result<(), CliError> {
    let announce = StageAnnounce::new(size, sha256)
        .map_err(|error| Failure::new("THEME_INVALID", error.to_string()))?;
    let begun = answer(&adb.run(serial, &DeviceCommand::StageBegin(announce))?)?;
    let nonce = begun
        .nonce
        .as_deref()
        .and_then(|nonce| rusttv_core::HistoryNonce::new(nonce).ok())
        .ok_or_else(|| {
            Failure::new(
                "LAUNCHER_TOO_OLD",
                "the TV gave no staging nonce".to_owned(),
            )
        })?;
    let write = DeviceCommand::StageWrite {
        nonce: nonce.clone(),
        payload: InboxPayload::new(bytes),
    };
    let stdin = write.stdin().unwrap_or_default().to_vec();
    adb.run_binary(serial, &write, &stdin)?;
    answer(&adb.run(serial, &DeviceCommand::StageEnd(nonce))?)?;
    Ok(())
}

/// The TV's answer; a refusal is an error with its code.
fn answer(output: &str) -> Result<StageResult, CliError> {
    let result = StageResult::parse(output).map_err(|_| {
        Failure::new(
            "LAUNCHER_TOO_OLD",
            "the launcher on this TV can't take packages yet; run `rusttv update launcher`"
                .to_owned(),
        )
    })?;
    match result.error.as_deref() {
        None => Ok(result),
        Some(code) => {
            let code = STAGE_ERRORS
                .iter()
                .copied()
                .find(|known| *known == code)
                .unwrap_or("STAGING_STORAGE");
            let hint = match code {
                "STAGING_BUSY" => ": ten packages were sent this hour; try again later",
                "STAGING_MISMATCH" | "STAGING_TOO_BIG" => {
                    ": the bytes arrived damaged; send it again"
                }
                "STAGING_EXPIRED" => ": it took longer than a minute; send it again",
                _ => "",
            };
            Err(Failure::new(code, format!("the TV refused the package ({code}){hint}")).into())
        }
    }
}

/// One line in this computer's staging log (device, size, SHA-256, time);
/// a log that can't be written is only a warning.
fn journal(serial: &str, size: u64, sha256: &str) {
    let written = rusttv_core::state_dir()
        .map_err(|error| std::io::Error::other(error.to_string()))
        .and_then(|dir| {
            fs::create_dir_all(&dir)?;
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |time| time.as_secs());
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("staging.log"))?
                .write_all(format!("{seconds} {serial} {size} {sha256}\n").as_bytes())
        });
    if let Err(error) = written {
        output::print_note(&format!("warning: couldn't write the staging log: {error}"));
    }
}

//! `rusttv approve`: grant, show or revoke a time-limited delegation of
//! approval (SECURITY.md § Delegated approval).

use std::io::{self, IsTerminal};
use std::time::Duration;

use clap::Args;
use rusttv_core::{Adb, DeviceCommand, Grant, GrantScope, MAX_GRANT_DURATION, PlanError};
use serde::Serialize;

use crate::launcher::{clock, now_unix_ms, read_confirmation};
use crate::{CliError, Target, output};

/// The word a person types to grant a delegation.
const WORD: &str = "delegate";

#[derive(Debug, Args)]
pub struct ApproveArgs {
    #[command(flatten)]
    target: Target,
    /// How long the delegation lasts: minutes (`30m`) or hours (`1h`), at most 2h.
    #[arg(
        long = "for",
        value_name = "DURATION",
        default_value = "30m",
        value_parser = parse_duration,
        conflicts_with_all = ["status", "revoke"]
    )]
    duration: Duration,
    /// What it may approve. `launcher`: launcher install / upgrade (also
    /// `update launcher`), takeover and restore; not uninstall.
    #[arg(
        long,
        required_unless_present_any = ["status", "revoke"],
        conflicts_with_all = ["status", "revoke"]
    )]
    only: Option<GrantScope>,
    /// Show the delegation and whether it is still valid.
    #[arg(long, conflicts_with = "revoke")]
    status: bool,
    /// Delete the delegation (no terminal needed).
    #[arg(long)]
    revoke: bool,
}

/// `<n>m` or `<n>h`, more than 0 and at most [`MAX_GRANT_DURATION`].
fn parse_duration(text: &str) -> Result<Duration, String> {
    let (number, seconds_per_unit) = if let Some(minutes) = text.strip_suffix('m') {
        (minutes, 60)
    } else if let Some(hours) = text.strip_suffix('h') {
        (hours, 60 * 60)
    } else {
        return Err("expected minutes or hours, e.g. `30m` or `1h`".to_owned());
    };
    let duration = number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(seconds_per_unit))
        .map(Duration::from_secs)
        .ok_or_else(|| "expected minutes or hours, e.g. `30m` or `1h`".to_owned())?;
    if duration.is_zero() || duration > MAX_GRANT_DURATION {
        return Err("a delegation lasts more than 0 and at most 2h".to_owned());
    }
    Ok(duration)
}

/// The grant file and what it holds, in `--json` output.
#[derive(Debug, Serialize)]
struct GrantStatus {
    path: String,
    /// The grant, or `None` without a file.
    grant: Option<Grant>,
    /// It approves plans now (in its scope, for its device and firmware).
    valid: bool,
    /// Why it isn't valid, or `None`.
    reason: Option<String>,
}

/// Result of `approve --revoke` in `--json` output.
#[derive(Debug, Serialize)]
struct Revoked {
    path: String,
    /// False when there was no delegation.
    revoked: bool,
}

/// `rusttv approve`.
pub fn run(adb: &dyn Adb, args: &ApproveArgs, json: bool) -> Result<(), CliError> {
    let path = rusttv_core::grant_path()?;
    if args.revoke {
        let revoked = rusttv_core::revoke_grant(&path)
            .map_err(|error| PlanError::journal(format!("{}: {error}", path.display())))?;
        if json {
            output::print_json(&Revoked {
                path: path.display().to_string(),
                revoked,
            });
        } else if revoked {
            println!("Revoked the delegation ({}).", path.display());
        } else {
            println!("There is no delegation to revoke.");
        }
        return Ok(());
    }
    if args.status {
        let (grant, reason) = match rusttv_core::load_grant(&path) {
            Ok(Some(grant)) => {
                let reason = grant.invalid_at(now_unix_ms());
                (Some(grant), reason)
            }
            Ok(None) => (None, Some("there is no delegation".to_owned())),
            Err(error) => (None, Some(error.to_string())),
        };
        let status = GrantStatus {
            path: path.display().to_string(),
            valid: grant.is_some() && reason.is_none(),
            grant,
            reason,
        };
        if json {
            output::print_json(&status);
        } else {
            print_status(&status);
        }
        return Ok(());
    }
    // Clap requires `--only` without `--status` / `--revoke`.
    let Some(scope) = args.only else {
        return Err(PlanError::not_approved("no scope given (`--only launcher`)").into());
    };
    grant(adb, &args.target, scope, args.duration, json)
}

/// Asks the person at the terminal to type `delegate`, then stores the grant.
fn grant(
    adb: &dyn Adb,
    target: &Target,
    scope: GrantScope,
    duration: Duration,
    json: bool,
) -> Result<(), CliError> {
    if !io::stdin().is_terminal() {
        return Err(PlanError::not_approved(
            "granting a delegation needs a person at an interactive terminal; agents must never \
             create or edit the delegation file; nothing changed",
        )
        .into());
    }
    let serial = target.resolve(adb)?;
    let fingerprint = rusttv_core::device_fingerprint(adb, &serial)?;
    let properties =
        rusttv_core::parse_properties(&adb.run(&serial, &DeviceCommand::GetProperties)?);
    let property = |name: &str| properties.get(name).map_or("unknown", String::as_str);
    let grant = Grant::new(&serial, &fingerprint, scope, now_unix_ms(), duration)?;

    let summary = format!(
        "Delegation for device {serial} ({} {})\nFirmware:    {fingerprint}\nScope:       \
         {scope}: {}\nValid:       from {} until {} ({} min)\n\nUntil then, `rusttv` run without \
         a terminal (e.g. by an agent) applies these plans on this device without asking you. \
         Revoke it any time with `rusttv approve --revoke`.",
        property("ro.product.manufacturer"),
        property("ro.product.model"),
        scope.description(),
        clock(grant.granted_at_unix_ms()),
        clock(grant.expires_at_unix_ms()),
        duration.as_secs() / 60,
    );
    if json {
        output::print_note(&summary);
    } else {
        println!("{summary}");
    }
    if read_confirmation(WORD, "grant this delegation")?.trim() != WORD {
        return Err(PlanError::not_approved(&format!(
            "the confirmation `{WORD}` was not typed; nothing changed"
        ))
        .into());
    }
    let path = rusttv_core::grant_path()?;
    rusttv_core::save_grant(&path, &grant)
        .map_err(|error| PlanError::journal(format!("{}: {error}", path.display())))?;
    if json {
        output::print_json(&GrantStatus {
            path: path.display().to_string(),
            grant: Some(grant),
            valid: true,
            reason: None,
        });
    } else {
        println!(
            "\nGranted until {} ({}).",
            clock(grant.expires_at_unix_ms()),
            path.display()
        );
    }
    Ok(())
}

fn print_status(status: &GrantStatus) {
    let Some(grant) = &status.grant else {
        println!(
            "No delegation: {}.",
            status.reason.as_deref().unwrap_or("none")
        );
        return;
    };
    println!("Device:      {}", grant.serial());
    println!("Firmware:    {}", grant.fingerprint());
    println!("Scope:       {}", grant.scope());
    println!("Granted at:  {}", clock(grant.granted_at_unix_ms()));
    println!("Valid until: {}", clock(grant.expires_at_unix_ms()));
    match &status.reason {
        None => println!("Valid now."),
        Some(reason) => println!("Not valid: {reason}."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_minutes_or_hours_up_to_two_hours() {
        assert_eq!(parse_duration("30m"), Ok(Duration::from_secs(1800)));
        assert_eq!(parse_duration("2h"), Ok(MAX_GRANT_DURATION));
        for text in ["", "m", "0m", "121m", "3h", "30", "30s", "-1m", "1.5h"] {
            assert!(parse_duration(text).is_err(), "{text}");
        }
    }
}

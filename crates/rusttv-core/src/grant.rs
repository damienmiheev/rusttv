//! Time-limited delegation of approval (SECURITY.md § Delegated approval).
//!
//! A [`Grant`] lets the CLI approve plans of one [`GrantScope`] for one
//! device and firmware until it expires, without a person typing each
//! plan's confirmation. A person creates it with `rusttv approve` at an
//! interactive terminal; it is stored in [`grant_path`]. Agents and MCP
//! servers must never create or edit that file: this is policy, not
//! cryptography, on a machine where the agent can write files.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::install::LAUNCHER_INSTALL;
use crate::journal::state_dir;
use crate::plan::PlanError;
use crate::takeover::HOME_TAKEOVER;

/// Version of the grant file format. Bump on any incompatible change.
pub const GRANT_SCHEMA_VERSION: u32 = 1;

/// The longest a grant may last (owner decision 2026-10-24).
pub const MAX_GRANT_DURATION: Duration = Duration::from_secs(2 * 60 * 60);

/// What a [`Grant`] may approve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum GrantScope {
    /// Installing or upgrading the RustTV launcher (`launcher install` /
    /// `upgrade`, `update launcher`), `launcher takeover` and `launcher
    /// restore`. Not `launcher uninstall`.
    Launcher,
}

impl GrantScope {
    /// Every scope, for help texts and parsing.
    pub const ALL: [Self; 1] = [Self::Launcher];

    /// The scope's name, as typed after `--only`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
        }
    }

    /// Whether a plan named `plan` (e.g. `launcher-install`) is in this scope.
    pub fn covers(self, plan: &str) -> bool {
        match self {
            Self::Launcher => {
                plan == LAUNCHER_INSTALL
                    || plan == HOME_TAKEOVER
                    || plan == format!("{HOME_TAKEOVER}-restore")
            }
        }
    }

    /// The plans this scope covers, for people.
    pub fn description(self) -> &'static str {
        match self {
            Self::Launcher => {
                "launcher install / upgrade (also `update launcher`), launcher takeover, launcher \
                 restore; not launcher uninstall, optimize, tools or update cli"
            }
        }
    }
}

impl fmt::Display for GrantScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GrantScope {
    type Err = InvalidGrantScope;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.as_str() == name)
            .ok_or_else(|| InvalidGrantScope(name.to_owned()))
    }
}

/// Returned when parsing a [`GrantScope`] fails.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not an approval scope; expected `launcher`")]
pub struct InvalidGrantScope(String);

/// Where a [`Grant`] comes from; decides how journals record approvals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum GrantOrigin {
    /// `rusttv approve`, stored in the grant file.
    #[default]
    File,
    /// `upgrade` typed for this one `launcher upgrade` run; never stored.
    ThisRun,
}

/// A time-limited delegation of approval for one device. See the module docs.
///
/// Stored as TOML in [`grant_path`]; [`load_grant`] re-checks it, since the
/// file is untrusted input like a journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    schema_version: u32,
    serial: String,
    fingerprint: String,
    scope: GrantScope,
    granted_at_unix_ms: u64,
    expires_at_unix_ms: u64,
    #[serde(skip)]
    origin: GrantOrigin,
}

impl Grant {
    /// A grant for the device `serial` with build `fingerprint`, from
    /// `granted_at_unix_ms` for `duration` (at most [`MAX_GRANT_DURATION`]).
    ///
    /// Only `rusttv approve` calls this, after a person typed `delegate` at
    /// an interactive terminal.
    pub fn new(
        serial: &str,
        fingerprint: &str,
        scope: GrantScope,
        granted_at_unix_ms: u64,
        duration: Duration,
    ) -> Result<Self, PlanError> {
        if duration.is_zero() || duration > MAX_GRANT_DURATION {
            return Err(PlanError::not_approved(
                "a delegation lasts more than 0 and at most 2 h",
            ));
        }
        let duration_ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let grant = Self {
            schema_version: GRANT_SCHEMA_VERSION,
            serial: serial.to_owned(),
            fingerprint: fingerprint.to_owned(),
            scope,
            granted_at_unix_ms,
            expires_at_unix_ms: granted_at_unix_ms.saturating_add(duration_ms),
            origin: GrantOrigin::File,
        };
        grant.validate()?;
        Ok(grant)
    }

    /// A grant that lives only in this process, for the stages of one
    /// `rusttv launcher upgrade` after a person typed `upgrade` at an
    /// interactive terminal. Never stored; journals record it as `upgrade`.
    pub fn for_this_run(
        serial: &str,
        fingerprint: &str,
        scope: GrantScope,
        confirmed_at_unix_ms: u64,
    ) -> Result<Self, PlanError> {
        let mut grant = Self::new(
            serial,
            fingerprint,
            scope,
            confirmed_at_unix_ms,
            MAX_GRANT_DURATION,
        )?;
        grant.origin = GrantOrigin::ThisRun;
        Ok(grant)
    }

    fn validate(&self) -> Result<(), PlanError> {
        let max_ms = u64::try_from(MAX_GRANT_DURATION.as_millis()).unwrap_or(u64::MAX);
        if self.schema_version != GRANT_SCHEMA_VERSION {
            return Err(PlanError::not_approved(&format!(
                "unsupported schema_version {}",
                self.schema_version
            )));
        }
        if self.serial.is_empty() || self.fingerprint.is_empty() {
            return Err(PlanError::not_approved("no serial or fingerprint"));
        }
        if self.expires_at_unix_ms <= self.granted_at_unix_ms
            || self.expires_at_unix_ms - self.granted_at_unix_ms > max_ms
        {
            return Err(PlanError::not_approved(
                "it doesn't last more than 0 and at most 2 h",
            ));
        }
        Ok(())
    }

    /// adb serial of the device the grant is for.
    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// `ro.build.fingerprint` of the device when the grant was made.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// What the grant may approve.
    pub fn scope(&self) -> GrantScope {
        self.scope
    }

    /// When it was granted (host clock).
    pub fn granted_at_unix_ms(&self) -> u64 {
        self.granted_at_unix_ms
    }

    /// When it stops approving anything (host clock).
    pub fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    /// Why the grant approves nothing at `now_unix_ms`, if it doesn't.
    pub fn invalid_at(&self, now_unix_ms: u64) -> Option<String> {
        if now_unix_ms < self.granted_at_unix_ms {
            Some("it was granted in the future (did the clock change?)".to_owned())
        } else if now_unix_ms >= self.expires_at_unix_ms {
            Some("it has expired".to_owned())
        } else {
            None
        }
    }

    /// Why the grant doesn't approve the plan `plan` for `serial` with
    /// `fingerprint` at `now_unix_ms`, if it doesn't.
    pub(crate) fn refuses(
        &self,
        plan: &str,
        serial: &str,
        fingerprint: &str,
        now_unix_ms: u64,
    ) -> Option<String> {
        if let Some(reason) = self.invalid_at(now_unix_ms) {
            Some(reason)
        } else if self.serial != serial {
            Some(format!(
                "it is for device `{}`, not `{serial}`",
                self.serial
            ))
        } else if self.fingerprint != fingerprint {
            Some(format!(
                "it is for firmware `{}`, and the plan is for `{fingerprint}`",
                self.fingerprint
            ))
        } else if !self.scope.covers(plan) {
            Some(format!(
                "its scope `{}` doesn't cover a `{plan}` plan",
                self.scope
            ))
        } else {
            None
        }
    }

    /// How a plan approved by this grant is recorded in its journal.
    pub(crate) fn approved_by(&self) -> crate::journal::ApprovedBy {
        match self.origin {
            GrantOrigin::File => crate::journal::ApprovedBy::Delegation {
                granted_at_unix_ms: self.granted_at_unix_ms,
            },
            GrantOrigin::ThisRun => crate::journal::ApprovedBy::Upgrade {
                confirmed_at_unix_ms: self.granted_at_unix_ms,
            },
        }
    }
}

/// `<state dir>/approval-grant.toml`, the one grant file (see [`state_dir`]).
pub fn grant_path() -> Result<PathBuf, PlanError> {
    Ok(state_dir()?.join("approval-grant.toml"))
}

/// Reads the grant at `path`: `None` if there is no file. A file that
/// doesn't parse or holds an impossible grant is `NOT_APPROVED`.
pub fn load_grant(path: &Path) -> Result<Option<Grant>, PlanError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(PlanError::not_approved(&format!(
                "the delegation {} can't be read: {error}",
                path.display()
            )));
        }
    };
    let invalid = |reason: &dyn fmt::Display| {
        PlanError::not_approved(&format!(
            "the delegation {} is invalid: {reason}; run `rusttv approve --revoke`",
            path.display()
        ))
    };
    let grant: Grant = toml::from_str(&text).map_err(|error| invalid(&error))?;
    grant.validate().map_err(|error| invalid(&error))?;
    Ok(Some(grant))
}

/// Writes `grant` to `path` atomically, replacing any grant there.
pub fn save_grant(path: &Path, grant: &Grant) -> io::Result<()> {
    let text = toml::to_string(grant).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("toml.partial");
    let mut file = fs::File::create(&partial)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&partial, path)
}

/// Deletes the grant at `path`; `false` if there was none.
pub fn revoke_grant(path: &Path) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIAL: &str = "tv";
    const FINGERPRINT: &str = "google/tv/x:14/A/1:user/release-keys";
    const T0: u64 = 1_761_300_000_000;
    const MINUTE: u64 = 60_000;

    fn grant() -> Grant {
        Grant::new(
            SERIAL,
            FINGERPRINT,
            GrantScope::Launcher,
            T0,
            Duration::from_secs(30 * 60),
        )
        .unwrap()
    }

    #[test]
    fn a_grant_lasts_more_than_zero_and_at_most_two_hours() {
        let make = |duration| Grant::new(SERIAL, FINGERPRINT, GrantScope::Launcher, T0, duration);
        assert!(make(MAX_GRANT_DURATION).is_ok());
        for duration in [Duration::ZERO, MAX_GRANT_DURATION + Duration::from_secs(1)] {
            assert_eq!(make(duration).unwrap_err().code(), "NOT_APPROVED");
        }
        assert_eq!(grant().expires_at_unix_ms(), T0 + 30 * MINUTE);
    }

    #[test]
    fn a_valid_grant_covers_only_its_device_firmware_scope_and_time() {
        let grant = grant();
        let now = T0 + MINUTE;
        for plan in ["launcher-install", "home-takeover", "home-takeover-restore"] {
            assert_eq!(
                grant.refuses(plan, SERIAL, FINGERPRINT, now),
                None,
                "{plan}"
            );
        }
        for plan in [
            "launcher-uninstall",
            "privacy",
            "optimize-reduced-animations-v1",
            "tools-airplay",
        ] {
            let reason = grant.refuses(plan, SERIAL, FINGERPRINT, now).unwrap();
            assert!(reason.contains("scope"), "{plan}: {reason}");
        }
        let install = "launcher-install";
        let reason = grant.refuses(install, "other", FINGERPRINT, now).unwrap();
        assert!(reason.contains("device"), "{reason}");
        let reason = grant.refuses(install, SERIAL, "other", now).unwrap();
        assert!(reason.contains("firmware"), "{reason}");
        let expired = T0 + 30 * MINUTE;
        let reason = grant
            .refuses(install, SERIAL, FINGERPRINT, expired)
            .unwrap();
        assert!(reason.contains("expired"), "{reason}");
        let early = grant.refuses(install, SERIAL, FINGERPRINT, T0 - 1).unwrap();
        assert!(early.contains("future"), "{early}");
    }

    #[test]
    fn the_scope_parses_from_its_name() {
        assert_eq!("launcher".parse(), Ok(GrantScope::Launcher));
        assert!("optimize".parse::<GrantScope>().is_err());
    }

    #[test]
    fn a_grant_file_round_trips_and_impossible_ones_are_refused() {
        let dir = std::env::temp_dir().join(format!("rusttv-grant-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("approval-grant.toml");
        assert_eq!(load_grant(&path).unwrap(), None);

        save_grant(&path, &grant()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("scope = \"launcher\""), "{text}");
        assert!(!text.contains("origin"), "{text}");
        assert_eq!(load_grant(&path).unwrap(), Some(grant()));

        // Edited to last a day: refused, not trusted.
        let edited = text.replace(
            &format!("expires_at_unix_ms = {}", T0 + 30 * MINUTE),
            &format!("expires_at_unix_ms = {}", T0 + 24 * 60 * MINUTE),
        );
        assert_ne!(edited, text);
        fs::write(&path, edited).unwrap();
        assert_eq!(load_grant(&path).unwrap_err().code(), "NOT_APPROVED");
        fs::write(&path, text.replace("launcher", "everything")).unwrap();
        assert_eq!(load_grant(&path).unwrap_err().code(), "NOT_APPROVED");

        assert!(revoke_grant(&path).unwrap());
        assert!(!revoke_grant(&path).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
}

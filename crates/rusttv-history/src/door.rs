//! What crosses the door between the Mac and the launcher (HISTORY.md § 5):
//! the export header, inbox messages, commit results and pending approvals.
//! Both sides parse everything here strictly: it is untrusted input.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::HistoryError;
use crate::entry::EntryId;
use crate::state::{Device, PackageState, State, is_package_name, is_setting_value};
use crate::store::HistoryId;

/// The provider's authority.
pub const AUTHORITY: &str = "ai.rusttv.launcher.history";

/// Version of the door protocol.
pub const DOOR_VERSION: u32 = 1;

/// Largest inbox message (notes and observations; imports are packs).
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// The first line of an export: `rusttv-export 1 <id|none> <head|none> <0|1>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportHeader {
    /// The history's id; `None` while it is empty.
    pub history: Option<HistoryId>,
    /// The newest entry in this part.
    pub head: Option<EntryId>,
    /// Whether newer entries follow.
    pub more: bool,
}

impl ExportHeader {
    /// The header line, with its `\n`.
    pub fn encode(&self) -> String {
        format!(
            "rusttv-export {DOOR_VERSION} {} {} {}\n",
            self.history.as_ref().map_or("none", HistoryId::as_str),
            self.head.as_ref().map_or("none", EntryId::as_str),
            u8::from(self.more)
        )
    }

    /// Splits an export into its header and pack.
    pub fn split(export: &[u8]) -> Result<(Self, &[u8]), HistoryError> {
        let bad = || HistoryError::invalid("not a RustTV history export".to_owned());
        let end = export
            .iter()
            .take(200)
            .position(|b| *b == b'\n')
            .ok_or_else(bad)?;
        let line = std::str::from_utf8(&export[..end]).map_err(|_| bad())?;
        let fields: Vec<&str> = line.split(' ').collect();
        let [magic, version, history, head, more] = fields.as_slice() else {
            return Err(bad());
        };
        if *magic != "rusttv-export" {
            return Err(bad());
        }
        let version: u32 = version.parse().map_err(|_| bad())?;
        if version > DOOR_VERSION {
            return Err(HistoryError::FormatNewer(version));
        }
        if version != DOOR_VERSION {
            return Err(bad());
        }
        let header = Self {
            history: match *history {
                "none" => None,
                id => Some(HistoryId::parse(id)?),
            },
            head: match *head {
                "none" => None,
                id => Some(EntryId::parse(id)?),
            },
            more: match *more {
                "0" => false,
                "1" => true,
                _ => return Err(bad()),
            },
        };
        Ok((header, &export[end + 1..]))
    }
}

/// What the Mac saw on the device: everything but the launcher's own state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    /// The allowlisted global settings; a missing key is unset.
    #[serde(default)]
    pub settings: BTreeMap<String, String>,
    /// Catalog packages and their state.
    #[serde(default)]
    pub packages: BTreeMap<String, PackageState>,
    /// Firmware and takeover.
    #[serde(default)]
    pub device: Device,
}

impl Observation {
    /// `state` with this observation's sections in place of its own; the
    /// launcher's state stays (the Mac never changes it).
    pub fn applied_to(&self, state: &State) -> Result<State, HistoryError> {
        let mut new = state.clone();
        new.settings.global.clone_from(&self.settings);
        new.packages.clone_from(&self.packages);
        new.device = self.device.clone();
        new.validate()?;
        Ok(new)
    }
}

/// How a Mac plan was approved, as a note may say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteApproval {
    /// Typed at a terminal.
    Terminal,
    /// A delegation.
    Delegation,
    /// `rusttv launcher upgrade`.
    Upgrade,
    /// A pending approval the TV made; only accepted with the matching
    /// [`Note::consumed`].
    Tv,
    /// Not approved (e.g. refused before applying).
    None,
}

/// How a Mac plan ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every step ran and was verified.
    Applied,
    /// Undone after a failure.
    RolledBack,
    /// Undoing failed too.
    RollbackFailed,
    /// A restore ran.
    Restored,
    /// A restore failed for at least one step.
    RestoreFailed,
}

/// A note from the Mac after a plan (HISTORY.md § 4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// The plan's name, e.g. `optimize-reduced-animations-v1`.
    pub plan: String,
    /// Its journal file name.
    pub journal: String,
    /// How it was approved.
    pub approved_by: NoteApproval,
    /// How it ended.
    pub outcome: Outcome,
    /// The approval hash this note reports on, after the Mac consumed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed: Option<String>,
    /// The device as the Mac sees it now.
    pub observation: Observation,
}

/// A message written to the inbox, then committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    /// Before a plan: what the device looks like.
    Observe {
        /// It.
        observation: Observation,
    },
    /// After a plan.
    Note {
        /// It.
        note: Note,
    },
    /// Claim a pending approval before applying it.
    Consume {
        /// The approval's hash.
        approval: String,
    },
}

impl Message {
    /// Parses and validates an inbox message.
    pub fn parse(bytes: &[u8]) -> Result<Self, HistoryError> {
        if bytes.len() > MAX_MESSAGE_BYTES {
            return Err(HistoryError::invalid("message too big".to_owned()));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|_| HistoryError::invalid("message is not UTF-8".to_owned()))?;
        let message: Self = toml::from_str(text)
            .map_err(|error| HistoryError::invalid(format!("message: {error}")))?;
        match &message {
            Self::Observe { observation } => observation.applied_to(&State::new()).map(|_| ())?,
            Self::Note { note } => {
                note.observation.applied_to(&State::new())?;
                if !is_word(&note.plan) || !is_word(&note.journal) {
                    return Err(HistoryError::invalid(
                        "note: bad plan or journal name".to_owned(),
                    ));
                }
                if let Some(hash) = &note.consumed
                    && !is_hash(hash)
                {
                    return Err(HistoryError::invalid("note: bad approval hash".to_owned()));
                }
            }
            Self::Consume { approval } if !is_hash(approval) => {
                return Err(HistoryError::invalid(
                    "consume: bad approval hash".to_owned(),
                ));
            }
            Self::Consume { .. } => {}
        }
        Ok(message)
    }

    /// The TOML for the inbox.
    pub fn to_toml(&self) -> Result<String, HistoryError> {
        toml::to_string(self).map_err(|error| HistoryError::invalid(error.to_string()))
    }
}

fn is_word(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// 64 lowercase hex digits.
pub fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The kinds of `content call … commit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// An inbox [`Message`].
    Message,
    /// An inbox export, into an empty history.
    Import,
}

impl CommitKind {
    /// The `--arg` of `content call … commit`: `<kind>:<16 hex>`.
    pub fn arg(self, nonce: &str) -> String {
        let kind = match self {
            Self::Message => "message",
            Self::Import => "import",
        };
        format!("{kind}:{nonce}")
    }

    /// Parses an `--arg`.
    pub fn parse_arg(arg: &str) -> Result<(Self, &str), HistoryError> {
        let bad = || HistoryError::invalid(format!("bad commit argument {arg:?}"));
        let (kind, nonce) = arg.split_once(':').ok_or_else(bad)?;
        if !is_nonce(nonce) {
            return Err(bad());
        }
        let kind = match kind {
            "message" => Self::Message,
            "import" => Self::Import,
            _ => return Err(bad()),
        };
        Ok((kind, nonce))
    }
}

/// 16 lowercase hex digits.
pub fn is_nonce(text: &str) -> bool {
    text.len() == 16
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A door read (`export`, `pending`) that failed answers one line,
/// `rusttv-error <CODE>`, instead of its data.
pub fn read_error(code: &str) -> Vec<u8> {
    format!("rusttv-error {code}\n").into_bytes()
}

/// The error code if `bytes` is a failed read's answer.
pub fn parse_read_error(bytes: &[u8]) -> Option<String> {
    let rest = bytes.strip_prefix(b"rusttv-error ")?;
    let end = rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
    let code = std::str::from_utf8(&rest[..end]).ok()?;
    (!code.is_empty()
        && code.len() <= 64
        && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
    .then(|| code.to_owned())
}

/// What a commit answered: one line in the result bundle, key `rusttv`:
/// `rusttv-result 1 ok <head|none>` or `rusttv-result 1 error <CODE> <head|none>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResult {
    /// `None` when it worked, else the error code (e.g. `TV_BUSY`).
    pub error: Option<String>,
    /// The history's newest entry afterwards.
    pub head: Option<EntryId>,
}

impl CommitResult {
    /// The result line.
    pub fn encode(&self) -> String {
        let head = self.head.as_ref().map_or("none", EntryId::as_str);
        match &self.error {
            None => format!("rusttv-result {DOOR_VERSION} ok {head}"),
            Some(code) => format!("rusttv-result {DOOR_VERSION} error {code} {head}"),
        }
    }

    /// Finds and parses the result line in `content call`'s output
    /// (`Result: Bundle[{rusttv=rusttv-result …}]`).
    pub fn parse(output: &str) -> Result<Self, HistoryError> {
        let bad = || HistoryError::invalid(format!("no RustTV result in {output:?}"));
        let start = output.find("rusttv-result ").ok_or_else(bad)?;
        let line: String = output[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'))
            .collect();
        let fields: Vec<&str> = line.split(' ').collect();
        let head = |text: &str| match text {
            "none" => Ok(None),
            id => EntryId::parse(id).map(Some),
        };
        match fields.as_slice() {
            ["rusttv-result", "1", "ok", id] => Ok(Self {
                error: None,
                head: head(id)?,
            }),
            ["rusttv-result", "1", "error", code, id]
                if code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') =>
            {
                Ok(Self {
                    error: Some((*code).to_owned()),
                    head: head(id)?,
                })
            }
            _ => Err(bad()),
        }
    }
}

/// One change in a TV approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ApprovedChange {
    /// A global setting; `None` = unset.
    Setting {
        /// The key.
        key: String,
        /// Its value when approved.
        old: Option<String>,
        /// The value to set.
        new: Option<String>,
    },
    /// A catalog package.
    Package {
        /// The package.
        name: String,
        /// Its state as last seen.
        old: Option<PackageState>,
        /// The state to set.
        new: PackageState,
    },
}

impl ApprovedChange {
    fn validate(&self) -> Result<(), HistoryError> {
        match self {
            Self::Setting { key, old, new } => {
                if !crate::state::SETTING_KEYS.contains(&key.as_str())
                    || old.iter().chain(new).any(|value| !is_setting_value(value))
                {
                    return Err(HistoryError::invalid(format!("bad setting change {key}")));
                }
            }
            Self::Package { name, .. } if !is_package_name(name) => {
                return Err(HistoryError::invalid(format!("bad package {name:?}")));
            }
            Self::Package { .. } => {}
        }
        Ok(())
    }
}

/// The SHA-256 (hex) of a change list in its TOML form: what an approval
/// binds to.
pub fn approval_hash(changes: &[ApprovedChange]) -> Result<String, HistoryError> {
    #[derive(Serialize)]
    struct List<'a> {
        change: &'a [ApprovedChange],
    }
    let text = toml::to_string(&List { change: changes })
        .map_err(|error| HistoryError::invalid(error.to_string()))?;
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// A pending approval as the door lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingApproval {
    /// [`approval_hash`] of `change`.
    pub hash: String,
    /// Seconds of TV uptime left.
    pub seconds_left: u64,
    /// The approved changes.
    pub change: Vec<ApprovedChange>,
}

/// The `pending` read: every valid pending approval.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingList {
    /// Them.
    #[serde(default)]
    pub approval: Vec<PendingApproval>,
}

impl PendingList {
    /// Parses and validates the list; each hash must match its changes.
    pub fn parse(text: &str) -> Result<Self, HistoryError> {
        let list: Self = toml::from_str(text)
            .map_err(|error| HistoryError::invalid(format!("pending: {error}")))?;
        for approval in &list.approval {
            for change in &approval.change {
                change.validate()?;
            }
            if approval_hash(&approval.change)? != approval.hash {
                return Err(HistoryError::invalid(
                    "pending: hash doesn't match".to_owned(),
                ));
            }
        }
        Ok(list)
    }

    /// The TOML text.
    pub fn to_toml(&self) -> Result<String, HistoryError> {
        toml::to_string(self).map_err(|error| HistoryError::invalid(error.to_string()))
    }
}

pub(crate) fn validate_changes(changes: &[ApprovedChange]) -> Result<(), HistoryError> {
    if changes.is_empty() {
        return Err(HistoryError::invalid("no changes".to_owned()));
    }
    changes.iter().try_for_each(ApprovedChange::validate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> HistoryId {
        HistoryId::parse("0f8b6a2c-5d1e-4c3b-9a7f-1e2d3c4b5a69").unwrap()
    }

    #[test]
    fn export_header_round_trips_and_refuses_junk() {
        let header = ExportHeader {
            history: Some(id()),
            head: Some(EntryId::parse(&"ab".repeat(20)).unwrap()),
            more: true,
        };
        let mut bytes = header.encode().into_bytes();
        bytes.extend_from_slice(b"PACK...");
        let (parsed, pack) = ExportHeader::split(&bytes).unwrap();
        assert_eq!(parsed, header);
        assert_eq!(pack, b"PACK...");
        for bad in [
            &b"PACK"[..],
            b"rusttv-export 1 none none 2\n",
            b"rusttv-export 1 ../x none 0\n",
            b"rusttv-export 1 none none 0 extra\n",
        ] {
            assert!(ExportHeader::split(bad).is_err());
        }
        assert!(matches!(
            ExportHeader::split(b"rusttv-export 9 none none 0\n"),
            Err(HistoryError::FormatNewer(9))
        ));
    }

    #[test]
    fn messages_round_trip_and_are_validated() {
        let mut observation = Observation::default();
        observation
            .settings
            .insert("animator_duration_scale".into(), "0.5".into());
        observation
            .packages
            .insert("com.example.t".into(), PackageState::Disabled);
        let note = Message::Note {
            note: Note {
                plan: "optimize-reduced-animations-v1".into(),
                journal: "1792843149859-optimize.toml".into(),
                approved_by: NoteApproval::Terminal,
                outcome: Outcome::Applied,
                consumed: None,
                observation: observation.clone(),
            },
        };
        let text = note.to_toml().unwrap();
        assert_eq!(Message::parse(text.as_bytes()).unwrap(), note);

        let mut bad = observation;
        bad.settings.insert("adb_enabled".into(), "1".into());
        let text = Message::Observe { observation: bad }.to_toml().unwrap();
        assert!(Message::parse(text.as_bytes()).is_err());
        assert!(Message::parse(b"kind = \"consume\"\napproval = \"12\"\n").is_err());
        assert!(Message::parse(b"kind = \"launcher\"\n").is_err());
        assert!(Message::parse(&vec![b' '; MAX_MESSAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn commit_results_are_found_in_content_call_output() {
        let ok = CommitResult {
            error: None,
            head: Some(EntryId::parse(&"cd".repeat(20)).unwrap()),
        };
        let output = format!("Result: Bundle[{{rusttv={}}}]\n", ok.encode());
        assert_eq!(CommitResult::parse(&output).unwrap(), ok);
        let busy = CommitResult {
            error: Some("TV_BUSY".into()),
            head: None,
        };
        let output = format!("Result: Bundle[{{rusttv={}}}]", busy.encode());
        assert_eq!(CommitResult::parse(&output).unwrap(), busy);
        assert!(CommitResult::parse("Error: java.lang.SecurityException").is_err());
        assert!(CommitResult::parse("rusttv-result 1 error bad-code none").is_err());
    }

    #[test]
    fn a_failed_read_says_its_code() {
        assert_eq!(
            parse_read_error(&read_error("HISTORY_NOT_FOUND")).as_deref(),
            Some("HISTORY_NOT_FOUND")
        );
        assert_eq!(parse_read_error(b"rusttv-export 1 none none 0\n"), None);
        assert_eq!(parse_read_error(b"rusttv-error bad code\n"), None);
    }

    #[test]
    fn commit_arguments_are_kind_and_nonce() {
        assert_eq!(
            CommitKind::parse_arg("import:0123456789abcdef").unwrap(),
            (CommitKind::Import, "0123456789abcdef")
        );
        assert_eq!(
            CommitKind::Message.arg("0123456789abcdef"),
            "message:0123456789abcdef"
        );
        for bad in [
            "import:",
            "x:0123456789abcdef",
            "message:0123456789ABCDEF",
            "message:../../etc/x",
        ] {
            assert!(CommitKind::parse_arg(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pending_list_checks_each_hash() {
        let change = vec![ApprovedChange::Package {
            name: "com.example.t".into(),
            old: Some(PackageState::Enabled),
            new: PackageState::Disabled,
        }];
        let list = PendingList {
            approval: vec![PendingApproval {
                hash: approval_hash(&change).unwrap(),
                seconds_left: 3600,
                change,
            }],
        };
        let text = list.to_toml().unwrap();
        assert_eq!(PendingList::parse(&text).unwrap(), list);
        let tampered = text.replace("disabled", "enabled");
        assert!(PendingList::parse(&tampered).is_err());
    }
}

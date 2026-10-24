//! The Mac's side of the history door (HISTORY.md § 5): exports, pending
//! approvals, and inbox writes followed by `commit`. Everything the TV sends
//! is parsed strictly by `rusttv_history::door`.

use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttv_core::{Adb, DeviceCommand, DoorKind, HistoryNonce, HistorySince, InboxPayload};
use rusttv_history::door::{CommitResult, ExportHeader, Message, PendingList, parse_read_error};
use rusttv_history::{EntryId, HistoryError};

use super::Failure;
use crate::CliError;

/// What `content read …/export/<since>` answered.
#[derive(Debug)]
pub(crate) enum ExportReply {
    /// An export: its header and the bytes (header line included).
    Export {
        /// The header line.
        header: ExportHeader,
        /// Where the pack starts in `bytes`.
        pack_at: usize,
        /// Everything the TV printed.
        bytes: Vec<u8>,
    },
    /// A refusal (`rusttv-error <CODE>`): the TV's error code.
    Refused(String),
}

impl ExportReply {
    /// The pack of an export (empty for a refusal).
    pub(crate) fn pack(&self) -> &[u8] {
        match self {
            Self::Export { pack_at, bytes, .. } => &bytes[*pack_at..],
            Self::Refused(_) => &[],
        }
    }
}

/// Reads the TV's history after `since` (all of it for `None`). R0.
pub(crate) fn export(
    adb: &dyn Adb,
    serial: &str,
    since: Option<&EntryId>,
) -> Result<ExportReply, CliError> {
    let since = match since {
        None => HistorySince::all(),
        Some(entry) => HistorySince::entry(entry.as_str())
            .map_err(|error| Failure::invalid(error.to_string()))?,
    };
    let bytes = adb.run_binary(serial, &DeviceCommand::HistoryExport(since), &[])?;
    if let Some(code) = parse_read_error(&bytes) {
        return Ok(ExportReply::Refused(code));
    }
    let (header, pack) = ExportHeader::split(&bytes).map_err(Failure::from)?;
    let pack_at = bytes.len() - pack.len();
    Ok(ExportReply::Export {
        header,
        pack_at,
        bytes,
    })
}

/// The TV's valid pending approvals. R0, but it starts the launcher's
/// process: only `history` and plan-applying commands read it.
pub(crate) fn pending(adb: &dyn Adb, serial: &str) -> Result<PendingList, CliError> {
    let bytes = adb.run_binary(serial, &DeviceCommand::HistoryPending, &[])?;
    if let Some(code) = parse_read_error(&bytes) {
        return Err(Failure::refused(&code, "reading the approvals waiting on the TV").into());
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| Failure::invalid("the pending approvals aren't UTF-8".to_owned()))?;
    Ok(PendingList::parse(text).map_err(Failure::from)?)
}

/// Writes `payload` to a new inbox file and has the launcher process it as
/// `kind`: two launcher-private (RL) commands. Returns the TV's answer,
/// which may be a refusal.
pub(crate) fn send(
    adb: &dyn Adb,
    serial: &str,
    kind: DoorKind,
    payload: Vec<u8>,
) -> Result<CommitResult, CliError> {
    let output = answered(
        || {
            let nonce = nonce();
            let inbox = DeviceCommand::HistoryInbox {
                nonce: nonce.clone(),
                payload: InboxPayload::new(payload.clone()),
            };
            let stdin = inbox.stdin().unwrap_or_default().to_vec();
            adb.run_binary(serial, &inbox, &stdin)?;
            Ok(adb.run(serial, &DeviceCommand::HistoryCommit { kind, nonce })?)
        },
        || std::thread::sleep(RETRY_AFTER),
    )?;
    Ok(CommitResult::parse(&output).map_err(Failure::from)?)
}

/// Runs `send` (write the inbox, then commit) and returns the commit's
/// output. Right after the launcher is installed or upgraded, the first call
/// can come back empty: Android hasn't started the new package's door yet
/// (seen on the Philips 65OLED818). Nothing was committed then, so the whole
/// message is sent once more after `wait`.
fn answered(
    mut send: impl FnMut() -> Result<String, CliError>,
    wait: impl Fn(),
) -> Result<String, CliError> {
    let output = send()?;
    if !output.trim().is_empty() {
        return Ok(output);
    }
    wait();
    send()
}

/// How long to wait before sending a message again that got no answer.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

/// Sends one message; a refusal is an error naming the TV's code.
pub(crate) fn send_message(
    adb: &dyn Adb,
    serial: &str,
    message: &Message,
    what: &str,
) -> Result<CommitResult, CliError> {
    let text = message.to_toml().map_err(Failure::from)?;
    let result = send(adb, serial, DoorKind::Message, text.into_bytes())?;
    if let Some(code) = &result.error {
        return Err(Failure::refused(code, what).into());
    }
    Ok(result)
}

/// A new inbox nonce: 16 hex digits from the OS random source
/// (`/dev/urandom`). Nonces aren't secret, they only keep two writes from
/// sharing an inbox file, so if that can't be read (it always can on macOS
/// and Linux), a counter, the time and the process id do.
pub(crate) fn nonce() -> HistoryNonce {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0_u8; 8];
    let value = match File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)) {
        Ok(()) => u64::from_le_bytes(bytes),
        Err(_) => {
            // The time's fast-changing low 64 bits.
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos() as u64);
            nanos.rotate_left(17)
                ^ u64::from(std::process::id()) << 40
                ^ COUNTER.fetch_add(1, Ordering::Relaxed)
        }
    };
    HistoryNonce::from_u64(value)
}

impl From<HistoryError> for Failure {
    fn from(error: HistoryError) -> Self {
        let code = match &error {
            HistoryError::FormatNewer(_) => "HISTORY_FORMAT_NEWER",
            HistoryError::NoSuchEntry(_) => "HISTORY_NOT_FOUND",
            HistoryError::Storage(_) => "HISTORY_STORAGE",
            _ => "HISTORY_INVALID",
        };
        let message = match &error {
            HistoryError::FormatNewer(_) => format!("{error}: run `rusttv update cli`"),
            _ => error.to_string(),
        };
        Self { code, message }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_answer_is_sent_once_more() {
        let mut answers = vec!["rusttv-result 1 ok none".to_owned(), String::new()];
        let waited = std::cell::Cell::new(0);
        let output = answered(
            || Ok(answers.pop().unwrap()),
            || waited.set(waited.get() + 1),
        )
        .unwrap();
        assert_eq!(output, "rusttv-result 1 ok none");
        assert_eq!(waited.get(), 1);

        let mut sent = 0;
        let output = answered(
            || {
                sent += 1;
                Ok("rusttv-result 1 ok none".to_owned())
            },
            || {},
        )
        .unwrap();
        assert_eq!((sent, output.as_str()), (1, "rusttv-result 1 ok none"));
    }

    #[test]
    fn nonces_are_valid_and_differ() {
        let a = nonce();
        let b = nonce();
        assert_eq!(a.as_str().len(), 16);
        assert_ne!(a, b);
        assert_eq!(HistoryNonce::new(a.as_str()).unwrap(), a);
    }
}

//! History entries: the metadata of one commit (HISTORY.md § 2).

use std::fmt;

use crate::HistoryError;

/// Version of the entry format (the commit message trailers).
pub const ENTRY_FORMAT: u32 = 1;

/// A commit id: 40 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryId(String);

impl EntryId {
    /// Validates `hex` as a full commit id.
    pub fn parse(hex: &str) -> Result<Self, HistoryError> {
        if hex.len() == 40
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(hex.to_owned()))
        } else {
            Err(HistoryError::invalid(format!("{hex:?} is not an entry id")))
        }
    }

    /// The id as hex.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_object(id: gix::ObjectId) -> Self {
        Self(id.to_hex().to_string())
    }

    pub(crate) fn object(&self) -> Result<gix::ObjectId, HistoryError> {
        gix::ObjectId::from_hex(self.0.as_bytes()).map_err(HistoryError::storage)
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Who made the change an entry records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Author {
    /// The TV: the Settings page, or moving and hiding apps.
    Tv,
    /// The Mac CLI, through a plan.
    Mac,
    /// Changed outside RustTV, as the Mac observed it.
    Drift,
    /// The live state when a history was put back from the Mac.
    Import,
}

impl Author {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tv => "tv",
            Self::Mac => "mac",
            Self::Drift => "drift",
            Self::Import => "import",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "tv" => Self::Tv,
            "mac" => Self::Mac,
            "drift" => Self::Drift,
            "import" => Self::Import,
            _ => return None,
        })
    }
}

impl fmt::Display for Author {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the change was approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovedBy {
    /// The code and hold OK on the TV; the approval's SHA-256 (64 hex).
    TvCode(String),
    /// Typed at a terminal on the Mac.
    Terminal,
    /// A delegation granted at a terminal.
    Delegation,
    /// `rusttv launcher upgrade`'s one confirmation.
    Upgrade,
}

impl ApprovedBy {
    fn render(&self) -> String {
        match self {
            Self::TvCode(hash) => format!("tv-code {hash}"),
            Self::Terminal => "terminal".to_owned(),
            Self::Delegation => "delegation".to_owned(),
            Self::Upgrade => "upgrade".to_owned(),
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "terminal" => Self::Terminal,
            "delegation" => Self::Delegation,
            "upgrade" => Self::Upgrade,
            _ => {
                let hash = text.strip_prefix("tv-code ")?;
                if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                Self::TvCode(hash.to_owned())
            }
        })
    }
}

/// What a writer says about an entry (the commit's trailers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// Who made the change.
    pub author: Author,
    /// The Mac journal file it came from (`mac` entries).
    pub journal: Option<String>,
    /// How it was approved; `None` for drift, imports and the launcher's
    /// direct edits.
    pub approved_by: Option<ApprovedBy>,
}

impl Meta {
    /// Metadata with only an author.
    pub fn new(author: Author) -> Self {
        Self {
            author,
            journal: None,
            approved_by: None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), HistoryError> {
        if let Some(journal) = &self.journal
            && !is_journal_name(journal)
        {
            return Err(HistoryError::invalid(format!(
                "{journal:?} is not a journal file name"
            )));
        }
        if let Some(ApprovedBy::TvCode(hash)) = &self.approved_by
            && (hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(HistoryError::invalid(
                "approval hash is not 64 hex digits".to_owned(),
            ));
        }
        Ok(())
    }

    /// The commit message: `summary`, a blank line, the trailers.
    pub(crate) fn message(&self, summary: &str) -> String {
        let summary: String = summary
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        let mut message = format!(
            "{summary}\n\nRustTV-Format: {ENTRY_FORMAT}\nRustTV-Author: {}\n",
            self.author
        );
        if let Some(journal) = &self.journal {
            message.push_str(&format!("RustTV-Journal: {journal}\n"));
        }
        if let Some(approved) = &self.approved_by {
            message.push_str(&format!("RustTV-Approved-By: {}\n", approved.render()));
        }
        message
    }

    /// Reads the trailers of a commit message; also returns the summary.
    pub(crate) fn from_message(message: &str) -> Result<(String, Self), HistoryError> {
        let bad = |what: &str| HistoryError::invalid(format!("entry message: {what}"));
        let (summary, rest) = message
            .split_once("\n\n")
            .ok_or_else(|| bad("no trailers"))?;
        let mut format = None;
        let mut author = None;
        let mut meta = Self::new(Author::Tv);
        for line in rest.lines().filter(|line| !line.is_empty()) {
            let (key, value) = line.split_once(": ").ok_or_else(|| bad("not a trailer"))?;
            match key {
                "RustTV-Format" => format = value.parse::<u32>().ok(),
                "RustTV-Author" => author = Author::parse(value),
                "RustTV-Journal" => meta.journal = Some(value.to_owned()),
                "RustTV-Approved-By" => {
                    meta.approved_by =
                        Some(ApprovedBy::parse(value).ok_or_else(|| bad("approved-by"))?);
                }
                _ => return Err(bad("unknown trailer")),
            }
        }
        match format {
            Some(ENTRY_FORMAT) => {}
            Some(newer) if newer > ENTRY_FORMAT => return Err(HistoryError::FormatNewer(newer)),
            _ => return Err(bad("format")),
        }
        meta.author = author.ok_or_else(|| bad("author"))?;
        meta.validate()?;
        Ok((summary.to_owned(), meta))
    }
}

/// A Mac journal file name: `<digits>-<plan>.toml`.
fn is_journal_name(name: &str) -> bool {
    name.len() <= 128
        && name.ends_with(".toml")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !name.starts_with('.')
}

/// One entry of the history, newest first in [`crate::Store::log`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its commit id.
    pub id: EntryId,
    /// The entry before it; `None` for the first.
    pub parent: Option<EntryId>,
    /// When it was recorded, by the clock of the device that recorded it
    /// (Unix seconds; may be wrong before the TV synced its clock).
    pub time_unix: i64,
    /// The one-line English summary.
    pub summary: String,
    /// Who and how.
    pub meta: Meta,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_round_trips_through_the_message() {
        let meta = Meta {
            author: Author::Mac,
            journal: Some("1792843149859-optimize.toml".into()),
            approved_by: Some(ApprovedBy::TvCode("a".repeat(64))),
        };
        let message = meta.message("Animations 0.5×\nsecond line");
        let (summary, parsed) = Meta::from_message(&message).unwrap();
        assert_eq!(summary, "Animations 0.5×second line");
        assert_eq!(parsed, meta);
    }

    #[test]
    fn refuses_unknown_trailers_and_bad_values() {
        for message in [
            "x\n\nRustTV-Format: 1\nRustTV-Author: root\n",
            "x\n\nRustTV-Format: 1\nRustTV-Author: tv\nRustTV-Shell: rm\n",
            "x\n\nRustTV-Format: 1\nRustTV-Author: tv\nRustTV-Journal: ../x.toml\n",
            "x\n\nRustTV-Format: 1\nRustTV-Author: tv\nRustTV-Approved-By: tv-code 12\n",
            "x\n\nRustTV-Author: tv\n",
        ] {
            assert!(Meta::from_message(message).is_err(), "{message}");
        }
        assert!(matches!(
            Meta::from_message("x\n\nRustTV-Format: 7\nRustTV-Author: tv\n"),
            Err(HistoryError::FormatNewer(7))
        ));
    }

    #[test]
    fn entry_ids_are_full_lowercase_hex() {
        assert!(EntryId::parse(&"0a".repeat(20)).is_ok());
        assert!(EntryId::parse(&"0A".repeat(20)).is_err());
        assert!(EntryId::parse("abc").is_err());
    }
}

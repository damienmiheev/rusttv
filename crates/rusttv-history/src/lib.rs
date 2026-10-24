//! RustTV history: snapshots of what RustTV manages on a TV, kept in git
//! (HISTORY.md).
//!
//! [`State`] is one snapshot (`state.toml`); [`Store`] keeps them as commits
//! on `main` in a bare repository, packs them, and moves them between the TV
//! and the Mac as git packs ([`Store::export`], [`Store::import`]); [`diff`]
//! says what changed between two snapshots. Shared by the launcher and the
//! CLI; it depends on neither.

mod diff;
pub mod door;
mod entry;
mod state;
mod store;
pub mod tv;

pub use diff::{Change, diff};
pub use entry::{ApprovedBy, Author, Entry, EntryId, Meta};
pub use state::{
    Device, Launcher, PackageState, SETTING_KEYS, STATE_FORMAT, Settings, State, is_package_name,
    is_setting_value, is_tile_name,
};
pub use store::{EXPORT_PART_COMMITS, Export, HistoryId, Limits, PackOutcome, Store};

/// Why a history operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HistoryError {
    /// Something in the history, a snapshot or a received pack breaks the
    /// format or its limits.
    #[error("invalid history: {0}")]
    Invalid(String),
    /// Written by a newer RustTV (format version inside).
    #[error("this history was written by a newer RustTV (format {0}); update RustTV")]
    FormatNewer(u32),
    /// A received pack doesn't continue this history.
    #[error("the received history doesn't continue this one: {0}")]
    NotAContinuation(String),
    /// No entry with that id.
    #[error("no history entry {0}")]
    NoSuchEntry(String),
    /// Reading or writing the repository failed.
    #[error("history storage: {0}")]
    Storage(String),
}

impl HistoryError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub(crate) fn storage(error: impl std::fmt::Display) -> Self {
        Self::Storage(error.to_string())
    }
}

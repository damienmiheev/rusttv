//! History entries and changes in words (English; the TV says them in its
//! own language) and as JSON (CLI.md § HistoryEntry, HistoryChange).

use rusttv_history::{ApprovedBy, Author, Change, Entry, PackageState};
use serde::Serialize;

/// Before this (2024-01-01 UTC) a TV clock hasn't been set: "time unknown".
const CLOCK_SET_AFTER: i64 = 1_704_067_200;

/// One entry in `--json` output.
#[derive(Debug, Serialize)]
pub(crate) struct EntryView {
    id: String,
    parent: Option<String>,
    /// Unix seconds by the recording device's clock; `null` before 2024
    /// (the TV's clock wasn't set).
    time_unix: Option<i64>,
    /// `tv` | `mac` | `drift` | `import`.
    author: String,
    summary: String,
    /// The Mac journal file a `mac` entry came from.
    journal: Option<String>,
    /// `tv-code <sha256>` | `terminal` | `delegation` | `upgrade`, or null.
    approved_by: Option<String>,
}

impl EntryView {
    pub(crate) fn new(entry: &Entry) -> Self {
        Self {
            id: entry.id.to_string(),
            parent: entry.parent.as_ref().map(ToString::to_string),
            time_unix: (entry.time_unix >= CLOCK_SET_AFTER).then_some(entry.time_unix),
            author: entry.meta.author.to_string(),
            summary: entry.summary.clone(),
            journal: entry.meta.journal.clone(),
            approved_by: entry.meta.approved_by.as_ref().map(|by| match by {
                ApprovedBy::TvCode(hash) => format!("tv-code {hash}"),
                ApprovedBy::Terminal => "terminal".to_owned(),
                ApprovedBy::Delegation => "delegation".to_owned(),
                ApprovedBy::Upgrade => "upgrade".to_owned(),
            }),
        }
    }
}

/// One line for `history log`: short id, time, who, summary.
pub(crate) fn entry_line(entry: &Entry) -> String {
    format!(
        "{}  {}  {:24}  {}",
        &entry.id.as_str()[..12],
        time(entry.time_unix),
        who(entry.meta.author),
        entry.summary
    )
}

/// Who made an entry, as the TV says it.
pub(crate) fn who(author: Author) -> &'static str {
    match author {
        Author::Tv => "This TV",
        Author::Mac => "Your computer",
        Author::Drift => "Changed outside RustTV",
        Author::Import => "Restored from your computer",
    }
}

/// `YYYY-MM-DD HH:MM UTC`, or "time unknown" before the TV's clock was set.
pub(crate) fn time(unix: i64) -> String {
    if unix < CLOCK_SET_AFTER {
        return "time unknown        ".to_owned();
    }
    let days = unix.div_euclid(86_400);
    let seconds = unix.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3600,
        seconds % 3600 / 60
    )
}

/// One change in `--json` output.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ChangeView {
    /// Tiles moved on Home.
    Reordered { moved: Vec<String> },
    /// An app hidden or shown again.
    Hidden { tile: String, hidden: bool },
    /// The launcher's update switch.
    UpdateCheck { on: bool },
    /// A global setting; null = unset.
    Setting {
        key: String,
        old: Option<String>,
        new: Option<String>,
    },
    /// A catalog package; null = not observed.
    Package {
        package: String,
        old: Option<String>,
        new: Option<String>,
    },
    /// The firmware fingerprint.
    Firmware {
        old: Option<String>,
        new: Option<String>,
    },
    /// A Home takeover started or ended.
    Takeover { active: bool },
}

impl ChangeView {
    pub(crate) fn new(change: &Change) -> Option<Self> {
        let state = |state: &Option<PackageState>| state.map(|state| state.to_string());
        Some(match change {
            Change::Reordered { moved } => Self::Reordered {
                moved: moved.clone(),
            },
            Change::Hidden { tile, hidden } => Self::Hidden {
                tile: tile.clone(),
                hidden: *hidden,
            },
            Change::UpdateCheck { on } => Self::UpdateCheck { on: *on },
            Change::Setting { key, old, new } => Self::Setting {
                key: key.clone(),
                old: old.clone(),
                new: new.clone(),
            },
            Change::Package { name, old, new } => Self::Package {
                package: name.clone(),
                old: state(old),
                new: state(new),
            },
            Change::Firmware { old, new } => Self::Firmware {
                old: old.clone(),
                new: new.clone(),
            },
            Change::Takeover { active } => Self::Takeover { active: *active },
            _ => return None,
        })
    }
}

/// A change in words.
pub(crate) fn change(change: &Change) -> String {
    let setting = |value: &Option<String>| match value {
        Some(value) => format!("{value}×"),
        None => "1× (unset)".to_owned(),
    };
    let package = |state: &Option<PackageState>| {
        state.map_or_else(|| "not seen".to_owned(), |state| state.to_string())
    };
    let text = |value: &Option<String>| value.clone().unwrap_or_else(|| "unknown".to_owned());
    match change {
        Change::Reordered { moved } => format!("Moved on Home: {}", moved.join(", ")),
        Change::Hidden { tile, hidden: true } => format!("Hid {tile}"),
        Change::Hidden { tile, hidden: false } => format!("Showed {tile} again"),
        Change::UpdateCheck { on } => format!(
            "Check for updates automatically: {}",
            if *on { "on" } else { "off" }
        ),
        Change::Setting { key, old, new } => {
            format!("{}: {} → {}", setting_name(key), setting(old), setting(new))
        }
        Change::Package { name, old, new } => {
            format!("{name}: {} → {}", package(old), package(new))
        }
        Change::Firmware { old, new } => format!("Firmware: {} → {}", text(old), text(new)),
        Change::Takeover { active: true } => "Home takeover started".to_owned(),
        Change::Takeover { active: false } => "Home takeover ended".to_owned(),
        other => format!("{other:?}"),
    }
}

fn setting_name(key: &str) -> &str {
    match key {
        "window_animation_scale" => "Window animation scale",
        "transition_animation_scale" => "Transition animation scale",
        "animator_duration_scale" => "Animator duration scale",
        other => other,
    }
}

/// Seconds as `1 h 40 min` / `12 min`.
pub(crate) fn duration(seconds: u64) -> String {
    let minutes = seconds.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_utc_dates_and_unset_clocks_say_so() {
        assert_eq!(time(1_792_843_149), "2026-10-24 11:59 UTC");
        assert_eq!(time(1_704_067_200), "2024-01-01 00:00 UTC");
        assert!(time(0).starts_with("time unknown"));
    }

    #[test]
    fn durations_round_up_to_minutes() {
        assert_eq!(duration(6000), "1 h 40 min");
        assert_eq!(duration(7200), "2 h");
        assert_eq!(duration(1), "1 min");
    }
}

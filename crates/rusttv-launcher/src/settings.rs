//! The Settings page (HISTORY.md § 8): its sections, D-pad navigation and
//! what OK asks for. No UI or platform code; ui.rs draws it and carries out
//! the returned [`Action`]s.

use std::collections::BTreeMap;

use rusttv_history::door::{ApprovedChange, PendingApproval};
use rusttv_history::{Author, Change, EntryId, Launcher, SETTING_KEYS};

use crate::model::Input;

/// What the history thread knows when the page opens.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Overview {
    /// The allowlisted settings now; a missing key is unset.
    pub settings: BTreeMap<String, String>,
    /// The launcher's own state now.
    pub launcher: Launcher,
    /// Whether the launcher may write settings itself.
    pub can_write_settings: bool,
    /// The history, newest first.
    pub entries: Vec<EntryView>,
    /// Approvals waiting for the computer.
    pub pending: Vec<PendingApproval>,
    /// Milliseconds approvals stay locked after wrong codes.
    pub locked_ms: u64,
    /// A debuggable or profileable build: approvals are off.
    pub test_build: bool,
    /// Reading the history failed (shown instead of the list).
    pub error: Option<String>,
}

/// One history entry, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EntryView {
    /// Its id.
    pub id: EntryId,
    /// When (TV clock, Unix seconds).
    pub time_unix: i64,
    /// Who.
    pub author: Author,
    /// Put back from the computer (at or before the newest import).
    pub imported: bool,
    /// What changed from the entry before it.
    pub changes: Vec<Change>,
}

/// A section of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Animations,
    Updates,
    History,
    Waiting,
}

/// The animation choices, from off to slowest: every scale set to this
/// value (Android's developer options offer the same range).
pub(crate) const ANIMATION_CHOICES: [&str; 7] = ["0", "0.25", "0.5", "0.75", "1", "1.5", "2"];

/// The choice that is Android's default.
pub(crate) const NORMAL_ANIMATIONS: usize = 4;

/// Something ui.rs must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Leave Settings.
    Close,
    /// Review these changes (HISTORY.md § 8.5).
    Review(Draft),
    /// Restore the state of this entry: ui.rs asks the history thread for
    /// the changes from now to it, then reviews them.
    Restore(EntryId),
}

/// Changes waiting for review.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    /// Settings and packages.
    pub changes: Vec<ApprovedChange>,
    /// The launcher's state to write, if it changes.
    pub launcher: Option<Launcher>,
}

impl Draft {
    /// Whether there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.launcher.is_none()
    }
}

/// Where focus is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    /// The section list.
    List,
    /// A row of the selected section's panel.
    Panel(usize),
    /// A history entry's detail; its one row is "Restore this state".
    Detail(usize),
}

/// The Settings page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SettingsPage {
    overview: Overview,
    section: usize,
    focus: Focus,
    right_to_left: bool,
}

impl SettingsPage {
    /// The page as it opens, on its first section.
    pub fn new(overview: Overview, right_to_left: bool) -> Self {
        Self {
            overview,
            section: 0,
            focus: Focus::List,
            right_to_left,
        }
    }

    /// Replaces the data (after a change or a refresh), keeping focus where
    /// it still fits.
    pub fn refresh(&mut self, overview: Overview) {
        self.overview = overview;
        self.section = self.section.min(self.sections().len() - 1);
        let rows = self.rows();
        self.focus = match self.focus {
            Focus::Panel(row) if rows > 0 => Focus::Panel(row.min(rows - 1)),
            Focus::Detail(entry) if entry < self.overview.entries.len() => Focus::Detail(entry),
            _ => Focus::List,
        };
    }

    /// What the page shows.
    pub fn overview(&self) -> &Overview {
        &self.overview
    }

    /// The sections, in order; "Waiting" only when something waits.
    pub fn sections(&self) -> Vec<Section> {
        let mut sections = vec![Section::Animations, Section::Updates, Section::History];
        if !self.overview.pending.is_empty() {
            sections.push(Section::Waiting);
        }
        sections
    }

    /// The selected section.
    pub fn section(&self) -> Section {
        self.sections()[self.section.min(self.sections().len() - 1)]
    }

    /// The selected section's index in [`Self::sections`].
    pub fn section_index(&self) -> usize {
        self.section
    }

    /// The focused panel row, `None` while the section list has focus.
    pub fn panel_row(&self) -> Option<usize> {
        match self.focus {
            Focus::Panel(row) => Some(row),
            Focus::List | Focus::Detail(_) => None,
        }
    }

    /// The history entry whose detail is open.
    pub fn detail(&self) -> Option<&EntryView> {
        match self.focus {
            Focus::Detail(entry) => self.overview.entries.get(entry),
            _ => None,
        }
    }

    /// Whether the open detail's entry can be restored (not the newest).
    pub fn can_restore(&self) -> bool {
        matches!(self.focus, Focus::Detail(entry) if entry > 0)
    }

    /// The current animation choice, `None` when the scales differ or are
    /// set to something else.
    pub fn animation_choice(&self) -> Option<usize> {
        let values: Vec<String> = SETTING_KEYS
            .iter()
            .map(|key| {
                self.overview
                    .settings
                    .get(*key)
                    .map_or_else(|| "1".to_owned(), Clone::clone)
            })
            .collect();
        ANIMATION_CHOICES.iter().position(|choice| {
            values
                .iter()
                .all(|value| value.parse::<f64>().ok() == choice.parse::<f64>().ok())
        })
    }

    /// Where focus lands in the panel: on the current choice, else the top.
    fn entry_row(&self) -> usize {
        match self.section() {
            Section::Animations => self.animation_choice().unwrap_or(NORMAL_ANIMATIONS),
            Section::Updates => usize::from(!self.overview.launcher.update_check),
            Section::History | Section::Waiting => 0,
        }
    }

    fn rows(&self) -> usize {
        match self.section() {
            Section::Animations => ANIMATION_CHOICES.len(),
            Section::Updates => 2,
            Section::History => self.overview.entries.len(),
            Section::Waiting => self.overview.pending.len(),
        }
    }

    /// Applies one key; returns what ui.rs must do.
    pub fn handle(&mut self, input: Input) -> Option<Action> {
        // The panel is on the end side: Right in left-to-right, Left in right-to-left.
        let (toward_panel, toward_list) = if self.right_to_left {
            (Input::Left, Input::Right)
        } else {
            (Input::Right, Input::Left)
        };
        match self.focus {
            Focus::List => match input {
                Input::Up => self.section = self.section.saturating_sub(1),
                Input::Down => self.section = (self.section + 1).min(self.sections().len() - 1),
                Input::OkReleased if self.rows() > 0 => self.focus = Focus::Panel(self.entry_row()),
                input if input == toward_panel && self.rows() > 0 => {
                    self.focus = Focus::Panel(self.entry_row());
                }
                Input::Back => return Some(Action::Close),
                _ => {}
            },
            Focus::Panel(row) => match input {
                Input::Up => self.focus = Focus::Panel(row.saturating_sub(1)),
                Input::Down => self.focus = Focus::Panel((row + 1).min(self.rows() - 1)),
                Input::Back => self.focus = Focus::List,
                input if input == toward_list => self.focus = Focus::List,
                Input::OkReleased => return self.activate(row),
                _ => {}
            },
            Focus::Detail(entry) => match input {
                Input::Back => self.focus = Focus::Panel(entry),
                input if input == toward_list => self.focus = Focus::Panel(entry),
                Input::OkReleased if entry > 0 => {
                    return Some(Action::Restore(self.overview.entries[entry].id.clone()));
                }
                _ => {}
            },
        }
        None
    }

    fn activate(&mut self, row: usize) -> Option<Action> {
        match self.section() {
            Section::Animations => {
                if self.animation_choice() == Some(row) {
                    return None;
                }
                let value = ANIMATION_CHOICES[row];
                let changes = SETTING_KEYS
                    .iter()
                    .map(|key| ApprovedChange::Setting {
                        key: (*key).to_owned(),
                        old: self.overview.settings.get(*key).cloned(),
                        new: Some(value.to_owned()),
                    })
                    .collect();
                Some(Action::Review(Draft {
                    changes,
                    launcher: None,
                }))
            }
            Section::Updates => {
                let on = row == 0;
                if self.overview.launcher.update_check == on {
                    return None;
                }
                let mut launcher = self.overview.launcher.clone();
                launcher.update_check = on;
                Some(Action::Review(Draft {
                    changes: Vec::new(),
                    launcher: Some(launcher),
                }))
            }
            Section::History => {
                self.focus = Focus::Detail(row);
                None
            }
            Section::Waiting => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: u8) -> EntryView {
        EntryView {
            id: EntryId::parse(&format!("{n:02x}").repeat(20)).unwrap(),
            time_unix: 1_790_000_000 + i64::from(n),
            author: Author::Tv,
            imported: false,
            changes: Vec::new(),
        }
    }

    fn page() -> SettingsPage {
        SettingsPage::new(
            Overview {
                entries: vec![entry(2), entry(1)],
                can_write_settings: true,
                ..Overview::default()
            },
            false,
        )
    }

    #[test]
    fn choosing_faster_animations_reviews_all_three_scales() {
        let mut page = page();
        assert_eq!(page.animation_choice(), Some(NORMAL_ANIMATIONS));
        assert_eq!(page.handle(Input::Right), None);
        assert_eq!(
            page.panel_row(),
            Some(NORMAL_ANIMATIONS),
            "focus starts on the current choice"
        );
        assert_eq!(page.handle(Input::OkReleased), None, "Normal is already on");
        page.handle(Input::Up);
        page.handle(Input::Up);
        let Some(Action::Review(draft)) = page.handle(Input::OkReleased) else {
            panic!("expected a review");
        };
        assert_eq!(draft.changes.len(), 3);
        assert!(draft.changes.iter().all(|change| matches!(
            change,
            ApprovedChange::Setting { old: None, new: Some(new), .. } if new == "0.5"
        )));
    }

    #[test]
    fn turning_updates_off_reviews_the_launcher_state() {
        let mut page = page();
        page.handle(Input::Down);
        page.handle(Input::Right);
        page.handle(Input::Down);
        let Some(Action::Review(draft)) = page.handle(Input::OkReleased) else {
            panic!("expected a review");
        };
        assert_eq!(draft.launcher.map(|l| l.update_check), Some(false));
    }

    #[test]
    fn an_older_entry_can_be_restored_but_not_the_newest() {
        let mut page = page();
        page.handle(Input::Down);
        page.handle(Input::Down);
        assert_eq!(page.section(), Section::History);
        page.handle(Input::Right);
        page.handle(Input::OkReleased);
        assert!(!page.can_restore());
        assert_eq!(page.handle(Input::OkReleased), None);
        page.handle(Input::Back);
        page.handle(Input::Down);
        page.handle(Input::OkReleased);
        assert!(page.can_restore());
        assert_eq!(
            page.handle(Input::OkReleased),
            Some(Action::Restore(entry(1).id))
        );
    }

    #[test]
    fn back_walks_out_and_right_to_left_mirrors_the_panel_key() {
        let mut page = SettingsPage::new(Overview::default(), true);
        assert_eq!(page.handle(Input::Right), None);
        assert_eq!(page.panel_row(), None);
        page.handle(Input::Left);
        assert_eq!(page.panel_row(), Some(NORMAL_ANIMATIONS));
        page.handle(Input::Back);
        assert_eq!(page.panel_row(), None);
        assert_eq!(page.handle(Input::Back), Some(Action::Close));
    }

    #[test]
    fn waiting_shows_only_with_pending_approvals() {
        let mut page = page();
        assert_eq!(page.sections().len(), 3);
        let mut overview = page.overview().clone();
        overview.pending.push(PendingApproval {
            hash: "a".repeat(64),
            seconds_left: 60,
            change: Vec::new(),
        });
        page.refresh(overview);
        assert_eq!(page.sections().last(), Some(&Section::Waiting));
    }
}

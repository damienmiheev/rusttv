//! The Settings page (HISTORY.md § 8): its sections, D-pad navigation and
//! what OK asks for. No UI or platform code; ui.rs draws it and carries out
//! the returned [`Action`]s.
//!
//! Its tabs are the launcher's own sections, then the tabs the platform adds
//! (device.rs), then About.

use std::collections::BTreeMap;

use rusttv_history::door::{ApprovedChange, PendingApproval};
use rusttv_history::{Author, Change, EntryId, Launcher, SETTING_KEYS};

use crate::device::{Adjust, ControlKind, Tab, Value, adjust};
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
    /// The first entry: the state when the history started.
    pub first: bool,
    /// What changed from the entry before it.
    pub changes: Vec<Change>,
}

/// A section of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Animations,
    /// Full or reduced effects (shadows, backdrop) for a weak GPU.
    Effects,
    /// The idle screen on or off (PANELS.md § 9 Q6): after a while on
    /// Home, a dim clock.
    Ambient,
    Updates,
    History,
    Waiting,
    /// A tab the platform adds, by its index in [`SettingsPage::tabs`].
    Tab(usize),
    /// RustTV and the host; always last.
    About,
}

/// A row of the About section, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AboutRow {
    /// Name, version and licence.
    RustTv,
    /// The updater's status; OK checks or installs.
    Update,
    /// Where the source code is.
    Source,
    /// A fact about the host, by its index in [`SettingsPage::host`].
    Host(usize),
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
    /// Send a control's new value to the device (device.rs); the page
    /// already shows it.
    Set {
        tab: String,
        control: String,
        value: Value,
    },
    /// Ask for a text (on-screen keyboard), then send it to the device as
    /// [`Value::Text`].
    AskText {
        tab: String,
        control: String,
        title: String,
        secret: bool,
    },
    /// Install the available launcher update, else check for one.
    Update,
    /// Open the licence and third-party notices (the About page).
    Notices,
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
    /// The platform's tabs, once read.
    tabs: Vec<Tab>,
    /// Facts about the host for About, as (label, value).
    host: Vec<(String, String)>,
    section: usize,
    focus: Focus,
    right_to_left: bool,
    /// Whether the platform's tabs have been read (or there are none), so
    /// the section list is complete and won't grow while it is shown.
    loaded: bool,
    /// Whether the About tab is there ([`SettingsPage::with_about`]).
    about: bool,
}

impl SettingsPage {
    /// The page as it opens, on its first section, waiting for the
    /// platform's tabs ([`Self::set_device`], [`Self::mark_loaded`]).
    pub fn new(overview: Overview, right_to_left: bool) -> Self {
        Self {
            overview,
            tabs: Vec::new(),
            host: Vec::new(),
            section: 0,
            focus: Focus::List,
            right_to_left,
            loaded: false,
            about: true,
        }
    }

    /// Whether the section list is complete (see [`Self::new`]).
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Whether Settings has its About tab: where a top-right menu replaces
    /// the About tile (Linux), and while the About tile is hidden, so the
    /// licence notices and Slint's attribution stay reachable (PANELS.md
    /// § 6). Otherwise the About tile opens the About page and Settings
    /// stays as it was.
    pub fn with_about(mut self, about: bool) -> Self {
        self.about = about;
        self
    }

    /// The platform has no tabs to read: the list is complete as it is.
    pub fn mark_loaded(&mut self) {
        self.loaded = true;
    }

    /// Selects the platform tab `id` with focus on its first row; returns
    /// whether there is one.
    pub fn show_tab(&mut self, id: &str) -> bool {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        let wanted = Section::Tab(index);
        let Some(section) = self
            .sections()
            .iter()
            .position(|section| *section == wanted)
        else {
            return false;
        };
        self.section = section;
        self.focus = if self.rows() > 0 {
            Focus::Panel(0)
        } else {
            Focus::List
        };
        true
    }

    /// Selects the History section, with focus on the section list (its
    /// entries arrive later): History's tile opens Settings here.
    pub fn show_history(&mut self) {
        if let Some(section) = self
            .sections()
            .iter()
            .position(|section| *section == Section::History)
        {
            self.section = section;
            self.focus = Focus::List;
        }
    }

    /// Replaces the data (after a change or a refresh), keeping focus where
    /// it still fits.
    pub fn refresh(&mut self, overview: Overview) {
        let section = self.section();
        self.overview = overview;
        self.keep(section);
    }

    /// Replaces the platform's tabs and facts about the host (read off the
    /// UI thread), keeping focus where it still fits.
    pub fn set_device(&mut self, tabs: Vec<Tab>, host: Vec<(String, String)>) {
        let section = self.section();
        self.tabs = tabs;
        self.host = host;
        self.loaded = true;
        self.keep(section);
    }

    /// The platform's tabs.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// Facts about the host, as (label, value).
    pub fn host(&self) -> &[(String, String)] {
        &self.host
    }

    /// Selects `section` again after the sections changed (a tab added or
    /// gone), else the nearest one; keeps the panel row where it fits.
    fn keep(&mut self, section: Section) {
        let sections = self.sections();
        self.section = sections
            .iter()
            .position(|known| *known == section)
            .unwrap_or_else(|| self.section.min(sections.len() - 1));
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

    /// The sections, in order; "Waiting" only when something waits, then
    /// the platform's tabs and About.
    pub fn sections(&self) -> Vec<Section> {
        let mut sections = vec![
            Section::Animations,
            Section::Effects,
            Section::Ambient,
            Section::Updates,
            Section::History,
        ];
        if !self.overview.pending.is_empty() {
            sections.push(Section::Waiting);
        }
        sections.extend((0..self.tabs.len()).map(Section::Tab));
        if self.about {
            sections.push(Section::About);
        }
        sections
    }

    /// The About section's rows, top to bottom.
    pub fn about_rows(&self) -> Vec<AboutRow> {
        // OK on RustTV opens the About page (licences and notices).
        let mut rows = vec![AboutRow::RustTv, AboutRow::Update, AboutRow::Source];
        rows.extend((0..self.host.len()).map(AboutRow::Host));
        rows
    }

    /// The selected section.
    pub fn section(&self) -> Section {
        self.sections()[self.section.min(self.sections().len() - 1)]
    }

    /// The selected section's index in [`Self::sections`].
    pub fn section_index(&self) -> usize {
        self.section
    }

    /// The pointer is over section `index`: the list has focus, on it.
    pub fn point_section(&mut self, index: usize) {
        if index < self.sections().len() {
            self.section = index;
            self.focus = Focus::List;
        }
    }

    /// The pointer is over row `row` of the panel: it has focus.
    pub fn point_row(&mut self, row: usize) {
        if row < self.rows() && !matches!(self.focus, Focus::Detail(_)) {
            self.focus = Focus::Panel(row);
        }
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
            Section::Effects => usize::from(self.overview.launcher.reduced_effects),
            Section::Ambient => usize::from(self.overview.launcher.ambient_off),
            Section::Updates => usize::from(!self.overview.launcher.update_check),
            Section::History | Section::Waiting | Section::Tab(_) | Section::About => 0,
        }
    }

    fn rows(&self) -> usize {
        match self.section() {
            Section::Animations => ANIMATION_CHOICES.len(),
            Section::Effects | Section::Ambient | Section::Updates => 2,
            Section::History => self.overview.entries.len(),
            Section::Waiting => self.overview.pending.len(),
            Section::Tab(tab) => self.tabs.get(tab).map_or(0, |tab| tab.controls.len()),
            Section::About => self.about_rows().len(),
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
            Focus::Panel(row) => {
                // A platform tab's control takes the keys it uses first (a
                // slider takes Left and Right).
                if let Some(action) = self.adjust_control(row, input) {
                    return action;
                }
                match input {
                    Input::Up => self.focus = Focus::Panel(row.saturating_sub(1)),
                    Input::Down => self.focus = Focus::Panel((row + 1).min(self.rows() - 1)),
                    Input::Back => self.focus = Focus::List,
                    input if input == toward_list => self.focus = Focus::List,
                    Input::OkReleased => return self.activate(row),
                    _ => {}
                }
            }
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

    /// Applies `input` to the control on `row` of a platform tab: `None`
    /// when it isn't for the control, else what to do.
    fn adjust_control(&mut self, row: usize, input: Input) -> Option<Option<Action>> {
        let Section::Tab(index) = self.section() else {
            return None;
        };
        let right_to_left = self.right_to_left;
        let tab = self.tabs.get_mut(index)?;
        let control = tab.controls.get_mut(row)?;
        match adjust(control, input, right_to_left) {
            Adjust::Ignored => None,
            Adjust::Kept => Some(None),
            Adjust::Changed(value) => Some(Some(Action::Set {
                tab: tab.id.clone(),
                control: control.id.clone(),
                value,
            })),
            Adjust::AskText => Some(Some(Action::AskText {
                tab: tab.id.clone(),
                control: control.id.clone(),
                title: control.label.clone(),
                secret: matches!(control.kind, ControlKind::Text { secret: true }),
            })),
        }
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
            Section::Effects => {
                let reduced = row == 1;
                if self.overview.launcher.reduced_effects == reduced {
                    return None;
                }
                let mut launcher = self.overview.launcher.clone();
                launcher.reduced_effects = reduced;
                Some(Action::Review(Draft {
                    changes: Vec::new(),
                    launcher: Some(launcher),
                }))
            }
            Section::Ambient => {
                let off = row == 1;
                if self.overview.launcher.ambient_off == off {
                    return None;
                }
                let mut launcher = self.overview.launcher.clone();
                launcher.ambient_off = off;
                Some(Action::Review(Draft {
                    changes: Vec::new(),
                    launcher: Some(launcher),
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
            Section::Waiting | Section::Tab(_) => None,
            Section::About => match self.about_rows().get(row)? {
                AboutRow::Update => Some(Action::Update),
                AboutRow::RustTv => Some(Action::Notices),
                AboutRow::Source | AboutRow::Host(_) => None,
            },
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
            first: false,
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
    fn reducing_effects_reviews_the_launcher_state() {
        let mut page = page();
        page.handle(Input::Down);
        page.handle(Input::Right);
        page.handle(Input::Down);
        let Some(Action::Review(draft)) = page.handle(Input::OkReleased) else {
            panic!("expected a review");
        };
        assert_eq!(draft.launcher.map(|l| l.reduced_effects), Some(true));
    }

    #[test]
    fn turning_the_idle_screen_off_reviews_the_launcher_state() {
        let mut page = page();
        page.handle(Input::Down);
        page.handle(Input::Down);
        assert_eq!(page.section(), Section::Ambient);
        page.handle(Input::Right);
        assert_eq!(page.panel_row(), Some(0), "focus starts on On, the default");
        assert_eq!(page.handle(Input::OkReleased), None, "On is already on");
        page.handle(Input::Down);
        let Some(Action::Review(draft)) = page.handle(Input::OkReleased) else {
            panic!("expected a review");
        };
        assert_eq!(draft.launcher.map(|l| l.ambient_off), Some(true));
    }

    #[test]
    fn turning_updates_off_reviews_the_launcher_state() {
        let mut page = page();
        page.handle(Input::Down);
        page.handle(Input::Down);
        page.handle(Input::Down);
        assert_eq!(page.section(), Section::Updates);
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
        for _ in 0..4 {
            page.handle(Input::Down);
        }
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
        assert_eq!(page.sections().len(), 6);
        let mut overview = page.overview().clone();
        overview.pending.push(PendingApproval {
            hash: "a".repeat(64),
            seconds_left: 60,
            change: Vec::new(),
        });
        page.refresh(overview);
        assert_eq!(page.sections()[5], Section::Waiting);
        assert_eq!(page.sections().last(), Some(&Section::About));
    }

    #[test]
    fn platform_tabs_come_before_about_and_send_their_changes() {
        use crate::device::{DeviceSettings, FakeDevice};

        let device = FakeDevice::new();
        let mut page = page();
        // Down to About, where the tabs will be inserted before it.
        for _ in 0..6 {
            page.handle(Input::Down);
        }
        assert_eq!(page.section(), Section::About);
        page.set_device(device.tabs(), device.about());
        assert_eq!(
            page.sections()[5..],
            [Section::Tab(0), Section::About],
            "the tabs sit before About"
        );
        assert_eq!(
            page.section(),
            Section::About,
            "the selection stays on About"
        );
        page.handle(Input::Up);
        assert_eq!(page.section(), Section::Tab(0));
        page.handle(Input::Right);
        assert_eq!(
            page.handle(Input::OkReleased),
            Some(Action::Set {
                tab: "display".into(),
                control: "night-light".into(),
                value: Value::Toggle(true),
            })
        );
        match &page.tabs()[0].controls[0].kind {
            crate::device::ControlKind::Toggle { on } => assert!(*on, "shown at once"),
            other => panic!("not a toggle: {other:?}"),
        }
        // Left on a toggle goes back to the sections.
        page.handle(Input::Left);
        assert_eq!(page.panel_row(), None);
    }

    #[test]
    fn about_rows_check_for_updates_and_open_the_notices() {
        let mut page = page();
        page.set_device(Vec::new(), vec![("Kernel".into(), "6.18".into())]);
        for _ in 0..5 {
            page.handle(Input::Down);
        }
        assert_eq!(page.section(), Section::About);
        assert_eq!(
            page.about_rows(),
            [
                AboutRow::RustTv,
                AboutRow::Update,
                AboutRow::Source,
                AboutRow::Host(0),
            ]
        );
        page.handle(Input::Right);
        assert_eq!(
            page.handle(Input::OkReleased),
            Some(Action::Notices),
            "RustTV opens the About page"
        );
        page.handle(Input::Down);
        assert_eq!(page.handle(Input::OkReleased), Some(Action::Update));
        for _ in 0..5 {
            page.handle(Input::Down);
        }
        assert_eq!(page.handle(Input::OkReleased), None, "a host fact");
    }

    #[test]
    fn the_list_is_complete_once_loaded_and_a_tab_opens_directly() {
        let mut page = page();
        assert!(!page.is_loaded());
        let control = crate::device::Control {
            id: "wifi".into(),
            label: "Wi-Fi".into(),
            detail: String::new(),
            kind: crate::device::ControlKind::Toggle { on: true },
        };
        let tab = |id: &str| Tab {
            id: id.into(),
            title: id.into(),
            controls: vec![control.clone()],
        };
        page.set_device(vec![tab("sound"), tab("network")], Vec::new());
        assert!(page.is_loaded());
        assert!(page.show_tab("network"));
        assert_eq!(page.section(), Section::Tab(1));
        assert_eq!(page.panel_row(), Some(0));
        assert!(!page.show_tab("display"));
    }

    /// History's tile opens Settings on the History section with the
    /// section list focused, wherever focus was, and it stays there when
    /// the platform's tabs or the history's data arrive.
    #[test]
    fn show_history_selects_history_with_the_list_focused() {
        let mut page = page();
        page.handle(Input::Right);
        assert!(page.panel_row().is_some(), "focus in Animations' rows");
        page.show_history();
        assert_eq!(page.section(), Section::History);
        assert_eq!(page.panel_row(), None);
        page.set_device(Vec::new(), Vec::new());
        page.refresh(page.overview().clone());
        assert_eq!(page.section(), Section::History);
        assert_eq!(page.panel_row(), None);
        // OK then enters its entries.
        page.handle(Input::OkReleased);
        assert_eq!(page.panel_row(), Some(0));
    }
}

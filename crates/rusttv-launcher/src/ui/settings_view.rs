//! The Settings page and the review screen in the window (HISTORY.md § 8):
//! opening them, routing keys, approving and applying, and drawing them.
//!
//! Data comes from the history thread off the UI thread and is picked up
//! by a short timer, so Home never waits for it.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use rusttv_history::door::ApprovedChange;
use rusttv_history::tv::{ApplyOutcome, Refused};
use rusttv_history::{Author, Change, PackageState, SETTING_KEYS, State, diff};
use slint::{ComponentHandle, Model, SharedString, Timer, TimerMode};

use super::{Controller, HomeWindow, ReviewLine, SettingsRow, Strings};
use crate::history::History;
use crate::model::Input;
use crate::order_file;
use crate::review::{Arrow, Outcome, Phase, Review};
use crate::settings::{Action, Draft, EntryView, Overview, Section, SettingsPage};

/// How long after leaving the screen the history is packed (HISTORY.md § 3):
/// not at once, so it doesn't compete with the app being opened.
const PACK_AFTER: Duration = Duration::from_secs(5);

/// Panel rows visible at once (the rest scroll by rows).
const PANEL_ROWS: usize = 6;

/// Review lines visible at once.
const REVIEW_ROWS: usize = 8;

/// Times before this (2024-01-01) mean the TV's clock wasn't set.
const CLOCK_SET_AFTER: i64 = 1_704_067_200;

thread_local! {
    /// The controller, for results that arrive from other threads.
    static CONTROLLER: RefCell<Weak<RefCell<Controller>>> = const { RefCell::new(Weak::new()) };
}

/// Lets results from the history thread find the controller.
pub(super) fn register(controller: &Rc<RefCell<Controller>>) {
    CONTROLLER.with(|slot| *slot.borrow_mut() = Rc::downgrade(controller));
}

/// How often the UI looks for a result from a worker thread, only while
/// one is pending.
const POLL: Duration = Duration::from_millis(16);

/// Runs `work` on a new thread and `done` with its result on the UI thread.
/// The UI picks the result up with a short timer that runs only until it
/// arrives (no event-loop proxy needed, and it works with mocked time).
fn in_background<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut Controller, T) + 'static,
) -> std::io::Result<()> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        })?;
    poll(receiver, done);
    Ok(())
}

fn poll<T: 'static>(
    receiver: std::sync::mpsc::Receiver<T>,
    done: impl FnOnce(&mut Controller, T) + 'static,
) {
    Timer::single_shot(POLL, move || match receiver.try_recv() {
        Ok(result) => {
            let controller = CONTROLLER.with(|slot| slot.borrow().upgrade());
            if let Some(controller) = controller {
                done(&mut controller.borrow_mut(), result);
            }
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => poll(receiver, done),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {}
    });
}

/// `settings-status` kinds (home.slint).
mod status {
    pub const NONE: i32 = 0;
    pub const APPLIED: i32 = 1;
    pub const SAVED_FOR_COMPUTER: i32 = 2;
    pub const NOTHING_CHANGED: i32 = 3;
    pub const WRONG_CODE: i32 = 4;
    pub const LOCKED: i32 = 5;
    pub const TEST_BUILD: i32 = 6;
    pub const BUSY: i32 = 7;
    pub const ROLLBACK_FAILED: i32 = 8;
    pub const NOTHING_TO_RESTORE: i32 = 9;
    pub const FORMAT_NEWER: i32 = 10;
    pub const READING: i32 = 11;
    pub const UNREADABLE: i32 = 12;
}

impl Controller {
    /// OK on the Settings tile.
    pub(super) fn open_settings(&mut self) {
        self.settings = Some(SettingsPage::new(
            Overview::default(),
            self.options.right_to_left,
        ));
        self.settings_status = (status::READING, String::new());
        self.refresh_settings_data();
    }

    /// Asks the history thread for the page's data; it arrives later.
    fn refresh_settings_data(&mut self) {
        let Some(history) = History::get() else {
            self.settings_status = (status::UNREADABLE, "history is off".to_owned());
            return;
        };
        let started = in_background(
            "rusttv-settings",
            move || history.overview(),
            Controller::overview_arrived,
        );
        if let Err(error) = started {
            self.settings_status = (status::UNREADABLE, error.to_string());
        }
    }

    fn overview_arrived(&mut self, overview: Overview) {
        let Some(page) = &mut self.settings else {
            return;
        };
        match &overview.error {
            Some(error) => self.settings_status = (status::UNREADABLE, error.clone()),
            None if self.settings_status.0 == status::READING => {
                self.settings_status = (status::NONE, String::new());
            }
            None => {}
        }
        page.refresh(overview);
        self.render();
    }

    /// A key while Settings (or its review screen) is open.
    pub(super) fn settings_input(&mut self, input: Input) {
        if self.review.is_some() {
            self.review_input(input);
            return;
        }
        if input == Input::OkReleased && std::mem::take(&mut self.swallow_ok_release) {
            return;
        }
        let Some(page) = &mut self.settings else {
            return;
        };
        if matches!(input, Input::Up | Input::Down | Input::OkReleased)
            && !matches!(self.settings_status.0, status::READING | status::UNREADABLE)
        {
            self.settings_status = (status::NONE, String::new());
        }
        match page.handle(input) {
            Some(Action::Close) => {
                self.settings = None;
                self.settings_status = (status::NONE, String::new());
                self.home.close_settings();
            }
            Some(Action::Review(draft)) => self.open_review(draft),
            Some(Action::Restore(id)) => {
                let Some(history) = History::get() else {
                    return;
                };
                match history.restore_draft(id) {
                    Ok(draft) if draft.is_empty() => {
                        self.settings_status = (status::NOTHING_TO_RESTORE, String::new());
                    }
                    Ok(draft) => self.open_review(draft),
                    Err(error) => self.settings_status = (status::NOTHING_CHANGED, error),
                }
            }
            None => {}
        }
    }

    fn open_review(&mut self, draft: Draft) {
        let Some(page) = &self.settings else {
            return;
        };
        let overview = page.overview();
        if overview.test_build {
            self.settings_status = (status::TEST_BUILD, String::new());
            return;
        }
        if overview.locked_ms > 0 {
            self.settings_status = (status::LOCKED, self.duration_text(overview.locked_ms));
            return;
        }
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let lines = self.review_lines_for(&window, &draft, overview);
        let code = self.platform.random_code().map(Arrow::from_random);
        let count = lines.len();
        self.review_lines.set_vec(lines);
        self.review_code.set_vec(
            code.iter()
                .map(|arrow| arrow_value(*arrow))
                .collect::<Vec<_>>(),
        );
        // Secure before the code can ever be drawn (it appears after Approve).
        self.platform.set_secure(true);
        self.review = Some(Review::new(draft, count, code, (self.clock)()));
        self.settings_status = (status::NONE, String::new());
    }

    fn review_input(&mut self, input: Input) {
        let now = (self.clock)();
        let Some(review) = &mut self.review else {
            return;
        };
        let outcome = review.handle(input, now);
        match review.hold_ends() {
            Some(end) => {
                let me = self.me.clone();
                self.hold_timer
                    .start(TimerMode::SingleShot, end.saturating_sub(now), move || {
                        if let Some(controller) = me.upgrade() {
                            controller.borrow_mut().input(Input::Tick);
                        }
                    });
            }
            None => self.hold_timer.stop(),
        }
        match outcome {
            None => {}
            Some(Outcome::Cancelled) => self.close_review(),
            Some(Outcome::WrongCode) => {
                self.close_review();
                let locked = History::get().map_or(0, History::wrong_code);
                self.settings_status = if locked > 0 {
                    (status::LOCKED, self.duration_text(locked))
                } else {
                    (status::WRONG_CODE, String::new())
                };
                self.refresh_settings_data();
            }
            Some(Outcome::Confirmed) => {
                let draft = review.draft().clone();
                self.close_review();
                self.swallow_ok_release = true;
                let Some(history) = History::get() else {
                    return;
                };
                history.correct_code();
                self.settings_status = (status::BUSY, String::new());
                let started = in_background(
                    "rusttv-apply",
                    move || history.apply(draft),
                    Controller::applied,
                );
                if let Err(error) = started {
                    self.settings_status = (status::NOTHING_CHANGED, error.to_string());
                }
            }
        }
    }

    fn applied(&mut self, result: Result<ApplyOutcome, Refused>) {
        self.settings_status = match result {
            Ok(ApplyOutcome::Applied { pending }) if pending.is_empty() => {
                (status::APPLIED, String::new())
            }
            Ok(ApplyOutcome::Applied { .. }) => (status::SAVED_FOR_COMPUTER, String::new()),
            Ok(ApplyOutcome::RolledBack { reason }) => (status::NOTHING_CHANGED, reason),
            Ok(ApplyOutcome::RollbackFailed { reason, left }) => {
                let names: Vec<String> = left
                    .iter()
                    .map(|(key, value)| format!("{key} = {}", value.as_deref().unwrap_or("-")))
                    .collect();
                (
                    status::ROLLBACK_FAILED,
                    format!("{reason} ({})", names.join(", ")),
                )
            }
            Err(refused) => match refused.code {
                "TEST_BUILD" => (status::TEST_BUILD, String::new()),
                "TV_BUSY" => (status::BUSY, String::new()),
                "HISTORY_FORMAT_NEWER" => (status::FORMAT_NEWER, String::new()),
                _ => (status::NOTHING_CHANGED, refused.message),
            },
        };
        if History::get().is_some_and(History::take_launcher_changed) {
            self.reload_launcher_state();
        }
        self.refresh_settings_data();
        self.render();
    }

    fn close_review(&mut self) {
        if self.review.take().is_some() {
            self.hold_timer.stop();
            self.platform.set_secure(false);
        }
    }

    /// The launcher left the screen: drafts and reviews are dropped.
    pub(super) fn close_settings_view(&mut self) {
        self.close_review();
        self.settings = None;
        self.settings_status = (status::NONE, String::new());
    }

    /// Packs the history a few seconds after a pause, unless the launcher
    /// is back by then (HISTORY.md § 3).
    pub(super) fn schedule_pack(&mut self) {
        if History::get().is_none() {
            return;
        }
        self.pack_timer
            .start(TimerMode::SingleShot, PACK_AFTER, || {
                if let Some(history) = History::get() {
                    history.pack();
                }
            });
    }

    /// Reloads the tile order and hidden apps after the history rewrote them.
    pub(super) fn reload_launcher_state(&mut self) {
        let Some(path) = &self.options.order_file else {
            return;
        };
        let loaded = order_file::load(path);
        self.home.replace_order(loaded.order, loaded.hidden);
        self.shown.clear();
        self.render();
    }

    fn duration_text(&self, ms: u64) -> String {
        let Some(window) = self.window.upgrade() else {
            return String::new();
        };
        duration_text(&window, ms)
    }

    /// The review screen's lines: what the TV changes now, then what the
    /// computer applies (HISTORY.md § 8.5).
    fn review_lines_for(
        &self,
        window: &HomeWindow,
        draft: &Draft,
        overview: &Overview,
    ) -> Vec<ReviewLine> {
        let strings = window.global::<Strings>();
        let mut tv = Vec::new();
        let mut computer = Vec::new();
        for change in &draft.changes {
            let line = match change {
                ApprovedChange::Setting { key, old, new } => change_line(
                    window,
                    &Change::Setting {
                        key: key.clone(),
                        old: old.clone(),
                        new: new.clone(),
                    },
                    |tile| self.tile_label(tile),
                ),
                ApprovedChange::Package { name, old, new } => change_line(
                    window,
                    &Change::Package {
                        name: name.clone(),
                        old: *old,
                        new: Some(*new),
                    },
                    |tile| self.tile_label(tile),
                ),
            };
            let on_tv =
                matches!(change, ApprovedChange::Setting { .. }) && overview.can_write_settings;
            if on_tv { &mut tv } else { &mut computer }.push(line);
        }
        if let Some(target) = &draft.launcher {
            let mut before = State::new();
            before.launcher = overview.launcher.clone();
            let mut after = State::new();
            after.launcher = target.clone();
            for change in diff(&before, &after) {
                tv.push(change_line(window, &change, |tile| self.tile_label(tile)));
            }
        }
        let mut lines = Vec::new();
        for (group, texts) in [(0, tv), (1, computer)] {
            if texts.is_empty() {
                continue;
            }
            lines.push(ReviewLine {
                text: strings.invoke_review_group(group),
                header: true,
            });
            lines.extend(texts.into_iter().map(|text| ReviewLine {
                text,
                header: false,
            }));
        }
        lines
    }

    /// An app's label with its package id, for the review screen and the
    /// history (labels come from apps, so bidi controls and line breaks go).
    fn tile_label(&self, tile: &str) -> String {
        let package = tile.split('/').next().unwrap_or(tile);
        match self.apps.get(tile) {
            Some(app) => {
                let label: String = app
                    .label
                    .chars()
                    .filter(|c| !c.is_control() && !is_bidi_control(*c))
                    .collect();
                format!("{label} ({package})")
            }
            None => package.to_owned(),
        }
    }

    /// Draws the Settings page and the review screen.
    pub(super) fn render_settings(&mut self, window: &HomeWindow) {
        let strings = window.global::<Strings>();
        let detail = match self.settings_status.0 {
            status::SAVED_FOR_COMPUTER => strings
                .invoke_run_apply_pending(
                    duration_text(window, rusttv_history::tv::PENDING_VALID_MS).into(),
                )
                .to_string(),
            _ => self.settings_status.1.clone(),
        };
        window.set_settings_status(
            strings.invoke_settings_status(self.settings_status.0, detail.into()),
        );

        if let Some(review) = &self.review {
            let (phase, row, entered) = match review.phase() {
                Phase::List { row } => (0, row, 0),
                Phase::Code { entered } => (1, self.review_lines.row_count(), entered),
                Phase::Hold { .. } => (2, self.review_lines.row_count(), 4),
            };
            window.set_review_phase(phase);
            window.set_review_row(to_i32(row));
            window.set_review_first_line(to_i32(row.saturating_sub(REVIEW_ROWS - 1)));
            window.set_review_entered(to_i32(entered));
            window.set_review_holding(matches!(review.phase(), Phase::Hold { since: Some(_) }));
        }

        let Some(page) = &self.settings else {
            return;
        };
        let names: Vec<SharedString> = page
            .sections()
            .iter()
            .map(|section| strings.invoke_section_name(section_value(*section)))
            .collect();
        if !self.settings_sections.iter().eq(names.iter().cloned()) {
            self.settings_sections.set_vec(names);
        }
        window.set_settings_section(to_i32(page.section_index()));
        let overview = page.overview();
        let mut note = Vec::new();
        let (title, rows, focused): (SharedString, Vec<SettingsRow>, Option<usize>) =
            if let Some(entry) = page.detail() {
                let mut rows: Vec<SettingsRow> = entry
                    .changes
                    .iter()
                    .map(|change| {
                        row(
                            change_line(window, change, |tile| self.tile_label(tile)),
                            "",
                        )
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push(row(
                        strings.invoke_change_line(8, "".into(), "".into(), "".into()),
                        "",
                    ));
                }
                let focused = if page.can_restore() {
                    rows.push(row(strings.invoke_restore_this(), ""));
                    Some(rows.len() - 1)
                } else {
                    Some(rows.len())
                };
                (self.when_and_who(window, entry).into(), rows, focused)
            } else {
                let section = page.section();
                let rows = match section {
                    Section::Animations => {
                        if !overview.can_write_settings {
                            note.push(strings.invoke_grant_hint().to_string());
                        }
                        if page.animation_choice() == Some(0) {
                            note.push(strings.invoke_off_note().to_string());
                        }
                        (0..crate::settings::ANIMATION_CHOICES.len())
                            .map(|choice| SettingsRow {
                                title: strings.invoke_animation_choice(to_i32(choice)),
                                detail: "".into(),
                                current: page.animation_choice() == Some(choice),
                                dim: !overview.can_write_settings,
                            })
                            .collect()
                    }
                    Section::Updates => [true, false]
                        .into_iter()
                        .map(|on| SettingsRow {
                            title: strings.invoke_on_off(on),
                            detail: "".into(),
                            current: overview.launcher.update_check == on,
                            dim: false,
                        })
                        .collect(),
                    Section::History => {
                        if overview.entries.is_empty()
                            && overview.error.is_none()
                            && self.settings_status.0 != status::READING
                        {
                            note.push(strings.invoke_history_empty().to_string());
                        }
                        overview
                            .entries
                            .iter()
                            .map(|entry| {
                                let title = summary_line(window, &entry.changes, |tile| {
                                    self.tile_label(tile)
                                });
                                row(title.into(), &self.when_and_who(window, entry))
                            })
                            .collect()
                    }
                    Section::Waiting => overview
                        .pending
                        .iter()
                        .map(|approval| {
                            let changes: Vec<Change> =
                                approval.change.iter().map(as_change).collect();
                            let title =
                                summary_line(window, &changes, |tile| self.tile_label(tile));
                            let left =
                                duration_text(window, approval.seconds_left.saturating_mul(1000));
                            row(title.into(), &strings.invoke_run_apply_pending(left.into()))
                        })
                        .collect(),
                };
                let title = if section == Section::Updates {
                    strings.invoke_update_check_title()
                } else {
                    strings.invoke_section_name(section_value(section))
                };
                (title, rows, page.panel_row())
            };
        window.set_settings_panel_title(title);
        window.set_settings_note(note.join("\n").into());
        let first = focused.map_or(0, |row| row.saturating_sub(PANEL_ROWS - 1));
        window.set_settings_first_row(to_i32(first));
        window.set_settings_row(focused.map_or(-1, to_i32));
        if !self.settings_rows.iter().eq(rows.iter().cloned()) {
            self.settings_rows.set_vec(rows);
        }
    }

    fn when_and_who(&self, window: &HomeWindow, entry: &EntryView) -> String {
        let strings = window.global::<Strings>();
        let author = match (entry.imported, entry.author) {
            (true, _) | (false, Author::Import) => 3,
            (false, Author::Tv) => 0,
            (false, Author::Mac) => 1,
            (false, Author::Drift) => 2,
        };
        let when = if entry.time_unix < CLOCK_SET_AFTER {
            strings.invoke_time_unknown().to_string()
        } else {
            format_time(entry.time_unix + i64::from(self.platform.utc_offset_secs()))
        };
        format!("{when} · {}", strings.invoke_who(author))
    }
}

fn row(title: SharedString, detail: &str) -> SettingsRow {
    SettingsRow {
        title,
        detail: detail.into(),
        current: false,
        dim: false,
    }
}

fn to_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn section_value(section: Section) -> i32 {
    match section {
        Section::Animations => 0,
        Section::Updates => 1,
        Section::History => 2,
        Section::Waiting => 3,
    }
}

fn arrow_value(arrow: Arrow) -> i32 {
    match arrow {
        Arrow::Up => 0,
        Arrow::Down => 1,
        Arrow::Left => 2,
        Arrow::Right => 3,
    }
}

fn as_change(change: &ApprovedChange) -> Change {
    match change {
        ApprovedChange::Setting { key, old, new } => Change::Setting {
            key: key.clone(),
            old: old.clone(),
            new: new.clone(),
        },
        ApprovedChange::Package { name, old, new } => Change::Package {
            name: name.clone(),
            old: *old,
            new: Some(*new),
        },
    }
}

/// Unicode bidi controls (embeddings, overrides, isolates, marks).
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// One change in words, in the window's language.
fn change_line(
    window: &HomeWindow,
    change: &Change,
    label: impl Fn(&str) -> String,
) -> SharedString {
    let strings = window.global::<Strings>();
    let value = |value: &Option<String>| -> SharedString {
        value
            .as_deref()
            .map_or_else(|| strings.invoke_unset_value(), SharedString::from)
    };
    let package = |state: &Option<PackageState>| -> SharedString {
        strings.invoke_package_state(match state {
            Some(PackageState::Enabled) => 0,
            Some(PackageState::Disabled) => 1,
            None => 2,
        })
    };
    let (kind, name, old, new): (i32, SharedString, SharedString, SharedString) = match change {
        Change::Reordered { .. } => (0, "".into(), "".into(), "".into()),
        Change::Hidden { tile, hidden } => (
            if *hidden { 1 } else { 2 },
            label(tile).into(),
            "".into(),
            "".into(),
        ),
        Change::UpdateCheck { on } => (3, "".into(), "".into(), strings.invoke_on_off(*on)),
        Change::Setting { key, old, new } => {
            let index = SETTING_KEYS
                .iter()
                .position(|known| known == key)
                .unwrap_or(0);
            (
                4,
                strings.invoke_setting_name(to_i32(index)),
                value(old),
                value(new),
            )
        }
        Change::Package { name, old, new } => (5, name.as_str().into(), package(old), package(new)),
        Change::Firmware { .. } => (6, "".into(), "".into(), "".into()),
        Change::Takeover { active } => (7, "".into(), "".into(), strings.invoke_on_off(*active)),
        _ => (8, "".into(), "".into(), "".into()),
    };
    strings.invoke_change_line(kind, name, old, new)
}

/// The first change, and how many more.
fn summary_line(window: &HomeWindow, changes: &[Change], label: impl Fn(&str) -> String) -> String {
    let strings = window.global::<Strings>();
    let Some(first) = changes.first() else {
        return strings
            .invoke_change_line(8, "".into(), "".into(), "".into())
            .to_string();
    };
    let line = change_line(window, first, label);
    match changes.len() {
        1 => line.to_string(),
        n => strings.invoke_and_more(line, to_i32(n - 1)).to_string(),
    }
}

/// "1 h 20 min" or "40 min", minutes rounded up.
fn duration_text(window: &HomeWindow, ms: u64) -> String {
    let minutes = ms.div_ceil(60_000);
    let strings = window.global::<Strings>();
    strings
        .invoke_duration(
            i32::try_from(minutes / 60).unwrap_or(i32::MAX),
            i32::try_from(minutes % 60).unwrap_or(0),
        )
        .to_string()
}

/// `YYYY-MM-DD HH:MM` for a local Unix time (days-from-civil, Howard Hinnant).
fn format_time(local_unix: i64) -> String {
    let days = local_unix.div_euclid(86_400);
    let seconds = local_unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        seconds % 3600 / 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_formatted_as_local_dates() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        assert_eq!(format_time(1_792_842_708), "2026-10-24 11:51");
        assert_eq!(format_time(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn bidi_controls_are_recognised() {
        assert!(is_bidi_control('\u{202E}'));
        assert!(is_bidi_control('\u{2067}'));
        assert!(!is_bidi_control('a'));
    }
}

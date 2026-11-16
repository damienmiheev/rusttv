//! The top-right menu, its quick panels, the platform's Settings tabs and
//! the volume bar in the window (device.rs).
//!
//! Every call to the platform's [`crate::device::DeviceSettings`] runs on a
//! worker thread ([`in_background`]); the window shows the change at once
//! and the device's answer when it arrives, so Home never waits for a
//! system tool.

use std::time::Duration;

use slint::{ComponentHandle, Model, SharedString, TimerMode};

use super::settings_view::in_background;
use super::{Controller, HomeWindow, SettingsRow, Strings};
use crate::device::{
    Control, ControlKind, Link, PanelAction, PanelPage, QuickPanel, Tab, Value, Volume,
    VolumeChange,
};
use crate::model::{Input, QuickAction};
use crate::platform::Platform;

/// How long the volume bar stays after the last volume key.
const VOLUME_SHOWN: Duration = Duration::from_millis(1500);

/// Rows of a quick panel visible at once (quick.slint `QuickPanel`).
const PANEL_ROWS: usize = 6;

/// How often the Network entry reads the network interfaces again.
const LINKS_EVERY: Duration = Duration::from_secs(5);

/// A network interface's code in quick.slint `QuickBar` (`links`).
fn link_code(link: &Link) -> i32 {
    match link {
        Link::Wifi(None) => 100,
        Link::Wifi(Some(bars)) => 100 + i32::from((*bars).clamp(1, 4)),
        Link::Wired(up) => 200 + i32::from(*up),
    }
}

/// The top-right menu's entries: Modify and Settings always, the others
/// when the platform offers them.
pub(super) fn quick_actions(platform: &dyn Platform) -> Vec<QuickAction> {
    let offered = platform.quick_actions();
    QuickAction::ALL
        .into_iter()
        .filter(|action| {
            matches!(action, QuickAction::Modify | QuickAction::Settings)
                || offered.contains(action)
        })
        .collect()
}

/// A quick action's code in quick.slint `QuickBar`.
fn quick_code(action: QuickAction) -> i32 {
    match action {
        QuickAction::Search => 0,
        QuickAction::Modify => 1,
        QuickAction::Audio => 2,
        QuickAction::Network => 3,
        QuickAction::KdeConnect => 4,
        QuickAction::Settings => 5,
        QuickAction::Power => 6,
    }
}

/// How a platform control shows as a row: toggles, sliders and choices
/// show their value when the platform gave no detail.
pub(super) fn control_row(window: &HomeWindow, control: &Control) -> SettingsRow {
    let given = |value: String| -> SharedString {
        if control.detail.is_empty() {
            value.into()
        } else {
            control.detail.as_str().into()
        }
    };
    let mut row = SettingsRow {
        title: control.label.as_str().into(),
        detail: control.detail.as_str().into(),
        ..SettingsRow::default()
    };
    match &control.kind {
        ControlKind::Toggle { on } => {
            row.detail = given(window.global::<Strings>().invoke_on_off(*on).into());
            row.current = *on;
        }
        ControlKind::Slider {
            value, min, max, ..
        } => {
            row.detail = given(value.to_string());
            row.slider = true;
            let span = (max - min).max(1);
            row.level = (value - min) as f32 / span as f32;
        }
        ControlKind::Choice { options, selected } => {
            row.detail = given(options.get(*selected).cloned().unwrap_or_default());
        }
        ControlKind::Action | ControlKind::Info => {}
    }
    row
}

impl Controller {
    /// Carries out a top-right menu entry the platform does: opens its
    /// quick panel, or its app.
    pub(super) fn open_quick(&mut self, action: QuickAction) {
        if let Some(panel) = action.panel() {
            self.quick = Some(PanelPage::new(panel, self.options.right_to_left));
            self.quick_status = "…".to_owned();
            self.refresh_quick();
            return;
        }
        let Some(id) = self.platform.quick_action_app(action) else {
            return;
        };
        // Not a launch from a tile: no splash, Home stays as it is under
        // the app.
        if let Err(error) = self.platform.launch(&id) {
            log::warn!("opening {id} for {action:?} failed: {error}");
            let label = self.apps.get(&id).map_or(id.as_str(), |app| &app.label);
            let text = self
                .window
                .upgrade()
                .map(|window| window.global::<Strings>().invoke_couldnt_open(label.into()));
            self.show_toast(text.unwrap_or_default());
        }
    }

    /// Reads the open quick panel's controls again; they arrive later.
    fn refresh_quick(&mut self) {
        let (Some(page), Some(device)) = (&self.quick, &self.device) else {
            return;
        };
        let panel = page.panel();
        let device = device.clone();
        let started = in_background(
            "rusttv-device",
            move || device.panel(panel),
            move |controller: &mut Controller, tab: Option<Tab>| {
                if controller.device_pending > 0 {
                    // A change is still on its way; its answer reads again.
                    return;
                }
                if let Some(page) = &mut controller.quick
                    && page.panel() == panel
                {
                    page.set_tab(tab);
                    controller.quick_status.clear();
                    controller.render();
                }
            },
        );
        if let Err(error) = started {
            self.quick_status = error.to_string();
        }
    }

    /// A key while a quick panel is open.
    pub(super) fn quick_input(&mut self, input: Input) {
        let Some(page) = &mut self.quick else {
            return;
        };
        let action = page.handle(input);
        self.panel_action(action);
    }

    /// A mouse click on a row of the open quick panel.
    pub(super) fn quick_row_clicked(&mut self, row: i32) {
        let (Some(page), Ok(row)) = (&mut self.quick, usize::try_from(row)) else {
            return;
        };
        let action = page.click(row);
        self.panel_action(action);
        self.restart_idle();
        self.render();
    }

    /// The scrim around the open quick panel was clicked: it closes.
    pub(super) fn quick_dismissed(&mut self) {
        if self.quick.is_some() {
            self.panel_action(Some(PanelAction::Close));
            self.render();
        }
    }

    fn panel_action(&mut self, action: Option<PanelAction>) {
        let Some(page) = &self.quick else {
            return;
        };
        match action {
            Some(PanelAction::Close) => {
                self.quick = None;
                self.home.close_quick();
            }
            Some(PanelAction::Set { control, value }) => {
                let tab = page.panel().id().to_owned();
                self.change_device(tab, control, value);
            }
            None => {}
        }
    }

    /// Sends a control's new value to the device; then reads the open
    /// panel and Settings tabs again. A failure shows on the panel or page.
    pub(super) fn change_device(&mut self, tab: String, control: String, value: Value) {
        let Some(device) = self.device.clone() else {
            return;
        };
        self.device_pending += 1;
        let started = in_background(
            "rusttv-device",
            move || device.set(&tab, &control, value),
            |controller: &mut Controller, result: Result<(), String>| {
                controller.device_pending = controller.device_pending.saturating_sub(1);
                if let Err(reason) = result {
                    log::warn!("changing a setting failed: {reason}");
                    if controller.quick.is_some() {
                        controller.quick_status = reason;
                    } else {
                        controller.settings_failed(reason);
                    }
                }
                if controller.device_pending == 0 {
                    controller.refresh_quick();
                    controller.refresh_device_tabs();
                }
                controller.render();
            },
        );
        if let Err(error) = started {
            self.device_pending -= 1;
            self.quick_status = error.to_string();
        }
    }

    /// Reads the platform's Settings tabs and facts for About while the
    /// Settings page is open; they arrive later.
    pub(super) fn refresh_device_tabs(&mut self) {
        if self.settings.is_none() {
            return;
        }
        let Some(device) = self.device.clone() else {
            return;
        };
        let started = in_background(
            "rusttv-device",
            move || (device.tabs(), device.about()),
            |controller: &mut Controller, (tabs, host)| {
                if controller.device_pending > 0 {
                    return;
                }
                if let Some(page) = &mut controller.settings {
                    page.set_device(tabs, host);
                    // Opened from the top-right menu: straight to that tab.
                    if let Some(tab) = controller.settings_tab.take() {
                        page.show_tab(tab);
                    }
                    controller.render();
                }
            },
        );
        if let Err(error) = started {
            log::warn!("no thread to read the device's settings: {error}");
            if let Some(page) = &mut self.settings {
                page.mark_loaded();
            }
        }
    }

    /// A volume key (the remote's, or `--volume` on Linux): changes the
    /// volume and shows the volume bar. Nothing without volume control.
    pub(super) fn volume_key(&mut self, change: VolumeChange) {
        let Some(device) = self.device.clone() else {
            return;
        };
        let started = in_background(
            "rusttv-volume",
            move || device.volume(change),
            |controller: &mut Controller, volume: Option<Volume>| {
                if let Some(volume) = volume {
                    controller.show_volume(volume);
                    // The Audio panel shows the new level too.
                    controller.refresh_quick();
                }
            },
        );
        if let Err(error) = started {
            log::warn!("no thread to change the volume: {error}");
        }
    }

    /// Reads the network interfaces for the Network entry every
    /// [`LINKS_EVERY`], off the UI thread; only while that entry is on the bar.
    pub(super) fn start_links(&mut self) {
        if !self.home.quick_actions().contains(&QuickAction::Network) {
            return;
        }
        self.read_links();
        let me = self.me.clone();
        self.links_timer
            .start(TimerMode::Repeated, LINKS_EVERY, move || {
                if let Some(controller) = me.upgrade() {
                    controller.borrow_mut().read_links();
                }
            });
    }

    fn read_links(&mut self) {
        let Some(device) = self.device.clone() else {
            return;
        };
        let started = in_background(
            "rusttv-links",
            move || device.links(),
            |controller: &mut Controller, links: Vec<Link>| {
                let codes: Vec<i32> = links.iter().map(link_code).collect();
                if !controller.link_codes.iter().eq(codes.iter().copied()) {
                    controller.link_codes.set_vec(codes);
                }
            },
        );
        if let Err(error) = started {
            log::warn!("no thread to read the network interfaces: {error}");
        }
    }

    /// Shows the volume bar for [`VOLUME_SHOWN`].
    fn show_volume(&mut self, volume: Volume) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        window.set_volume_level(volume.level);
        window.set_volume_muted(volume.muted);
        window.set_volume_shown(true);
        let window = self.window.clone();
        self.volume_timer
            .start(TimerMode::SingleShot, VOLUME_SHOWN, move || {
                if let Some(window) = window.upgrade() {
                    window.set_volume_shown(false);
                }
            });
    }

    /// The launcher left the screen (or Home was pressed): the panel closes.
    pub(super) fn close_quick_view(&mut self) {
        self.quick = None;
        self.quick_status.clear();
    }

    /// Draws the top-right menu and the open quick panel.
    pub(super) fn render_quick(&mut self, window: &HomeWindow) {
        let codes: Vec<i32> = self
            .home
            .quick_actions()
            .iter()
            .map(|action| quick_code(*action))
            .collect();
        if !self.quick_codes.iter().eq(codes.iter().copied()) {
            self.quick_codes.set_vec(codes);
        }
        window.set_quick_selected(
            self.home
                .quick_selected()
                .map_or(-1, |selected| i32::try_from(selected).unwrap_or(i32::MAX)),
        );
        let Some(page) = &self.quick else {
            return;
        };
        window.set_quick_power(page.panel() == QuickPanel::Power);
        let (title, rows) = page.tab().map_or_else(
            || (SharedString::new(), Vec::new()),
            |tab| {
                (
                    tab.title.as_str().into(),
                    tab.controls
                        .iter()
                        .map(|control| control_row(window, control))
                        .collect(),
                )
            },
        );
        window.set_quick_title(title);
        window.set_quick_status(self.quick_status.as_str().into());
        let row = page.row();
        window.set_quick_row(i32::try_from(row).unwrap_or(i32::MAX));
        window.set_quick_first_row(
            i32::try_from(row.saturating_sub(PANEL_ROWS - 1)).unwrap_or(i32::MAX),
        );
        if !self.quick_rows.iter().eq(rows.iter().cloned()) {
            self.quick_rows.set_vec(rows);
        }
    }
}

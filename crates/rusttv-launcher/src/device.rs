//! Settings that come from the platform: its own Settings tabs, the quick
//! panels of the top-right menu (Audio, Network, Power), the volume and
//! facts about the host for About.
//!
//! The platform describes each tab as typed [`Control`]s and carries out
//! changes ([`DeviceSettings`]); the launcher draws every tab with the same
//! generic rows (ui/device_view.rs), so a platform or device adds tabs
//! without touching UI code. No UI code here.

use crate::model::Input;

/// A quick panel, opened from the top-right menu over Home.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuickPanel {
    /// Volume, mute and the output device.
    Audio,
    /// Wi-Fi, the current network, other networks, Bluetooth.
    Network,
    /// Restart and power off (the Boot Menu).
    Power,
}

impl QuickPanel {
    /// The panel's tab id, passed to [`DeviceSettings::set`].
    pub fn id(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Network => "network",
            Self::Power => "power",
        }
    }
}

impl crate::model::QuickAction {
    /// The quick panel it opens, if it opens one. Network opens its
    /// Settings tab instead (the tab with id [`QuickPanel::Network`]'s).
    pub fn panel(self) -> Option<QuickPanel> {
        match self {
            Self::Audio => Some(QuickPanel::Audio),
            Self::Power => Some(QuickPanel::Power),
            // The RustTV panel's rows are the launcher's own (ui/device_view.rs).
            Self::Network
            | Self::Search
            | Self::Modify
            | Self::KdeConnect
            | Self::Panels
            | Self::Settings => None,
        }
    }
}

/// A network interface as the top-right menu's Network entry shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// Wi-Fi: the signal in bars (1 to 4) while connected, `None` when not.
    Wifi(Option<u8>),
    /// A cable (Ethernet): whether it is connected.
    Wired(bool),
}

/// A tab of Settings, or a quick panel: a title and its controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    /// Stable id, passed back to [`DeviceSettings::set`].
    pub id: String,
    /// The tab's name, in the UI's language.
    pub title: String,
    /// Its rows, top to bottom.
    pub controls: Vec<Control>,
}

/// One row of a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    /// Stable id within its tab, passed back to [`DeviceSettings::set`].
    pub id: String,
    /// What the row is called.
    pub label: String,
    /// A second, dim line (e.g. a network's signal); the value of an
    /// [`ControlKind::Info`] row. Toggles, sliders and choices show their
    /// value when this is empty.
    pub detail: String,
    /// What the row does.
    pub kind: ControlKind,
}

/// What a control does, with its current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlKind {
    /// On or off; OK flips it.
    Toggle { on: bool },
    /// A number from `min` to `max`; Left and Right change it by `step`.
    Slider {
        value: i32,
        min: i32,
        max: i32,
        step: i32,
    },
    /// One of `options`; Left and Right pick the one beside, OK the next.
    Choice {
        options: Vec<String>,
        selected: usize,
    },
    /// OK does something (connect, restart).
    Action,
    /// OK asks for a text (a Wi-Fi password: `secret`, shown as dots) and
    /// sends it as [`Value::Text`].
    Text { secret: bool },
    /// Shows `detail` only.
    Info,
}

/// A control's new value, for [`DeviceSettings::set`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A [`ControlKind::Toggle`]'s new state.
    Toggle(bool),
    /// A [`ControlKind::Slider`]'s new value.
    Number(i32),
    /// The index of a [`ControlKind::Choice`]'s new option.
    Choice(usize),
    /// A [`ControlKind::Action`] was chosen.
    Action,
    /// The text entered for a [`ControlKind::Text`].
    Text(String),
}

/// A volume key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeChange {
    /// One step louder.
    Up,
    /// One step quieter.
    Down,
    /// Mute, or unmute when muted.
    Mute,
}

impl VolumeChange {
    /// `up`, `down` or `mute`, as the Linux binary's `--volume` takes them.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "mute" => Some(Self::Mute),
            _ => None,
        }
    }
}

/// The volume after a change, for the on-screen volume bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volume {
    /// 0 silent, 1 full; can be above 1 where the system allows it.
    pub level: f32,
    /// Muted (the level is kept).
    pub muted: bool,
}

/// Reads and changes the device's settings. Every method may block (it can
/// run system tools), so the launcher calls them on worker threads only.
pub trait DeviceSettings: Send + Sync {
    /// Tabs this device adds to Settings, after the launcher's own and
    /// before About.
    fn tabs(&self) -> Vec<Tab> {
        Vec::new()
    }

    /// A quick panel's controls; `None` where it has none. Its tab id is
    /// [`QuickPanel::id`].
    fn panel(&self, _panel: QuickPanel) -> Option<Tab> {
        None
    }

    /// Changes `control` of tab (or panel) `tab`. `Err` is a short reason
    /// to show.
    fn set(&self, tab: &str, control: &str, value: Value) -> Result<(), String>;

    /// Facts about the host for About, as (label, value): the operating
    /// system, kernel, device, memory.
    fn about(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    /// Changes the volume; the volume after, or `None` without volume
    /// control.
    fn volume(&self, _change: VolumeChange) -> Option<Volume> {
        None
    }

    /// The volume now, without changing it; `None` without volume control.
    fn volume_now(&self) -> Option<Volume> {
        None
    }

    /// The network interfaces and their state, Wi-Fi first; empty when
    /// the platform doesn't say.
    fn links(&self) -> Vec<Link> {
        Vec::new()
    }
}

/// What a key did to a control (see [`adjust`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Adjust {
    /// The key is not for this control (e.g. Up, or Left on a toggle).
    Ignored,
    /// The key is for it, but nothing changes (a slider at its end).
    Kept,
    /// The control has this new value now; send it to the device.
    Changed(Value),
    /// Ask for a text for this [`ControlKind::Text`] (a text field; the system's on-screen keyboard types into it).
    AskText,
}

/// Applies a key to the focused control: OK flips a toggle, chooses an
/// action and picks a choice's next option; Left and Right (mirrored
/// right to left) move a slider or a choice. Updates `control` at once, so
/// the row shows the new value before the device answers.
pub(crate) fn adjust(control: &mut Control, input: Input, right_to_left: bool) -> Adjust {
    let step = match (input, right_to_left) {
        (Input::Right, false) | (Input::Left, true) => Some(1),
        (Input::Left, false) | (Input::Right, true) => Some(-1),
        _ => None,
    };
    match (&mut control.kind, input, step) {
        (ControlKind::Toggle { on }, Input::OkReleased, _) => {
            *on = !*on;
            Adjust::Changed(Value::Toggle(*on))
        }
        (ControlKind::Action, Input::OkReleased, _) => Adjust::Changed(Value::Action),
        (ControlKind::Text { .. }, Input::OkReleased, _) => Adjust::AskText,
        (
            ControlKind::Slider {
                value,
                min,
                max,
                step: by,
            },
            _,
            Some(direction),
        ) => {
            let new = value
                .saturating_add(direction * (*by).max(1))
                .clamp(*min, (*max).max(*min));
            if new == *value {
                return Adjust::Kept;
            }
            *value = new;
            Adjust::Changed(Value::Number(new))
        }
        (ControlKind::Choice { options, selected }, _, Some(direction)) => {
            let new = if direction > 0 {
                (*selected + 1).min(options.len().saturating_sub(1))
            } else {
                selected.saturating_sub(1)
            };
            if new == *selected {
                return Adjust::Kept;
            }
            *selected = new;
            Adjust::Changed(Value::Choice(new))
        }
        (ControlKind::Choice { options, selected }, Input::OkReleased, _) if options.len() > 1 => {
            *selected = (*selected + 1) % options.len();
            Adjust::Changed(Value::Choice(*selected))
        }
        (ControlKind::Slider { .. } | ControlKind::Choice { .. }, Input::OkReleased, _) => {
            Adjust::Kept
        }
        _ => Adjust::Ignored,
    }
}

/// Something ui.rs must do for an open quick panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PanelAction {
    /// Close the panel.
    Close,
    /// Send this change to the device.
    Set { control: String, value: Value },
}

/// An open quick panel: its controls (once read) and the focused row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PanelPage {
    panel: QuickPanel,
    tab: Option<Tab>,
    row: usize,
    right_to_left: bool,
}

impl PanelPage {
    /// A panel whose controls are still being read.
    pub fn new(panel: QuickPanel, right_to_left: bool) -> Self {
        Self {
            panel,
            tab: None,
            row: 0,
            right_to_left,
        }
    }

    /// Which panel this is.
    pub fn panel(&self) -> QuickPanel {
        self.panel
    }

    /// Its controls; `None` until read (or when the device has none).
    pub fn tab(&self) -> Option<&Tab> {
        self.tab.as_ref()
    }

    /// The focused row.
    pub fn row(&self) -> usize {
        self.row
    }

    /// Shows freshly read controls, keeping focus on the same row where it
    /// still exists.
    pub fn set_tab(&mut self, tab: Option<Tab>) {
        let rows = tab.as_ref().map_or(0, |tab| tab.controls.len());
        self.row = self.row.min(rows.saturating_sub(1));
        self.tab = tab;
    }

    /// Applies one key; returns what ui.rs must do.
    pub fn handle(&mut self, input: Input) -> Option<PanelAction> {
        let rows = self.tab.as_ref().map_or(0, |tab| tab.controls.len());
        if let Some(control) = self
            .tab
            .as_mut()
            .and_then(|tab| tab.controls.get_mut(self.row))
        {
            match adjust(control, input, self.right_to_left) {
                Adjust::Changed(value) => {
                    return Some(PanelAction::Set {
                        control: control.id.clone(),
                        value,
                    });
                }
                Adjust::Kept | Adjust::AskText => return None,
                Adjust::Ignored => {}
            }
        }
        match input {
            Input::Up => self.row = self.row.saturating_sub(1),
            Input::Down => self.row = (self.row + 1).min(rows.saturating_sub(1)),
            Input::Back => return Some(PanelAction::Close),
            _ => {}
        }
        None
    }

    /// A mouse click on `row`: focuses it and presses OK there.
    pub fn click(&mut self, row: usize) -> Option<PanelAction> {
        let rows = self.tab.as_ref().map_or(0, |tab| tab.controls.len());
        if row >= rows {
            return None;
        }
        self.row = row;
        self.handle(Input::OkReleased)
    }
}

/// A made-up device for the desktop dev mode and tests: a Display tab with
/// every kind of control, and the three quick panels, kept in memory.
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Default)]
pub struct FakeDevice {
    state: std::sync::Mutex<FakeState>,
}

#[cfg(any(test, feature = "test-util"))]
#[derive(Debug, Default)]
struct FakeState {
    volume: i32,
    muted: bool,
    output: usize,
    wifi: bool,
    network: Option<String>,
    bluetooth: bool,
    night_light: bool,
    /// Every change, as "tab/control=value".
    changes: Vec<String>,
}

#[cfg(any(test, feature = "test-util"))]
impl FakeDevice {
    /// Volume 40 %, Wi-Fi on and connected to "Home".
    pub fn new() -> Self {
        Self {
            state: std::sync::Mutex::new(FakeState {
                volume: 40,
                wifi: true,
                network: Some("Home".to_owned()),
                ..FakeState::default()
            }),
        }
    }

    /// Every change made so far, as "tab/control=value".
    pub fn changes(&self) -> Vec<String> {
        self.lock().changes.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        // A panic while holding the lock can only come from a test.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(any(test, feature = "test-util"))]
fn control(id: &str, label: &str, detail: &str, kind: ControlKind) -> Control {
    Control {
        id: id.to_owned(),
        label: label.to_owned(),
        detail: detail.to_owned(),
        kind,
    }
}

#[cfg(any(test, feature = "test-util"))]
impl DeviceSettings for FakeDevice {
    fn tabs(&self) -> Vec<Tab> {
        let state = self.lock();
        vec![Tab {
            id: "display".to_owned(),
            title: "Display".to_owned(),
            controls: vec![
                control(
                    "night-light",
                    "Night light",
                    "",
                    ControlKind::Toggle {
                        on: state.night_light,
                    },
                ),
                control("resolution", "Resolution", "1920 × 1080", ControlKind::Info),
                control(
                    "name",
                    "Device name",
                    "",
                    ControlKind::Text { secret: false },
                ),
            ],
        }]
    }

    fn panel(&self, panel: QuickPanel) -> Option<Tab> {
        let state = self.lock();
        let (title, controls) = match panel {
            QuickPanel::Audio => (
                "Audio",
                vec![
                    control(
                        "volume",
                        "Volume",
                        "",
                        ControlKind::Slider {
                            value: state.volume,
                            min: 0,
                            max: 100,
                            step: 5,
                        },
                    ),
                    control("mute", "Mute", "", ControlKind::Toggle { on: state.muted }),
                    control(
                        "output",
                        "Output",
                        "",
                        ControlKind::Choice {
                            options: vec!["HDMI".to_owned(), "Speakers".to_owned()],
                            selected: state.output,
                        },
                    ),
                ],
            ),
            QuickPanel::Network => {
                let mut controls = vec![control(
                    "wifi",
                    "Wi-Fi",
                    "",
                    ControlKind::Toggle { on: state.wifi },
                )];
                if state.wifi {
                    controls.push(control(
                        "current",
                        "Connected to",
                        state.network.as_deref().unwrap_or("Not connected"),
                        ControlKind::Info,
                    ));
                    for (name, signal) in [("Home", 80), ("Neighbour", 30)] {
                        controls.push(control(
                            &format!("connect:{name}"),
                            name,
                            &format!("{signal} %"),
                            ControlKind::Action,
                        ));
                    }
                }
                controls.push(control(
                    "bluetooth",
                    "Bluetooth",
                    "",
                    ControlKind::Toggle {
                        on: state.bluetooth,
                    },
                ));
                ("Network", controls)
            }
            QuickPanel::Power => (
                "Power",
                vec![
                    control("reboot", "Restart", "", ControlKind::Action),
                    control("poweroff", "Power off", "", ControlKind::Action),
                ],
            ),
        };
        Some(Tab {
            id: panel.id().to_owned(),
            title: title.to_owned(),
            controls,
        })
    }

    fn set(&self, tab: &str, control: &str, value: Value) -> Result<(), String> {
        let mut state = self.lock();
        state.changes.push(format!("{tab}/{control}={value:?}"));
        match (control, value) {
            ("volume", Value::Number(level)) => state.volume = level,
            ("mute", Value::Toggle(on)) => state.muted = on,
            ("output", Value::Choice(index)) => state.output = index,
            ("wifi", Value::Toggle(on)) => state.wifi = on,
            ("bluetooth", Value::Toggle(on)) => state.bluetooth = on,
            ("night-light", Value::Toggle(on)) => state.night_light = on,
            ("name", Value::Text(_)) => {}
            (control, Value::Action) if control.starts_with("connect:") => {
                state.network = control.strip_prefix("connect:").map(str::to_owned);
            }
            ("reboot" | "poweroff", Value::Action) => {}
            _ => return Err(format!("no control {tab}/{control}")),
        }
        Ok(())
    }

    fn about(&self) -> Vec<(String, String)> {
        vec![
            ("Operating system".to_owned(), "Fake OS 1.0".to_owned()),
            ("Memory".to_owned(), "1.0 GB".to_owned()),
        ]
    }

    fn volume(&self, change: VolumeChange) -> Option<Volume> {
        let mut state = self.lock();
        match change {
            VolumeChange::Up => state.volume = (state.volume + 5).min(100),
            VolumeChange::Down => state.volume = (state.volume - 5).max(0),
            VolumeChange::Mute => state.muted = !state.muted,
        }
        state.changes.push(format!("volume {change:?}"));
        Some(Volume {
            level: state.volume as f32 / 100.0,
            muted: state.muted,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slider(value: i32) -> Control {
        control(
            "volume",
            "Volume",
            "",
            ControlKind::Slider {
                value,
                min: 0,
                max: 100,
                step: 5,
            },
        )
    }

    #[test]
    fn left_and_right_move_a_slider_within_its_ends() {
        let mut volume = slider(95);
        assert_eq!(
            adjust(&mut volume, Input::Right, false),
            Adjust::Changed(Value::Number(100))
        );
        assert_eq!(adjust(&mut volume, Input::Right, false), Adjust::Kept);
        assert_eq!(
            adjust(&mut volume, Input::Right, true),
            Adjust::Changed(Value::Number(95)),
            "right to left, Right lowers it"
        );
        assert_eq!(adjust(&mut volume, Input::Up, false), Adjust::Ignored);
    }

    #[test]
    fn ok_flips_toggles_cycles_choices_and_chooses_actions() {
        let mut toggle = control("t", "T", "", ControlKind::Toggle { on: false });
        assert_eq!(
            adjust(&mut toggle, Input::OkReleased, false),
            Adjust::Changed(Value::Toggle(true))
        );
        assert_eq!(adjust(&mut toggle, Input::Left, false), Adjust::Ignored);
        let mut choice = control(
            "c",
            "C",
            "",
            ControlKind::Choice {
                options: vec!["a".into(), "b".into()],
                selected: 1,
            },
        );
        assert_eq!(
            adjust(&mut choice, Input::OkReleased, false),
            Adjust::Changed(Value::Choice(0)),
            "OK wraps around"
        );
        assert_eq!(adjust(&mut choice, Input::Left, false), Adjust::Kept);
        let mut action = control("a", "A", "", ControlKind::Action);
        assert_eq!(
            adjust(&mut action, Input::OkReleased, false),
            Adjust::Changed(Value::Action)
        );
        let mut info = control("i", "I", "x", ControlKind::Info);
        assert_eq!(adjust(&mut info, Input::OkReleased, false), Adjust::Ignored);
    }

    #[test]
    fn a_panel_walks_its_rows_sends_changes_and_closes_on_back() {
        let device = FakeDevice::new();
        let mut page = PanelPage::new(QuickPanel::Audio, false);
        assert_eq!(page.handle(Input::Down), None, "nothing to walk yet");
        page.set_tab(device.panel(QuickPanel::Audio));
        assert_eq!(
            page.handle(Input::Left),
            Some(PanelAction::Set {
                control: "volume".into(),
                value: Value::Number(35)
            })
        );
        page.handle(Input::Down);
        assert_eq!(page.row(), 1);
        assert_eq!(
            page.handle(Input::OkReleased),
            Some(PanelAction::Set {
                control: "mute".into(),
                value: Value::Toggle(true)
            })
        );
        page.handle(Input::Down);
        page.handle(Input::Down);
        assert_eq!(page.row(), 2, "stops at the last row");
        page.set_tab(Some(Tab {
            id: "audio".into(),
            title: "Audio".into(),
            controls: vec![slider(10)],
        }));
        assert_eq!(page.row(), 0, "focus stays on a row that exists");
        assert_eq!(page.handle(Input::Back), Some(PanelAction::Close));
    }

    #[test]
    fn the_fake_device_keeps_what_it_is_set_to() {
        let device = FakeDevice::new();
        device
            .set("network", "connect:Neighbour", Value::Action)
            .expect("set");
        let network = device.panel(QuickPanel::Network).expect("panel");
        assert!(
            network
                .controls
                .iter()
                .any(|c| c.id == "current" && c.detail == "Neighbour")
        );
        assert!(device.set("network", "nope", Value::Action).is_err());
        let volume = device.volume(VolumeChange::Mute).expect("volume");
        assert!(volume.muted);
        assert!((volume.level - 0.4).abs() < f32::EPSILON);
        assert_eq!(VolumeChange::parse("down"), Some(VolumeChange::Down));
        assert_eq!(VolumeChange::parse("loud"), None);
    }
}

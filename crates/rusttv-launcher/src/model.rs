//! Home screen state, independent of UI and platform (ARCHITECTURE.md § Launcher).
//!
//! Time comes from the caller as a monotonic [`Duration`] (for example time
//! since start-up), so the model is deterministic and testable on the host.

use std::collections::BTreeSet;
use std::time::Duration;

/// Tiles per grid row.
///
/// The 1920-wide layout (250 rpx tiles, 48 gutters,
/// 90 margins) is computed for this value; changing it changes the tile size.
const COLUMNS: usize = 6;

/// How many row slots at the top of the screen the focused row may use when scrolled.
///
/// Three rows of 251 rpx from y = 90 leave the focused label inside the safe
/// area at 1080 rpx.
const FOCUS_SLOTS: usize = 3;

/// How long OK must be held to pick a tile up instead of launching it.
///
/// Long enough not to trigger on a normal press, short enough not to feel
/// stuck. The caller sends [`Input::Tick`] once this
/// has passed after [`Input::OkPressed`].
pub const LONG_PRESS: Duration = Duration::from_millis(600);

/// How long a launch may stay in flight before it counts as failed.
///
/// Apps normally pause the launcher well within this; after it, input is
/// unblocked and an error is shown. The caller sends
/// [`Input::Tick`] once this has passed after a launch.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(3);

/// How long move mode may go without input before the tile is dropped and saved.
///
/// Also bounds how long the jiggle animation can run unattended.
/// The caller sends [`Input::Tick`] once this has passed after the last input.
pub const MOVE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Gap between order keys when keys are assigned or renumbered.
///
/// Sparse keys let a moved tile take a key between its neighbours without
/// renumbering the rest. Persisted in the order file.
pub(crate) const KEY_STEP: i64 = 1000;

/// One tile on the home screen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TileId {
    /// An installed app, identified by its component name (`package/activity`).
    App(String),
    /// The built-in About tile.
    About,
    /// The built-in Settings tile (HISTORY.md § 8).
    Settings,
    /// Opens the hidden apps. Shown last, and only while an installed app is hidden;
    /// never part of the [`Order`].
    Hidden,
}

/// A launchable app as reported by the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Component name (`package/activity`).
    pub id: String,
    /// User-visible name; used to sort apps on first run.
    pub label: String,
}

/// A key or lifecycle event for the home screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// D-pad up.
    Up,
    /// D-pad down.
    Down,
    /// D-pad left.
    Left,
    /// D-pad right.
    Right,
    /// OK went down. Key repeats while held are ignored.
    OkPressed,
    /// OK went up.
    OkReleased,
    /// The Back key.
    Back,
    /// Time passed; checks the long-press and launch timers.
    Tick,
    /// The launcher left the screen (an app opened, or Home was pressed).
    Paused,
    /// The platform could not start the app being launched.
    LaunchFailed,
}

/// An entry in the menu that a long press on a tile opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    /// Pick the tile up to move it.
    Move,
    /// Take the app off Home; it stays installed and is listed under Hidden apps.
    Hide,
}

/// Something the caller must do after [`Home::handle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Start this app (component name).
    Launch(String),
    /// Show the About page.
    OpenAbout,
    /// OK on the About page: install the available update, else check for one.
    Update,
    /// Show the hidden apps page.
    OpenHidden,
    /// Show the Settings page.
    OpenSettings,
    /// A key while Settings is open: the Settings page handles it.
    Settings(Input),
    /// Persist [`Home::order`] and [`Home::hidden`].
    SaveOrder,
    /// Tell the user this app (component name) could not be opened.
    ShowLaunchError(String),
}

/// Vertical scroll state of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    /// Focus is in the first row; the hero is visible above it.
    Shelf,
    /// Focus is below the first row; the grid shows rows from `first_row` on.
    Grid {
        /// Index of the topmost visible row.
        first_row: usize,
    },
}

/// What the home screen is doing, as far as the UI needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Normal browsing.
    Browse,
    /// The focused tile is picked up and moves with the D-pad.
    Moving,
    /// A launch is in flight; input is ignored.
    Launching,
    /// The About page is open.
    About,
    /// The long-press menu of the focused tile is open.
    Menu,
    /// The hidden apps page is open.
    Hidden,
    /// The Settings page (or its review screen) is open.
    Settings,
}

/// User-defined tile order as sparse integer keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Order {
    /// Sorted by key; each id appears once.
    keys: Vec<(TileId, i64)>,
}

impl Order {
    /// Builds an order from stored keys; later duplicates of an id are dropped.
    pub fn from_keys(keys: impl IntoIterator<Item = (TileId, i64)>) -> Self {
        let mut order = Self::default();
        for (id, key) in keys {
            if !order.contains(&id) {
                order.keys.push((id, key));
            }
        }
        order.keys.sort_by_key(|&(_, key)| key);
        order
    }

    /// Tile ids with their keys, in display order.
    pub fn keys(&self) -> &[(TileId, i64)] {
        &self.keys
    }

    fn contains(&self, id: &TileId) -> bool {
        self.keys.iter().any(|(known, _)| known == id)
    }

    fn key(&self, id: &TileId) -> Option<i64> {
        self.keys
            .iter()
            .find_map(|(known, key)| (known == id).then_some(*key))
    }

    fn ids(&self) -> Vec<TileId> {
        self.keys.iter().map(|(id, _)| id.clone()).collect()
    }

    /// Drops apps that are gone and appends new ones; returns whether anything changed.
    ///
    /// New apps are appended sorted by label, before the built-in tiles
    /// (About, then Settings) while those are at the end, so a first run
    /// gives an alphabetical grid ending with About and Settings. A
    /// built-in tile the user moved elsewhere stays where it is; a missing
    /// one is added to the end block.
    fn reconcile(&mut self, apps: &[App]) -> bool {
        let before = self.keys.clone();
        self.keys.retain(|(id, _)| match id {
            TileId::App(component) => apps.iter().any(|app| app.id == *component),
            TileId::About | TileId::Settings => true,
            TileId::Hidden => false,
        });
        // The built-in tiles at the end stay at the end, in their order.
        let tail = self
            .keys
            .iter()
            .rev()
            .take_while(|(id, _)| matches!(id, TileId::About | TileId::Settings))
            .count();
        let tail: Vec<(TileId, i64)> = self.keys.split_off(self.keys.len() - tail);

        let mut new: Vec<&App> = apps
            .iter()
            .filter(|app| !self.contains(&TileId::App(app.id.clone())))
            .filter(|app| {
                !tail
                    .iter()
                    .any(|(id, _)| *id == TileId::App(app.id.clone()))
            })
            .collect();
        new.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        new.dedup_by(|a, b| a.id == b.id);

        let mut ids: Vec<TileId> = self.keys.iter().map(|(id, _)| id.clone()).collect();
        ids.extend(new.into_iter().map(|app| TileId::App(app.id.clone())));
        ids.extend(tail.iter().map(|(id, _)| id.clone()));
        for builtin in [TileId::About, TileId::Settings] {
            if !ids.contains(&builtin) {
                ids.push(builtin);
            }
        }
        // Keep each tile's key while it still sorts; give later ones the next step.
        let old: Vec<(TileId, i64)> = self.keys.drain(..).chain(tail).collect();
        let mut last = None;
        for id in ids {
            let kept = old
                .iter()
                .find(|(old_id, _)| *old_id == id)
                .map(|&(_, key)| key);
            let key = match (kept, last) {
                (Some(key), Some(last)) if key > last => key,
                (Some(key), None) => key,
                (_, Some(last)) => last + KEY_STEP,
                (None, None) => KEY_STEP,
            };
            self.keys.push((id, key));
            last = Some(key);
        }
        self.keys != before
    }

    /// Gives `tiles[index]` a key between its neighbours in `tiles`.
    ///
    /// `tiles` must hold exactly the ids of this order. Renumbers everything when
    /// there is no free key between the neighbours.
    fn place(&mut self, tiles: &[TileId], index: usize) {
        let neighbour = |i: Option<usize>| i.and_then(|i| tiles.get(i)).map(|id| self.key(id));
        let before = neighbour(index.checked_sub(1));
        let after = neighbour(index.checked_add(1));
        let key = match (before, after) {
            (None, None) => Some(KEY_STEP),
            (Some(Some(before)), None) => before.checked_add(KEY_STEP),
            (None, Some(Some(after))) => after.checked_sub(KEY_STEP),
            (Some(Some(before)), Some(Some(after))) => after
                .checked_sub(before)
                .filter(|gap| *gap >= 2)
                .map(|gap| before + gap / 2),
            // A neighbour without a key: `tiles` and the order disagree.
            _ => None,
        };

        match (key, tiles.get(index)) {
            (Some(key), Some(id)) => {
                self.keys.retain(|(known, _)| known != id);
                self.keys.push((id.clone(), key));
                self.keys.sort_by_key(|&(_, key)| key);
            }
            _ => {
                self.keys = (1..)
                    .zip(tiles)
                    .map(|(n, id)| (id.clone(), n * KEY_STEP))
                    .collect();
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    /// `original` is the tile list before the pick-up, restored on cancel;
    /// `last_input` is when the user last pressed a key in this mode.
    Moving {
        original: Vec<TileId>,
        last_input: Duration,
    },
    Launching {
        app: String,
        since: Duration,
    },
    About,
    Settings,
    /// `selected` indexes [`menu_items`] of the focused tile.
    Menu {
        selected: usize,
    },
    /// `focus` indexes `Home::hidden_apps`.
    Hidden {
        focus: usize,
    },
}

/// What the long-press menu offers for a tile; empty means no menu.
fn menu_items(tile: Option<&TileId>) -> &'static [MenuItem] {
    match tile {
        Some(TileId::App(_)) => &[MenuItem::Move, MenuItem::Hide],
        Some(TileId::About | TileId::Settings) => &[MenuItem::Move],
        Some(TileId::Hidden) | None => &[],
    }
}

/// The home screen: tile order, hidden apps, focus, scroll position and mode.
#[derive(Debug, Clone)]
pub struct Home {
    order: Order,
    /// Component names of apps kept off Home, installed or not.
    hidden: BTreeSet<String>,
    /// The last app list from the platform.
    apps: Vec<App>,
    /// Installed hidden apps, by label.
    hidden_apps: Vec<String>,
    /// Tiles in display order: the order, then [`TileId::Hidden`] if any app is
    /// hidden. During a move this differs from `order`.
    tiles: Vec<TileId>,
    focus: usize,
    first_row: usize,
    mode: Mode,
    /// OK is physically down; further presses are key repeats.
    ok_held: bool,
    /// When OK went down in browse mode; cleared once it launches, moves or is cancelled.
    ok_since: Option<Duration>,
    right_to_left: bool,
}

impl Home {
    /// Creates a home screen with a stored order and hidden apps; call [`Home::set_apps`] next.
    pub fn new(order: Order, hidden: BTreeSet<String>, right_to_left: bool) -> Self {
        Self {
            tiles: order.ids(),
            order,
            hidden,
            apps: Vec::new(),
            hidden_apps: Vec::new(),
            focus: 0,
            first_row: 0,
            mode: Mode::Browse,
            ok_held: false,
            ok_since: None,
            right_to_left,
        }
    }

    /// Replaces the app list, keeping focus on the same tile; returns whether the order changed.
    pub fn set_apps(&mut self, apps: &[App]) -> bool {
        match self.mode {
            Mode::Moving { .. } => self.cancel_move(),
            Mode::Menu { .. } => self.mode = Mode::Browse,
            _ => {}
        }
        self.apps = apps.to_vec();
        self.refresh()
    }

    /// Rebuilds the tiles from the app list, hidden apps and order, keeping
    /// focus on the same tile; returns whether the order changed.
    fn refresh(&mut self) -> bool {
        let focused = self.focused().cloned();
        let (mut hidden, shown): (Vec<&App>, Vec<&App>) = self
            .apps
            .iter()
            .partition(|app| self.hidden.contains(&app.id));
        let shown: Vec<App> = shown.into_iter().cloned().collect();
        hidden.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        hidden.dedup_by(|a, b| a.id == b.id);
        self.hidden_apps = hidden.into_iter().map(|app| app.id.clone()).collect();

        let changed = self.order.reconcile(&shown);
        self.tiles = self.order.ids();
        if !self.hidden_apps.is_empty() {
            self.tiles.push(TileId::Hidden);
        }
        self.focus = focused
            .and_then(|id| self.tiles.iter().position(|tile| *tile == id))
            .unwrap_or_else(|| self.focus.min(self.tiles.len().saturating_sub(1)));
        if let Mode::Hidden { focus } = &mut self.mode {
            match self.hidden_apps.len() {
                0 => self.mode = Mode::Browse,
                len => *focus = (*focus).min(len - 1),
            }
        }
        self.update_scroll();
        changed
    }

    /// Moves focus to `id` if it is shown; returns whether it was.
    ///
    /// For restoring focus on start-up, so it ignores input state and works in any mode.
    pub fn focus_on(&mut self, id: &TileId) -> bool {
        let Some(index) = self.tiles.iter().position(|tile| tile == id) else {
            return false;
        };
        self.focus = index;
        self.update_scroll();
        true
    }

    /// Tiles in display order.
    pub fn tiles(&self) -> &[TileId] {
        &self.tiles
    }

    /// Index of the focused tile in [`Home::tiles`].
    pub fn focus(&self) -> usize {
        self.focus
    }

    /// The focused tile, if there are any tiles.
    pub fn focused(&self) -> Option<&TileId> {
        self.tiles.get(self.focus)
    }

    /// The saved tile order.
    pub fn order(&self) -> &Order {
        &self.order
    }

    /// Component names of all hidden apps, including ones no longer installed.
    pub fn hidden(&self) -> &BTreeSet<String> {
        &self.hidden
    }

    /// Installed hidden apps as listed on the hidden apps page, by label.
    pub fn hidden_apps(&self) -> &[String] {
        &self.hidden_apps
    }

    /// The focused index in [`Home::hidden_apps`] while that page is open.
    pub fn hidden_focus(&self) -> Option<usize> {
        match self.mode {
            Mode::Hidden { focus } => Some(focus),
            _ => None,
        }
    }

    /// The open long-press menu's items and the selected index.
    pub fn menu(&self) -> Option<(&'static [MenuItem], usize)> {
        match self.mode {
            Mode::Menu { selected } => Some((menu_items(self.focused()), selected)),
            _ => None,
        }
    }

    /// Current scroll state.
    pub fn scroll(&self) -> Scroll {
        match self.focus / COLUMNS {
            0 => Scroll::Shelf,
            _ => Scroll::Grid {
                first_row: self.first_row,
            },
        }
    }

    /// Replaces the order and hidden apps (the history rewrote them),
    /// keeping focus on the same tile where it still exists.
    pub fn replace_order(&mut self, order: Order, hidden: BTreeSet<String>) {
        self.order = order;
        self.hidden = hidden;
        self.refresh();
    }

    /// Leaves the Settings page (it decides when: Back on its section list).
    pub fn close_settings(&mut self) {
        if matches!(self.mode, Mode::Settings) {
            self.mode = Mode::Browse;
        }
    }

    /// Current state.
    pub fn state(&self) -> State {
        match self.mode {
            Mode::Browse => State::Browse,
            Mode::Moving { .. } => State::Moving,
            Mode::Launching { .. } => State::Launching,
            Mode::About => State::About,
            Mode::Settings => State::Settings,
            Mode::Menu { .. } => State::Menu,
            Mode::Hidden { .. } => State::Hidden,
        }
    }

    /// Applies one input at time `now` and returns what the caller must do.
    pub fn handle(&mut self, input: Input, now: Duration) -> Option<Effect> {
        match input {
            Input::OkPressed if self.ok_held => return None,
            Input::OkPressed => self.ok_held = true,
            Input::OkReleased => self.ok_held = false,
            Input::Paused => {
                self.ok_held = false;
                self.ok_since = None;
            }
            _ => {}
        }

        match (&self.mode, input) {
            (
                Mode::Moving { .. }
                | Mode::About
                | Mode::Settings
                | Mode::Launching { .. }
                | Mode::Menu { .. }
                | Mode::Hidden { .. },
                Input::Paused,
            ) => {
                if matches!(self.mode, Mode::Moving { .. }) {
                    self.cancel_move();
                }
                self.mode = Mode::Browse;
                None
            }
            (Mode::Launching { app, .. }, Input::LaunchFailed) => {
                let effect = Effect::ShowLaunchError(app.clone());
                self.mode = Mode::Browse;
                Some(effect)
            }
            (Mode::Launching { app, since }, Input::Tick) => {
                if now.saturating_sub(*since) < LAUNCH_TIMEOUT {
                    return None;
                }
                let effect = Effect::ShowLaunchError(app.clone());
                self.mode = Mode::Browse;
                Some(effect)
            }
            (Mode::About, Input::Back) => {
                self.mode = Mode::Browse;
                None
            }
            (Mode::About, Input::OkPressed) => Some(Effect::Update),
            (Mode::Settings, input) => Some(Effect::Settings(input)),
            (Mode::Moving { .. }, _) => self.handle_moving(input, now),
            (Mode::Menu { .. }, _) => self.handle_menu(input, now),
            (Mode::Hidden { .. }, _) => self.handle_hidden(input),
            (Mode::Browse, _) => self.handle_browse(input, now),
            _ => None,
        }
    }

    fn handle_browse(&mut self, input: Input, now: Duration) -> Option<Effect> {
        if let Some(direction) = self.direction(input) {
            self.ok_since = None;
            if let Some(target) = neighbour(self.focus, self.tiles.len(), direction) {
                self.focus = target;
                self.update_scroll();
            }
            return None;
        }
        match input {
            Input::OkPressed => {
                self.ok_since = Some(now);
                None
            }
            Input::OkReleased => {
                let since = self.ok_since.take()?;
                if now.saturating_sub(since) < LONG_PRESS {
                    self.activate(now)
                } else {
                    // No Tick arrived in time; treat the release as the end of a long press.
                    self.open_menu();
                    None
                }
            }
            Input::Tick => {
                if now.saturating_sub(self.ok_since?) >= LONG_PRESS {
                    self.ok_since = None;
                    self.open_menu();
                }
                None
            }
            Input::Back => {
                self.focus %= COLUMNS;
                self.update_scroll();
                None
            }
            _ => None,
        }
    }

    fn handle_moving(&mut self, input: Input, now: Duration) -> Option<Effect> {
        if let Mode::Moving { last_input, .. } = &mut self.mode {
            if input == Input::Tick {
                if now.saturating_sub(*last_input) >= MOVE_IDLE_TIMEOUT {
                    return self.drop_tile();
                }
                return None;
            }
            *last_input = now;
        }
        if let Some(direction) = self.direction(input) {
            if let Some(target) = neighbour(self.focus, self.movable(), direction) {
                let tile = self.tiles.remove(self.focus);
                self.tiles.insert(target, tile);
                self.focus = target;
                self.update_scroll();
            }
            return None;
        }
        match input {
            Input::OkPressed => self.drop_tile(),
            Input::Back => {
                self.cancel_move();
                None
            }
            _ => None,
        }
    }

    /// Up and Down pick an item, OK runs it (the release of the long press
    /// that opened the menu is ignored), Back closes the menu.
    fn handle_menu(&mut self, input: Input, now: Duration) -> Option<Effect> {
        let items = menu_items(self.focused());
        let Mode::Menu { selected } = &mut self.mode else {
            return None;
        };
        match input {
            Input::Up => *selected = selected.saturating_sub(1),
            Input::Down => *selected = (*selected + 1).min(items.len().saturating_sub(1)),
            Input::OkPressed => match items.get(*selected) {
                Some(MenuItem::Move) => self.start_move(now),
                Some(MenuItem::Hide) => return self.hide_focused(),
                None => self.mode = Mode::Browse,
            },
            Input::Back => self.mode = Mode::Browse,
            _ => {}
        }
        None
    }

    /// The D-pad moves within the page, OK puts the focused app back on Home,
    /// Back returns to Home.
    fn handle_hidden(&mut self, input: Input) -> Option<Effect> {
        let Mode::Hidden { focus } = self.mode else {
            return None;
        };
        if let Some(direction) = self.direction(input) {
            if let Some(target) = neighbour(focus, self.hidden_apps.len(), direction) {
                self.mode = Mode::Hidden { focus: target };
            }
            return None;
        }
        match input {
            Input::OkPressed => {
                let id = self.hidden_apps.get(focus)?.clone();
                self.hidden.remove(&id);
                self.refresh();
                if self.hidden_focus().is_none() {
                    // That was the last one: back on Home, on the app.
                    self.focus_on(&TileId::App(id));
                }
                Some(Effect::SaveOrder)
            }
            Input::Back => {
                self.mode = Mode::Browse;
                None
            }
            _ => None,
        }
    }

    /// Hides the focused app; the next tile takes its place and focus.
    fn hide_focused(&mut self) -> Option<Effect> {
        self.mode = Mode::Browse;
        let Some(TileId::App(id)) = self.focused().cloned() else {
            return None;
        };
        self.hidden.insert(id);
        self.refresh();
        Some(Effect::SaveOrder)
    }

    /// How many tiles from the start can move: all but [`TileId::Hidden`].
    fn movable(&self) -> usize {
        self.tiles.len() - usize::from(self.tiles.last() == Some(&TileId::Hidden))
    }

    /// Ends move mode, keeping the tile where it is.
    fn drop_tile(&mut self) -> Option<Effect> {
        let movable = &self.tiles[..self.movable()];
        let moved = movable != self.order.ids();
        self.mode = Mode::Browse;
        if !moved {
            return None;
        }
        self.order.place(movable, self.focus);
        Some(Effect::SaveOrder)
    }

    fn direction(&self, input: Input) -> Option<Direction> {
        match (input, self.right_to_left) {
            (Input::Up, _) => Some(Direction::Up),
            (Input::Down, _) => Some(Direction::Down),
            (Input::Left, false) | (Input::Right, true) => Some(Direction::Left),
            (Input::Right, false) | (Input::Left, true) => Some(Direction::Right),
            _ => None,
        }
    }

    fn activate(&mut self, now: Duration) -> Option<Effect> {
        match self.focused()? {
            TileId::App(component) => {
                let component = component.clone();
                self.mode = Mode::Launching {
                    app: component.clone(),
                    since: now,
                };
                Some(Effect::Launch(component))
            }
            TileId::About => {
                self.mode = Mode::About;
                Some(Effect::OpenAbout)
            }
            TileId::Settings => {
                self.mode = Mode::Settings;
                Some(Effect::OpenSettings)
            }
            TileId::Hidden => {
                self.mode = Mode::Hidden { focus: 0 };
                Some(Effect::OpenHidden)
            }
        }
    }

    fn open_menu(&mut self) {
        if !menu_items(self.focused()).is_empty() {
            self.mode = Mode::Menu { selected: 0 };
        }
    }

    fn start_move(&mut self, now: Duration) {
        if self.focus < self.movable() {
            self.mode = Mode::Moving {
                original: self.tiles.clone(),
                last_input: now,
            };
        }
    }

    fn cancel_move(&mut self) {
        let Mode::Moving { original, .. } = std::mem::replace(&mut self.mode, Mode::Browse) else {
            return;
        };
        let moving = self.focused().cloned();
        self.tiles = original;
        self.focus = moving
            .and_then(|id| self.tiles.iter().position(|tile| *tile == id))
            .unwrap_or(0);
        self.update_scroll();
    }

    /// Keeps the focused row within the first [`FOCUS_SLOTS`] visible rows, scrolling as little as possible.
    fn update_scroll(&mut self) {
        let row = self.focus / COLUMNS;
        if row == 0 || row < self.first_row {
            self.first_row = row;
        } else if row >= self.first_row + FOCUS_SLOTS {
            self.first_row = row + 1 - FOCUS_SLOTS;
        }
    }
}

/// The tile reached from `index` in `direction` in a grid of `len` tiles, if any.
///
/// No wrap-around. Moving down into a short last row lands on the last tile.
fn neighbour(index: usize, len: usize, direction: Direction) -> Option<usize> {
    if index >= len {
        return None;
    }
    let column = index % COLUMNS;
    match direction {
        Direction::Left => (column > 0).then(|| index - 1),
        Direction::Right => (column + 1 < COLUMNS && index + 1 < len).then(|| index + 1),
        Direction::Up => index.checked_sub(COLUMNS),
        Direction::Down => {
            let below = index + COLUMNS;
            if below < len {
                Some(below)
            } else if index / COLUMNS < (len - 1) / COLUMNS {
                Some(len - 1)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const ALL_DIRECTIONS: [Direction; 4] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ];

    fn app(id: &str, label: &str) -> App {
        App {
            id: id.to_owned(),
            label: label.to_owned(),
        }
    }

    /// `count` apps whose labels sort in id order: a00, a01, ...
    fn apps(count: usize) -> Vec<App> {
        (0..count)
            .map(|n| app(&format!("a{n:02}"), &format!("App {n:02}")))
            .collect()
    }

    fn id(component: &str) -> TileId {
        TileId::App(component.to_owned())
    }

    /// A home screen with `count` apps plus About, focus on the first tile.
    fn home(count: usize) -> Home {
        let mut home = Home::new(Order::default(), BTreeSet::new(), false);
        home.set_apps(&apps(count));
        home
    }

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    /// Sends inputs at time zero and returns the last effect.
    fn press(home: &mut Home, inputs: &[Input]) -> Option<Effect> {
        inputs
            .iter()
            .fold(None, |_, input| home.handle(*input, Duration::ZERO))
    }

    fn component(home: &Home) -> &str {
        match home.focused() {
            Some(TileId::App(component)) => component,
            other => panic!("focus is not on an app: {other:?}"),
        }
    }

    // --- navigation -------------------------------------------------------

    #[test]
    fn every_tile_is_reachable_from_the_first() {
        for len in 1..=40 {
            let mut seen = BTreeSet::from([0]);
            let mut queue = vec![0];
            while let Some(index) = queue.pop() {
                for direction in ALL_DIRECTIONS {
                    if let Some(next) = neighbour(index, len, direction)
                        && seen.insert(next)
                    {
                        queue.push(next);
                    }
                }
            }
            assert_eq!(seen.len(), len, "unreachable tiles with {len} tiles");
        }
    }

    #[test]
    fn every_move_stays_in_bounds_and_left_right_stay_in_the_row() {
        for len in 0..=40 {
            for index in 0..len {
                for direction in ALL_DIRECTIONS {
                    let Some(next) = neighbour(index, len, direction) else {
                        continue;
                    };
                    assert!(next < len && next != index);
                    if matches!(direction, Direction::Left | Direction::Right) {
                        assert_eq!(next / COLUMNS, index / COLUMNS, "wrapped at {index}");
                    }
                }
            }
        }
    }

    #[test]
    fn edges_do_not_wrap() {
        let len = 14;
        assert_eq!(neighbour(0, len, Direction::Left), None);
        assert_eq!(neighbour(5, len, Direction::Right), None);
        assert_eq!(neighbour(6, len, Direction::Left), None);
        assert_eq!(neighbour(3, len, Direction::Up), None);
        assert_eq!(neighbour(13, len, Direction::Right), None);
        assert_eq!(neighbour(13, len, Direction::Down), None);
    }

    #[test]
    fn down_into_a_short_last_row_lands_on_the_last_tile() {
        // Rows: 0..6, 6..8.
        assert_eq!(neighbour(1, 8, Direction::Down), Some(7));
        assert_eq!(neighbour(5, 8, Direction::Down), Some(7));
        assert_eq!(neighbour(0, 8, Direction::Down), Some(6));
        // Up from there goes straight up, not back to where we came from.
        assert_eq!(neighbour(7, 8, Direction::Up), Some(1));
    }

    #[test]
    fn right_to_left_swaps_left_and_right() {
        let mut home = Home::new(Order::default(), BTreeSet::new(), true);
        home.set_apps(&apps(5));
        press(&mut home, &[Input::Left]);
        assert_eq!(home.focus(), 1);
        press(&mut home, &[Input::Right]);
        assert_eq!(home.focus(), 0);
    }

    #[test]
    fn an_empty_home_ignores_every_input() {
        let mut home = Home::new(Order::default(), BTreeSet::new(), false);
        for input in [
            Input::Up,
            Input::Down,
            Input::Left,
            Input::Right,
            Input::OkPressed,
            Input::OkReleased,
            Input::Back,
            Input::Tick,
        ] {
            assert_eq!(home.handle(input, ms(1000)), None);
        }
        assert_eq!(home.focused(), None);
        assert_eq!(home.state(), State::Browse);
    }

    // --- scrolling --------------------------------------------------------

    #[test]
    fn scrolling_follows_focus_by_whole_rows() {
        let mut home = home(29); // 30 tiles, 5 rows
        assert_eq!(home.scroll(), Scroll::Shelf);
        let downs: Vec<Scroll> = (0..4)
            .map(|_| {
                press(&mut home, &[Input::Down]);
                home.scroll()
            })
            .collect();
        let grid = |first_row| Scroll::Grid { first_row };
        assert_eq!(downs, [grid(0), grid(0), grid(1), grid(2)]);

        let ups: Vec<Scroll> = (0..4)
            .map(|_| {
                press(&mut home, &[Input::Up]);
                home.scroll()
            })
            .collect();
        assert_eq!(ups, [grid(2), grid(2), grid(1), Scroll::Shelf]);
    }

    #[test]
    fn the_focused_row_always_stays_in_the_focus_slots() {
        // Every sequence of 7 D-pad presses on a 5-row grid.
        let keys = [Input::Up, Input::Down, Input::Left, Input::Right];
        for mut sequence in 0..keys.len().pow(7) {
            let mut home = home(29);
            for _ in 0..7 {
                press(&mut home, &[keys[sequence % keys.len()]]);
                sequence /= keys.len();
                let row = home.focus() / COLUMNS;
                match home.scroll() {
                    Scroll::Shelf => assert_eq!(row, 0),
                    Scroll::Grid { first_row } => {
                        assert!((first_row..first_row + FOCUS_SLOTS).contains(&row));
                    }
                }
            }
        }
    }

    #[test]
    fn back_returns_to_the_first_row_in_the_same_column() {
        let mut home = home(29);
        press(
            &mut home,
            &[Input::Down, Input::Down, Input::Right, Input::Right],
        );
        assert_eq!(home.focus(), 14);
        press(&mut home, &[Input::Back]);
        assert_eq!(home.focus(), 2);
        assert_eq!(home.scroll(), Scroll::Shelf);
        // Back in the first row does nothing.
        assert_eq!(press(&mut home, &[Input::Back]), None);
        assert_eq!(home.focus(), 2);
    }

    // --- app list changes -------------------------------------------------

    #[test]
    fn focus_stays_on_the_same_app_when_others_change() {
        let mut home = home(5);
        press(&mut home, &[Input::Right, Input::Right, Input::Right]);
        assert_eq!(component(&home), "a03");

        let mut fewer = apps(5);
        fewer.remove(1);
        assert!(home.set_apps(&fewer));
        assert_eq!(component(&home), "a03");
        assert_eq!(home.focus(), 2);

        fewer.push(app("z", "Zed"));
        assert!(home.set_apps(&fewer));
        assert_eq!(component(&home), "a03");
    }

    #[test]
    fn removing_the_focused_app_keeps_the_index() {
        let mut home = home(5);
        press(&mut home, &[Input::Right, Input::Right]);
        let mut fewer = apps(5);
        fewer.remove(2);
        home.set_apps(&fewer);
        assert_eq!(home.focus(), 2);
        assert_eq!(component(&home), "a03");
    }

    #[test]
    fn removing_the_last_tiles_clamps_focus() {
        let mut home = home(10);
        press(&mut home, &[Input::Down, Input::Right, Input::Right]);
        assert_eq!(home.focus(), 8);
        home.set_apps(&apps(3));
        assert_eq!(home.focused(), Some(&TileId::Settings));
        assert_eq!(home.focus(), 4);
        assert_eq!(home.scroll(), Scroll::Shelf);
    }

    #[test]
    fn focus_can_be_restored_by_identity_before_and_after_the_app_list() {
        // Before the app list: the stored order already has the tile.
        let order = Order::from_keys([(id("a"), 1000), (id("b"), 2000), (TileId::About, 3000)]);
        let mut home = Home::new(order, BTreeSet::new(), false);
        assert!(home.focus_on(&id("b")));
        home.set_apps(&[app("a", "A"), app("b", "B")]);
        assert_eq!(component(&home), "b");

        let mut home = self::home(29);
        assert!(home.focus_on(&id("a20")));
        assert_eq!(home.focus(), 20);
        assert_eq!(home.scroll(), Scroll::Grid { first_row: 1 });
        assert!(!home.focus_on(&id("gone")));
        assert_eq!(home.focus(), 20);
    }

    #[test]
    fn unchanged_apps_report_no_change() {
        let mut home = home(5);
        assert!(!home.set_apps(&apps(5)));
    }

    // --- order ------------------------------------------------------------

    #[test]
    fn first_run_sorts_by_label_ignoring_case_with_about_last() {
        let mut order = Order::default();
        order.reconcile(&[app("c", "cinema"), app("a", "Bravo"), app("b", "alpha")]);
        assert_eq!(
            order.ids(),
            [id("b"), id("a"), id("c"), TileId::About, TileId::Settings]
        );
        let keys: Vec<i64> = order.keys().iter().map(|&(_, key)| key).collect();
        assert!(keys.windows(2).all(|pair| pair[1] - pair[0] == KEY_STEP));
    }

    #[test]
    fn new_apps_go_after_the_last_tile_and_removed_apps_are_dropped() {
        let mut order = Order::from_keys([(TileId::About, 500), (id("b"), 1000)]);
        assert!(order.reconcile(&[app("b", "B"), app("a", "A")]));
        assert_eq!(
            order.ids(),
            [TileId::About, id("b"), id("a"), TileId::Settings]
        );

        assert!(order.reconcile(&[app("a", "A")]));
        assert_eq!(order.ids(), [TileId::About, id("a"), TileId::Settings]);
    }

    #[test]
    fn new_apps_go_before_about_when_it_is_last() {
        let mut order = Order::from_keys([(id("a"), 1000), (TileId::About, 2000)]);
        assert!(order.reconcile(&[app("a", "A"), app("c", "C"), app("b", "B")]));
        assert_eq!(
            order.ids(),
            [id("a"), id("b"), id("c"), TileId::About, TileId::Settings]
        );
        let keys: Vec<i64> = order.keys().iter().map(|&(_, key)| key).collect();
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn duplicate_apps_from_the_platform_appear_once() {
        let mut order = Order::default();
        order.reconcile(&[app("a", "A"), app("a", "A")]);
        assert_eq!(order.ids(), [id("a"), TileId::About, TileId::Settings]);
    }

    #[test]
    fn stored_keys_are_sorted_and_duplicate_ids_dropped() {
        let order = Order::from_keys([(id("b"), 2000), (id("a"), 3000), (id("b"), 1000)]);
        assert_eq!(order.keys(), [(id("b"), 2000), (id("a"), 3000)]);
    }

    #[test]
    fn placing_between_neighbours_takes_the_midpoint_and_keeps_others() {
        let mut order = Order::from_keys([(id("a"), 1000), (id("b"), 2000), (id("c"), 3000)]);
        order.place(&[id("a"), id("c"), id("b")], 1);
        assert_eq!(
            order.keys(),
            [(id("a"), 1000), (id("c"), 1500), (id("b"), 2000)]
        );
    }

    #[test]
    fn placing_at_either_end_steps_past_the_neighbour() {
        let mut order = Order::from_keys([(id("a"), 1000), (id("b"), 2000), (id("c"), 3000)]);
        order.place(&[id("c"), id("a"), id("b")], 0);
        assert_eq!(order.ids(), [id("c"), id("a"), id("b")]);
        assert_eq!(order.key(&id("c")), Some(0));

        order.place(&[id("a"), id("b"), id("c")], 2);
        assert_eq!(order.key(&id("c")), Some(2000 + KEY_STEP));
    }

    #[test]
    fn placing_without_a_free_key_renumbers_everything() {
        let mut order = Order::from_keys([(id("a"), 1), (id("b"), 2), (id("c"), 3)]);
        order.place(&[id("a"), id("c"), id("b")], 1);
        assert_eq!(
            order.keys(),
            [(id("a"), 1000), (id("c"), 2000), (id("b"), 3000)]
        );
    }

    #[test]
    fn placing_near_the_key_limit_renumbers_instead_of_overflowing() {
        let mut order = Order::from_keys([(id("a"), 0), (id("b"), i64::MAX)]);
        order.place(&[id("b"), id("a")], 1);
        assert_eq!(order.ids(), [id("b"), id("a")]);
    }

    // --- launch -----------------------------------------------------------

    #[test]
    fn a_short_press_launches_on_release() {
        let mut home = home(3);
        assert_eq!(home.handle(Input::OkPressed, ms(0)), None);
        assert_eq!(
            home.handle(Input::OkReleased, ms(100)),
            Some(Effect::Launch("a00".to_owned()))
        );
        assert_eq!(home.state(), State::Launching);
    }

    #[test]
    fn ok_is_ignored_while_a_launch_is_in_flight() {
        let mut home = home(3);
        press(&mut home, &[Input::OkPressed, Input::OkReleased]);
        assert_eq!(
            press(
                &mut home,
                &[Input::OkPressed, Input::OkReleased, Input::Right]
            ),
            None
        );
        assert_eq!(home.focus(), 0);
    }

    #[test]
    fn a_launch_times_out_with_an_error() {
        let mut home = home(3);
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::OkReleased, ms(100));
        assert_eq!(
            home.handle(Input::Tick, ms(100) + LAUNCH_TIMEOUT - ms(1)),
            None
        );
        assert_eq!(
            home.handle(Input::Tick, ms(100) + LAUNCH_TIMEOUT),
            Some(Effect::ShowLaunchError("a00".to_owned()))
        );
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn a_failed_launch_shows_an_error_and_frees_input() {
        let mut home = home(3);
        press(&mut home, &[Input::OkPressed, Input::OkReleased]);
        assert_eq!(
            press(&mut home, &[Input::LaunchFailed]),
            Some(Effect::ShowLaunchError("a00".to_owned()))
        );
        press(&mut home, &[Input::Right]);
        assert_eq!(home.focus(), 1);
    }

    #[test]
    fn pausing_ends_the_launch() {
        let mut home = home(3);
        press(
            &mut home,
            &[Input::OkPressed, Input::OkReleased, Input::Paused],
        );
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.focus(), 0);
    }

    #[test]
    fn moving_focus_while_holding_ok_does_not_launch() {
        let mut home = home(3);
        assert_eq!(
            press(
                &mut home,
                &[Input::OkPressed, Input::Right, Input::OkReleased]
            ),
            None
        );
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn about_opens_and_closes_with_back() {
        let mut home = home(2);
        press(&mut home, &[Input::Right, Input::Right]);
        assert_eq!(home.focused(), Some(&TileId::About));
        assert_eq!(
            press(&mut home, &[Input::OkPressed, Input::OkReleased]),
            Some(Effect::OpenAbout)
        );
        assert_eq!(home.state(), State::About);
        press(&mut home, &[Input::Right, Input::Back]);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.focus(), 2);
    }

    #[test]
    fn ok_on_about_asks_for_an_update_once_per_press() {
        let mut home = home(2);
        press(&mut home, &[Input::Right, Input::Right]);
        press(&mut home, &[Input::OkPressed, Input::OkReleased]);
        assert_eq!(home.handle(Input::OkPressed, ms(0)), Some(Effect::Update));
        // A held OK's repeats are ignored; the release does nothing.
        assert_eq!(home.handle(Input::OkPressed, ms(50)), None);
        assert_eq!(home.handle(Input::OkReleased, ms(100)), None);
        assert_eq!(home.state(), State::About);
        assert_eq!(home.handle(Input::OkPressed, ms(200)), Some(Effect::Update));
    }

    // --- move mode --------------------------------------------------------

    /// Holds OK on the focused tile until the menu opens, then picks Move.
    fn pick_up(home: &mut Home) {
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::OkPressed, ms(300)); // key repeat
        assert_eq!(home.handle(Input::Tick, LONG_PRESS - ms(1)), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.handle(Input::Tick, LONG_PRESS), None);
        assert_eq!(home.state(), State::Menu);
        // The release of the long press does not pick anything.
        assert_eq!(home.handle(Input::OkReleased, ms(700)), None);
        assert_eq!(
            home.menu().map(|(items, selected)| items[selected]),
            Some(MenuItem::Move)
        );
        assert_eq!(home.handle(Input::OkPressed, ms(800)), None);
        assert_eq!(home.state(), State::Moving);
        assert_eq!(home.handle(Input::OkReleased, ms(850)), None);
    }

    #[test]
    fn a_long_press_picks_up_and_ok_drops_and_saves() {
        let mut home = home(8);
        pick_up(&mut home);
        press(&mut home, &[Input::Right, Input::Right, Input::Down]);
        assert_eq!(home.focus(), 8);
        assert_eq!(component(&home), "a00");
        assert_eq!(home.scroll(), Scroll::Grid { first_row: 0 });

        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert_eq!(home.state(), State::Browse);
        let saved: Vec<TileId> = home.order().ids();
        assert_eq!(saved, home.tiles());
        assert_eq!(saved[8], id("a00"));
        // The release that follows the drop does not launch.
        assert_eq!(press(&mut home, &[Input::OkReleased]), None);
    }

    #[test]
    fn a_long_release_without_a_tick_still_opens_the_menu() {
        let mut home = home(3);
        home.handle(Input::OkPressed, ms(0));
        assert_eq!(home.handle(Input::OkReleased, LONG_PRESS), None);
        assert_eq!(home.state(), State::Menu);
    }

    #[test]
    fn back_cancels_a_move_and_restores_the_exact_order() {
        let mut home = home(8);
        press(&mut home, &[Input::Right]);
        let before = home.tiles().to_vec();
        pick_up(&mut home);
        press(&mut home, &[Input::Right, Input::Down, Input::Left]);
        assert_ne!(home.tiles(), before);

        assert_eq!(press(&mut home, &[Input::Back]), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.tiles(), before);
        assert_eq!(home.focus(), 1);
        assert_eq!(home.order().ids(), before);
    }

    #[test]
    fn pausing_cancels_a_move() {
        let mut home = home(4);
        let before = home.tiles().to_vec();
        pick_up(&mut home);
        press(&mut home, &[Input::Right, Input::Paused]);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.tiles(), before);
    }

    #[test]
    fn dropping_in_place_saves_nothing() {
        let mut home = home(4);
        pick_up(&mut home);
        press(&mut home, &[Input::Right, Input::Left]);
        assert_eq!(press(&mut home, &[Input::OkPressed]), None);
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn an_unattended_move_drops_and_saves_after_the_idle_timeout() {
        let mut home = home(4);
        pick_up(&mut home);
        let last_key = ms(1000);
        home.handle(Input::Right, last_key);
        assert_eq!(
            home.handle(Input::Tick, last_key + MOVE_IDLE_TIMEOUT - ms(1)),
            None
        );
        assert_eq!(home.state(), State::Moving);
        assert_eq!(
            home.handle(Input::Tick, last_key + MOVE_IDLE_TIMEOUT),
            Some(Effect::SaveOrder)
        );
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.order().ids()[1], id("a00"));
    }

    #[test]
    fn an_unattended_move_without_changes_ends_without_saving() {
        let mut home = home(4);
        pick_up(&mut home);
        // pick_up's last key is the OK release at 850 ms.
        assert_eq!(home.handle(Input::Tick, ms(850) + MOVE_IDLE_TIMEOUT), None);
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn a_moved_tile_cannot_leave_the_grid() {
        let mut home = home(4);
        pick_up(&mut home);
        press(&mut home, &[Input::Left, Input::Up, Input::Down]);
        assert_eq!(home.focus(), 0);
        press(&mut home, &[Input::Right; 10]);
        assert_eq!(home.focus(), 5);
        assert_eq!(home.tiles().len(), 6);
    }

    // --- menu and hidden apps ---------------------------------------------

    /// Opens the long-press menu on the focused tile.
    fn open_menu(home: &mut Home) {
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::Tick, LONG_PRESS);
        home.handle(Input::OkReleased, LONG_PRESS + ms(50));
        assert_eq!(home.state(), State::Menu);
    }

    /// Hides the focused app through its menu.
    fn hide(home: &mut Home) -> Option<Effect> {
        open_menu(home);
        let effect = press(home, &[Input::Down, Input::OkPressed]);
        press(home, &[Input::OkReleased]);
        effect
    }

    fn hidden_page(home: &Home) -> Vec<&str> {
        home.hidden_apps().iter().map(String::as_str).collect()
    }

    #[test]
    fn the_menu_offers_move_and_hide_for_apps_and_only_move_for_about() {
        let mut home = home(2);
        open_menu(&mut home);
        assert_eq!(
            home.menu(),
            Some((&[MenuItem::Move, MenuItem::Hide][..], 0))
        );
        press(&mut home, &[Input::Down, Input::Down]);
        assert_eq!(home.menu().map(|(_, selected)| selected), Some(1));
        press(&mut home, &[Input::Up, Input::Up]);
        assert_eq!(home.menu().map(|(_, selected)| selected), Some(0));
        // Back closes it without doing anything.
        assert_eq!(press(&mut home, &[Input::Back]), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.focus(), 0);

        press(&mut home, &[Input::Right, Input::Right]);
        assert_eq!(home.focused(), Some(&TileId::About));
        open_menu(&mut home);
        assert_eq!(home.menu(), Some((&[MenuItem::Move][..], 0)));
        press(&mut home, &[Input::Down, Input::OkPressed]);
        assert_eq!(home.state(), State::Moving);
    }

    #[test]
    fn hiding_an_app_takes_it_off_home_and_the_next_tile_gets_focus() {
        let mut home = home(3);
        press(&mut home, &[Input::Right]);
        assert_eq!(hide(&mut home), Some(Effect::SaveOrder));
        assert_eq!(home.state(), State::Browse);
        assert_eq!(
            home.tiles(),
            [
                id("a00"),
                id("a02"),
                TileId::About,
                TileId::Settings,
                TileId::Hidden
            ]
        );
        assert_eq!(component(&home), "a02");
        assert!(home.hidden().contains("a01"));
        assert!(!home.order().ids().contains(&id("a01")));
        assert_eq!(hidden_page(&home), ["a01"]);
    }

    #[test]
    fn hidden_apps_cannot_get_focus_on_home() {
        let mut home = home(3);
        press(&mut home, &[Input::Right]);
        hide(&mut home);
        // Every tile reachable with the D-pad is a shown one.
        for input in [
            Input::Left,
            Input::Right,
            Input::Right,
            Input::Right,
            Input::Down,
        ] {
            press(&mut home, &[input]);
            assert_ne!(home.focused(), Some(&id("a01")));
        }
        assert!(!home.focus_on(&id("a01")));
    }

    #[test]
    fn hidden_apps_stay_hidden_when_the_app_list_is_read_again() {
        let hidden = BTreeSet::from(["a01".to_owned()]);
        let mut home = Home::new(Order::default(), hidden, false);
        assert!(home.set_apps(&apps(3)));
        assert_eq!(
            home.tiles(),
            [
                id("a00"),
                id("a02"),
                TileId::About,
                TileId::Settings,
                TileId::Hidden
            ]
        );
        // An uninstalled hidden app is not listed, and the tile goes with the last one.
        home.set_apps(&[app("a00", "App 00")]);
        assert_eq!(home.tiles(), [id("a00"), TileId::About, TileId::Settings]);
        assert!(home.hidden().contains("a01"));
    }

    #[test]
    fn the_hidden_apps_tile_cannot_be_moved_or_passed_by_a_moving_tile() {
        let mut home = home(3);
        hide(&mut home);
        // a01, a02, About, Settings, Hidden apps.
        press(
            &mut home,
            &[Input::Right, Input::Right, Input::Right, Input::Right],
        );
        assert_eq!(home.focused(), Some(&TileId::Hidden));
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::Tick, LONG_PRESS);
        assert_eq!(home.state(), State::Browse);
        home.handle(Input::OkReleased, LONG_PRESS);

        press(
            &mut home,
            &[Input::Left, Input::Left, Input::Left, Input::Left],
        );
        pick_up(&mut home);
        press(&mut home, &[Input::Right; 5]);
        assert_eq!(home.focus(), 3);
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert_eq!(
            home.tiles(),
            [
                id("a02"),
                TileId::About,
                TileId::Settings,
                id("a01"),
                TileId::Hidden
            ]
        );
        assert_eq!(
            home.order().ids(),
            [id("a02"), TileId::About, TileId::Settings, id("a01")]
        );
    }

    #[test]
    fn showing_a_hidden_app_again_puts_it_before_about() {
        let mut home = home(4);
        hide(&mut home); // a00
        press(&mut home, &[Input::Right]);
        hide(&mut home); // a02
        assert_eq!(hidden_page(&home), ["a00", "a02"]);

        // Open the page from the last tile.
        press(&mut home, &[Input::Right, Input::Right, Input::Right]);
        assert_eq!(home.focused(), Some(&TileId::Hidden));
        assert_eq!(
            press(&mut home, &[Input::OkPressed, Input::OkReleased]),
            Some(Effect::OpenHidden)
        );
        assert_eq!(home.state(), State::Hidden);
        assert_eq!(home.hidden_focus(), Some(0));

        press(&mut home, &[Input::Right]);
        assert_eq!(home.hidden_focus(), Some(1));
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        // The page stays open on the one left.
        assert_eq!(home.hidden_focus(), Some(0));
        assert_eq!(hidden_page(&home), ["a00"]);
        assert_eq!(
            home.tiles(),
            [
                id("a01"),
                id("a03"),
                id("a02"),
                TileId::About,
                TileId::Settings,
                TileId::Hidden
            ]
        );

        // The last one closes the page, with focus on it on Home.
        press(&mut home, &[Input::OkReleased]);
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert_eq!(home.state(), State::Browse);
        assert!(home.hidden().is_empty());
        assert_eq!(
            home.tiles(),
            [
                id("a01"),
                id("a03"),
                id("a02"),
                id("a00"),
                TileId::About,
                TileId::Settings
            ]
        );
        assert_eq!(component(&home), "a00");
    }

    #[test]
    fn back_and_pause_close_the_menu_and_the_hidden_page() {
        let mut home = home(3);
        open_menu(&mut home);
        press(&mut home, &[Input::Paused]);
        assert_eq!(home.state(), State::Browse);

        hide(&mut home);
        press(
            &mut home,
            &[Input::Right, Input::Right, Input::Right, Input::Right],
        );
        press(&mut home, &[Input::OkPressed, Input::OkReleased]);
        assert_eq!(home.state(), State::Hidden);
        press(&mut home, &[Input::Back]);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.focused(), Some(&TileId::Hidden));
        press(
            &mut home,
            &[Input::OkPressed, Input::OkReleased, Input::Paused],
        );
        assert_eq!(home.state(), State::Browse);
    }
}

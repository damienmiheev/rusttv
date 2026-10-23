//! Home screen state, independent of UI and platform (ARCHITECTURE.md § Launcher).
//!
//! Time comes from the caller as a monotonic [`Duration`] (for example time
//! since start-up), so the model is deterministic and testable on the host.

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
const KEY_STEP: i64 = 1000;

/// One tile on the home screen.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TileId {
    /// An installed app, identified by its component name (`package/activity`).
    App(String),
    /// The built-in About tile.
    About,
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

/// Something the caller must do after [`Home::handle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Start this app (component name).
    Launch(String),
    /// Show the About page.
    OpenAbout,
    /// Persist [`Home::order`].
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
    /// New apps are appended sorted by label, and About is added last if missing,
    /// so a first run gives an alphabetical grid ending with About.
    fn reconcile(&mut self, apps: &[App]) -> bool {
        let before = self.keys.len();
        self.keys.retain(|(id, _)| match id {
            TileId::App(component) => apps.iter().any(|app| app.id == *component),
            TileId::About => true,
        });
        let mut changed = self.keys.len() != before;

        let mut new: Vec<&App> = apps
            .iter()
            .filter(|app| !self.contains(&TileId::App(app.id.clone())))
            .collect();
        new.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        new.dedup_by(|a, b| a.id == b.id);

        let new_ids = new
            .into_iter()
            .map(|app| TileId::App(app.id.clone()))
            .chain((!self.contains(&TileId::About)).then_some(TileId::About));
        for id in new_ids {
            let key = self
                .keys
                .last()
                .map_or(KEY_STEP, |&(_, last)| last + KEY_STEP);
            self.keys.push((id, key));
            changed = true;
        }
        changed
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
}

/// The home screen: tile order, focus, scroll position and mode.
#[derive(Debug, Clone)]
pub struct Home {
    order: Order,
    /// Tiles in display order; during a move this differs from `order`.
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
    /// Creates a home screen with a stored order; call [`Home::set_apps`] next.
    pub fn new(order: Order, right_to_left: bool) -> Self {
        Self {
            tiles: order.ids(),
            order,
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
        if matches!(self.mode, Mode::Moving { .. }) {
            self.cancel_move();
        }
        let focused = self.focused().cloned();
        let changed = self.order.reconcile(apps);
        self.tiles = self.order.ids();
        self.focus = focused
            .and_then(|id| self.tiles.iter().position(|tile| *tile == id))
            .unwrap_or_else(|| self.focus.min(self.tiles.len().saturating_sub(1)));
        self.update_scroll();
        changed
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

    /// Current scroll state.
    pub fn scroll(&self) -> Scroll {
        match self.focus / COLUMNS {
            0 => Scroll::Shelf,
            _ => Scroll::Grid {
                first_row: self.first_row,
            },
        }
    }

    /// Current state.
    pub fn state(&self) -> State {
        match self.mode {
            Mode::Browse => State::Browse,
            Mode::Moving { .. } => State::Moving,
            Mode::Launching { .. } => State::Launching,
            Mode::About => State::About,
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
            (Mode::Moving { .. } | Mode::About | Mode::Launching { .. }, Input::Paused) => {
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
            (Mode::Moving { .. }, _) => self.handle_moving(input, now),
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
                    self.start_move(now);
                    None
                }
            }
            Input::Tick => {
                if now.saturating_sub(self.ok_since?) >= LONG_PRESS {
                    self.ok_since = None;
                    self.start_move(now);
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
            if let Some(target) = neighbour(self.focus, self.tiles.len(), direction) {
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

    /// Ends move mode, keeping the tile where it is.
    fn drop_tile(&mut self) -> Option<Effect> {
        let moved = self.tiles != self.order.ids();
        self.mode = Mode::Browse;
        if !moved {
            return None;
        }
        self.order.place(&self.tiles, self.focus);
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
        }
    }

    fn start_move(&mut self, now: Duration) {
        if self.focused().is_some() {
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
        let mut home = Home::new(Order::default(), false);
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
        let mut home = Home::new(Order::default(), true);
        home.set_apps(&apps(5));
        press(&mut home, &[Input::Left]);
        assert_eq!(home.focus(), 1);
        press(&mut home, &[Input::Right]);
        assert_eq!(home.focus(), 0);
    }

    #[test]
    fn an_empty_home_ignores_every_input() {
        let mut home = Home::new(Order::default(), false);
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
        assert_eq!(home.focused(), Some(&TileId::About));
        assert_eq!(home.focus(), 3);
        assert_eq!(home.scroll(), Scroll::Shelf);
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
        assert_eq!(order.ids(), [id("b"), id("a"), id("c"), TileId::About]);
        let keys: Vec<i64> = order.keys().iter().map(|&(_, key)| key).collect();
        assert!(keys.windows(2).all(|pair| pair[1] - pair[0] == KEY_STEP));
    }

    #[test]
    fn new_apps_go_after_the_last_tile_and_removed_apps_are_dropped() {
        let mut order = Order::from_keys([(TileId::About, 500), (id("b"), 1000)]);
        assert!(order.reconcile(&[app("b", "B"), app("a", "A")]));
        assert_eq!(order.ids(), [TileId::About, id("b"), id("a")]);

        assert!(order.reconcile(&[app("a", "A")]));
        assert_eq!(order.ids(), [TileId::About, id("a")]);
    }

    #[test]
    fn duplicate_apps_from_the_platform_appear_once() {
        let mut order = Order::default();
        order.reconcile(&[app("a", "A"), app("a", "A")]);
        assert_eq!(order.ids(), [id("a"), TileId::About]);
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

    // --- move mode --------------------------------------------------------

    /// Holds OK on the focused tile until move mode starts.
    fn pick_up(home: &mut Home) {
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::OkPressed, ms(300)); // key repeat
        assert_eq!(home.handle(Input::Tick, LONG_PRESS - ms(1)), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.handle(Input::Tick, LONG_PRESS), None);
        assert_eq!(home.state(), State::Moving);
        assert_eq!(home.handle(Input::OkReleased, ms(700)), None);
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
    fn a_long_release_without_a_tick_still_picks_up() {
        let mut home = home(3);
        home.handle(Input::OkPressed, ms(0));
        assert_eq!(home.handle(Input::OkReleased, LONG_PRESS), None);
        assert_eq!(home.state(), State::Moving);
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
        // pick_up's last key is the OK release at 700 ms.
        assert_eq!(home.handle(Input::Tick, ms(700) + MOVE_IDLE_TIMEOUT), None);
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn a_moved_tile_cannot_leave_the_grid() {
        let mut home = home(4);
        pick_up(&mut home);
        press(&mut home, &[Input::Left, Input::Up, Input::Down]);
        assert_eq!(home.focus(), 0);
        press(&mut home, &[Input::Right; 10]);
        assert_eq!(home.focus(), 4);
        assert_eq!(home.tiles().len(), 5);
    }
}

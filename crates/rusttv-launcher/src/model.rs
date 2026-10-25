//! Home screen state, independent of UI and platform (ARCHITECTURE.md § Launcher).
//!
//! Time comes from the caller as a monotonic [`Duration`] (for example time
//! since start-up), so the model is deterministic and testable on the host.

use std::collections::{BTreeMap, BTreeSet};
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

/// How long a moving app must stay on top of another app (or a folder)
/// before they make a folder (or it goes in).
///
/// An arrow pressed sooner passes over the tile; OK sooner drops the app in
/// that place. The caller sends [`Input::Tick`] once this has passed after
/// the move.
pub const FOLDER_HOVER: Duration = Duration::from_millis(600);

/// Apps per row in an open folder.
///
/// The folder panel (`home.slint`) is laid out for this value.
const FOLDER_COLUMNS: usize = 4;

/// Apps per page of an open folder: three rows. Pages sit side by side;
/// Left and Right cross from one to the next.
const FOLDER_PAGE: usize = 3 * FOLDER_COLUMNS;

/// The longest folder name, in characters.
pub const FOLDER_NAME_MAX: usize = 40;

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
    /// A folder of apps, by its number (the key of [`Home::folders`]).
    Folder(u32),
}

/// A launchable app as reported by the platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    /// Component name (`package/activity`).
    pub id: String,
    /// User-visible name; used to sort apps on first run.
    pub label: String,
    /// What kind of app it says it is; names a new folder.
    pub category: Category,
}

/// An app's own category (Android's `ApplicationInfo.category`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Category {
    /// Games.
    Game,
    /// Music, podcasts, radio.
    Audio,
    /// Films, TV, video.
    Video,
    /// Photos and pictures.
    Image,
    /// Social networks and messaging.
    Social,
    /// News and magazines.
    News,
    /// Maps and navigation.
    Maps,
    /// Productivity.
    Productivity,
    /// Accessibility.
    Accessibility,
    /// No category (most apps).
    #[default]
    Other,
}

impl Category {
    /// Every category, in the order of [`Category::index`].
    pub const ALL: [Self; 10] = [
        Self::Game,
        Self::Audio,
        Self::Video,
        Self::Image,
        Self::Social,
        Self::News,
        Self::Maps,
        Self::Productivity,
        Self::Accessibility,
        Self::Other,
    ];

    /// From Android's `ApplicationInfo.CATEGORY_*` value (0–8); anything
    /// else, including `CATEGORY_UNDEFINED` (-1), is [`Category::Other`].
    pub fn from_android(value: i32) -> Self {
        usize::try_from(value)
            .ok()
            .and_then(|index| Self::ALL.get(index))
            .copied()
            .filter(|category| *category != Self::Other)
            .unwrap_or(Self::Other)
    }

    /// Position in [`Category::ALL`]; [`FolderNames`] is indexed by it.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The first name of a new folder for each [`Category`], in the UI's
/// language; [`Category::Other`] is the plain "Folder".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderNames(pub [String; 10]);

impl Default for FolderNames {
    fn default() -> Self {
        Self(
            [
                "Games",
                "Music",
                "Video",
                "Photos",
                "Social",
                "News",
                "Maps",
                "Productivity",
                "Accessibility",
                "Folder",
            ]
            .map(String::from),
        )
    }
}

/// A folder on Home: its name and its apps (component names) in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// What the person called it, or the first name it got.
    pub name: String,
    /// Component names, in the folder's order. Never empty on Home: a
    /// folder goes when its last app leaves.
    pub apps: Vec<String>,
}

/// An open folder: which one, and what has focus in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFolder {
    /// The folder's number.
    pub id: u32,
    /// The focused app's index in [`Folder::apps`]; `None` while the name
    /// has focus.
    pub focus: Option<usize>,
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
    /// Persist [`Home::order`], [`Home::hidden`] and [`Home::folders`].
    SaveOrder,
    /// OK on an open folder's name: let the person type a new one, then
    /// call [`Home::rename_folder`].
    RenameFolder(u32),
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
    /// A folder is open over Home ([`Home::open_folder`]).
    Folder,
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

    /// Drops apps and folders that are gone and appends new ones; returns
    /// whether anything changed.
    ///
    /// `apps` are the apps shown outside folders, `folders` the folders on
    /// Home. New apps are appended sorted by label, then folders missing
    /// from the order, before the built-in tiles (About, then Settings)
    /// while those are at the end, so a first run gives an alphabetical
    /// grid ending with About and Settings. A built-in tile the user moved
    /// elsewhere stays where it is; a missing one is added to the end block.
    fn reconcile(&mut self, apps: &[App], folders: &BTreeSet<u32>) -> bool {
        let before = self.keys.clone();
        self.keys.retain(|(id, _)| match id {
            TileId::App(component) => apps.iter().any(|app| app.id == *component),
            TileId::About | TileId::Settings => true,
            TileId::Hidden => false,
            TileId::Folder(folder) => folders.contains(folder),
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
        for folder in folders {
            let id = TileId::Folder(*folder);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
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
            _ => self.renumber(tiles),
        }
    }

    /// Makes `tiles` the order, with fresh keys.
    fn renumber(&mut self, tiles: &[TileId]) {
        self.keys = (1..)
            .zip(tiles)
            .map(|(n, id)| (id.clone(), n * KEY_STEP))
            .collect();
    }
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Home before a move, put back when it is cancelled.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Before {
    /// The moving tile.
    tile: TileId,
    tiles: Vec<TileId>,
    folders: BTreeMap<u32, Folder>,
    /// The folder the app was picked up in, open again after a cancel.
    folder: Option<u32>,
}

/// Where a moving app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// On Home at `Home::focus`. `over` is the tile it just passed onto and
    /// when: drawn under the app until the next key, it makes a folder
    /// with the app after [`FOLDER_HOVER`].
    Home { over: Option<(usize, Duration)> },
    /// In an open folder, at this index of its apps.
    Folder { id: u32, index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    /// `before` is put back on cancel; `last_input` is when the user last
    /// pressed a key in this mode; `made` is a folder this move made, which
    /// goes again if the app leaves it before the drop.
    Moving {
        before: Box<Before>,
        last_input: Duration,
        place: Place,
        made: Option<u32>,
    },
    /// `folder` is the open folder it was launched from, open again on return.
    Launching {
        app: String,
        since: Duration,
        folder: Option<OpenFolder>,
    },
    About,
    Settings,
    /// `selected` indexes [`menu_items`] of the focused tile, or of the
    /// focused app in `folder`.
    Menu {
        selected: usize,
        folder: Option<OpenFolder>,
    },
    /// `focus` indexes `Home::hidden_apps`.
    Hidden {
        focus: usize,
    },
    Folder(OpenFolder),
}

/// What the long-press menu offers for a tile; empty means no menu.
fn menu_items(tile: Option<&TileId>) -> &'static [MenuItem] {
    match tile {
        Some(TileId::App(_)) => &[MenuItem::Move, MenuItem::Hide],
        Some(TileId::About | TileId::Settings | TileId::Folder(_)) => &[MenuItem::Move],
        Some(TileId::Hidden) | None => &[],
    }
}

/// The home screen: tile order, folders, hidden apps, focus, scroll position and mode.
#[derive(Debug, Clone)]
pub struct Home {
    order: Order,
    /// Folders on Home by number; each one is in `order` once.
    folders: BTreeMap<u32, Folder>,
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
    /// When OK went down in browse mode (or in a folder); cleared once it
    /// launches, moves or is cancelled.
    ok_since: Option<Duration>,
    right_to_left: bool,
    folder_names: FolderNames,
}

impl Home {
    /// Creates a home screen with a stored order, folders and hidden apps;
    /// call [`Home::set_apps`] next.
    pub fn new(
        order: Order,
        folders: BTreeMap<u32, Folder>,
        hidden: BTreeSet<String>,
        right_to_left: bool,
    ) -> Self {
        Self {
            tiles: order.ids(),
            order,
            folders,
            hidden,
            apps: Vec::new(),
            hidden_apps: Vec::new(),
            focus: 0,
            first_row: 0,
            mode: Mode::Browse,
            ok_held: false,
            ok_since: None,
            right_to_left,
            folder_names: FolderNames::default(),
        }
    }

    /// Sets the first names of new folders (in the UI's language).
    pub fn set_folder_names(&mut self, names: FolderNames) {
        self.folder_names = names;
    }

    /// Replaces the app list, keeping focus on the same tile; returns whether the order changed.
    pub fn set_apps(&mut self, apps: &[App]) -> bool {
        match self.mode {
            Mode::Moving { .. } => self.cancel_move(),
            Mode::Menu { folder, .. } => self.mode = folder.map_or(Mode::Browse, Mode::Folder),
            _ => {}
        }
        self.apps = apps.to_vec();
        self.refresh()
    }

    /// Rebuilds the tiles from the app list, folders, hidden apps and order,
    /// keeping focus on the same tile; returns whether the order or a folder
    /// changed.
    fn refresh(&mut self) -> bool {
        let focused = self.focused().cloned();
        let (mut hidden, shown): (Vec<&App>, Vec<&App>) = self
            .apps
            .iter()
            .partition(|app| self.hidden.contains(&app.id));
        hidden.sort_by(|a, b| {
            a.label
                .to_lowercase()
                .cmp(&b.label.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        hidden.dedup_by(|a, b| a.id == b.id);
        self.hidden_apps = hidden.into_iter().map(|app| app.id.clone()).collect();

        // Folders keep only shown apps, each once; an empty folder goes.
        let folders_before = self.folders.clone();
        let mut placed = BTreeSet::new();
        for folder in self.folders.values_mut() {
            folder
                .apps
                .retain(|id| shown.iter().any(|app| app.id == *id) && placed.insert(id.clone()));
        }
        self.folders.retain(|_, folder| !folder.apps.is_empty());
        let top: Vec<App> = shown
            .into_iter()
            .filter(|app| !placed.contains(&app.id))
            .cloned()
            .collect();

        let ids: BTreeSet<u32> = self.folders.keys().copied().collect();
        let changed = self.order.reconcile(&top, &ids) || self.folders != folders_before;
        self.tiles = self.order.ids();
        if !self.hidden_apps.is_empty() {
            self.tiles.push(TileId::Hidden);
        }
        self.focus = focused
            .and_then(|id| self.tiles.iter().position(|tile| *tile == id))
            .unwrap_or_else(|| self.focus.min(self.tiles.len().saturating_sub(1)));
        match &mut self.mode {
            Mode::Hidden { focus } => match self.hidden_apps.len() {
                0 => self.mode = Mode::Browse,
                len => *focus = (*focus).min(len - 1),
            },
            Mode::Folder(open) => match self.folders.get(&open.id) {
                None => self.mode = Mode::Browse,
                Some(folder) => {
                    open.focus = open.focus.map(|focus| focus.min(folder.apps.len() - 1));
                }
            },
            _ => {}
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

    /// The folders on Home, by number. During a move this differs from what
    /// is saved until the drop.
    pub fn folders(&self) -> &BTreeMap<u32, Folder> {
        &self.folders
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

    /// The open folder, also while its menu is open or an app moves in it.
    pub fn open_folder(&self) -> Option<OpenFolder> {
        match self.mode {
            Mode::Folder(open)
            | Mode::Menu {
                folder: Some(open), ..
            }
            | Mode::Launching {
                folder: Some(open), ..
            } => Some(open),
            Mode::Moving {
                place: Place::Folder { id, index },
                ..
            } => Some(OpenFolder {
                id,
                focus: Some(index),
            }),
            _ => None,
        }
    }

    /// While an app moves on Home: the index in [`Home::tiles`] of the tile
    /// it is on top of, drawn under it until the next key.
    pub fn moving_over(&self) -> Option<usize> {
        match self.mode {
            Mode::Moving {
                place: Place::Home {
                    over: Some((index, _)),
                },
                ..
            } => Some(index),
            _ => None,
        }
    }

    /// When the tile under the moving app makes a folder with it (or takes
    /// it in): `None` when nothing will.
    pub fn folder_due(&self) -> Option<Duration> {
        match self.mode {
            Mode::Moving {
                place:
                    Place::Home {
                        over: Some((index, since)),
                    },
                ..
            } if self.can_take(index) => Some(since + FOLDER_HOVER),
            _ => None,
        }
    }

    /// The open long-press menu's items and the selected index.
    pub fn menu(&self) -> Option<(&'static [MenuItem], usize)> {
        match self.mode {
            Mode::Menu { selected, .. } => Some((menu_items(self.current().as_ref()), selected)),
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

    /// Replaces the order, folders and hidden apps (the history rewrote
    /// them), keeping focus on the same tile where it still exists.
    pub fn replace_order(
        &mut self,
        order: Order,
        folders: BTreeMap<u32, Folder>,
        hidden: BTreeSet<String>,
    ) {
        if matches!(self.mode, Mode::Moving { .. }) {
            self.cancel_move();
        }
        self.order = order;
        self.folders = folders;
        self.hidden = hidden;
        self.refresh();
    }

    /// Gives folder `id` a new name; returns whether it changed (then save).
    ///
    /// Spaces at either end, control characters and bidi controls are
    /// dropped and the name is cut to [`FOLDER_NAME_MAX`] characters; an
    /// empty name keeps the old one.
    pub fn rename_folder(&mut self, id: u32, name: &str) -> bool {
        let name: String = name
            .trim()
            .chars()
            .filter(|c| {
                !c.is_control()
                    && !matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
            })
            .take(FOLDER_NAME_MAX)
            .collect();
        let name = name.trim_end();
        match self.folders.get_mut(&id) {
            Some(folder) if !name.is_empty() && folder.name != name => {
                name.clone_into(&mut folder.name);
                true
            }
            _ => false,
        }
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
            Mode::Folder(_) => State::Folder,
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
            // Back from an app opened in a folder: the folder is still open.
            (Mode::Launching { folder, .. }, Input::Paused) => {
                self.mode = folder.map_or(Mode::Browse, Mode::Folder);
                None
            }
            (
                Mode::Moving { .. }
                | Mode::About
                | Mode::Settings
                | Mode::Menu { .. }
                | Mode::Hidden { .. }
                | Mode::Folder(_),
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
                self.end_launch();
                Some(effect)
            }
            (Mode::Launching { app, since, .. }, Input::Tick) => {
                if now.saturating_sub(*since) < LAUNCH_TIMEOUT {
                    return None;
                }
                let effect = Effect::ShowLaunchError(app.clone());
                self.end_launch();
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
            (Mode::Folder(open), _) => {
                let open = *open;
                self.handle_folder(open, input, now)
            }
            (Mode::Browse, _) => self.handle_browse(input, now),
            _ => None,
        }
    }

    fn end_launch(&mut self) {
        if let Mode::Launching { folder, .. } = self.mode {
            self.mode = folder.map_or(Mode::Browse, Mode::Folder);
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
            Input::Back => {
                self.focus %= COLUMNS;
                self.update_scroll();
                None
            }
            _ => self.handle_ok(input, now),
        }
    }

    /// OK on the focused tile (or the focused app in an open folder): a
    /// short press opens it, a long one opens its menu.
    fn handle_ok(&mut self, input: Input, now: Duration) -> Option<Effect> {
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
            _ => None,
        }
    }

    /// The D-pad moves within the folder (Up from its first row reaches the
    /// name), OK opens an app or renames the folder, Back closes it.
    fn handle_folder(&mut self, open: OpenFolder, input: Input, now: Duration) -> Option<Effect> {
        let len = self
            .folders
            .get(&open.id)
            .map_or(0, |folder| folder.apps.len());
        if let Some(direction) = self.direction(input) {
            self.ok_since = None;
            let focus = match (open.focus, direction) {
                (None, Direction::Down) => Some(0),
                (None, _) => None,
                (Some(index), Direction::Up) if index % FOLDER_PAGE < FOLDER_COLUMNS => None,
                (Some(index), _) => Some(folder_neighbour(index, len, direction).unwrap_or(index)),
            };
            self.mode = Mode::Folder(OpenFolder { focus, ..open });
            return None;
        }
        match (open.focus, input) {
            (_, Input::Back) => {
                self.ok_since = None;
                self.mode = Mode::Browse;
                None
            }
            (None, Input::OkPressed) => Some(Effect::RenameFolder(open.id)),
            (None, _) => None,
            (Some(_), _) => self.handle_ok(input, now),
        }
    }

    fn handle_moving(&mut self, input: Input, now: Duration) -> Option<Effect> {
        let Mode::Moving {
            last_input, place, ..
        } = &mut self.mode
        else {
            return None;
        };
        if input == Input::Tick {
            if now.saturating_sub(*last_input) >= MOVE_IDLE_TIMEOUT {
                return self.drop_tile();
            }
            if self.folder_due().is_some_and(|due| now >= due) {
                self.take_into_folder(now);
            }
            return None;
        }
        *last_input = now;
        let place = *place;
        if let Some(direction) = self.direction(input) {
            match place {
                Place::Home { .. } => self.move_on_home(direction, now),
                Place::Folder { id, index } => self.move_in_folder(id, index, direction),
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

    /// Moves the app on Home one tile in `direction`, onto that tile.
    fn move_on_home(&mut self, direction: Direction, now: Duration) {
        let over = neighbour(self.focus, self.movable(), direction).map(|target| {
            let tile = self.tiles.remove(self.focus);
            self.tiles.insert(target, tile);
            // The tile that was there moved one step towards the gap.
            let over = if self.focus < target {
                target - 1
            } else {
                target + 1
            };
            self.focus = target;
            self.update_scroll();
            (over, now)
        });
        if let Mode::Moving { place, .. } = &mut self.mode {
            *place = Place::Home { over };
        }
    }

    /// Moves the app within its folder; past the folder's edge it leaves the
    /// folder for Home, just after the folder's tile.
    fn move_in_folder(&mut self, id: u32, index: usize, direction: Direction) {
        let Some(folder) = self.folders.get_mut(&id) else {
            return;
        };
        if let Some(target) = folder_neighbour(index, folder.apps.len(), direction) {
            let app = folder.apps.remove(index);
            folder.apps.insert(target, app);
            if let Mode::Moving { place, .. } = &mut self.mode {
                *place = Place::Folder { id, index: target };
            }
            return;
        }
        let app = folder.apps.remove(index);
        let left = folder.apps.len();
        let made = matches!(self.mode, Mode::Moving { made: Some(made), .. } if made == id);
        let at = self.focus;
        if left == 0 {
            self.folders.remove(&id);
            self.tiles.remove(at);
            self.tiles.insert(at, TileId::App(app));
        } else {
            if made && left == 1 {
                // A folder this move made goes again: the other app is back.
                if let Some(other) = self
                    .folders
                    .remove(&id)
                    .and_then(|f| f.apps.into_iter().next())
                {
                    self.tiles[at] = TileId::App(other);
                }
            }
            self.tiles.insert(at + 1, TileId::App(app));
            self.focus = at + 1;
        }
        self.update_scroll();
        if let Mode::Moving { place, made, .. } = &mut self.mode {
            *place = Place::Home { over: None };
            if *made == Some(id) && !self.folders.contains_key(&id) {
                *made = None;
            }
        }
    }

    /// The moving app has stayed on top of an app or a folder: they make a
    /// folder in that tile's place (or it goes into that folder), which opens
    /// with the app still moving, at its end.
    fn take_into_folder(&mut self, now: Duration) {
        let Some(over) = self.moving_over() else {
            return;
        };
        let (TileId::App(app), Some(target)) = (
            self.tiles[self.focus].clone(),
            self.tiles.get(over).cloned(),
        ) else {
            return;
        };
        let (id, made) = match target {
            TileId::Folder(id) => (id, false),
            TileId::App(other) => {
                let id = self.folders.keys().max().map_or(1, |max| max + 1);
                let name = self.folder_name(&other, &app);
                self.folders.insert(
                    id,
                    Folder {
                        name,
                        apps: vec![other],
                    },
                );
                (id, true)
            }
            _ => return,
        };
        let Some(folder) = self.folders.get_mut(&id) else {
            return;
        };
        folder.apps.push(app);
        let index = folder.apps.len() - 1;
        // The folder takes the place the tile under the app had: the app's.
        self.tiles[self.focus] = TileId::Folder(id);
        self.tiles.remove(over);
        if over < self.focus {
            self.focus -= 1;
        }
        self.update_scroll();
        if let Mode::Moving {
            place,
            made: made_now,
            last_input,
            ..
        } = &mut self.mode
        {
            *place = Place::Folder { id, index };
            *last_input = now;
            if made {
                *made_now = Some(id);
            }
        }
    }

    /// A new folder's first name: the apps' shared category, else the
    /// category of the app it was made on (`target`), else "Folder".
    fn folder_name(&self, target: &str, moved: &str) -> String {
        let category = |id: &str| {
            self.apps
                .iter()
                .find(|app| app.id == id)
                .map_or(Category::Other, |app| app.category)
        };
        let category = match (category(target), category(moved)) {
            (Category::Other, _) => Category::Other,
            (target, _) => target,
        };
        self.folder_names.0[category.index()].clone()
    }

    /// Whether the tile at `index` can take the moving app into a folder:
    /// only an app can go into a folder, and only an app or a folder can
    /// take it (no folders in folders).
    fn can_take(&self, index: usize) -> bool {
        matches!(self.tiles.get(self.focus), Some(TileId::App(_)))
            && matches!(
                self.tiles.get(index),
                Some(TileId::App(_) | TileId::Folder(_))
            )
    }

    /// Up and Down pick an item, OK runs it (the release of the long press
    /// that opened the menu is ignored), Back closes the menu.
    fn handle_menu(&mut self, input: Input, now: Duration) -> Option<Effect> {
        let items = menu_items(self.current().as_ref());
        let Mode::Menu { selected, folder } = &mut self.mode else {
            return None;
        };
        let folder = *folder;
        match input {
            Input::Up => *selected = selected.saturating_sub(1),
            Input::Down => *selected = (*selected + 1).min(items.len().saturating_sub(1)),
            Input::OkPressed => match items.get(*selected) {
                Some(MenuItem::Move) => self.start_move(now),
                Some(MenuItem::Hide) => return self.hide_current(),
                None => self.mode = folder.map_or(Mode::Browse, Mode::Folder),
            },
            Input::Back => self.mode = folder.map_or(Mode::Browse, Mode::Folder),
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

    /// The tile OK and the menu act on: the focused app of an open folder,
    /// else the focused tile.
    fn current(&self) -> Option<TileId> {
        match self.open_folder() {
            Some(OpenFolder {
                id,
                focus: Some(index),
            }) => self
                .folders
                .get(&id)
                .and_then(|folder| folder.apps.get(index))
                .cloned()
                .map(TileId::App),
            Some(_) => None,
            None => self.focused().cloned(),
        }
    }

    /// Hides the app the menu is for; the next tile (or app in the folder)
    /// takes its place and focus. A folder whose last app this was goes.
    fn hide_current(&mut self) -> Option<Effect> {
        let open = self.open_folder();
        let Some(TileId::App(id)) = self.current() else {
            self.mode = Mode::Browse;
            return None;
        };
        self.mode = open.map_or(Mode::Browse, Mode::Folder);
        self.hidden.insert(id);
        self.refresh();
        Some(Effect::SaveOrder)
    }

    /// How many tiles from the start can move: all but [`TileId::Hidden`].
    fn movable(&self) -> usize {
        self.tiles.len() - usize::from(self.tiles.last() == Some(&TileId::Hidden))
    }

    /// Ends move mode, keeping the app where it is: in an open folder, the
    /// folder stays open.
    fn drop_tile(&mut self) -> Option<Effect> {
        let Mode::Moving { before, place, .. } = std::mem::replace(&mut self.mode, Mode::Browse)
        else {
            return None;
        };
        if let Place::Folder { id, index } = place {
            self.mode = Mode::Folder(OpenFolder {
                id,
                focus: Some(index),
            });
        }
        let movable = &self.tiles[..self.movable()];
        let folders_changed = self.folders != before.folders;
        if movable == self.order.ids() && !folders_changed {
            return None;
        }
        if folders_changed {
            self.order.renumber(movable);
        } else {
            self.order.place(movable, self.focus);
        }
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
        let folder = self.open_folder();
        match self.current()? {
            TileId::App(component) => {
                self.mode = Mode::Launching {
                    app: component.clone(),
                    since: now,
                    folder,
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
            TileId::Folder(id) => {
                self.mode = Mode::Folder(OpenFolder { id, focus: Some(0) });
                None
            }
        }
    }

    fn open_menu(&mut self) {
        if !menu_items(self.current().as_ref()).is_empty() {
            self.mode = Mode::Menu {
                selected: 0,
                folder: self.open_folder(),
            };
        }
    }

    fn start_move(&mut self, now: Duration) {
        let folder = self.open_folder();
        let place = match folder {
            Some(OpenFolder {
                id,
                focus: Some(index),
            }) => Place::Folder { id, index },
            Some(_) => return,
            None if self.focus < self.movable() => Place::Home { over: None },
            None => return,
        };
        let Some(tile) = self.current() else {
            return;
        };
        self.mode = Mode::Moving {
            before: Box::new(Before {
                tile,
                tiles: self.tiles.clone(),
                folders: self.folders.clone(),
                folder: folder.map(|open| open.id),
            }),
            last_input: now,
            place,
            made: None,
        };
    }

    fn cancel_move(&mut self) {
        let Mode::Moving { before, .. } = std::mem::replace(&mut self.mode, Mode::Browse) else {
            return;
        };
        let Before {
            tile,
            tiles,
            folders,
            folder,
        } = *before;
        self.tiles = tiles;
        self.folders = folders;
        // Focus goes back to where the app (or its folder) was.
        match folder {
            Some(id) => {
                self.focus_on(&TileId::Folder(id));
                let index = self.folders.get(&id).and_then(|folder| {
                    folder
                        .apps
                        .iter()
                        .position(|app| TileId::App(app.clone()) == tile)
                });
                self.mode = Mode::Folder(OpenFolder {
                    id,
                    focus: Some(index.unwrap_or(0)),
                });
            }
            None => {
                if !self.focus_on(&tile) {
                    self.focus = 0;
                }
            }
        }
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
    neighbour_in(index, len, COLUMNS, direction)
}

/// The app reached from `index` in `direction` in an open folder of `len`
/// apps: pages of [`FOLDER_PAGE`] side by side, Left and Right cross to the
/// page beside, in the same row. No wrap-around.
fn folder_neighbour(index: usize, len: usize, direction: Direction) -> Option<usize> {
    if index >= len {
        return None;
    }
    let page = index / FOLDER_PAGE;
    let start = page * FOLDER_PAGE;
    let at = index - start;
    let (row, column) = (at / FOLDER_COLUMNS, at % FOLDER_COLUMNS);
    match direction {
        Direction::Up | Direction::Down => {
            let on_page = (len - start).min(FOLDER_PAGE);
            neighbour_in(at, on_page, FOLDER_COLUMNS, direction).map(|target| start + target)
        }
        Direction::Left if column > 0 => Some(index - 1),
        // Pages before the last one are full.
        Direction::Left => page
            .checked_sub(1)
            .map(|before| before * FOLDER_PAGE + row * FOLDER_COLUMNS + FOLDER_COLUMNS - 1),
        Direction::Right if column + 1 < FOLDER_COLUMNS && index + 1 < len => Some(index + 1),
        Direction::Right => {
            let next = start + FOLDER_PAGE;
            (next < len).then(|| (next + row * FOLDER_COLUMNS).min(len - 1))
        }
    }
}

/// [`neighbour`] in a grid `columns` wide.
fn neighbour_in(index: usize, len: usize, columns: usize, direction: Direction) -> Option<usize> {
    if index >= len {
        return None;
    }
    let column = index % columns;
    match direction {
        Direction::Left => (column > 0).then(|| index - 1),
        Direction::Right => (column + 1 < columns && index + 1 < len).then(|| index + 1),
        Direction::Up => index.checked_sub(columns),
        Direction::Down => {
            let below = index + columns;
            if below < len {
                Some(below)
            } else if index / columns < (len - 1) / columns {
                Some(len - 1)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

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
            category: Category::Other,
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
        let mut home = Home::new(Order::default(), BTreeMap::new(), BTreeSet::new(), false);
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
        let mut home = Home::new(Order::default(), BTreeMap::new(), BTreeSet::new(), true);
        home.set_apps(&apps(5));
        press(&mut home, &[Input::Left]);
        assert_eq!(home.focus(), 1);
        press(&mut home, &[Input::Right]);
        assert_eq!(home.focus(), 0);
    }

    #[test]
    fn an_empty_home_ignores_every_input() {
        let mut home = Home::new(Order::default(), BTreeMap::new(), BTreeSet::new(), false);
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
        let mut home = Home::new(order, BTreeMap::new(), BTreeSet::new(), false);
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
        order.reconcile(
            &[app("c", "cinema"), app("a", "Bravo"), app("b", "alpha")],
            &BTreeSet::new(),
        );
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
        assert!(order.reconcile(&[app("b", "B"), app("a", "A")], &BTreeSet::new()));
        assert_eq!(
            order.ids(),
            [TileId::About, id("b"), id("a"), TileId::Settings]
        );

        assert!(order.reconcile(&[app("a", "A")], &BTreeSet::new()));
        assert_eq!(order.ids(), [TileId::About, id("a"), TileId::Settings]);
    }

    #[test]
    fn new_apps_go_before_about_when_it_is_last() {
        let mut order = Order::from_keys([(id("a"), 1000), (TileId::About, 2000)]);
        assert!(order.reconcile(
            &[app("a", "A"), app("c", "C"), app("b", "B")],
            &BTreeSet::new()
        ));
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
        order.reconcile(&[app("a", "A"), app("a", "A")], &BTreeSet::new());
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
        let state = home.state();
        home.handle(Input::OkPressed, ms(0));
        home.handle(Input::OkPressed, ms(300)); // key repeat
        assert_eq!(home.handle(Input::Tick, LONG_PRESS - ms(1)), None);
        assert_eq!(home.state(), state);
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
        // a00, About, Settings: on top of About, no folder forms.
        let mut home = home(1);
        pick_up(&mut home);
        let last_key = ms(1000);
        home.handle(Input::Right, last_key);
        assert_eq!(home.folder_due(), None);
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
        assert!(home.folders().is_empty());
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
        let mut home = Home::new(Order::default(), BTreeMap::new(), hidden, false);
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

    // --- folders ----------------------------------------------------------

    fn app_in(id: &str, category: Category) -> App {
        App {
            category,
            ..app(id, &id.to_uppercase())
        }
    }

    /// Picks up the focused app and leaves it on the tile in `direction`
    /// until the folder forms; returns the time after.
    fn make_folder(home: &mut Home, direction: Input) -> Duration {
        pick_up(home);
        home.handle(direction, ms(1000));
        assert!(home.moving_over().is_some());
        assert_eq!(home.folder_due(), Some(ms(1000) + FOLDER_HOVER));
        assert_eq!(
            home.handle(Input::Tick, ms(1000) + FOLDER_HOVER - ms(1)),
            None
        );
        assert!(home.open_folder().is_none());
        assert_eq!(home.handle(Input::Tick, ms(1000) + FOLDER_HOVER), None);
        ms(1000) + FOLDER_HOVER
    }

    fn folder_apps(home: &Home, id: u32) -> Vec<&str> {
        home.folders()[&id]
            .apps
            .iter()
            .map(String::as_str)
            .collect()
    }

    #[test]
    fn staying_on_an_app_makes_a_folder_that_opens_with_the_app_still_moving() {
        let mut home = home(4); // a00 a01 a02 a03 About Settings
        let now = make_folder(&mut home, Input::Right);
        // The folder took a01's place; the app is at its end, still moving.
        assert_eq!(home.state(), State::Moving);
        assert_eq!(
            home.tiles(),
            [
                TileId::Folder(1),
                id("a02"),
                id("a03"),
                TileId::About,
                TileId::Settings
            ]
        );
        assert_eq!(home.focus(), 0);
        assert_eq!(folder_apps(&home, 1), ["a01", "a00"]);
        assert_eq!(
            home.open_folder(),
            Some(OpenFolder {
                id: 1,
                focus: Some(1)
            })
        );
        // Nothing is saved until the drop; then the folder stays open.
        assert_eq!(home.order().ids()[0], id("a00"));
        assert_eq!(
            home.handle(Input::OkPressed, now + ms(100)),
            Some(Effect::SaveOrder)
        );
        assert_eq!(home.state(), State::Folder);
        assert_eq!(home.order().ids()[0], TileId::Folder(1));
        assert_eq!(home.handle(Input::Back, now + ms(200)), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.focused(), Some(&TileId::Folder(1)));
    }

    #[test]
    fn an_arrow_before_the_hover_time_passes_over_and_ok_drops_in_place() {
        let mut home = home(4);
        pick_up(&mut home);
        home.handle(Input::Right, ms(1000));
        home.handle(Input::Tick, ms(1000) + FOLDER_HOVER - ms(1));
        home.handle(Input::Right, ms(1000) + FOLDER_HOVER - ms(1));
        // Now on top of a02; a01 went back into the gap.
        assert_eq!(home.moving_over(), Some(1));
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert!(home.folders().is_empty());
        assert_eq!(home.order().ids()[..3], [id("a01"), id("a02"), id("a00")]);
    }

    #[test]
    fn about_settings_and_folders_never_go_into_a_folder() {
        // a00 About Settings: About is not an app.
        let mut home = home(1);
        pick_up(&mut home);
        home.handle(Input::Right, ms(1000));
        assert_eq!(home.folder_due(), None);
        home.handle(Input::Tick, ms(5000));
        assert!(home.folders().is_empty());
        press(&mut home, &[Input::Back]);

        // A folder moved onto an app makes nothing: no folders in folders.
        let mut home = home_with_folder();
        press(&mut home, &[Input::Back, Input::Back]);
        assert_eq!(home.focused(), Some(&TileId::Folder(1)));
        pick_up(&mut home);
        home.handle(Input::Right, ms(1000));
        assert_eq!(home.folder_due(), None);
    }

    /// Home with folder 1 = [a01, a00], dropped and still open.
    fn home_with_folder() -> Home {
        let mut home = home(4);
        let now = make_folder(&mut home, Input::Right);
        home.handle(Input::OkPressed, now);
        home.handle(Input::OkReleased, now);
        home
    }

    #[test]
    fn an_app_left_on_a_folder_goes_in_at_its_end() {
        let mut home = home_with_folder();
        press(&mut home, &[Input::Back]);
        press(&mut home, &[Input::Right]); // a02
        pick_up(&mut home);
        home.handle(Input::Left, ms(1000));
        home.handle(Input::Tick, ms(1000) + FOLDER_HOVER);
        assert_eq!(folder_apps(&home, 1), ["a01", "a00", "a02"]);
        assert_eq!(home.folders().len(), 1);
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert_eq!(
            home.order().ids(),
            [
                TileId::Folder(1),
                id("a03"),
                TileId::About,
                TileId::Settings
            ]
        );
    }

    #[test]
    fn leaving_a_new_folder_before_the_drop_undoes_it() {
        let mut home = home(4);
        let now = make_folder(&mut home, Input::Right);
        // In the folder at index 1; Right is its edge.
        home.handle(Input::Right, now + ms(100));
        assert!(home.folders().is_empty());
        assert!(home.open_folder().is_none());
        assert_eq!(home.tiles()[..2], [id("a01"), id("a00")]);
        assert_eq!(component(&home), "a00");
        assert_eq!(home.state(), State::Moving);
    }

    #[test]
    fn moving_an_app_out_of_a_folder_keeps_the_folder_until_it_is_empty() {
        let mut home = home_with_folder();
        // Open on a00 (index 1); pick it up in the folder.
        pick_up(&mut home);
        assert_eq!(
            home.open_folder(),
            Some(OpenFolder {
                id: 1,
                focus: Some(1)
            })
        );
        press(&mut home, &[Input::Left]);
        assert_eq!(folder_apps(&home, 1), ["a00", "a01"]);
        // Up from the first row leaves the folder, just after its tile.
        press(&mut home, &[Input::Up]);
        assert!(home.open_folder().is_none());
        assert_eq!(home.tiles()[..2], [TileId::Folder(1), id("a00")]);
        assert_eq!(folder_apps(&home, 1), ["a01"]);
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        press(&mut home, &[Input::OkReleased]);

        // The last app out: the folder goes.
        press(
            &mut home,
            &[Input::Left, Input::OkPressed, Input::OkReleased],
        );
        assert_eq!(home.state(), State::Folder);
        pick_up(&mut home);
        press(&mut home, &[Input::Down]);
        assert!(home.folders().is_empty());
        assert_eq!(home.tiles()[..2], [id("a01"), id("a00")]);
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::SaveOrder)
        );
        assert!(!home.order().ids().contains(&TileId::Folder(1)));
    }

    #[test]
    fn back_cancels_a_move_that_made_a_folder() {
        let mut home = home(4);
        let before = home.tiles().to_vec();
        let now = make_folder(&mut home, Input::Right);
        assert_eq!(home.handle(Input::Back, now), None);
        assert_eq!(home.state(), State::Browse);
        assert_eq!(home.tiles(), before);
        assert!(home.folders().is_empty());
        assert_eq!(component(&home), "a00");
    }

    #[test]
    fn in_an_open_folder_up_reaches_the_name_and_ok_renames() {
        let mut home = home_with_folder();
        press(&mut home, &[Input::Left]);
        assert_eq!(home.open_folder().and_then(|open| open.focus), Some(0));
        press(&mut home, &[Input::Up]);
        assert_eq!(home.open_folder().map(|open| open.focus), Some(None));
        assert_eq!(
            press(&mut home, &[Input::OkPressed]),
            Some(Effect::RenameFolder(1))
        );
        assert!(home.rename_folder(1, "  Films \u{7}& TV  "));
        assert_eq!(home.folders()[&1].name, "Films & TV");
        assert!(!home.rename_folder(1, "   "));
        assert!(!home.rename_folder(1, "Films & TV"));
        assert!(home.rename_folder(1, &"x".repeat(60)));
        assert_eq!(home.folders()[&1].name.chars().count(), FOLDER_NAME_MAX);
        press(&mut home, &[Input::OkReleased, Input::Down]);
        assert_eq!(home.open_folder().and_then(|open| open.focus), Some(0));
    }

    #[test]
    fn ok_in_a_folder_launches_and_the_folder_is_open_again_on_return() {
        let mut home = home_with_folder();
        assert_eq!(
            press(&mut home, &[Input::OkPressed, Input::OkReleased]),
            Some(Effect::Launch("a00".to_owned()))
        );
        press(&mut home, &[Input::Paused]);
        assert_eq!(home.state(), State::Folder);
        // Home while the folder is open closes it.
        press(&mut home, &[Input::Paused]);
        assert_eq!(home.state(), State::Browse);
    }

    #[test]
    fn hiding_the_last_app_of_a_folder_removes_the_folder() {
        let mut home = home_with_folder();
        assert_eq!(hide(&mut home), Some(Effect::SaveOrder));
        assert_eq!(home.state(), State::Folder);
        assert_eq!(folder_apps(&home, 1), ["a01"]);
        assert_eq!(hide(&mut home), Some(Effect::SaveOrder));
        assert_eq!(home.state(), State::Browse);
        assert!(home.folders().is_empty());
        assert!(!home.tiles().contains(&TileId::Folder(1)));
        // Shown again, they come back on Home, not in the folder.
        assert_eq!(hidden_page(&home).len(), 2);
    }

    #[test]
    fn uninstalled_apps_leave_their_folder_and_an_empty_folder_goes() {
        let folders = BTreeMap::from([(
            7,
            Folder {
                name: "Old".to_owned(),
                apps: vec!["a00".to_owned(), "gone".to_owned()],
            },
        )]);
        let order = Order::from_keys([(TileId::Folder(7), 1000), (id("a01"), 2000)]);
        let mut home = Home::new(order, folders, BTreeSet::new(), false);
        assert!(home.set_apps(&apps(2)));
        assert_eq!(home.folders()[&7].apps, ["a00"]);
        assert_eq!(home.tiles()[..2], [TileId::Folder(7), id("a01")]);
        // a00 is not also on Home outside its folder.
        assert_eq!(home.tiles().len(), 4);
        home.set_apps(&apps(2)[1..]);
        assert!(home.folders().is_empty());
        assert_eq!(home.tiles(), [id("a01"), TileId::About, TileId::Settings]);
    }

    #[test]
    fn a_folder_missing_from_the_order_is_added_and_an_unknown_one_dropped() {
        let folders = BTreeMap::from([(
            2,
            Folder {
                name: "F".to_owned(),
                apps: vec!["a01".to_owned()],
            },
        )]);
        let order = Order::from_keys([(TileId::Folder(9), 500), (id("a00"), 1000)]);
        let mut home = Home::new(order, folders, BTreeSet::new(), false);
        home.set_apps(&apps(2));
        assert_eq!(
            home.tiles(),
            [
                id("a00"),
                TileId::Folder(2),
                TileId::About,
                TileId::Settings
            ]
        );
    }

    #[test]
    fn a_new_folder_is_named_after_the_apps_category() {
        let names = FolderNames::default();
        for (target, moved, expected) in [
            (Category::Game, Category::Game, "Games"),
            (Category::Video, Category::Audio, "Video"),
            (Category::Other, Category::Audio, "Folder"),
            (Category::Other, Category::Other, "Folder"),
        ] {
            let mut home = Home::new(Order::default(), BTreeMap::new(), BTreeSet::new(), false);
            home.set_folder_names(names.clone());
            home.set_apps(&[app_in("a", moved), app_in("b", target)]);
            pick_up(&mut home);
            home.handle(Input::Right, ms(1000));
            home.handle(Input::Tick, ms(1000) + FOLDER_HOVER);
            let folder = home.folders().values().next().expect("a folder");
            assert_eq!(folder.name, expected, "{target:?} + {moved:?}");
        }
        assert_eq!(Category::from_android(0), Category::Game);
        assert_eq!(Category::from_android(8), Category::Accessibility);
        assert_eq!(Category::from_android(-1), Category::Other);
        assert_eq!(Category::from_android(42), Category::Other);
    }

    #[test]
    fn random_keys_never_lose_or_duplicate_an_app() {
        // Every app stays exactly once on Home, in a folder or hidden, whatever is pressed.
        let keys = [
            Input::Up,
            Input::Down,
            Input::Left,
            Input::Right,
            Input::OkPressed,
            Input::OkReleased,
            Input::Back,
            Input::Tick,
        ];
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for _ in 0..200 {
            let mut home = home(9);
            let mut now = Duration::ZERO;
            let mut pressed = Vec::new();
            for _ in 0..60 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let input = keys[usize::try_from(seed % 8).unwrap()];
                now += ms(seed % 900);
                pressed.push((input, now.as_millis()));
                home.handle(input, now);
                let mut all: Vec<&str> = home
                    .tiles()
                    .iter()
                    .filter_map(|tile| match tile {
                        TileId::App(id) => Some(id.as_str()),
                        _ => None,
                    })
                    .chain(
                        home.folders()
                            .values()
                            .flat_map(|f| f.apps.iter().map(String::as_str)),
                    )
                    .chain(home.hidden_apps().iter().map(String::as_str))
                    .collect();
                all.sort_unstable();
                let expected: Vec<String> = (0..9).map(|n| format!("a{n:02}")).collect();
                assert_eq!(all, expected, "after {pressed:?}");
                assert!(home.folders().values().all(|f| !f.apps.is_empty()));
                for folder in home.folders().keys() {
                    assert_eq!(
                        home.tiles()
                            .iter()
                            .filter(|t| **t == TileId::Folder(*folder))
                            .count(),
                        1
                    );
                }
            }
        }
    }

    #[test]
    fn a_big_folder_has_pages_of_twelve_side_by_side() {
        let len = 27; // pages 0..12, 12..24, 24..27
        assert_eq!(folder_neighbour(3, len, Direction::Right), Some(12));
        assert_eq!(folder_neighbour(7, len, Direction::Right), Some(16));
        assert_eq!(folder_neighbour(12, len, Direction::Left), Some(3));
        assert_eq!(folder_neighbour(21, len, Direction::Left), Some(20));
        assert_eq!(folder_neighbour(20, len, Direction::Left), Some(11));
        assert_eq!(folder_neighbour(8, len, Direction::Down), None);
        assert_eq!(folder_neighbour(12, len, Direction::Up), None);
        // Into the short last page: the nearest app in that row, or the last.
        assert_eq!(folder_neighbour(19, len, Direction::Right), Some(26));
        assert_eq!(folder_neighbour(26, len, Direction::Right), None);
        assert_eq!(folder_neighbour(24, len, Direction::Down), None);
        assert_eq!(folder_neighbour(0, len, Direction::Left), None);
        // Every app is reachable from the first.
        for len in 1..=40 {
            let mut seen = BTreeSet::from([0]);
            let mut queue = vec![0];
            while let Some(index) = queue.pop() {
                for direction in ALL_DIRECTIONS {
                    if let Some(next) = folder_neighbour(index, len, direction) {
                        assert!(next < len);
                        if seen.insert(next) {
                            queue.push(next);
                        }
                    }
                }
            }
            assert_eq!(seen.len(), len, "{len} apps");
        }
    }
}

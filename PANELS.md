# Panels — design

Status (2026-11-20): **being built** on the `panels` branch: steps 1
and 1b (About's tile, its phone caption) and 2 (About's page and row) are
done (§ 10). Written from the owner's answers on 2026-10-25 (§ Owner decisions), checked against the code by an
independent review (§ Review log), and re-checked against main on 2026-11-20
(§ 10 has the build plan). Replaces ROADMAP.md § Parked
ideas → "Panels: add-ons with several faces". The theme builder gets its own
design later; this doc only says how panels sit in a theme.

A **panel** is one RustTV feature (About first, then History, later Clock, …)
that can show itself on several surfaces, called **faces**: a full page, a
tile on Home, a row in a list, and later the status corner, a quick panel,
the phone. "Panels" is the word for people; "add-ons" is the word in code.
"Widgets" is not used: it clashes with Android's App Widgets.

## Owner decisions (2026-10-25)

1. **Name:** Panels.
2. **Faces are possibilities, not duties.** A panel implements only the
   faces it wants. The full list is in § 2.
3. **Tiles are 1×1 only**, the size of an app tile.
4. **First panel: About** (changed later on 2026-10-25, replacing History).
   It already has live data (version, the updater's state) and a page, so it
   is the smallest real panel. History follows.
5. **Placement:** anywhere on the grid, exactly like an app: move, hide,
   put in a folder. *After review:* v1 ships Move only, as About does today;
   hide and folders come with the next state format (§ 6).
6. **Tile look:** the app tile's frame, radius and focus (scale + shadow +
   label), with live content inside instead of a banner. Focus adds nothing
   else.
7. **OK opens the panel's page.** Long press shows the usual tile menu plus
   up to two of the panel's own actions.
8. **Refresh:** no panel-owned timers or threads and no per-frame work
   (*after review*, replacing "push on change only"). Panels update from
   events Home already has; Home re-formats relative times ("2 h ago") on
   resume, focus change and the clock's one tick a minute.
9. **One instance, many faces.** A panel can show as a tile, a row and a
   corner glyph at once, all from one data source. **Never** two copies of
   one panel (no "Weather for Kyiv and for Lviv").
10. **Rendering, one generic contract:** a panel returns typed data. RustTV
    draws every face, the page included, from a small fixed set of shared
    templates and blocks (§ 4). Panels bring no Slint of their own, built-in
    ones too, so a new panel never fights the contract. *After review:*
    About's page moves onto page blocks in v1.
11. **Trust levels:** built-in panels (level 1) are in scope. *After
    review:* data-only panels (level 2) are parked like level 3 until their
    data source (§ 9 Q1) is decided; § 7 keeps their safety rules as notes.
    Code from other people (level 3) stays parked behind a SECURITY.md
    decision.
12. **Actions:** a fixed list (§ 5). Nothing else, ever.
13. **No off switch**, except panels that use the network or share data
    (*after review*): those get one, like "Check for updates automatically",
    and off stops the traffic. Other built-in panels are always there.
14. **Quick panel** opens with a long press of the Home/Back key.
    *Replaced 2026-11-20 by decision 25* (#40's top-right menu).
    *After review:* unverified. Home reaches a launcher as an intent and its
    long press usually belongs to the system; long-press Back works only
    while RustTV is in front. Decided after a read-only key check on the
    Philips (needs the owner's OK).
15. **Phone face** uses the same data, drawn by matching web templates on
    the RustTV Remote page. No panel code runs on the phone.
16. **Placement is History.** Panel tile positions live in the order file
    with the apps, so undo and "copy my setup" apply to them as well.
17. **Performance:** no measurable change. Cold start, resume and frame
    times stay within noise of today's, measured by BENCHMARK_PLAN.md.
18. **First build:** page + tile + row only.
19. **About tile:** RustTV logo and "RustTV <version>". When an update is
    available, the caption becomes "Update available" in the accent, in
    place of today's dot.
20. **Templates are written in JSON** (owner's idea, 2026-10-25; JSON chosen
    over HTML/CSS and TOML). The TV gets Slint generated from them at build
    time; the phone draws the same file (§ 4.1). Whether it is worth it is
    decided by a spike with the pass bar in § 4.1, before any template is
    built.

Added 2026-11-20:

21. **Order of work:** the JSON spike and this plan now; About's tile and
    page are built after the Linux home screen PR (#40) is merged, since
    both change the Home tiles and the About tile.
22. **The spike stays** as step 0, with the pass bar in § 4.1.
23. **Second panel: RustTV Remote** (replacing History in that place). Its
    tile exists since 2026-10-26 and has live data (paired phones, pairing
    open), so it tests the contract on a second real panel before History's
    no-I/O-at-start rule. History comes third.
24. **Remote's place is recorded in History**, accepting format 5 for
    every new snapshot (§ 9 Q9).
25. **One quick panel:** #40's top-right menu. Panels add rows to it;
    decision 14's long-press key is dropped (§ 9 Q10).

## 1. Goals and limits

- **One generic contract** for every feature and every face, so adding a
  panel never means changing the contract.
- **Same app:** panels look and move like the rest of the launcher
  (HISTORY.md § 8 tokens). No new colours, fonts, shadows or effects.
- **Safe by shape:** a panel describes what to show and which fixed actions
  it offers. It never touches the system directly. Every change to the TV
  goes through review → code → hold → journal (SECURITY.md), except a
  launcher update, which keeps its own path (SECURITY.md § Launcher updates:
  Android's install confirmation).
- **No extra work:** a data change touches one model row or one property;
  nothing polls for a tile.

Not in v1: status corner, quick panel, banner, ambient, hide and folders
for panel tiles, toggles and change requests, data-only panels, the theme
builder, History and Settings as panels. Never: two instances of one panel, panels
running other people's code before level 3 is approved.

## 2. Faces

| Face | Where | v1 | Status |
|---|---|---|---|
| **Page** | Full screen: a layout filled with blocks (§ 4) | yes | decided |
| **Tile** | 1×1 on the Home grid (folders later, § 6) | yes | decided |
| **Row** | One line in a list (the Settings list, the quick panel) | yes | decided |
| **Status corner** | Top-right of Home, not focusable | no | starts with a clock; the rest is open |
| **Quick panel** | A row in #40's top-right menu (decision 25) | no | decided; which rows is open |
| **Phone** | RustTV Remote page | partly | the Apps mirror already shows every tile (§ 6) |
| **Banner** | Short message that slides in | no | open (§ 9) |
| **Hero** | Area above the grid | partly | today's hero redraws the focused tile larger (§ 4) |
| **Ambient** | Idle / screensaver | no | open (§ 9) |

A panel lists the faces it implements. RustTV never shows a face that a
panel doesn't list.

## 3. The contract (level 1, built-in)

No trait until a second panel exists. v1 is one pure function per panel
that returns a small value; Home keeps the last value and applies only what
changed. Sketch (names open until built):

```rust
pub(crate) enum PanelId { About }                 // History, Clock later
pub(crate) enum Tone { Normal, Attention }        // Attention = the accent
/// Words are made at display time (Slint `@tr`, later the phone's i18n).
pub(crate) enum Line { Version(String), UpdateAvailable, Downloading, Installing }
pub(crate) struct PanelTile { value: Line, caption: Option<Line>, tone: Tone }
pub(crate) struct PanelRow { value: Option<Line>, tone: Tone }
pub(crate) struct PanelPage { layout: Layout, columns: Vec<Vec<Block>> }
/// Which faces a panel has = which fields are `Some`.
pub(crate) struct PanelView {
    tile: Option<PanelTile>,
    row: Option<PanelRow>,
    page: Option<PanelPage>,
}
pub(crate) enum PanelAction { OpenPage, UpdateLauncher }

/// About in v1, tested without Slint.
pub(crate) fn about_view(version: &str, status: &UpdateStatus) -> PanelView;
/// Built once when the page opens, not on every update poll.
```

- **No timers or threads of its own.** A panel updates from events Home
  already has: resume, the updater's existing poll while the About page is
  open or a step runs, and in #40 also while Settings is open (`UPDATE_POLL`, `ui.rs`), replies from the history
  thread. It never polls for a tile.
- **Applying a change:** one `set_row_data` on the tile's model row, or one
  window property, as today's update dot does. A data change never rebuilds
  the tile list (`render()` rebuilds only when tile ids change, and a
  rebuild requeues all artwork).
- **Can't fail on Home.** Building a view can't panic (`panic = "abort"`
  would kill Home). If data is missing, the tile shows the panel's name and
  icon only; any reason appears on its page.
- **Text:** views carry kinds and values, never translated strings. A
  version inside RTL text is an isolated LTR run. Tile text is checked
  elided in all 21 languages and in RTL.
- **Pointer:** mouse and air-mouse input (since #39) reach a panel's faces
  only through the shared UI's `pointer_reaches` guard (`ui.rs`), like every
  other tile and page: no pointer input behind pairing, trust or review.
- **Reduced effects:** templates draw shadows and backdrops only through
  the shared tile and page components, so the Effects setting (#38) turns
  them off for panels as for apps.

## 4. Templates

Every face is drawn from a small, fixed set. Each item has one Slint
component and, later, one web component for the phone face.

| View | Used by | Content |
|---|---|---|
| `PanelTile` | tile | icon, value (one short line), caption (dim, or accent for `Attention`) |
| `PanelRow` | row | label, value |
| `PanelPage` | page | a `Layout` (one column, or two: a narrow left and the 1110u surface panel on the right, as About and Settings are today; mirrored in RTL) filled with blocks |

**Page blocks (v1, what About needs):** one Slint component each in
`ui/blocks.slint` (`PageTitle`, `PageText`, …), which holds their sizes;
colours come from `Theme`. The JSON key and fields are in § 4.1.

| Block | Draws | Focus |
|---|---|---|
| `Title` | a title: `large` 76u (a page of its own, About), `medium` 57u (list pages), `small` 29u (a heading in the surface panel) | no |
| `Text` | one paragraph; normal (29u) or small (25u); dim or normal; wraps | no |
| `Status` | one line; accent for `Attention` (the updater's line); wraps | no |
| `Badge` | a built-in mark from a fixed list (`slint`: Slint's `AboutSlint`, at a fixed 240u so the attribution can't be shrunk) | no |
| `ScrollBox` | long text Home loads (a fixed list: `notices`) filling the surface panel's height; Up/Down scroll; at most one per page | yes |
| `Hint` | the dim key hint ("Up and Down scroll the notices. Back returns."); empty when `Hints.keys` is off | no |

Layouts: `one-column` (blocks in one 1740u column) or `two-columns` (a
560u start column and the 1110u surface panel at the end side, as About is
today); both mirrored in RTL.

Later blocks, when a panel needs them: `Rows` (a focusable list, for
Settings and History), toggle and chevron rows. Badges stay a fixed list:
nothing decodes an image from a panel.

Focus and keys on a page are Home's, not the panel's: Up/Down move between
focusable blocks or scroll a `ScrollBox`; Back leaves; OK runs the page's
one action if it has one (About: Update RustTV, § 5).

- **Hero:** the hero (`HeroLayer`, `home.slint`) redraws the focused tile's
  artwork larger. For a panel tile it draws the same `PanelTile` at hero
  size. A panel tile's glow is a fixed `Theme` colour, since it has no
  banner bitmap.

New templates are added only when a real panel needs one. Templates take
their sizes and colours from `Theme`, so a later theme changes every panel
at once.

### 4.1 Template source: JSON

Each template (tile, row, page blocks) is written once as a JSON file. At
build time a small converter in the launcher's `build.rs` turns it into a
`.slint` component; the phone page loads the same file and draws it with a
small JS renderer. One source, two surfaces, nothing to keep in sync by hand.

**Why JSON** (compared on 2026-10-25):

| | HTML/CSS | JSON | TOML |
|---|---|---|---|
| Preview | any browser | the phone renderer in a desktop browser | same as JSON |
| On the phone as-is | yes | yes, with a small JS renderer | needs a conversion |
| Parsing in Rust | strict subset; new crate or hand parser | `serde_json`, already in the workspace | `toml`, already in the workspace |
| Can say what Slint can't | yes (CSS is huge) | no: the schema holds only what we support | no |
| Data-only panels later | people expect full HTML | the same file loads at runtime | same |
| Nested layouts | natural | natural | awkward |

**Schema (sketch, About's tile):**

```json
{ "component": "AboutTile",
  "props": ["version", "status"],
  "root": { "column": {
    "width": 250, "height": 141, "radius": 12, "background": "surface",
    "padding": 16, "gap": 4, "align": "end",
    "children": [
      { "image": { "badge": "rusttv-logo", "height": 44 } },
      { "text": { "value": { "tr": "RustTV {}", "args": ["version"] },
                  "size": 27, "weight": 500, "color": "text" } },
      { "text": { "show": "status",
                  "value": { "select": "status", "cases": {
                    "update": { "tr": "Update available" },
                    "downloading": { "tr": "Downloading" },
                    "installing": { "tr": "Installing" } } },
                  "size": 23,
                  "color": { "select": "status",
                             "cases": { "update": "accent" }, "else": "text-dim" } } } ] } } }
```

Rules the schema enforces (unknown fields are errors):
- **Faces:** `"face"` is `tile`, `row` or `page` and fixes the root:
  - `tile`: a `column` (above), drawn as a component.
  - `row`: a `list-row` with `title`, optional `detail` (texts) and
    optional `attention` (a condition: the accent dot). Settings draws
    every row of its list with one component (`ControlRow`), so a row
    becomes a Slint global of pure functions (`title`, `detail`,
    `attention`, each taking the `props` in order) that Rust calls to fill
    the list.
  - `page`: a `page` with `layout` (`one-column` or `two-columns`),
    `start` and, for two columns, `end`: arrays of the § 4 blocks. Keys:
    `title` (`value`, `size`: `large`/`medium`/`small`), `text` (`value`,
    `size`: `normal`/`small`, `tone`: `normal`/`dim`), `status` (`value`,
    `attention`), `hint` (`value`), `badge` (`name`), `scroll-box`
    (`source`). `title`, `text`, `status` and `badge` take an optional
    `show`. A `scroll-box` goes in `end` only, once; its page gets
    `scroll-text`, `scroll-position` and `scroll-fraction` properties that
    Home fills (it owns the scrolling), so those names are reserved.
- **Conditions** (`show`, `attention`): a property's name (true when not
  empty) or a `select` of `true`/`false` (else `false`).
- **Nodes:** `column` (later `row`), `text`, `image` in a tile. Only the
  root sets size, radius and background.
- **Sizes** are numbers in `u` (the launcher's unit). The converter writes
  `N * u`; the phone page uses the same scale.
- **Colours** are `Theme` token names only (`bg`, `surface`, `text`,
  `text-dim`, `accent`) → `Theme.<token>` in Slint and `var(--<token>)` on
  the phone. A later theme changes both.
- **Images** are built-in badge names only; nothing decodes a panel's image.
- **Text** is `@tr` with a literal string and property arguments, or a
  choice between such strings by a property's value. So every string stays
  translatable: `slint-tr-extractor` runs on the generated files, and the
  phone page's i18n uses the same message ids.
- **Direction:** text aligns to `Dir.start`, images to the start side;
  mirrored in RTL on both surfaces. No `left`/`right` in templates.
- **Properties** are strings declared in `props`; values come from the
  panel's view (§ 3). Templates have no expressions beyond `show`,
  `attention` and `select`.

**Pass bar for the spike** (owner, 2026-10-25), all four must hold:
1. The generated tile looks identical to a hand-written Slint tile:
   pixel-compared with Slint's software renderer, LTR and RTL, every
   update state.
2. The same JSON looks right on the phone page.
3. `@tr` and `Theme` tokens work through the converter.
4. The converter stays small: under ~500 lines, readable, clear errors for
   anything unsupported.

If the spike fails, templates are hand-written in Slint and in the phone
page's JS, kept in sync by screenshot tests.

**Spike result (2026-11-20): passed, keep JSON.** Evidence and the full
findings are in `spikes/panel-templates/README.md`:

1. 16 of 16 software-renderer pictures are byte-identical to a
   hand-written tile: every update state, LTR and RTL, English and
   Ukrainian.
2. The phone page matches by eye.
3. `@tr` and `Theme` work through the converter.
4. The converter is 428 lines, with errors that name the JSON path.

What step 1 must add because of it:
- **Theme:** `Theme` (exported) and `Dir` (with an `image-start`) move to
  their own `theme.slint`. `home.slint`, `quick.slint` (its `Colours` and
  `start`) and generated files import it.
- **Messages:**
  - Template msgids must reach the `.pot`. Generated files sit in
    `OUT_DIR`, where `slint-tr-extractor` doesn't look.
  - Keep `DefaultTranslationContext::None`.
- **Phone i18n:** `page/i18n.js` uses its own keys, not msgids, so the
  phone needs a msgid → text table built from the launcher's `.po` files.
  Placeholders other than `{}` are refused there, or supported on both
  sides.
- **One set of bad-template tests** for `convert.rs` and the JS renderer.
  Duplicate JSON keys are refused.
- **Tokens and elision:** the phone's `text-dim` is `--dim`, mapped by the
  renderer. On both surfaces, text aligns to the UI's start side, and the
  text's own script decides where "…" goes.
- **Fit:** the Ukrainian "Update available" doesn't fit the caption at 23u,
  so the design pass checks every language.

**About's faces (v1):**
- **Tile:** the RustTV logo and "RustTV <version>". The caption follows the
  updater's state (`UpdateStatus`, `ui.rs`):
  - Available, NeedsPermission → "Update available" (accent);
  - Downloading → "Downloading" (no percent, so the 500 ms poll doesn't
    redraw Home);
  - Installing → "Installing";
  - anything else, including Failed → no caption (the error shows on the
    page only).
  This replaces today's dot.
- **Row:** "About RustTV", with the version as its value, or "Update
  available". It opens the page. It lives in Settings' About section, which
  PR #40 adds (`AboutRow`, `settings.rs`).
- **Page:** today's About page re-drawn from blocks, looking the same: two
  columns; left `Title` "RustTV", `Text` "Version … · MIT OR Apache-2.0",
  `Status` (the updater's line), `Text` (licences), `Text` (source),
  `Badge` (`AboutSlint`), `Hint`; right `ScrollBox` (the licence notices).
  OK runs Update RustTV, as today. Check it by eye against a screenshot of
  today's page in the desktop build.

**History's faces (later):**
- **Tile:** the History icon, "Last change", and "2 h ago" as the caption.
  The caption uses the accent when something waits for approval. Exact
  wording is open.
- **Row:** "History", with the last change's age as its value. It opens the
  page.
- **Page:** the Settings page opened on its History section (History is a
  Settings section, not a page of its own).
- **Data:** HISTORY.md § 9 forbids history I/O at start and resume, so the
  tile needs a cheap "last entry time + pending count" from the history
  thread. Until it arrives (the first Settings open or a change) the tile
  shows its plain state. "2 h ago" is formatted by Home on resume and on
  focus changes, not by a timer.

## 5. Actions

A face offers only these actions:

1. **Open page:** the panel's page.
2. **Update RustTV:** check for or install a launcher update, as OK on the
   About page does today (`Effect::Update`). Only About offers it; Android's
   installer confirms on the TV.
3. **Toggle** (later): *ask* to flip one of the panel's own settings. A
   setting recorded in History (e.g. "Check for updates automatically")
   opens review + code + hold like the Settings page. A toggle never writes
   directly.
4. **Request a change** (later): a typed change from the existing set,
   through review + code + hold.

From the phone, only Open page is accepted; the rest are dropped, as phone
input is already dropped on the review screen (SECURITY.md § RustTV
Remote). Long press on a panel tile shows the tile's usual menu (Move today;
§ 6), with up to two panel actions above it once a panel has some. A panel
can't launch apps, open links, reach the network, or talk to the phone on
its own.

## 6. Home, the order file and the phone

- **v1: About moves like today** (Move only, `menu_items`, `model.rs`). It
  keeps its id `about` in the order file and the snapshot, so v1 needs no
  format change.
- **State formats (2026-11-20):** main is at 4 (folders 2, phones 3,
  reduced effects 4; `needed_format`, `state.rs`). PR #40 adds format 5
  (`remote` and `.desktop` tile names, `needs_format_five`) and makes
  `remote` a valid tile name. Since #40 merges first (decision 21), "the
  next format" below means 6.
- **The Remote tile is not in History.** `launcher_state` (`order_file.rs`)
  leaves it out of the snapshot, on main and in #40, so a moved Remote tile
  goes back to the end on undo or "copy my setup". Recording it means every
  snapshot needs format 5 (every Home has a Remote tile), so builds older
  than #40 could no longer read any new history. Open: § 9 Q9.
- **About and Settings tiles after #40:** where a top-right menu has
  Settings (Linux), `reconcile` drops the About and Settings tiles
  (`settings_tiles`), and Settings gains an About section (`Section::About`,
  `AboutRow`). So About's tile face exists only where those tiles do
  (Android), and About's **row** face is that `AboutRow` section.
- **Hide and folders for panel tiles come later, with the next format**
  (decision 5). Today an app's menu is Move + Hide, folders are made by holding a
  moving tile over another, and a built-in tile can do neither. Making it
  possible means:
  - `needed_format` returns the next format when a built-in or panel tile is in `hidden`
    or in a folder. The id is already valid there (`is_tile_name`), so an
    older build would read such a snapshot without refusing it and get it
    wrong;
  - the Hidden apps page lists hidden panels (today it lists installed apps
    only);
  - `reconcile` stops re-adding a built-in tile that is inside a folder;
  - folder minis get a picture for panel tiles;
  - the licence notices and Slint's attribution stay reachable when About is
    hidden (e.g. from its Settings row). Check the Slint licence's
    attribution clause first.
- **A new panel tile** (e.g. History) is `TileId::Panel(PanelId)`, written
  `panel:<id>` in the order file and in `state.toml`. Ids have no `/`, so
  they can't clash with apps. It ships with the next format. Today's
  built-in names (`about`, `settings`, `remote`) stay as they are; only
  new panels use `panel:<id>`.
  - An older launcher reads an unknown name as an app and drops it as not
    installed, so it forgets the panel's place; after an upgrade the tile
    comes back at its first-run spot.
  - `reconcile`'s built-in tail knows About, Settings and Remote; the
    first-run spot of a new panel is open (§ 9 Q7).
  - CLI JSON: main is at `rusttv.v11`. A new `panel:<id>` value in `tile` is
    documented in CLI.md under its string rules; a bump to `rusttv.v12` is
    needed only if a new change kind or field appears (e.g. a built-in tile
    hidden).
- **Phone mirror:** the Apps mirror already sends every tile
  (`TileView {id, label, banner, folder}`, `protocol.rs`). It gains an
  optional `caption` so the phone shows "Update available" too (additive;
  unknown fields are ignored, REMOTE.md § 4); built in step 1b.
  `panel:<id>` is added to `is_tile_id` with the first new panel tile.

## 7. Data-only panels (level 2) — parked

Parked with level 3 (decision 11). Notes kept for when it is picked up:

A data-only panel is a file with no code: a name, an icon, the faces it
implements, and for each face a template plus what fills it. It offers only
the actions in § 5. Installation, signing and trust are undecided: no
signed-data channel exists yet (the launcher updates only its APK), and
recording installs in History needs a snapshot field and a format bump.

"No code" is not "no risk". Rules any format must keep:
- text follows the folder-name rules (`is_folder_name`) plus bidi stripping
  (HISTORY.md § 8.5), so a panel can't spoof "Press OK to approve";
- icons come from a built-in set only; no decoded images;
- a data-only panel never draws on the review screen, and a change it
  requests shows its source there.

**Open:** where its live content comes from (§ 9 Q1). Until that is
decided, this level stays a format sketch and nothing is built.

## 8. Performance and checks

Compare main and the branch with the BENCHMARK_PLAN.md protocol on the
Philips (30+ interleaved runs, medians with IQR). No measurable change in:

- cold start (`TotalTime`, COLD) and resume (HOT);
- fresh idle PSS and `idle_pss_growth_kib`;
- **idle CPU with the About tile focused and swaying**, against an app tile
  swaying. Sway costs ~44 % of one core (`ui.rs`), and live text in a
  rotating tile may cost more than a cached bitmap;
- input-to-frame and `gfxinfo` framestats for D-pad passes over About and
  with About in the hero;
- APK size (the About banner asset goes, the logo comes in);
- About page: opening it and scrolling the notices, against today's page.

Tests: `about_view` maps every `UpdateStatus`; an update-status change
causes no `set_vec` or artwork request; old order files still load.

## 9. Open questions

Recommendation first in each.

1. **Data-only content source:** fixed built-in sources only (clock, date,
   phones, storage, network, History counts) / static only / a signed web
   feed (needs network trust, later). Owner: "don't know for now".
2. **Status corner:** starts with the clock. Clock always on, at most two
   glyphs from panels in `Attention` / clock optional / no glyphs.
3. **Quick panel:** *answered by Q10:* #40's top-right menu; which panel
   rows go in it (About, Remote, Updates) is decided when it is built.
4. **Banner:** over Home only, one at a time, 4 s, no overlay permission /
   over other apps (`SYSTEM_ALERT_WINDOW`; not recommended).
5. **Hero:** the focused panel's tile larger (what exists today) / a pinned
   panel.
6. **Ambient:** leave it to the TV's screensaver for now / Home dims after
   N minutes / a RustTV idle screen.
7. **History tile:** caption "2 h ago" or "Waiting" (accent); first run at
   the end, before About and Settings / after Settings / not placed until
   opened once.
8. **Crate layout:** modules in the launcher until a panel needs its own
   dependencies / `rusttv-<name>` crates from the start.
9. **Remote's place in History** (§ 6): *decided 2026-11-20:* record it.
   Every new snapshot then needs format 5, so builds older than #40 can't
   read new history; updates only go forward, so that is accepted.
10. **Name clash with #40's quick panel:** *decided 2026-11-20:* fold this
    doc's quick-panel face into #40's top-right menu (`Mode::Quick`,
    `quick.slint`). Panels add rows to it later (About, Remote, Updates);
    the long-press key idea (decision 14) is dropped.

## 10. Build plan

One PR per step. Each one passes the CLAUDE.md checks, translates every new
string in all 20 `.po` files, and the owner tests the feel on the Philips.
Each PR is measured per § 8 **before it merges**, so nothing slower reaches
main or a release. "No measurable change" (decision 17) means each metric's
median stays within the main build's IQR in the same interleaved run.
After #40 merges, step 1 first re-checks every file and line reference in
this doc (#40 changes `menu_items`, `render()`, `pointer_reaches`,
`home.slint` and `reconcile`).

**Step 0 — JSON template spike** (now; touches no tile code). A throwaway
branch: the converter in a `build.rs`, About's tile JSON from § 4.1, the
generated tile next to a hand-written one, and the same JSON drawn on the
phone page. It also settles how generated files reach `Theme`: today
`Theme` is a non-exported global inside `home.slint` and `build.rs`
compiles only `home.slint`, while #40's `quick.slint` keeps a hand-synced
copy (`global Colours`). The spike moves `Theme` to its own exported file
or shows another way, with no second copy of the colours. Done when the
four pass-bar points in § 4.1 are answered with evidence (pixel diffs LTR
and RTL for every `UpdateStatus`, a phone screenshot, converter line count).
Result: keep JSON, or fall back to hand-written templates. Nothing from the
spike merges as is. **Done 2026-11-20: passed, keep JSON** (§ 4.1 "Spike
result"; code in `spikes/panel-templates/`, outside the workspace).

**Step 1 — About's tile on the contract** (after #40 is merged). The
contract and its first caller land together, so no dead code meets
`clippy -D warnings`: `PanelView`, `PanelTile`, `PanelRow`, `PanelPage`,
`about_view` and the v1 blocks (§ 3, § 4), and About's tile drawn from its
view (logo, version and caption instead of the banner and dot), the hero
copy, and the phone mirror's optional `caption`. Tests: every
`UpdateStatus` maps to the § 4.1 caption; building a view can't panic; an
update-status change does no `set_vec` and no artwork request. Visible
changes for the owner to check by eye: the caption in place of the dot,
and the hero glow, which today comes from About's banner and becomes a
fixed `Theme` colour. Docs in the same PR: REMOTE.md § 4 (`caption`),
ROADMAP.md (the parked "Panels" idea points here), ARCHITECTURE.md
decision log, CHANGELOG.

*Built 2026-11-20, with these differences from the plan above:*
- **Only the tile face exists.** `PanelView` has only `tile`;
  `PanelRow`, `PanelPage` and the page blocks come with step 2, which uses
  them, so no dead code lands now.
- **One shared source for the tile and the hero:** the tile reads the
  `AboutPanel` global, so a status change sets two properties and never the
  tile list.
- **The hero glow still comes from About's banner** (`assets/tiles/about.png`
  stays for the glow and the phone's picture). No visible glow change.
- **The phone mirror's `caption` came in a follow-up PR (step 1b, done
  2026-11-20).** `TileView.caption` is a fixed enum (`update`,
  `downloading`, `installing`) left out of the JSON when there is none;
  `remote::home_view` fills it on About from `about_view`'s `status`, and
  a change of the caption pushes the view while RustTV is in front (the
  download's percent sends nothing). The phone draws it as a small label on the
  picture's start corner, with the launcher's wordings in all 21 phone
  languages (`page/i18n.js`, keys `caption_*`). The phone doesn't run the
  JSON template yet; that waits for a panel whose phone face needs more
  than a caption.
- **Badges are images set from Rust** (`Badges` global in `ui/theme.slint`),
  like the tiles' banners. The launcher loads no `@image-url`.
- **The converter lives in `codegen/convert.rs`** (`build/` is git-ignored).
  Its tests run with the launcher's (`src/template_convert_tests.rs`), and
  a test checks every template text is in the `.pot` and translated
  everywhere (`scripts/launcher-pot.sh` extracts them).
- **Done from the spike's list:** duplicate JSON keys are refused, and
  `Theme`/`Dir` are imported by name through an include path (one file, one
  `Dir.rtl`; checked in Arabic). `build.rs` generates `Badges`
  (`badges.slint`) and its Rust setter from its one `BADGES` list.
- **Fit:** every caption was measured at the tile's size (23u in a 218u
  line, Inter), and ten long wordings were shortened (e.g. es "Nueva
  versión", de "Wird geladen"), so none is cut.
- **Sway cost:** the template's root has `cache-rendering-hint`, so a
  swaying About tile moves a cached layer instead of laying out text every
  frame. Measured with § 8 in the final pass on the TV (owner: all on-TV
  tests at the very end).

**Step 2 — About's page and row from blocks.** The page looks the same as
today (compared by eye against a screenshot of the desktop build); OK still
runs Update RustTV; pointer and reduced effects behave as today. The row
face is #40's `AboutRow` in Settings, fed from the same view.

*Built 2026-11-20, with these differences from the plan above:*
- **Templates hold the layout and blocks; views hold values.** `PanelPage`
  and `PanelRow` carry property values (`Props`) like `PanelTile`, not the
  § 3 sketch's `Layout` and `Vec<Vec<Block>>`: the page's layout is in
  `templates/about_page.json`, so a second copy in Rust would only drift
  (ARCHITECTURE.md D29). `about_view` fills all three faces; a name means
  the same value on every face (`AboutPanel` global).
- **Blocks are components with fixed variants** (`ui/blocks.slint`):
  templates choose `large`/`medium`/`small`, `normal`/`small`, `dim`, never
  sizes in `u`, so a later theme changes every page. `Hints` moved to
  `ui/theme.slint` so generated pages can read it.
- **The updater's line is words in the template**: the view names the
  state (`update`: `idle`, `checking`, …, `failed`) and its detail, and the
  page's `status` selects the text (the msgids of `Strings.update-line`,
  which Settings' Updates row still uses).
- **The row face is Settings' RustTV row** (`AboutRow::RustTv`, OK opens
  the page), drawn as today: "RustTV <version>" and the licence line. The
  plan's "Update available" stays on the Updates row right under it, so it
  isn't shown twice. The row template becomes the `AboutListRow` global's
  functions. Settings' unused "Licences and third-party notices" texts
  went with the old `Strings.about-row` cases.
- **No page action in the view yet:** OK on About stays the model's
  (`Effect::Update`); `PanelAction` comes with a second page that has one.
- **One new text:** the title "RustTV", kept as "RustTV" in every language
  (it was a literal before), so nothing new needs measuring for fit.
- **Look:** compared pixel by pixel with desktop snapshots of the Android
  layout before and after (`RUSTTV_NO_DEVICE=1`; About page and scrolled
  notices; English, Ukrainian while downloading, Arabic idle and with an
  update available). The surface panel is identical; in English and
  Arabic the blocks under the licence paragraph sit 1.5u lower (2 px at
  2560 × 1440), Ukrainian is identical but for a few pixels. A small
  heading keeps a 36u line box (`PageTitle`), so the notices start where
  they did whatever the font. Settings' RustTV row is checked by a test
  (`about_row_and_page_come_from_the_about_view`): the desktop demo
  doesn't reach Settings' About section.

**Step 3 — Remote as a panel** (decision 23). The § 3 sketch grows here:
`PanelId::Remote`, `Line` kinds for the phone count and "Pairing", and a
trait only if two `*_view` functions really share a shape. Its tile shows
the paired phones' count or "Pairing" in the accent while pairing is open,
from events the remote thread already sends (no new polling). Its page
stays the current Remote page, moved onto blocks; `Rows` is added here
because the phone list needs it. Its place is recorded in History (§ 9 Q9):
`launcher_state` stops leaving it out, and snapshots need format 5.

**Step 4 — History as a panel**, per § 4 "History's faces", after its
cheap "last entry + pending count" message exists.

**Later, each after a Claude design canvas** (owner's rule for visuals):
the faces left open in § 9 — status corner, panel rows in #40's quick panel (§ 9 Q10),
banner, ambient — and hide and folders for panel tiles (format 6).

**Review 2026-11-20 (fresh-eyes subagent, against main 1333a5a and #40's
head):** fixed here: format numbers after #40, About's tile and row after
#40's settings menu, `Theme` reach for generated files, dead-code step,
measuring before merge, the `UPDATE_POLL` text, the sketch for a second
panel, missing doc updates, the `TileView` name clash, the hero glow.
The owner decided § 9 Q9 and Q10 the same day.

## Review log

**2026-10-25, independent review against the code.** Fixed in this doc: the
About plan's clashes with the code (menu, folders, hidden, `reconcile`), the
pulled update status, About's update action, the Toggle that would bypass
review, `render()` rebuilding on data changes, the hero and glow, all
`UpdateStatus` states, the phone mirror, text as kinds not strings, History's
no-I/O-at-start rule, the missing ROADMAP section, the CLI schema rule, the
downgrade behaviour, failure states, and the measurement list. The contract
shrank to a pure function per panel until a second panel exists.

Challenges to owner decisions, answered by the owner on 2026-10-25:
- **5:** v1 ships Move only; hide and folders come with the next state format. Accepted.
- **8:** "no panel-owned timers" replaces "push only". Accepted.
- **10:** the review proposed letting built-in pages keep their own Slint.
  The owner wants one generic contract instead, "so we will not fight with
  contract each time": the page face becomes a layout of blocks, and
  About's page moves onto it in v1.
- **11:** level 2 is parked like level 3. Accepted.
- **13:** panels that use the network or share data get an off switch.
  Accepted.
- **14:** the quick-panel key is decided after a read-only key check on
  the Philips. Accepted.
- **19:** kept: logo and version always, the caption changes for updates.

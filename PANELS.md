# Panels — design

Status (2026-11-21): **being built** on the `panels` branch: steps 1
to 4 (About, RustTV Remote and History as panels) are done, and so are
the "Later" items: the panel rows in the top-right menu, hide and folders
for panel tiles, the status corner, banners and the idle screen (§ 10). Written from the owner's answers on 2026-10-25 (§ Owner
decisions), checked against the code by an independent review (§ Review
log), and re-checked against main on 2026-11-20 (§ 10 has the build
plan). Replaces ROADMAP.md § Parked
ideas → "Panels: add-ons with several faces". The theme builder gets its own
design later; this doc only says how panels sit in a theme.

A **panel** is one RustTV feature (About first, then History, later Clock, …)
that can show itself on several surfaces, called **faces**: a full page, a
tile on Home, a row in a list, a status-corner glyph, a row in the quick
panel, a banner, a line on the idle screen, the phone. "Panels" is the word for people; "add-ons" is the word in code.
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
   hide and folders come with the next state format (§ 6). *Built
   2026-11-21* with state format 7 (§ 6, § 10).
6. **Tile look:** the app tile's frame, radius and focus (scale + shadow +
   label), with live content inside instead of a banner. Focus adds nothing
   else.
7. **OK opens the panel's page.** Long press shows the usual tile menu plus
   up to two of the panel's own actions.
8. **Refresh:** no panel-owned timers or threads and no per-frame work
   (*after review*, replacing "push on change only"). Panels update from
   events Home already has; Home re-formats relative times ("2 h ago") on
   resume, focus change and the clock's one tick a minute. (*As built*,
   step 4: on resume and focus changes only; there is no clock yet.)
9. **One instance, many faces.** A panel can show as a tile, a row and a
   corner glyph at once, all from one data source. **Never** two copies of
   one panel (no "Weather for Kyiv and for Lviv"). *Amended 2026-11-24
   (PANEL_STUDIO.md Q14):* a panel may be added any number of times, each
   copy with its own knobs and placement, within the memory and node
   budget.
10. **Rendering, one generic contract:** a panel returns typed data. RustTV
    draws every face, the page included, from a small fixed set of shared
    templates and blocks (§ 4). Panels bring no Slint of their own, built-in
    ones too, so a new panel never fights the contract. *After review:*
    About's page moves onto page blocks in v1.
11. **Trust levels:** built-in panels (level 1) are in scope. *After
    review:* data-only panels (level 2) are parked like level 3 until their
    data source (§ 9 Q1) is decided; § 7 keeps their safety rules as notes.
    Code from other people (level 3) stays parked behind a SECURITY.md
    decision. *Amended 2026-11-24 (PANEL_STUDIO.md Q4):* level 2 is lifted;
    user packages are built per PANEL_STUDIO.md, always installed through
    the review and code on the TV.
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
    every new snapshot (§ 9 Q9; 6 since step 4, History's tile).
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

Not in v1: status corner, banner, ambient, toggles and change requests,
data-only panels, the theme builder, Settings as a panel (History became
one in step 4, § 10). The quick panel's rows, hide and folders for panel
tiles, the status corner, banners and the idle screen came after v1 (§ 10,
"Later"); toggles and change requests, data-only panels and the theme
builder belong to the later panel-studio design (§ 9 Q1). Never:
two instances of one panel, panels running other people's code before level
3 is approved.

## 2. Faces

| Face | Where | v1 | Status |
|---|---|---|---|
| **Page** | Full screen: a layout filled with blocks (§ 4) | yes | decided |
| **Tile** | 1×1 on the Home grid; hidden and in folders like an app (§ 6) | yes | built |
| **Row** | One line in a list (the Settings list, the quick panel) | yes | decided |
| **Status corner** | Top-right of Home, not focusable | later | built: the clock and up to two glyphs (§ 9 Q2, § 10) |
| **Quick panel** | The row face in the top-right menu's RustTV panel (decision 25) | later | built: About, RustTV Remote, History (§ 10) |
| **Phone** | RustTV Remote page | partly | the Apps mirror already shows every tile (§ 6) |
| **Banner** | Short message that slides in | later | built: over Home only, one at a time (§ 9 Q4, § 10) |
| **Hero** | Area above the grid | partly | decided: the hero redraws the focused tile larger (§ 4, § 9 Q5) |
| **Ambient** | Idle / screensaver | later | built: the idle screen after 5 minutes on Home (§ 9 Q6, § 10) |

A panel lists the faces it implements. RustTV never shows a face that a
panel doesn't list.

## 3. The contract (level 1, built-in)

No trait until a second panel exists. v1 is one pure function per panel
that returns a small value; Home keeps the last value and applies only what
changed. Sketch (names open until built; *as built in steps 1 to 3*,
`PanelTile`, `PanelRow` and `PanelPage` each carry the values of their
template's properties, `Props`, and a page's layout and blocks live in its
template, ARCHITECTURE.md D29):

```rust
pub(crate) enum PanelId { About }                 // as built: History only (§ 6)
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
/// (As built: the page's values are set with the tile's on every status
/// change; only an open page reads them.)
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
  version inside RTL text is an isolated LTR run (a text's `{"ltr": prop}`
  argument, § 4.1). Tile text is measured by hand to fit in all 21
  languages (no test pins it), and checked in RTL.
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
| `PanelRow` | row | title, dim detail, accent dot (Settings' row look) |
| `PanelPage` | page | a layout (one column, or two: a narrow start column and the 1110u surface panel at the end, as About is today; mirrored in RTL) filled with blocks |

**Page blocks (what About and Remote need):** one Slint component each in
`ui/blocks.slint` (`PageTitle`, `PageText`, …), which holds their sizes;
colours come from `Theme`. The JSON key and fields are in § 4.1.

| Block | Draws | Focus |
|---|---|---|
| `Title` | a title: `large` 76u (a page of its own, About), `medium` 57u (list pages), `small` 29u (a heading in the surface panel) | no |
| `Paragraph` (the plan's `Text` and `Status`) | one paragraph; normal (29u) or small (25u); tone normal, dim or accent (the updater's line turns to the accent for `Attention`); wraps | no |
| `Mark` (the plan's `Badge`) | a built-in mark from a fixed list (`slint`: Slint's `AboutSlint`, at a fixed 240u and never conditional, so the attribution can't be shrunk or hidden) | no |
| `ScrollBox` | long text Home loads (a fixed list: `notices`) filling the surface panel's height; Up/Down scroll; at most one per page | yes |
| `Hint` | the dim key hint ("Up and Down scroll the notices. Back returns."); empty when `Hints.keys` is off | no |
| `Picture` (step 3) | a picture Home provides from a fixed list (`remote-banner`, the `Pictures` global), the column's width and 270u tall, rounded | no |
| `Rows` (step 3) | a focusable list Home fills from a fixed list (`remote`), Settings' row look (title, dim detail, accent dot, dim rows); pointer hover and click through `pointer_reaches`; fills the height it is given; at most one focus block (`ScrollBox` or `Rows`) per page | yes |

Layouts: `one-column` (blocks in one 1740u column) or `two-columns` (a
560u start column and the 1110u surface panel at the end side, as About is
today); both mirrored in RTL. A page may have a **header** (step 3, a list
page as Settings and Remote are): a 57u title above both columns and an
elided line under it, Home's status line (what just happened) or, without
one, the dim key hint (empty without key hints). Under a header the start
column is 480u wide and starts at 260u with its blocks 36u apart, and the
surface panel runs from 260u to 1020u.

Later blocks, when a panel needs them: toggle and chevron rows, and the
pairing QR code as a picture (the pairing screen stays hand-written; it
is not a panel face). Plural forms ("2 hours ago": `@tr` with a count)
came with History (step 4, § 4.1). Later an editor may want stable
block ids, accessible labels, typed props (numbers formatted per
language, numeric conditions; today every prop is a string) and a JSON
Schema file for the schema, which lives only in `convert.rs` now.
Badges and marks stay fixed lists:
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
{ "schema": 1, "component": "AboutTile", "face": "tile",
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
- **Version:** `"schema": 1`, required; a converter, loader or editor
  refuses a version it doesn't know.
- **Faces:** `"face"` is `tile`, `row`, `page`, `corner`, `banner` or
  `ambient` and fixes the root. Both are kept: the face says where the
  template is shown, the root which node it is.
  - `tile`: a `column` (above), drawn as a component.
  - `row`: a `list-row` with `title`, optional `detail` (texts) and
    optional `attention` (a condition: the accent dot). Settings draws
    every row of its list with one component (`ControlRow`), so a row
    becomes a Slint global of pure functions (`title`, `detail`,
    `attention`) that Rust calls to fill the list. Each function takes the
    `props` as parameters in their order, so reordering `props` changes
    the call; Rust passes them by name from the view today
    (`settings_view.rs`, one prop), and a second row template should
    pass the view's `Props` in order. `title`, `detail` and
    `attention` can't be a row's prop names.
  - `corner` (*since 2026-11-21*): a `glyph` with `badge` (a built-in
    badge) and `show` (a condition, required): the panel's picture in
    Home's status corner while it needs attention. A global like a row's:
    `show(props)` and the `badge` picture (an `out` property). Home shows
    at most two, in About, RustTV Remote, History order.
  - `banner` (*since 2026-11-21*): a `banner` with `badge`, `title` and
    optional `detail` (texts): a global with `title(props)`,
    `detail(props)` and `badge`. A banner is an event, not a state, so its
    values come from the change that makes it (`about_banner`,
    `remote_banner`, `history_banner`, `panels.rs`), not from the view.
  - `ambient` (*since 2026-11-21*): a `glance` with `badge`, `text` and
    `show` (required): the panel's line on the idle screen, a global with
    `show(props)`, `text(props)` and `badge`.
  - Each field of a function face is a function of the same name (the
    picture the `badge` property); its props can't take those names, and
    `badge` is reserved on every face. One convert.rs list
    (`FUNCTION_FACES`) names each face's root, fields, their kinds and
    the defaults of the optional ones; build.rs calls them for the
    test-only templates.
  - `page`: a `page` with `layout` (`one-column` or `two-columns`),
    `start` and, for two columns, `end`: arrays of the § 4 blocks. Keys:
    `title` (`value`, `size`: `large`/`medium`/`small`, default `medium`),
    `paragraph` (`value`, `size`: `normal`/`small`, default `normal`,
    `tone`: `normal`/`dim`/`accent` or a `select` of them, default
    `normal`), `hint` (`value`), `picture` (`name`, from a fixed list
    Home provides: `remote-banner`), `mark` (`name`), `scroll-box`
    (`source`), `rows` (`source`, from a fixed list Home fills:
    `remote`). `title`, `paragraph`, `hint` and `picture` take an
    optional `show`; a mark, a scroll box and rows don't (an attribution
    is never hidden, and Home's focus can rely on its focus block). A
    page shows each mark once.
  - An optional `header` draws the list-page header. Its `title` and
    optional `hint` are texts, as a row's `title` and `detail` are (the
    header is a field of the page, not a block). Its page gets a
    `header-status` property that Home fills with its status line.
  - A page has at most one focus block (`scroll-box` or `rows`). `rows`
    go in either column and take the height the others leave; in the
    surface panel they reach 24u into its padding, as Settings' rows do.
    Their page gets `rows-model` (Settings' row data), `rows-focus`,
    `rows-first` (Home scrolls by rows) and the callback
    `rows-pointer(index, action)` (0 moved over, 1 clicked). Rows draw a
    title, a dim detail, the accent dot (`current`) and dim rows; slider,
    toggle and chevron rows come with Settings' page. What OK on a row
    does stays Home's, like About's OK (§ 5).
  - Page blocks have their own names, so no key means one thing in a
    tile and another on a page (a tile's `text` takes sizes in `u`, its
    `image` a picture `badge`). A `scroll-box` goes in `end` only; its
    page gets `scroll-text`, `scroll-position` and `scroll-fraction`
    properties that Home fills (it owns the scrolling). So `scroll-*`,
    `rows-*` and `header-*` are reserved names, on every face.
    A `source` (a scroll box's or rows') names what Home loads from a
    fixed list (`notices`, `remote`); today it is declarative (home.slint
    binds each page's properties), and a later loader reads it to know
    what to fetch. Such a loader also needs a page's OK action as data;
    it is Home's for now (§ 9 Q1). The box's
    scroll bar sits in the surface panel's end padding, which is one more
    reason it lives only there.
- **Conditions** (`show`, a row's `attention`): a property's name (true
  when not empty) or a `select` of `true`/`false` (else `false`). A
  `select`'s case values are lowercase words or numbers joined by `-`
  (props: words only).
- **Tile and page speak differently on purpose:** a tile's `text` takes a
  size in `u` and a colour token (`text-dim`, `accent`); a page block takes
  named variants and a `tone` (`dim`, `accent`). Tiles came first and are
  pixel-checked at a fixed size; a page reflows. If a theme needs tiles on
  variants too, the tile's `text` moves to them under a new `schema`.
- **Nodes:** `column` (later a horizontal one), `text`, `image` in a
  tile. Only the root sets size, radius and background.
- **No untranslated literals:** every text is `@tr`, brand names too
  ("RustTV" is a message translated as itself), so the "every text is
  translatable" rule has no exception to check.
- **Sizes** are numbers in `u` (the launcher's unit). The converter writes
  `N * u`; the phone page uses the same scale.
- **Colours** are `Theme` token names only (`bg`, `surface`, `text`,
  `text-dim`, `accent`) → `Theme.<token>` in Slint and `var(--<token>)` on
  the phone. A later theme changes both.
- **Images** are built-in badge names only; nothing decodes a panel's image.
- **Text** is `@tr` with a literal string and property arguments, or a
  choice between such strings by a property's value. An argument is a
  property's name, `{"auto": name}` (*since 2026-11-21*: a first-strong
  isolate, U+2068 … U+2069, so a name in any script keeps its own
  direction; a phone's name in a banner), or `{"ltr": name}`: that value as a left-to-right
  isolate (U+2066 … U+2069), so a version inside Arabic or Hebrew text
  keeps its order. The literal itself never holds bidi characters. So
  every string stays translatable: `slint-tr-extractor` runs on the
  generated files, and the phone page's i18n uses the same message ids.
- **Plural forms** (step 4): a text with a number in it adds `"plural"`
  (the English plural) and `"count"` (the property holding the number,
  a non-negative whole number as text; empty or not a number counts as
  0):
  `{"tr": "{n} hour ago", "plural": "{n} hours ago", "count": "count"}`.
  `{n}` shows the number in either form; other arguments follow as
  usual, with the same `{}` in both forms. It becomes Slint's
  `@tr("…" | "…" % n)`, so each language has its own gettext plural forms
  (`msgstr[0]`…, as many as its `Plural-Forms` header names; Arabic six,
  Ukrainian three). Both or neither of `plural` and `count`; `{n}` only
  with them. Any text may take them: a tile's, a row's or a page block's,
  inside a `select` too. The phone doesn't draw templates yet (History's
  caption there is a fixed kind, § 6); a phone renderer will need each
  language's forms too (`Intl.PluralRules`).
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
   (The bar was for the spike. With pages, rows and the strict JSON
   parser it is about 1050 lines in step 3, still one plain file.)

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

**About's faces (v1):** (as built: § 10 step 2's notes)
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

**History's faces** (as built: § 10 step 4's notes):
- **Tile:** the History icon, "Last change" and the age as the caption
  ("2 hours ago"); while an approval waits for the computer, "History"
  and "Waiting" in the accent (§ 9 Q7).
- **Row:** "History", with the last change's age as its value ("Waiting"
  and the accent dot while an approval waits). It opens the page.
- **Page:** the Settings page opened on its History section (History is a
  Settings section, not a page of its own).
- **Data:** HISTORY.md § 9 forbids history I/O at start and resume, so the
  tile needs a cheap "last entry time + pending count" from the history
  thread. Until it arrives (the first Settings open or a change) the tile
  shows its plain state. "2 h ago" is formatted by Home on resume and on
  focus changes, and (*since 2026-11-21*) at the status corner's minute
  tick, which exists for the clock: no timer of its own.

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
its own. *Amended 2026-11-24 (PANEL_STUDIO.md Q7):* a user panel may open
an app after a key press, only an app named on its install review and in
the launcher's visible app list (never Settings, SystemUI, installers or
the launcher).

## 6. Home, the order file and the phone

- **v1: About moved like before** (Move only, `menu_items`, `model.rs`).
  It keeps its id `about` in the order file and the snapshot, so v1 needed
  no format change. Since 2026-11-21 built-in tiles hide and go into
  folders too (below).
- **State formats (2026-11-20):** folders 2, phones 3, reduced effects 4
  and, since #40, format 5 (`remote` and `.desktop` tile names,
  `needs_format_five`; `needed_format`, `state.rs`). The latest release
  (0.3.2) reads up to format 3. "The next format" below meant 6, which
  step 4 took for panel tiles (`panel:<id>`); hide and folders for panel
  tiles took format 7.
- **The Remote tile is in History** (since step 3, § 9 Q9):
  `launcher_state` (`order_file.rs`) records `remote` like any tile, so
  every new snapshot is format 5, which released builds (0.3.2 and older)
  and builds before #40 can't read. A snapshot from before (no
  `remote`) still restores; Home then puts the tile at the end. History's
  changes never call `remote` added or removed (an older snapshot only
  lacks it), only moved.
- **About and Settings tiles after #40:** where a top-right menu has
  Settings (Linux), `reconcile` drops the About and Settings tiles
  (`settings_tiles`), and Settings gains an About section (`Section::About`,
  `AboutRow`). So About's tile face exists only where those tiles do
  (Android), and About's **row** face is that `AboutRow` section.
- **Hide and folders for panel tiles** (decision 5; *built 2026-11-21*,
  format 7). The About, Settings, RustTV Remote and History tiles' menu is
  Move + Hide, like an app's, and holding a moving tile over another makes
  a folder of them (with apps or other built-in tiles); a folder and the
  Hidden apps tile still can't go into one. As built:
  - **Names, not a new type:** `hidden` and a folder's `apps` hold tile
    names (`name_of`, `model.rs`): apps by component, built-in tiles by
    their order-file name (`about`, `settings`, `remote`, `panel:<id>`),
    which never hold a `/`. So `order.toml` keeps its shape; an older
    launcher reading it shows such a tile on Home anyway (an unknown
    hidden name is ignored, a foldered one dropped from its folder).
  - `needed_format` (`state.rs`) returns 7 when a built-in or panel tile
    is in `hidden` or in a folder: the names are already valid there
    (`is_tile_name`), so builds that read up to 6 must refuse such a
    snapshot (`HISTORY_FORMAT_NEWER`) rather than get it wrong. A change
    list names hiding a tile, or showing it again, only as `hidden`, never
    also as removed or added (apps too).
  - The Hidden apps page lists the hidden apps by label, then this Home's
    hidden built-in tiles in their first-run order, drawn as their tiles;
    OK (or a tap on the phone) puts one back at Home's end block.
  - `refresh` works out the built-in tiles that are hidden or in a
    folder, and `reconcile` neither keeps nor re-adds them on the grid.
    `refresh` leaves About and Settings out of folders and the Hidden apps
    page where they aren't tiles (a top-right menu; a hidden one stays in
    `hidden` for when they are); hidden wins over a folder, and a name is
    in one folder only.
  - Folder minis show a built-in tile's banner (the same picture its hero
    glow and the phone use).
  - **Settings stays reachable:** the Hidden apps tile is on Home whenever
    anything is hidden and can be neither hidden nor put in a folder, so a
    hidden Settings is two presses away (that tile, then OK on Settings).
    History's tile also opens Settings.
  - **The licence notices and Slint's attribution stay reachable:** Slint's
    Royalty-free licence 2.0 § 2 (a) asks for `AboutSlint` "in an 'About'
    screen or dialog that is accessible from the top level menu of the
    Application". While the About tile is hidden, Settings gains its About
    section (`SettingsPage::with_about`), whose RustTV row opens the About
    page with the mark; About in a folder is still on Home's top level, and
    a hidden one is on the Hidden apps page. Under a top-right menu the
    About section and the RustTV quick panel's About row are always there.
  - OK on a built-in tile in a folder opens its page; Back from it returns
    to that folder, as an app launched from a folder comes back to it
    (*since 2026-11-21*; Home if the folder went meanwhile, or after the
    launcher left the screen).
  - **Phone mirror:** built-in tiles in a folder are listed with their
    folder and their caption, the hidden apps page's tiles too; the menu
    offers Hide for them (`menu { choice: hide }`), and a tap on the hidden
    apps page puts one back. The phone asks the picture of `panel:history`
    like the other built-in tiles. On a built-in page (About, RustTV
    Remote, Settings, Settings opened on History) `focused` names the
    tile that opened that page, wherever it was (`Home::page`; REMOTE.md
    § 4).
  - The Hidden apps page keeps its name; its hint says "OK puts it back
    on Home. Back returns." (*since 2026-11-21*, in all 20 translations),
    right for apps and built-in tiles alike.
  - CLI JSON stays `rusttv.v11` (unreleased): `launcher.hidden` and
    folders' `apps` may hold built-in names, `hidden` and `in_folder` may
    name them (CLI.md).
- **A new panel tile** (History, step 4) is `TileId::Panel(PanelId)`,
  written `panel:<id>` in the order file and in `state.toml`
  (`panel:history`). Ids are 1–40 lowercase letters, digits and single `-`,
  starting with a letter and not ending with `-`, so never a `/`: they can't
  clash with apps. It ships with format 6 (`is_panel_name`, `state.rs`):
  every new snapshot with a launcher order holds `panel:history`, so builds
  that read up to format 5 refuse it (`HISTORY_FORMAT_NEWER`) instead of
  reading the name as an app. Today's built-in names (`about`, `settings`,
  `remote`) stay as they are; only new panels use `panel:<id>`.
  - An older launcher reads an unknown name as an app and drops it as not
    installed, so it forgets the panel's place; after an upgrade the tile
    comes back at its first-run spot. This build does the same with a
    `panel:<id>` it doesn't know (`tile_for_name`).
  - `reconcile`'s built-in tail is `BUILTINS` (`model.rs`): About,
    Settings, Remote, then History (§ 9 Q7). Like `remote`, a change list
    never names `panel:history` added or removed, only moved (an older
    snapshot only lacks it).
  - CLI JSON stays `rusttv.v11` (unreleased; 0.3.2 has v10):
    `panel:history` is a new value in `launcher.order` and `moved`,
    documented in CLI.md's `rusttv.v11` note and HistoryChange; no field
    or change kind changed.
- **Phone mirror:** the Apps mirror already sends every tile
  (`TileView {id, label, banner, folder}`, `protocol.rs`). It gains an
  optional `caption` so the phone shows "Update available" too (additive;
  unknown fields are ignored, REMOTE.md § 4); built in step 1b.
  `panel:history` is in `is_tile_id` and `is_builtin` since step 4 (only
  the panels this build has; the phone may tap, drag or ask its picture),
  and History's caption adds `waiting`, `just_now`, `minutes`, `hours`
  and `days` with an optional `count` (REMOTE.md § 4).

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
- icons come from a built-in set only; no decoded images (*amended
  2026-11-24, PANEL_STUDIO.md Q15:* PNG images are allowed in user
  packages under PANEL_STUDIO.md § 5.1's limits);
- a data-only panel never draws on the review screen, and a change it
  requests shows its source there.

**Out of this build:** where its live content comes from (§ 9 Q1) belongs
to the later panel-studio editor design, with levels 2 and 3; until then
this level stays a format sketch and nothing is built.

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

**Home's own faces** (*since 2026-11-21*, § 10 "Later"): no per-frame work.
The clock is one single-shot timer re-armed at each minute's start while
Home is on screen (stopped while away); it sets one text property, and on
the idle screen the drift step (a jump, no animation) and History's line.
The idle screen starts from the sway's own idle timer, so nothing new runs
between keys; it is opaque black, so Home under it draws nothing new.
Glyphs and lines are two small models set only when they change, from the
panels' existing events. A banner is one 4 s timer and a 300 ms fade/slide.
The final on-TV pass also measures:

- idle CPU on Home with the clock (no sway), against main: a minute tick
  must stay in the noise;
- idle CPU and frame count on the idle screen over 10 minutes (expect
  ~0 frames between minutes);
- the idle screen's 1.2 s fade-in and a banner's slide in `gfxinfo`
  framestats (no jank on the Philips), with reduced effects too;
- that the TV's own screensaver still starts (the launcher pauses and the
  idle screen goes), and that a key on the idle screen only wakes it.

## 9. Open questions

Recommendation first in each.

1. **Data-only content source:** fixed built-in sources only (clock, date,
   phones, storage, network, History counts) / static only / a signed web
   feed (needs network trust, later). Owner: "don't know for now".
   *Decided 2026-11-21 (orchestrator, owner delegated):* out of this build.
   Data-only (level 2) and code (level 3) panels, their content sources,
   toggles and change requests belong to the later panel-studio editor
   design; nothing here is left open for them beyond § 7's sketch.
2. **Status corner:** starts with the clock. Clock always on, at most two
   glyphs from panels in `Attention` / clock optional / no glyphs.
   *Decided and built 2026-11-21:* the top end of Home, not focusable,
   shown with the hero like the top-right menu. The clock is always on:
   24 hours or 12 (no AM/PM, as on a phone's status bar) as the TV is set
   (Android's `DateFormat.is24HourFormat`; elsewhere the language,
   `i18n::uses_24_hours`), updated once a minute. Before it (start side),
   at most two glyphs: the `corner` faces that show (About: an update
   available; RustTV Remote: pairing open; History: an approval waiting).
   With #40's top-right menu it sits before the menu, 40u from it, on the
   same line, so the two never overlap (checked on both layouts, LTR and
   Arabic).
3. **Quick panel:** *answered by Q10:* #40's top-right menu; which panel
   rows go in it (About, Remote, Updates) is decided when it is built.
   *Built 2026-11-21:* a RustTV entry with About's, RustTV Remote's and
   History's rows (§ 10); About's row carries the update caption, so no
   separate Updates row.
4. **Banner:** over Home only, one at a time, 4 s, no overlay permission /
   over other apps (`SYSTEM_ALERT_WINDOW`; not recommended).
   *Decided and built 2026-11-21:* the first. A card at the top end under
   the status corner (below the top-right menu where there is one), the
   badge, a title and a dim detail, for 4 s with a 300 ms fade and slide
   from the end edge (fade only, no shadow, with reduced effects). Queued
   (four kept, the oldest waiting one dropped), never stacked; shown only
   while plain Home is in front on its first row, with the hero and the
   corner (not over the tiles, a page, a folder, a menu, the top-right
   menu, a prompt, the review or pairing screen, the idle screen or
   another app; it is drawn under all of them, so one fading out as they
   open is covered at once); one cut short shows again, whole, when Home
   is plain again, and one that waited over 30 s is dropped as old news
   (the wait restarts when Home wakes from the idle screen: nobody saw it);
   events from while the launcher is away are dropped. Never focusable. Banners: an update that became available (not one already known at
   start: the glyph shows it) or was installed (About), a paired
   phone connected, once per phone per start (RustTV Remote; pairing itself
   happens on its screen, which says so), the computer applied approvals
   that waited for it (History: fewer wait and a newer entry; approvals that
   ran out make none).
5. **Hero:** the focused panel's tile larger (what exists today) / a pinned
   panel. *Decided 2026-11-21:* the first; nothing else is needed (the
   corner, banners and the idle screen carry the rest).
6. **Ambient:** leave it to the TV's screensaver for now / Home dims after
   N minutes / a RustTV idle screen.
   *Decided and built 2026-11-21:* Home dims to an idle screen after 5
   minutes with no input on plain Home: pure black (the one colour outside
   `Theme`, for OLED), the clock at 180u in the dim text colour at 60 %,
   and under it the `ambient` faces that show (About: "Update available";
   RustTV Remote: "Pairing"; History: "Waiting" or the last change's age),
   pictures at half. The whole face jumps to one of 25 places (±40u, ±28u)
   each minute, never animated. Any key, the pointer or a phone wakes Home
   and does nothing else (the key's repeats and release neither; a phone's
   page typing doesn't wake it; a volume key wakes it and still changes the
   volume). It fades in over 1.2 s (at once with
   reduced effects). If a page or screen is open when the time comes, it
   waits another 5 minutes. Never
   over another app; the TV's own screensaver pauses the launcher, which
   takes the idle screen away, and the launcher holds no wake lock, so the
   TV's screensaver and sleep keep their own timing. Settings → Idle
   screen (on by default), recorded in history (`ambient_off`, state
   format 8; HISTORY.md § 1).
7. **History tile:** *decided 2026-11-20 (step 4):* the caption is the
   last change's age ("2 hours ago") and "Waiting" in the accent while an
   approval waits for the computer. First run: at the very end, after
   RustTV Remote (and so after About and Settings), so a first run and an
   upgrade put it in the same place and no tile moves; it moves like the
   other built-in tiles.
8. **Crate layout:** modules in the launcher until a panel needs its own
   dependencies / `rusttv-<name>` crates from the start. *Decided
   2026-11-21:* the first (`panels.rs`, `ui/faces_view.rs` and the
   templates stay in `rusttv-launcher`).
9. **Remote's place in History** (§ 6): *decided 2026-11-20:* record it.
   Every new snapshot then needs format 5 (6 since step 4), so released
   builds (0.3.2 and older) and builds before #40 can't read new history;
   updates only go forward, so that is accepted. Built in step 3.
10. **Name clash with #40's quick panel:** *decided 2026-11-20:* fold this
    doc's quick-panel face into #40's top-right menu (`Mode::Quick`,
    `quick.slint`). Panels add rows to it later (About, Remote, Updates);
    the long-press key idea (decision 14) is dropped. *Built 2026-11-21*
    (§ 10).

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
- **Badges are images set from Rust** (the `Badges` global, which build.rs
  generates as `badges.slint`),
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
  state (`update`: `idle`, `checking`, …, `failed`) and its detail, and a
  `paragraph` selects the words and an accent `tone`. The plan's `Status`
  block is that paragraph, so the schema has one block less; its `Text`
  and `Badge` are `paragraph` and `mark` in JSON, so no key means one
  thing in a tile and another on a page. Settings' Updates
  row still uses `Strings.update-line`; a test (`ui.rs`) checks both
  say the same for every status.
- **Templates carry `"schema": 1`**, for the later runtime loader and
  editor.
- **Tests read the drawn page:** debug builds compile Slint with element
  names (`with_debug_info`, build.rs; never a release build), so tests
  read the About page's lines through Slint's test search, in English
  and Ukrainian.
- **The row face is Settings' RustTV row** (`AboutRow::RustTv`, OK opens
  the page), drawn as today: "RustTV <version>" and the licence line. The
  plan's "Update available" stays on the Updates row right under it, so it
  isn't shown twice. *Changed 2026-11-21* (§ 10 "Later"): the row face
  shows the tile's caption ("Update available" with the accent dot,
  "Downloading", "Installing") in place of its second line, as the RustTV
  quick panel needs it, so Settings shows it on this row too, next to the
  Updates row; without a caption the second line is "Third-party notices"
  (the licence line didn't fit the quick panel). The row template
  becomes the `AboutListRow` global's functions. `Strings.about-row` went: its unused "Licences and
  third-party notices" texts with it, the Updates row takes the Updates
  section's name and the Source row `Strings.source-code`.
- **Kept general on purpose** (the owner asked for general blocks):
  `one-column`, `medium` titles, `show` on page blocks and a fixed
  `accent` tone. About doesn't use them; tests convert them and check
  the output, and since step 3 every build compiles them too
  (`codegen/test_templates/`).
- **Measured on the TV at the end:** § 8's About page check (opening it,
  scrolling the notices) runs in the final on-TV pass with the rest.
- **Done in step 3:** a version inside right-to-left text is an isolated
  LTR run (§ 3) on the tile, the row and the page (`{"ltr": prop}`
  arguments, § 4.1), and in Settings' Updates line.
- **No page action in the view yet:** OK on About stays the model's
  (`Effect::Update`); `PanelAction` comes with a second page that has one.
- **One new text:** the title "RustTV", kept as "RustTV" in every language
  (it was a literal before), so nothing new needs measuring for fit.
- **Look:** compared pixel by pixel with desktop snapshots of the Android
  layout before and after (`RUSTTV_NO_DEVICE=1`; About page and scrolled
  notices; English, Ukrainian while downloading, Arabic idle and with an
  update available). A mid-review build had the blocks under the licence
  paragraph 1.5u lower in English and Arabic; re-measured in step 3, the
  merged step 2 draws the About page pixel-identical to the build before
  it (English and Ukrainian, page and scrolled notices; Arabic differs
  only in the updater's animated dots, caught at another frame). A small
  heading keeps a 36u line box (`PageTitle`), so the notices start where
  they did whatever the font. Settings' RustTV row is checked by a test
  (`about_row_and_page_come_from_the_about_view`): the desktop demo
  doesn't reach Settings' About section.

**Step 3 — Remote as a panel** (decision 23). The § 3 sketch was to grow
here (`PanelId::Remote`, `Line` kinds, a trait only if two `*_view`
functions share a shape); as built, see the notes below. Its tile shows
the paired phones' count or "Pairing" in the accent while pairing is open,
from events the remote thread already sends (no new polling). Its page
stays the current Remote page, moved onto blocks; `Rows` is added here
because the phone list needs it. Its place is recorded in History (§ 9 Q9):
`launcher_state` stops leaving it out, and snapshots need format 5.

*Built 2026-11-20, with these differences from the plan above:*
- **No `PanelId` or `Line`:** views carry template property values
  (step 2), so `remote_view(phones, pairing)` returns a `PanelView` with
  a tile and a page and no row (Remote isn't in Settings' list; a row
  came on 2026-11-21 for the RustTV quick panel, § 10 "Later"). No
  trait: About's and Remote's views take different data.
- **Tile:** the Remote badge (`rusttv-remote`, rendered by
  `scripts/brand/render.sh` from the phone icon, rounded like the RustTV
  logo), the paired phones as the value ("No phones" / "Phones: 2"), and
  "Pairing" in the accent while pairing is open; the About tile's size
  and look. The "Phones: {}" wording avoids plural forms, which the
  schema doesn't have yet (History needs them). The hero glow still
  comes from the Remote banner, which the phone still gets as its picture.
- **Events, not polling:** the count and the pairing state are set as two
  globals (`RemotePanel`) when Home starts the listener, pairing opens
  or closes (Back, a phone paired, too many wrong tries), a phone is
  removed, and when the pairing secret's own two minutes run out (a
  one-shot timer, `pairing_timer`). Never the tile list.
- **Page:** `templates/remote_page.json`: the new `header`, a `picture`
  (the banner, the `Pictures` global build.rs generates from
  `convert.rs` `PICTURES`), a dim `paragraph` (the note moved out of
  `Strings`) and `rows` in the surface panel. `MenuRow` moved to
  `ui/blocks.slint`, and home.slint feeds the page Home's rows, focus,
  first row, status line and pointer callback. Compared pixel by pixel
  with the page before (English, Ukrainian, Arabic): identical except one
  thing `Rows` now draws as a general block must: a row's `dim` and
  `current` (ui.rs always set them; the old page dropped them), so
  "Pointer in other apps" is dim where there is no pointer service and
  shows the accent dot when it is on.
- **History:** Remote's place is recorded (§ 6, § 9 Q9); see HISTORY.md
  § 1.
- **One list each:** `BADGES` and `PICTURES` live in `convert.rs`;
  build.rs generates the `Badges` and `Pictures` globals and their Rust
  setters from them, and a test checks Home has every picture. Templates
  can't take a file name build.rs or ui/ uses, or another template's
  component.
- **The general paths compile in every build:** `codegen/test_templates/`
  (a one-column page under a header with every column block and rows in
  the column, a nested tile column, a row with every function) is
  compiled by build.rs into `test_pages.rs`, and a test draws it: the
  rows take the column's height, text blocks keep theirs.
- **Checked:** the desktop demo (`--demo-remote`, which now also shows the
  tile) with `RUSTTV_NO_DEVICE=1`, `RUSTTV_REMOTE_PAIR=1` and
  `RUSTTV_LANG=uk`/`ar`; three new texts in all 20 translations, measured at
  the tile's sizes (27u and 23u in a 218u line, two-digit counts) by
  hand. Vietnamese "No phones" is "Chưa ghép nối" ("not paired yet"):
  "Chưa có điện thoại" doesn't fit. Tests: the view, the tile's globals
  per event (a phone on file, pairing opened, closed, spent, timed out, a
  phone removed) without a tile-list change, the page's rows under the
  pointer (hover, click, nothing behind the pairing screen), the
  snapshot, an older snapshot restoring with Remote at the end, and the
  history diff. `--demo-remote` goes Left in a right-to-left language.
- **Measured on the TV at the end**, with the rest (§ 8).

**Step 4 — History as a panel**, per § 4 "History's faces", after its
cheap "last entry + pending count" message exists.

*Built 2026-11-20, with these differences from the plan above:*
- **Tile id and format:** `TileId::Panel(PanelId::History)`, written
  `panel:history`; state format 6 (§ 6). First run puts it last, after
  RustTV Remote, on Android and Linux (§ 9 Q7); its menu is Move. On
  Linux it is a tile too, like Remote (Settings is in the top-right menu
  there).
- **Data, no history I/O on Home's paths:** the history thread keeps a
  `Summary` (newest entry's time, approvals waiting for the computer) that
  it reads after the work that changes it (a launcher edit, an approval, a
  door commit or `pending` read, the Settings page's data, and packing while
  the launcher is away) and before it answers; Home copies it under a lock
  on resume, on focus changes and when the Settings page's data or an
  approval's answer arrives, and formats the age with the clock then (never
  a timer; *since 2026-11-21* also at the status corner's minute tick). Until the thread has read the history (the first Settings open
  or change after start) the tile shows its plain state: the badge and
  "History". The history isn't opened for the tile.
- **Faces:** `history_view(summary, now)` gives the tile and the row the
  same two values: `status` (`waiting`, else the age's unit `now`,
  `minutes`, `hours`, `days`, else "") and `count`. Tile
  (`templates/history_tile.json`): the History badge (`rusttv-history`,
  rendered by `scripts/brand/render.sh`: the RustTV icon with a clock
  turning back), "Last change" over the age ("Just now", "2 hours ago"),
  or "History" over "Waiting" in the accent; no age when the clock isn't
  set or is behind the entry. Its hero glow and the phone's picture come
  from the new `assets/tiles/history.png` banner, like Remote's.
- **Row:** History's entry in Settings' section list
  (`templates/history_row.json`, the `HistoryListRow` global): "History"
  with the age as its dim second line, or "Waiting" and the accent dot
  while an approval waits (the Waiting section under it says for what).
  The section list became Settings' row look (`SettingsRow`: title,
  detail, dot) in a column; History's row keeps two lines from the start
  (`SettingsRow.tall`), so the sections under it don't move when its age
  arrives. OK on the tile opens Settings on the History section (list
  focus; its entries arrive from the thread).
- **Plural forms** in the schema (§ 4.1): general, for any text; three new
  plural texts, gettext forms in all 20 languages (each `Plural-Forms`
  header and every form checked by `template_texts_are_translated_everywhere`),
  and a test-only template (`general_plural.json`) drawn by the tests.
  `scripts/launcher-pot.sh` now names every file that shows a text (the
  extractor named only the first).
- **Phone:** History's tile shows its caption on the phone: `waiting`
  (accent) and `just_now` in the phone's words (`page/i18n.js`), and
  `minutes`/`hours`/`days` with `count` through the browser's
  `Intl.RelativeTimeFormat` (Western digits, as on the TV), so the
  phone's own plural forms apply.
- **Texts and fit:** "Last change", "Waiting", "Just now" and the three
  plural ages in all 20 translations; each measured at the tile's sizes
  (27u and 23u in a 218u line; Inter, or the script's system font) with
  59 minutes, 23 hours and up to 999 days: all fit (Dutch elides from
  1000 days). Several languages say "Changed" for "Last change"
  (de "Geändert", ru "Изменено") to fit.
- **Checked:** `--demo-history <dir>` with `RUSTTV_NO_DEVICE=1`, in
  English and Arabic, and with `RUSTTV_HISTORY_PENDING=1` (a TV without
  the settings grant: an approved change waits for the computer, so the
  tile shows "Waiting"). Tests: the summary from the thread, every age
  boundary and the waiting state, the view against its templates, the
  tile's globals on a focus change without a tile-list change, OK opening
  Settings on History with its row, plural forms in English, Ukrainian,
  Polish, French and Arabic, the phone's captions, format 6 and older snapshots
  restoring with History at the end.
- **Fresh-eyes review** (four reviewers): fixed a brace check that let
  `{{}n}` through, the row's height jump, the row's repeated "Waiting for
  your computer", ordering-dependent tests, stale docs; added tests for
  the clock-not-set ages, plural forms per language, the pointer over the
  taller row, the phone's drag of `panel:history`.
- **Measured on the TV at the end**, with the rest (§ 8).

**Later, each after a Claude design canvas** (owner's rule for visuals):
the faces left open in § 9 — status corner, banner, ambient. Panel rows in
#40's quick panel (§ 9 Q10) and hide and folders for panel tiles (format
7) are built (below), without a canvas: the owner asked for them to keep
the existing look (2026-11-21). The status corner, banners and the idle
screen are built too (below): the owner delegated the decisions, and they
were checked by eye in desktop snapshots against the existing look.

*Built 2026-11-21: panel rows in the top-right menu* (§ 9 Q3, Q10):
- **A RustTV entry** (`QuickAction::Panels`, code 7 in `quick.slint`,
  the RustTV name in every language, a four-tile icon) sits before
  Settings and is always there, like Modify and Settings; only where a
  top-right menu exists (`Platform::top_right_menu`; Android has the
  tiles instead). It opens a quick panel titled "RustTV" with three rows
  (`QUICK_PANELS`, `model.rs`): About, RustTV Remote, History.
- **Rows are the panels' row faces**, from the same views as their tiles
  (one source): About's `AboutListRow` (now with `status`: the tile's
  caption, "Update available" with the accent dot, "Downloading" or
  "Installing", else "Third-party notices", also on Settings' RustTV
  row),
  a new `RemoteListRow` (`templates/remote_row.json`: "RustTV Remote" over
  the tile's phones line, or "Pairing" with the dot; `remote_view` now has
  a row) and `HistoryListRow` (two lines from the start, `tall`). No new
  texts: the captions are the tiles', and "Third-party notices" is the
  About page's heading. Each was measured in all 21 languages at the
  row's width (620u: 25u details, 27u titles; widest "Third-party
  notices" in Russian, 424u). The old licence line, "MIT OR Apache-2.0 ·
  licences and third-party notices", went: it didn't fit 620u in twelve
  languages, English included.
- **One panel open at a time:** another entry (the power key too) closes
  the RustTV panel before opening its own; Home, the pointer's click
  beside it (`quick_dismissed`) and Back close it. Its rows follow the
  update and pairing while it is open; History's age is read when it
  opens.
- **Keys and pointer:** Up and Down pick a row, OK (on release, as in the
  other quick panels) opens its page (`Home::open_panel_row`: About's
  page, the RustTV Remote page, Settings on History), Back closes the
  panel. A click opens the row; hovering focuses it through the same
  `list_pointer` walk as the other quick panels.
- **Checked:** `--demo-panels <dir>` (desktop) in English and Arabic, with
  `RUSTTV_UPDATE=available RUSTTV_REMOTE_PAIR=1` for the accent rows, and
  with `RUSTTV_NO_DEVICE=1` (Android layout) for the hidden and foldered
  tiles. Tests: the rows from their views (update, pairing), OK, a click
  and the pointer opening each page, Back; the templates' cases against the
  tiles'.

*Built 2026-11-21: hide and folders for panel tiles* (§ 6 has the rules):
- **Model:** built-in tiles' menu is Move + Hide; a moving built-in tile
  makes or joins a folder like an app (`can_take`, `take_into_folder`),
  in a folder it moves, hides, leaves and opens like an app; folders and
  the Hidden apps tile never go into a folder.
- **History:** state format 7; older snapshots (formats 1 to 6) still
  load; a format-7 snapshot is refused by builds that read up to 6.
- **Checked:** the desktop demo (above), LTR and Arabic, Linux and
  Android layouts. Tests: every built-in tile hidden and shown again, the
  Hidden apps tile staying, Settings reached and its About section while
  About is hidden (and gone again after), a restart keeping them, folders
  of built-in tiles only (minis, opening their pages, hiding from inside),
  `reconcile` not re-adding them, About and Settings dropped from folders
  under a top-right menu, the phone's view and Hide, the order file and
  the history snapshot (format 7, older formats refused or kept), and the
  change list (only `hidden` / `in_folder`).

*Built 2026-11-21: the status corner, banners and the idle screen*
(§ 9 Q2, Q4, Q6; Q1, Q5, Q8 answered):
- **Three template face kinds** (§ 4.1): `corner`, `banner`, `ambient`,
  each a global of functions like a row; About, RustTV Remote and History
  have all three (`templates/*_corner.json`, `*_banner.json`,
  `*_ambient.json`), and test-only templates (`codegen/test_templates/
  general_corner.json`, `general_banner.json`, `general_ambient.json`)
  compile in every build. `PanelView` gains `corner` and `ambient`
  (`PanelCorner`, `PanelAmbient`); banners are `PanelBanner`s from
  `about_banner`, `remote_banner` and `history_banner`.
- **Home** (`ui/faces_view.rs`): the clock (one minute timer while Home is
  on screen), the glyph and line models (set from the panels' events: the
  updater's status, pairing, History's summary), the banner queue and the
  idle screen (from the sway's idle timer: the sway still stops at 20 s,
  then 4 min 40 s later the idle screen; back from another app the sway
  waits for a key, as before, and the idle screen for 5 minutes). Banners
  wait 400 ms between them and drop one that waited over 30 s. The Linux home screen reads its
  time zone with `date +%z` every 15 minutes at most (history times used
  UTC there); Android asks `TimeZone` and `DateFormat` as before.
- **Settings → Idle screen** (`Section::Ambient`, after Effects): On
  (with what it does) / Off, reviewed and recorded like Effects
  (`ambient_off`, format 8, the `ambient-off` flag file; HISTORY.md § 1),
  `ambient` in the change list and the CLI (CLI.md, still `rusttv.v11`).
- **Leftovers of step 5 fixed:** Back from a page opened in a folder
  returns to it (§ 6); the Hidden apps hint fits built-in tiles; an app
  list that couldn't be read (an empty list: a TV always has apps) no
  longer drops apps from the order, folders or hidden apps before the
  next save (`Home::set_apps` keeps the last list, or stand-ins for every
  stored app before the first).
- **Texts:** "Idle screen", its On line, "Idle screen: {}", "RustTV
  updated", "{} connected", "Your computer applied {n} change" (plural)
  and the Hidden apps hint, in all 20 translations, measured at their
  sizes (banner title 29u in 720u, Settings detail 25u in about 990u):
  all fit. The phone shows none of them.
- **Checked:** `--demo-faces <dir>` (desktop), with
  `RUSTTV_UPDATE=available`, in English and Arabic, with and without
  `RUSTTV_NO_DEVICE=1` (both layouts). Tests: the views and banners per
  event, the templates' functions and conversion errors, the corner's at
  most two glyphs, banners over plain Home only (a menu, a page, away), the
  queue, the phone banner once per phone, History's banner and not for
  approvals that ran out, the idle screen's rest, wake (key, release,
  pointer, phone) and drift, never over a menu or while away, its
  setting, the state format 8, the flag file, Back into a folder and the
  unreadable app list.
- **Measured on the TV at the end** (§ 8 "Home's own faces").

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

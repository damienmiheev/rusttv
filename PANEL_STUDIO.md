# Panel Studio — plan

Status (2026-11-27): **plan, step 2a under way; review rounds 1–20 done (round 20 found nothing new); owner request 13 added full customisation (§ 5.7, § 5.8, § 6.7, § 10.4), reviewed in rounds 21 and 22; request 14 added § 5.9 and the theme track, reviewed in round 23** (§ Review
log). Written from the owner's requests of 2026-11-20, 2026-11-21 and
2026-11-24 (§ 1) and research into FigCraft and its sibling apps, Rust GUI
toolkits and RustTV's own code (§ 17). Nothing here is built. § 16 lists
the decisions the owner still has to make; § 15 is the build order, which
starts with two spikes, not with the editor.

**Panel Studio** is a desktop app for building RustTV **panels** and
**themes** without code: a Figma-like canvas, an IntelliJ-like workspace,
its own MCP server so Claude can edit panels and themes, and a way to
install what you made on the TV (and,
optionally, to see it live on the TV while you edit). It builds on
PANELS.md: a panel is one feature with several **faces** (tile, row, page,
corner, banner, idle screen…), and Panel Studio is where people make their
own. People who don't want a canvas get **knobs**: simple settings a panel
or theme offers, changed on the TV, the phone or a small form (§ 7).

Words: *package* = one installable file (a panel or a theme); *face* = one
of a panel's views (PANELS.md § 2 names); *surface* = a place on the TV a
face can show (Home grid, Settings list, top-right menu, status corner,
banner, idle screen, phone mirror); *source* = live data RustTV provides;
*knob* = a typed setting a package offers; *state* = a value a document
keeps while shown (§ 5.7); *locked surface* = a screen no package can
change (§ 4).

## 1. Owner requests

1. **Codeless panels** (2026-11-20): data (JSON or our own format), or
   code in a defined structure when data isn't enough.
2. **One panel, many views; placement is the person's choice**
   (2026-11-20, 2026-11-24): "show it as panel / show it as settings / show
   it as whatever we want, fully customisable".
3. **Figma-grade editing, and more** (2026-11-20): everything Figma can
   do — drag and drop, resize, multi-select, snapping and guides, align
   and distribute, layers, copy and paste, undo, zoom and pan; per-corner
   radii, several shadows, gradient and image fills, strokes, blurs,
   opacity, blend modes, auto layout, constraints, components and
   variants, typography, animations, transitions, prototyping. Then more.
4. **IntelliJ-like workspace** (2026-11-21): dockable tool windows and
   menus (split, tab, float, move, saved layouts); the same thinking for
   arranging panels on the TV.
5. **Rust, smooth on macOS, Linux and Windows** (2026-11-20). **The
   studio doesn't have to use Slint** (2026-11-24: Slint was chosen for
   fast drawing on the TV). Its UI uses whatever fits a canvas and docking
   best; only the preview pane reuses the TV's components (§ 10).
6. **Its own MCP so Claude can edit** panels and themes (2026-11-20;
   corrected 2026-11-27: the studio's MCP only edits; installing goes
   the normal RustTV way, where `rusttv-mcp` gives the plan and a person
   approves on the TV, § 12.6).
7. **Our own predefined panels stay** (2026-11-20).
8. **Themes** built in the same editor, installed like panels, picked in
   a **Themes panel on the TV** (2026-11-20).
9. **Live preview on the TV, as an option** (2026-11-20).
10. **FigCraft's code may be reused** (MIT or Apache-2.0), notices kept
    (2026-11-21).
11. **A design system first, and the editor's UI designed in a design
    file before it is coded** (2026-11-21).
12. **Plan reviews loop** with "all scenarios? all customisations?" until
    a round finds nothing new (2026-11-20).
13. **Fully customisable, Home included: data first, code later**
    (2026-11-27, after PR #79 and a Philips measurement, § 6.7). A theme
    may lay Home out any way it likes (grid, dock, top tabs, carousel,
    floating pills…), react to keys, sort and filter lists and animate,
    all as data the studio edits visually (§ 5.7, § 10.4). Scripts come
    later as an opt-in layer for power users (§ 5.8).
14. **One theme system, two runtimes, nothing built in** (2026-11-27,
    agreed with PR #79's author on the PR): one name, *theme*, and one zip
    format; `"runtime": "data"` themes are drawn by compiled components,
    `"runtime": "code"` themes are run by `slint-interpreter` (§ 5.9). A
    theme always carries its Home layout. No theme is built into the
    launcher, the default look included: every theme is a package installed
    through `rusttv-mcp` and approved on the TV; the launcher keeps only a
    plain emergency Home (§ 10.4). A bigger APK is fine as long as
    performance doesn't drop much.

Safety stays as it is (CLAUDE.md, SECURITY.md): every change to the TV is
plan → dry run → approval → record → apply → verify, approved by a person
on the TV or at a terminal, never by an agent or the MCP.

## 2. What exists today (v0.4.0)

- **Panels** are compiled into the launcher: 17 JSON templates
  (`"schema": 1`) become Slint at build time (`codegen/convert.rs`). The
  schema is small on purpose (one container, text, built-in badges, a
  fixed list of page blocks, five colour tokens, one radius). D28 rejected
  runtime loading "until user panels arrive".
- **Data** is only what Home already has (updater, phones, History
  summary, clock).
- **Themes** are five colours (`ui/theme.slint`); sizes, radii, type,
  motion and ~25 colours are literals in `ui/*.slint`.
- **Trust levels** (ROADMAP.md): 1 built-in (built); 2 data-only (parked
  by PANELS.md decision 11); 3 code from others (needs a dated SECURITY.md
  decision).
- **MCP** is read-only and plan-only. **Approval on the TV**: review
  screen (`FLAG_SECURE`), Approve, a 4-arrow code, a 2 s hold, bound to the
  SHA-256 of the exact change list. **History**: `state.toml` format 8, one
  file per commit. **The door**: an adb-only content provider that appends
  observations and notes (risk class RL). **Network**: RustTV Remote's
  listener only.
- **Renderers**: Android uses Slint with **Skia** (armv7 build of our
  own); the desktop and Linux builds use Slint's **software renderer**
  (`Cargo.toml`, ARCHITECTURE.md). The phone mirror does not draw
  templates yet (PANELS.md).

## 3. The big decision: whose editor?

FigCraft (storytold/figcraft, Rust, MIT or Apache-2.0, early alpha, "~49%"
by its own estimate) has much of request 3: canvas, auto layout,
components with variants and slots, variables with modes, a command
registry shared by menus, shortcuts, CLI and MCP, batches as one undo
step, WASM plugins, headless and attached MCP, Figma JSON, Penpot and SVG
import. It draws with tiny-skia and egui.

| | A. Our own editor | B. FigCraft as the editor + a RustTV layer | C. Fork FigCraft |
|---|---|---|---|
| We build | everything | the RustTV layer: export/import of our format, a RustTV component library, checks, preview, install | the RustTV layer inside the fork, plus docking and RustTV UI |
| First usable | longest | shortest | medium |
| Figma parity | ours to build | FigCraft's, upstream | FigCraft's, then ours |
| Docking (req. 4) | ours | not there today | add egui_dock |
| "Never offer what the TV can't draw" | enforced in the editor | only as a lint after the fact | enforceable in the fork |
| Upstream risk | none | changes or stalls | merge cost |

**Recommendation: decide by the step 0 spike, with B as the first
candidate.** Everything in §§ 4–14 (format, TV renderer, install,
themes, checks, MCP, preview) is needed in every option, lives in our own
crates, and doesn't depend on the editor. The spike answers pass/fail:

- B passes only if: FigCraft's plugin or host API can add our export,
  checks and a component library without forking; its document format is
  versioned and stable enough to map; a headless export runs in our CI;
  our preview can run beside it. Install and preview live in **our**
  binary and CLI only, never in a FigCraft plugin (§ 13.4).
- Otherwise C (fork, pinned by commit, vendored with notices) or A.

### 3.1 Spike 0 result (2026-11-24)

FigCraft at commit `c01d391` (2026-11-23, workspace 0.18.1), checked
against the pass/fail list above; plus a docking and text prototype on
stock egui 0.37 and egui_dock 0.22. Nothing ran on the TV.

| Item | Result |
|---|---|
| Add our export, checks and components without forking | **Partial.** The command registry is a fixed list inside `figcraft-engine` (`crates/engine/src/cmd.rs`); outside code can't add commands to menus, palette or MCP. Plugins (wasmi, no WASI) can edit the document and report problems, but write files only through a save dialog, one file ≤ 16 MiB. Our exporter works as a library that depends on `figcraft-engine`/`figcraft-format` (git dependencies). Enforcing "never offer what the TV can't draw" needs a fork. |
| Document format versioned and stable | **Weak.** `VERSION = 1`, newer files refused, no migrations; about 430 commits touched the document and format crates in the last month. A mapping needs a pinned commit. |
| Headless export (CI) | **Passes** on macOS with an empty environment: export to SVG in 2 s, scripted render to PNG. Linux without a display not tried. |
| Our preview beside it | **Passes** as a separate process. Embedding is possible (`FigApp::ui(&mut egui::Ui)`) but draws the whole app and needs FigCraft's patched egui and winit. |
| Text | **Mostly passes.** The canvas shapes with harfrust plus `unicode-bidi`: mixed English, Hebrew and Arabic come out in the right order; CJK fine; Devanagari correct with a Devanagari font, boxes with the default one. Its egui fork adds bidi to the UI. |
| Network | An update check is switched on unconditionally in the desktop app (`apps/figcraft/src/main.rs:243`); font downloads are opt-in; collaboration only in live sessions; no telemetry found. |
| Health | Created 2026-10-09, releases every 1–3 days, 652 commands, about 60k lines in the UI crate, 273 open issues, edition 2024, MSRV 1.95. |
| Docking | Not there (no egui_dock); it already opens extra OS windows through egui viewports. |

**Docking prototype** (stock egui 0.37 + egui_dock 0.22): split, tabs,
a tab floated into a **separate OS window** and docked back when that
window closes — all pass. egui_dock's own "windows" are drawn inside the
main window, so the OS-window part is about 30 lines of ours on egui
viewports.

**Text on stock egui 0.37**: shaping passes (Arabic joining, Devanagari
conjuncts, CJK), but there is no bidi yet (egui's source has a TODO):
mixed right-to-left and left-to-right text comes out with its runs in
the wrong order. Putting whole runs in visual order (a small helper over
`unicode-bidi`) fixes labels; editing mixed text in a text field stays
weak until egui adds bidi. FigCraft's egui fork already has it.

**Recommendation: C — fork FigCraft, pinned by commit**, with our layer
(exporter, checks, component library, preview, install) in our own
crates that depend on its engine, so the fork's own changes stay small:
docking, the "can't draw on the TV" limits, our commands in the
registry, and the update check off. Reasons: the closed registry and
plugin sandbox rule out B; the unstable v1 format means pinning anyway;
its engine, headless CLI and bidi text are strong and would take long to
rebuild (A). Main risk: merge cost — upstream moves every few days and
carries its own egui and winit forks, so we rebase on our schedule, not
theirs. **Owner decision 2026-11-24: C** (§ 16 Q1).

## 4. Locked surfaces and safety rules

These hold for every option, format version and theme.

- **Locked surfaces** use a fixed `LockedTheme` (built-in colours, fonts
  and metrics, never theme tokens), and no package can draw on, over, or
  inside them: the review screen and code entry, pairing and trust
  screens, the preview "Allow" prompt, the safe-look notice, the update
  confirmation, the Panels page's install review. A test renders each
  under a hostile theme and checks it is unchanged; a hostile Home
  document (§ 10.4) gets the same test, and can't cover the launcher's
  notices either. For low vision they
  have large-text and high-contrast variants driven only by a
  launcher-owned accessibility setting (never by a package or theme).
  **Accessible approvals** (owner decision Q13, 2026-11-24), both opt-in
  in that setting: pressing OK 3 times within 2 s replaces the 2 s hold,
  and the TV speaks the code through Android's text-to-speech, only while
  the `FLAG_SECURE` review is in front. Both go into SECURITY.md with the
  approval rules.
- **Safe look** (one name for the blind reset and safe mode). A key
  chord the launcher can actually see — Android never delivers Home to
  an app as a key, so the candidate is holding Back for 5 s or Back plus
  a colour key, proven on the Philips remote and one other in step 3 —
  and `rusttv panel safe-look` from the CLI (a plan whose apply waits for
  Approve on the TV, no code, like remove) switch to the emergency Home
  (no theme, § 10.4),
  turn the look tweaks off (accessibility tweaks — text scale, bold text,
  focus frame, banner time — and the high-contrast or night mode stay; the
  emergency Home follows them) and disable every user package, keeping them and
  their placement for later. It needs no readable screen and no code
  because it only takes things away (so an adb peer pressing it is a
  nuisance, not a risk). The chord skipping plan and Approve is an
  exception to the CLAUDE.md pipeline, so it needs a dated SECURITY.md
  decision (§ 16 Q17); the TV still records what it did in History, and
  the safe-look notice offers "undo", which restores exactly the theme,
  look tweaks and packages that were on before — the first record is kept
  through repeated presses until it is undone or a person changes the
  look. If a person turns packages back on or picks a theme by hand
  instead, paused schedules stay paused, and the notice and the Panels
  page show "schedules paused" with a resume that lists what will come
  back and, because it brings content back, takes the full approval, the
  first-activation code for any surface it would switch on, and the PIN
  under the look lock. The record binds
  to the recorded hashes (schedules, clock-range theme
  modes and named-look switches are paused by the safe look and restored
  by the undo, so nothing re-applies the old look on the next tick), so nothing quarantine
  disabled or updated since comes back under the old approval — and, because it adds things, takes the full approval and
  the look-lock PIN; a package or theme can't change or block the
  chord. The chord itself (not its undo) stays open under the look lock
  (§ 7), so a child can switch a parent's look back to the emergency Home; that is accepted, because it can't add anything. The
  safe-look notice says what happened and how to turn packages back on. Until the chord is proven, the CLI and Recovery are the real
  reset. The emergency Home and the chord exist since T4 (below); the
  safe-look notice and the CLI come with step 3 (§ 15).
- **As built (theme track T4, 2026-11-27; owner chose "Home only")**: on
  the Philips, a held Back repeats about every 50 ms from about 0.4 s and
  the colour keys never reach an app (the TV opens its manual), so the
  chord is **hold Back for 5 s**. It switches the safe look on or off
  (`safe-look` flag file, History `safe_look`, snapshot format 9) over
  everything but the pairing, trust and review screens and the text
  prompt. The emergency Home is the hidden-apps page's layout with
  Settings first, then every shown app by label, OK on release; no
  folders, menus, moves, panel tiles, banners or idle screen. Its notice
  is the page's own title and hint ("Hold Back for 5 seconds to bring
  them back") and a toast; undo is the same hold, or History. With no
  themes or packages yet, the off switch only brings the compiled Home
  back, so it takes no approval; T3 adds disabling packages, the full
  approval to turn them back on, and `rusttv panel safe-look`. **Not
  yet**: proving the chord on a second remote (only the Philips is
  measured), turning look tweaks off (none exist but effects and the idle
  screen, which the emergency Home doesn't draw anyway), the look lock
  (§ 7), and a notice with its own Undo; the emergency Home uses the
  launcher's compiled look like the hidden-apps page, with no text-scale
  or high-contrast setting to follow yet.
- **Package content never appears on the review screen.** The review
  shows only facts the launcher derived: the name (rules below, shown in
  a fixed font, marked "unverified", with the author key's fingerprint
  beside it), the id, the hash prefix, the size, the faces, the sources
  with their privacy tier, the surfaces it will show on (the Home tile
  for a new install, § 8.2; more only on restore), the actions, and for
  each QR widget only its scheme, its host (punycode, fixed font) and its
  length. Icons
  are a fixed placeholder; no package image or text is drawn while the
  code is typed.
- **The review's facts come from the validated document**, not from
  `meta`: the faces, sources, surfaces and actions it lists are computed
  from what the document uses, and the validator refuses a package whose
  document uses anything its `meta` doesn't declare (the runtime refuses
  it again).
- **Names** (every translation of a package's name), in order:
  1. cleaned (§ 5.1 "Text") and NFC-normalised (NFC, with the script and
     confusable checks below, lands in the name-rules slice of step 2a;
     until then names are only cleaned and length-checked);
  2. at most 24 characters after that;
  3. every word in one script, the whole name in at most two scripts
     plus common digits and punctuation (Latin with one other — "Київ
     Clock", "Sony ソニー" — or the UTS #39 Han combinations);
  4. its confusable skeleton (UTS #39) must not match a built-in name.
  A name that fails is refused at validation with a message saying which
  word, never stripped. The checks need `unicode-script` and
  `unicode-security`, new dependencies justified in step 2a.
- **Privilege comes from where a package was loaded**, never from its
  `meta`: built-in panels (inside the APK) may use built-in-only sources
  and actions (pairing, update install, History details); stored packages
  get the closed user list (§ 5.5, § 6.4). A test asserts stored packages
  can't use built-in-only ones.
- **User faces on RustTV surfaces are marked.** A user face in the
  Settings list, the top-right menu, the status corner, a banner, the
  hero area (only while that panel's own tile is focused, within the
  hero's size) or the idle screen is drawn inside a launcher-owned slot with a small
  launcher-drawn "user panel · name" mark it can't cover; banners and
  corner glyphs have size caps; a full-screen face (page, idle screen) gets
  launcher chrome it can't cover. User tiles on Home carry a small user
  badge; corner glyphs from users get a distinct frame. A face can't
  imitate RustTV's own rows or the review screen; names colliding with
  built-in names are refused. A QR widget's content is a literal, never a
  source, and only `https:` (refused otherwise at validation, so no
  `WIFI:`, `tel:`, `sms:` or `intent:` codes); user QR codes carry the user
  badge; the review shows only facts derived from it (above).
- **The TV never opens a review by itself** for something sent from
  outside. Staged packages wait in Settings → Panels as "1 waiting to
  install" until the person opens them (§ 9.1).
- **Crash quarantine** (§ 9.6) and the **safe look** (above) keep a bad
  package from taking Home away.
- **Linux**: the desktop launcher's data folder is writable by any
  process of the same user, so approval there is a convenience, not a
  trust boundary; packages are validated and hash-checked at every load,
  not only at install.
- **The code proves presence, not who.** A child holding the remote can
  enter it. Families use the look lock (§ 7), which covers installs too;
  it is a convenience against children, not a security boundary (clearing
  the launcher's data in Android settings removes it, with everything
  else).
- **Text the launcher didn't write** (package names, paired phone names,
  computer names on the preview prompt and layer mark; a computer is told
  apart by its key fingerprint on the RustTV Remote route, and shown as
  "adb peer, identity unknown" over adb, where the launcher sees only the
  shell uid — never by its name alone)
  follows the review-name rules wherever it appears next to a review or
  the waiting row.
- **Errors people see** are short translated messages with a next step
  (each error code maps to one, in the same PR, 21 languages).
- **Agent output is untrusted**: everything an agent writes passes the
  same validator; strings from packages are marked as data in MCP results.

## 5. The package format (version 2)

One format for every panel and theme, built-in and user-made, read at
**runtime** by the TV, the CLI and the studio (`rusttv-panels` crate). It
replaces `"schema": 1` (§ 15 step 2e).

### 5.1 Files

A package is a zip read **in memory** by exact names; nothing is extracted
to disk. Allowed entries, nothing else (no directories, duplicates,
case-colliding or overlapping entries, encryption, zip64, nested
archives; only stored or deflate entries; local and central headers must
agree; CRCs checked; the end-of-central-directory record exactly at the
end of the file, nothing before the first entry; sizes enforced on the
bytes actually inflated, not the declared ones; the zip and inflate crates
named and justified in step 2a):

```
package.json          meta + document (a panel's faces, or a theme's tokens and Home)
strings/<lang>.json   message tables, one per supported language
code/<name>.slint     a `runtime: code` theme's layout (§ 5.9)
scripts/<name>.rhai   scripts (§ 5.8), later
assets/<name>.png     images ([a-z0-9_-]{1,40}.png)
assets/<lang>/<name>.png, assets/<mode>/<name>.png   locale or mode variants
thumbnail.png         for the studio and the Panels page (never on locked surfaces)
```

- **As built (theme track T1, 2026-11-27)**: `meta.runtime` is `data`
  (the default) or `code`; a panel can't be `code` (`NOT_FOR_KIND` at
  `meta.runtime`). `code/<name>.slint` and `scripts/<name>.rhai` (names
  like images, each at most 256 KiB) are read only in a `runtime: code`
  theme (elsewhere `NOT_FOR_KIND`), which must have `code/home.slint`
  (`MISSING_FILE` at `code/home.slint`); a panel or data theme with them
  gets `NOT_FOR_KIND` before their content is read. Each must be UTF-8
  with no control character but tab and line breaks and no invisible or
  direction-changing character, no lone carriage return, braille blank or
  noncharacter, and no byte order mark (`BAD_SOURCE`), so code can't read
  differently from what it does in a viewer, diff or editor (user-facing
  text belongs in `strings/<lang>.json`); all of them together are at most 1 MiB
  (`TOO_LARGE`). `parse_package` reads only `package.json`; the zip reader
  checks the files. The field and codes are additive while format 2 is
  unreleased: the launcher uses only `RESERVED_NAMES` from this crate and
  reads no packages until 4b; a code theme sets
  `min_launcher` to the first launcher with T5. Nothing interprets them
  yet (T5).
- **As built (step 2a, zip slice, 2026-11-24)**: our own strict reader
  of the central directory (the `zip` crate is lenient about exactly what
  this list forbids); inflate by `miniz_oxide` and CRCs by `crc32fast`,
  both already in the launcher's build (Slint → `image` → `png` →
  `flate2`), so no new crates ship.
  Entries are read only as `rusttv panel pack` writes them: back to back
  from byte 0, then the directory, then the end record as the last 22
  bytes; stored or deflate; no encryption, data descriptors, zip64,
  comments or extra disks; local and central headers agree (flags,
  method, CRC, sizes, name); each entry inflates to exactly its declared
  size with all its data used; declared sizes are checked against 16 MiB
  and 20:1 (over the whole zip) before anything is inflated; per-file caps (`package.json`
  2 MiB, tables 1 MiB). Names: only the list above (languages spelled as
  the launcher's folders, modes `light`, `dark`, `high-contrast`,
  `night`), no other folders, unique also ignoring case; names and
  per-file caps are checked before anything is inflated (an image at most
  4 MiB). A package needs `package.json` (an empty zip is
  `MISSING_FILE`); PNGs must start with the signature and end with the
  `IEND` chunk, so no archive hides behind one (`BAD_IMAGE`), and are
  decoded by the renderer; every image node's asset and every variant's
  base image must be there; tables are checked against each other and the document.
  Codes: `NOT_ZIP`, `BAD_ZIP`, `ZIP_UNSUPPORTED`, `BAD_CRC`,
  `TOO_MUCH_INFLATED`, `BAD_ENTRY_NAME`, `DUPLICATE_ENTRY`,
  `MISSING_FILE`, `MISSING_ASSET`, `BAD_IMAGE`.
- **Images on the TV are PNG only**, decoded by the memory-safe `png`
  crate. On Android `png` already ships inside the launcher through
  Slint's image loading (`i-slint-core` → `image` → `png`, checked
  2026-11-24), so using it directly adds no new code (PANELS.md § 7's "no
  decoded images" was reversed by the owner on 2026-11-24, § 16 Q15): images are in v2 either
  way; its size and cold-start cost on armv7 are measured in step 1 to set
  the limits, and justified in its PR. Width and height are checked
  before decoding, one frame, metadata and ICC stripped, decoded lazily with a
  memory budget. SVG, WebP and others are converted at export by the
  studio or the CLI; SVG becomes path nodes; EXIF, GPS and text chunks
  are stripped at export so a shared package doesn't say where a photo was
  taken. No animated images, video or Lottie in v2.
- **No fonts in packages, ever, at level 2** (font parsers are a classic
  attack surface). Text uses the launcher's font set (§ 6.3).
- **Limits** (first guesses, to be measured in step 1 and fixed in the
  validator with numbers): package ≤ 4 MiB compressed, ≤ 16 MiB inflated,
  ratio ≤ 20:1, ≤ 64 entries; each image ≤ 1920×1080 and the package's
  decoded images ≤ 24 MiB (the limits are the Philips numbers; each
  launcher reports its own, lower on small devices, in its capability set,
  § 12.3, and the validator and studio check against them); ≤ 500 nodes per face variant (one variant draws at a time) counted after `repeat`
  expansion (a repeat draws at most 50 items, and its children's nodes,
  bindings and shadows count once per item; nested repeats are refused), depth ≤ 8; ≤ 2 000
  strings of ≤ 500 characters per language table (each plural form
  counts; ≤ 16 placeholders a string; `strings/<lang>.json` ≤ 1 MiB); ≤ 32 variants, and a total ceiling of
  8 192 JSON objects (nodes, strokes, shadows) across the whole document,
  counted before parsing so a 2 MiB file can't build a large tree (so not
  every variant can use its full node budget); per-variant caps on shadows (4), blur layers
  (0 on TV v2, § 6.2), running animations (2), bindings (100); installed
  packages ≤ 32 and ≤ 64 MiB in total; `package.json` ≤ 2 MiB, JSON
  nested ≤ 40 deep (a node 8 levels down with an expression 8 deep fits;
  readers need a 1 MiB stack); expressions ≤ 8 deep and ≤ 64 parts, every
  value inside them counted in the document's 8 192; meta lists (`requires`, `faces`, `sources`,
  `actions`) ≤ 32 entries, no duplicates; texts after cleaning: name 24,
  author 40, licence 64, description 280 characters, homepage 200 bytes.
  Over a limit = refused with a code.
- **Strict parsing after a version check**: `format`, `min_launcher` and
  the `requires` feature list are read first; a package needing a newer
  launcher is refused with `PACKAGE_TOO_NEW` and the version it needs, and
  one needing a renderer feature the launcher lacks with
  `REQUIRES_FEATURE` and its index. Every later format keeps
  `meta.format` where it is, so this check can always read it. Then strict
  parsing: unknown fields and duplicate keys are errors, and the file and
  its `meta` must be JSON objects (no array spelling of the same data). A
  JSON Schema is published, generated from the Rust types.
- **Numbers** are finite and range-checked per property (no NaN,
  infinity, or 1e30 sizes reaching layout or allocation).
- **Text** from packages (names, strings, knob labels) is cleaned
  (`rusttv_panels::clean_text`): tabs and line breaks become spaces, other
  controls go; the bidi embeddings and overrides (LRE, RLE, LRO, RLO, PDF)
  go; the isolates (LRI, RLI, FSI, PDI) stay only in matched pairs and the
  marks (LRM, RLM, ALM) stay, because Arabic and Hebrew text with Latin
  names and numbers needs them; invisible characters go (zero-width space,
  word joiner, soft hyphen, Hangul fillers, tags, most variation
  selectors, U+2028/2029); the zero-width non-joiner and joiner stay only
  between two letters or emoji, where Persian, Indic scripts and emoji
  need them; runs of combining marks are capped at four; a text with
  nothing visible left is refused (`EMPTY_TEXT`); length is counted after
  cleaning. The host wraps source values in isolates, and every text node
  is clipped to its box (the folder-name rules, extended).
- **Errors** are codes with positions, never echoes of package content
  (they reach the CLI, MCP and History).
- **Fuzz targets** (zip reader, JSON, message tables, expressions,
  image header, layout) are an exit criterion of step 2a.

### 5.2 Meta

`id`, `kind` (panel | theme), `name` (per language), `version` (semver),
`author` (self-declared, shown "unverified"), `licence`, `homepage`,
`description`, `format`, `min_launcher`, `requires` (renderer features),
`faces` it implements (only the Home tile starts on, § 8.2), `sources` it
reads, `actions` it uses, `knobs` (§ 7), `default_language`, and for a
theme its `runtime`: `data` (the default) or `code` (§ 5.9).

- **Ids**: `user.` + `[a-z0-9-]{1,32}`, not starting or ending with `-`
  — e.g. `user.weather-tiles`. User ids can't collide with
  `panel:<built-in>`; the `rusttv` prefix and every built-in tile's name
  (`about`, `settings`, `remote`, `history`, `hidden-apps`, plus `themes`
  and `panels`) are reserved, and a launcher test keeps that list in step
  with the built-in tiles.
- **Versions** (`version`, `min_launcher`): `<major>.<minor>.<patch>`,
  plain numbers without leading zeros, no pre-release or build part (the
  release-version rule, RELEASE.md), not full semver.
- **Homepage**: an `https://` link with a plain ASCII host and no spaces,
  quotes, angle brackets or backslashes; shown as text, never opened.
- **Author key**: the studio makes each author an ed25519 key (kept in
  the OS keychain) and signs every package's content; the launcher pins
  the key on first install (trust on first use, like SSH). An update
  signed by a different key shows "different author" in red and needs the
  full approval as a new install; an unsigned package always shows
  "origin unknown". An unsigned update over a signed install is treated as
  a different author (red, full approval, no inherited placement). An
  signed update over an unsigned install pins the key on that update (its
  review says "now signed by <fingerprint>") and keeps the placement. An
  unsigned update over an unsigned install keeps its placement, since the
  same id is all there is to go on; its review says "origin unknown". `ed25519-compact` is only a dev dependency today, so
  verification on the TV is a new runtime dependency, justified with its
  armv7 size. A signature means only "same author as before", not "a
  person checked this": a CI file key or an agent with a shell can sign
  too. **In the studio, signing is a person's action** in the UI, never
  an MCP, registry or plugin command: an agent exports unsigned and the
  person signs. **Key lifecycle**: an encrypted export for backup and a
  new computer; a file key where there is no keychain (headless Linux,
  CI); **rotation** by a statement signed with the old key, which the TV
  shows on the review and re-pins after approval; a lost key means
  "different author" and the full approval. A catalog with verified
  authors is later (§ 9.8).

### 5.3 The document

**A panel is a component set; each face is a variant** (`face=tile`,
`row`, `page`, `corner`, `banner`, `ambient`, `hero`, `phone`; themes
also have `home` and `folder`, which panels refuse; `menu` is a slot, not
a face), with
optional extra variant properties (`state=attention`, `size=…`). Nodes
form a tree with stable ids (the studio keeps them across edits so
bindings survive).

- **Node kinds**: frame, group, text, image, vector (path subset),
  rectangle, ellipse, line, instance, slot, **repeat** (a list bound to a
  collection source: item template, max count ≤ 50, empty state, focus
  order), and a small widget set: progress bar, ring, QR code, analog
  clock, sparkline.
- **Properties** are the Figma checklist as data, each tagged with a
  **TV support tier** (§ 6.2); the studio and validator know which tier
  the connected launcher supports (§ 12.3).
- **Units and sizes**: `u` (the launcher's unit; the window is 1920 u
  wide). Each face is designed at a reference size (a tile is 250×141 u
  today; 1×1 tiles only in v2, PANELS.md decision 3) and the host scales
  it when theme tokens or tweaks change the grid, the text scale or the
  window (other aspect ratios and Linux windows). Faces may offer
  size-class variants (`size=compact|regular|large`); at a large text
  scale text shrinks to a minimum, then the host picks a larger size class
  or a fallback face. Safe-area rules per surface and aspect ratio.
- **Direction**: only logical properties (start/end); images and icons
  carry `mirror: rtl` when they should flip; gradients, slides and
  constraints follow the reading direction.
- **Right to left**: in RTL languages `repeat` lists, rows and pages
  mirror, and Left/Right on the D-pad follow the visual order (as Home
  does today); a face can opt a node out (a clock's digits, a media bar).
- **Accessibility**: `label`, `role`, `decorative`, `reading_order`;
  every focusable node has a focused look (its own variant or the host's
  focus frame); state is never shown by colour alone (a lint). Whether
  these reach TalkBack on Android through Slint's `accessible-*`
  properties is checked in step 1; until then the plan doesn't promise
  screen-reader support for user panels.
- **Runtime contrast**: in the high-contrast mode, or when a pair fails
  the contrast floor, the host remaps a panel's literal colours to theme
  tokens, and applies the minimum text size (themes and panels alike).
- **Instances**: a panel can be added more than once (a home clock and a
  Kyiv clock), any number of times (owner decision Q14, 2026-11-24,
  amending PANELS.md decision 9), each instance with its own knobs and
  placement. The only limits are the shared ones: the user-face memory
  and node budget (§ 6.5) and the watchdog; an instance that doesn't fit
  isn't added, and the Panels page says why.
- **As built (step 2a, document slice, 2026-11-24)**: a document is
  `{"variants": [{face, size, state, root}]}`; `size` defaults to
  `regular` and `state` to `default`; each listed face needs a
  regular/default variant, and no variant may name an unlisted face or
  repeat a face+size+state (`FACE_MISMATCH`, `DUPLICATE_VARIANT`). The root
  is an unrotated frame at 0,0 (`BAD_ROOT`). Root sizes (`BAD_SIZE`): tile
  and phone 250×141 u, page and ambient 1920×1080 u exactly at `regular`;
  `compact` no larger and `large` no smaller than that, up to the window;
  other faces more than 0 and up to the window. Node kinds so far: frame,
  group, rectangle, ellipse, line, text, image, progress, repeat. `x` is measured
  from the parent's start edge (mirrored in RTL), which is how the format
  stays logical. Ranges (`OUT_OF_RANGE`): x −1920…3840, y −1080…2160,
  w 0…1920, h 0…1080, opacity 0…1, rotation ±360°, radius 0…960, stroke
  width 0…20, shadow x/y ±200 and blur 0…100, text size 6…200, weight
  100…900, max_lines 1…10, progress value 0…1, repeat max 1…50, gap
  0…200, item_w and item_h more than 0 and within the repeat; text ≤ 500 and label ≤ 200
  characters. A property of another kind is `NOT_FOR_KIND`; text needs
  text and size, image an asset, progress a value, line a stroke, repeat
  a list, max, item_w and item_h (`MISSING_PROPERTY`). Shadows and rotation must be listed in
  `meta.requires` (`UNDECLARED_FEATURE`; unlike `REQUIRES_FEATURE`, which
  is a feature the launcher lacks). Later slices add `role`, `decorative`,
  `reading_order`, `mirror`, vectors, instances, slots and widgets; the error codes go into CLI.md with `rusttv panel`.
- **As built (step 2a, repeat slice, 2026-11-27)**: a `repeat` node
  (`list`, `max` 1…50, `item_w`/`item_h` more than 0 and within the
  repeat's box, all four required; optional `direction` row or column,
  default column, and `gap` 0…200, default 0; `clip` and `children`
  allowed, and a repeat with no children is valid; other kinds' properties
  are `NOT_FOR_KIND`) draws its children once per item, in a cell
  each. Lists: `apps.list` and `phones.list`, each item a `name` (tier 2;
  the group goes in `meta.sources`). Inside a repeat, `{"item": "name"}`
  reads a field; anywhere, `{"count": "apps.list"}` gives how many items
  the list has (an empty state is a node whose `visible` binding compares
  it with 0). A repeat's children count `max` times against the node,
  binding and shadow caps; a repeat inside a repeat is `NESTED_REPEAT`;
  `{"item"}` outside one is `NOT_IN_REPEAT`; an unknown list or field is
  `UNKNOWN_SOURCE`, an undeclared group `UNDECLARED_SOURCE`. Windowing,
  `apps.view`, sort, filter and more item fields are step 2f1.
- **Four kinds of value** (one table in the format docs): **token**
  (from the theme), **source** (live data, § 6.4), **knob** (a setting the
  person chose, persisted), **state** (§ 5.7, not persisted). Literal
  colours are allowed but flagged "doesn't follow the theme".

### 5.4 Motion (v2)

- **Transitions** on a fixed set of properties (opacity, position, size,
  background colour, rotation) with a fixed easing set (linear, ease,
  ease-in, ease-out, ease-in-out, two springs), between variants or on a
  source change.
- **Loops** (pulse, rotate, fade, and `orbit`: a bounded drift of x, y
  and rotation, today's sway) run only while the face is visible and
  focused, at most one per surface at a time (a Home document: up to
  three, one per layer, for sway and glows), and stop on the idle screen
  except at the minute tick (PANELS.md decision 8: no per-frame work for
  still panels).
- **Triggers**: on focus, on press, on a source change, after a delay
  while visible. Timer and source triggers may only switch the panel's
  own variants, never run actions (§ 5.7 also lets them set states and
  play animations). A banner is a separate, host-owned
  declaration (which source change raises which banner), not a variant.
- **No flashing**: a loop's period is at least 1 s, at most 3 flashes per
  second and no large-area luminance flash (WCAG 2.3.1), enforced in the
  validator and in theme motion tokens.
- **System settings**: reduced effects, text scale and high contrast
  default from Android's own accessibility settings where they exist.
- **Reduced effects** (Settings → Effects) and reduced motion: each
  effect or animation may name a reduced alternative; the default is an
  instant change.
- **Keyframes** on any animatable property, **staggered lists** (each
  item a set delay after the one before) and animations started by events
  (§ 5.7) come with full customisation (§ 15 step 2f); shared transitions
  between faces stay later.
- **Effects** beyond shadows (blur, gradients, particles, 3D tilt) are
  added one at a time, each with a support tier per renderer (§ 6.2): a
  device that can't draw one gets its reduced alternative, and the studio
  says so. Scripts (§ 5.8) can't draw anything new either: they only
  compute values.

### 5.5 Actions and keys

Actions run **only after a key press on a focused node** (never from a
timer or a source):

- open the panel's page or another of its faces; switch one of its own
  variants or change one of its one-click knobs (§ 7), never an image,
  text, app or list knob, and always under the look lock and the
  first-activation rule (§ 8.2);
- open a RustTV page (Settings, History, Remote, Themes, Panels);
- open an app (owner decision Q7): a package id **named on the install
  review** (each one listed; an update adding one shows it in red),
  resolved through the launcher's own app list, only apps in the
  launcher's visible app list, and never Settings,
  SystemUI, package installers or the launcher itself (YouTube, Netflix
  and the Play Store stay allowed even though Android marks them as
  system apps; no intents, URLs or extras). This reverses PANELS.md § 5,
  decided by the owner on 2026-11-24 (§ 16 Q7).

Pages may map remote keys (colour keys, Play/Pause, Menu, numbers) to
these actions. The reserved keys of § 5.7 never reach a package, and Back
only as § 5.7 allows. Built-in-only
actions (pairing, installing updates, approving) can't be named by a
stored package.

**Settings actions** (a panel offering a system settings change) are out
of this plan until a dated SECURITY.md decision (§ 16 Q9). Switching TV
inputs (HDMI) and global key bindings on Home are out of scope for v2
too; each is its own later decision (§ 15 step 8).

### 5.6 Strings

- **Message tables** per language (`strings/<lang>.json`): message id →
  text, with plural forms by CLDR category and named placeholders. Not
  gettext `.po` at runtime: Slint's `@tr` is build-time, so user packages
  need our own lookup anyway, and a JSON table avoids a `.po` parser and
  runtime plural expressions.
- **Namespaced per package**: a package's messages are never consulted by
  RustTV's own UI, so a package can't rewrite "Approve".
- Languages: the launcher's 21, spelled like its translation folders
  (`pt_BR`, `zh_CN`, not `pt-BR`), kept in step by a test; placeholders
  checked against the default language; isolates for user text as today.
- **Formatting** follows the person's locale: numbers and digit shapes
  (Arabic-Indic digits), dates, week start, calendar (Gregorian, Hijri
  and others the platform offers), units chosen by a knob (°C/°F).
- **Per panel language**: a panel can be set to another language than the
  TV's (a Ukrainian TV with an English clock); its layout direction
  follows the panel's language, its place on Home follows the TV's.
- **Fallback**: TV language → its base language → the package's default
  language → English. A missing translation is a warning for user
  packages (built-ins keep the "always translated" rule).
- **As built (step 2a, strings slice, 2026-11-24)**: a table is
  `{"messages": {id: text | {"plural": name, zero…other: text}}}`, its
  shape governed by `meta.format`, no notes (those live in the studio
  project). Ids `[a-z0-9_.-]{1,64}`, placeholders `{[a-z0-9_]{1,32}}`,
  `{{` and `}}` for braces; texts cleaned like every package text. Plural
  forms must be ones the table's language has (CLDR cardinal rules for
  the 21 languages, whole numbers; fractions use `other`), and `other` is
  required. A translation may leave out messages and placeholders and may
  be plural where the default is plain, but may not add a message or a
  placeholder. A filled-in text is cut to 500 characters, cleaned and
  isolated; counts are ASCII digits until expressions bring locale
  formatting. None of the 21 languages is another's base, so the chain is
  TV (or panel) language → default → English; the caller maps the TV
  locale first. Codes: `BAD_MESSAGE_ID`, `BAD_PLACEHOLDER`, `BAD_PLURAL`,
  `UNKNOWN_MESSAGE`, `PLACEHOLDER_MISMATCH`, `NO_DEFAULT_STRINGS`;
  missing translations are listed for a warning. The size limits are
  § 5.1's (2 000 strings, 16 placeholders a string, 1 MiB a table;
  `TOO_MANY`, `BAD_PLACEHOLDER`, `TOO_LARGE`).

### 5.7 Full customisation as data: layout, lists, states, events

Owner request 13: everything a theme or panel needs to look and behave
its own way is data, so the studio edits it visually and the review can
say what it does. First numbers are guesses, measured in step 2f.

**Layout and drawing**

- Frames may set `layout` (row, column, grid or wrap) with gap, padding
  and alignment, each child fixed, hug or fill with a min and max
  (mapped onto Slint's layouts, § 6.1), so large text and long
  translations reflow instead of overflowing.
- A `scroller` node: an axis, how focus is kept in view (start, centre,
  nearest or a page at a time), snap and a scroll animation; `pages`: a
  stack of pages across or down with a page indicator.
- A Home root fills the window inside the safe area; children anchor to
  start, end or stretch (other aspect ratios, Linux windows, overscan).
- `mask` (an alpha gradient or image) on a group, for edge fades;
  `transform` (scale, move, skew, 2D rotation); image focal point, zoom,
  offset, saturation and brightness; text shadow, marquee on focus and fit
  (shrink or cut); gradient stops that bind and animate. Bindings may also
  set x, y, w, h and the transform's parts (a carousel scaling by distance
  from focus), animated by transitions; moves are in start/end terms, and
  rotation and skew mirror in right-to-left languages. Each has a tier
  per renderer (§ 6.2) and a fallback the studio shows.

**Lists**

- `repeat` (§ 5.3) gains `sort` (a list of keys, each an expression,
  either way, stable), `filter` (a yes/no expression), `from` and `count`
  (a dock with the first five, a shelf with the rest), `group_by`
  (sections with headers, expanded into one flat list of headers and
  items, so it isn't a nested repeat), per-item stagger and a layout transition
  (move, enter, exit). Sort and filter run again only when their inputs
  change.
- **The app list stays the person's**: a `repeat` over `apps.list` is
  windowed (only the visible items and a margin are built, so a TV with
  120 apps fits the node budget and § 5.1's cap of 50 doesn't apply; until
  that slice, 2f1, the cap of 50 holds) and
  always shows every app the person hasn't hidden, in the person's order
  (§ 8.2); `sort`, `filter`, `from`, `count` and `group_by` are refused on
  it. `apps.view` is a read-only copy of it that allows them all (a "most
  used" row, a tab per kind, a dock with the first five); it is never the
  only way to an app (§ 10.4).
- `apps.list` items have: name, banner, icon, a monochrome icon and a
  small colour palette (both made by the host), kind (app, game, folder,
  panel), new (until first opened) and launch count. A folder's `children`
  are laid out by the theme's `folder` document, or with tokens only.
- **Item and focus values**: inside a `repeat`, `{"item": …}` reads the
  item's fields and its index, first or last, row, column, whether it is
  focused, pressed, being moved or launching, and how far it is from the
  focused item; `home.mode` says whether Home is browsing, moving a tile,
  launching, or showing the tile menu or top-right menu (while moving or
  launching the keys are the launcher's, and the theme only draws); anywhere,
  `{"focus": …}` reads the focused item's fields and box. These are
  in-document values, not the focused-app source § 6.4 keeps out. A
  focus-frame node may follow the focus box with a transition.
- Items and `{"focus"}` also give `banner_blurred` (blurred once per item
  by the host, for cinema and spotlight backdrops), `art_kind` (banner,
  icon, none, loading) and a generated fallback; a bound image may
  crossfade when it changes (Costly, § 6.2).
- **Functions** added to expressions (§ 6.4): colour (mix, alpha,
  lighten, darken, a readable text colour on a background), time (hour,
  minute, weekday, between two times) and `select` (several branches),
  with a matching `select` form in message tables for grammar.

**States**

- A document declares named states (yes/no, number, text, or one of a
  list) with a start value; `{"state": name}` reads one; a `repeat`
  item may have its own. States replace § 5.3's "local" values: kept
  while the document is shown, never persisted. To remember a choice (the
  last tab), a step may write the package's `remember` slot: up to 1 KiB,
  once every 5 s, not a setting, not recorded in History per write, not
  under the look lock; it resets with the safe look and removal and can't
  switch a surface by itself.
- A variant may be picked by an expression (`variant_when`), with a hard
  cut (shared transitions stay later); a reduced-motion variant is picked
  this way.

**Events**

- **Keys**: OK, the arrows, Menu (unless reserved), colour keys, numbers,
  Play/Pause; long press (600 ms or more); double press (the single
  press then waits a declared window of at most 400 ms, shown on the
  review); hold and key repeat (from 300 ms, or after N repeats); a
  sequence of up to 8 keys with at most 2 s between them. `start` and
  `end` name the arrows in reading direction; `left` and `right` are
  physical.
- **Other events**: focus entering or leaving a node, a list's first or
  last item reached, shown or hidden (`on_show`, for icons bouncing in),
  a source change, a timer while visible (every 1 s or slower), idle for
  N s since the last key.
- Each handler has an optional `when` (a yes/no expression) and up to 8
  steps: set a state; move focus (to a node, an item by index, first,
  last, a row, or the first item where an expression holds); scroll; open
  or close a focus scope (a menu that keeps focus until it closes, then
  focus goes back); switch a variant; play an animation; write the
  `remember` slot (above); and § 5.5's actions.
- **Only key presses act**: § 5.5's actions and focus moves run only
  from a key handler on a focused node. Document-level key handlers (key
  sequences) and focus, edge, show, timer, idle and source handlers only
  set states, switch variants and play animations. So nothing moves focus
  or opens anything without a key press, and focus handlers can't bounce
  focus back and forth.
- **OK opens what it shows**: `open_item` (Home and folder documents,
  themes only) opens the focused item, resolved by the host from the
  person's own visible list. It is the one open that needs no review
  entry, and the review says "can open any visible app"; § 5.5's
  exclusions still apply (Settings, SystemUI, installers, the launcher),
  and a `panel` item opens only that panel's page. While an item is
  focused, the launcher draws its name in its own slot beside the
  "custom Home" mark (§ 10.4), so a theme can't show one app and open
  another; the lint that name and image come from the same item is only
  a first warning.
- **No loops**: a handler runs its steps once per event, and a step
  can't raise another event in the same frame. Fuel and the watchdog
  (§ 6.5) cover the rest.
- **Reserved keys** go through the launcher first and never reach a
  package: Home, long-press Home, long-press Back, the safe-look chord
  (and any hold or sequence that contains it), the Settings way in and
  the all-apps key (§ 10.4), and a long press of OK on app items and tiles
  (the tile menu: move, hide, folders). A package may bind Back only while
  a state differs from its start, and only to put states back or close a
  focus scope; a second Back within 1 s always goes to the launcher.
- **Default navigation**: an arrow no handler takes moves focus to the
  nearest focusable node that way (list order inside a `repeat`, mirrored
  in right to left); at an edge with nowhere to go, the key goes to the
  launcher (the top-right menu above the first row, as today). A handler
  takes a key only when it runs a step.
- While a locked surface, a review, a launcher page or the idle screen is
  in front, a Home document gets no keys, timers or sources.

**Limits** (first guesses): 32 states, 64 handlers, 8 steps each, 4
timers; sort and filter within the frame's fuel. § 5.4's no-flash rule
covers keyframes, stagger and animations and state changes from events.
Pointer and touch (the phone touchpad, REMOTE.md) move focus in v2;
hover and scroll events are later; sound stays later (§ 9.8). The error
codes come with each slice.

### 5.8 Scripts (later, opt-in)

Owner decision 2026-11-27 (level 3, Q9): code is a later layer on top of
§ 5.7 for power users, never needed for a look or behaviour the data can
describe.

- A package (panel or theme) may carry small scripts, used in two places
  only: a `script` function in expressions (sort keys and filters
  included) that computes a value from its arguments, and a `script` step
  in an event handler that returns state changes and steps.
- **Returned steps are checked at run time** against what the event may
  do (§ 5.7) and against the package's declared actions, apps, sources
  and knobs; anything else is dropped and counted. A script can't bind
  keys, change Back, touch the floors or the app list, or call actions
  itself.
- **What a script can read**: its arguments (declared sources, knobs,
  states, the item), nothing else: no files, network, clipboard,
  settings, other packages, or time beyond the clock source; randomness
  only from a seed the host passes.
- **Budget**: an operation limit per call, one shared limit per frame
  (every item's calls count against it) and a memory cap; over budget,
  the call gives no value (the fallback shows) and the watchdog counts
  it; again and again, the package is paused (§ 6.5), or a Home document
  falls back to the emergency Home (§ 10.4). The crash marker names
  scripts (§ 9.6).
- **Review**: "contains code" in amber, with the scripts listed by size;
  it says what they can read (the highest tier among their sources), not
  what they compute; the approval covers the scripts through the package
  hash.
- **"Allow packages with code"** (Settings, off by default): turning it
  on takes the code and hold, on the TV only (never from the phone or an
  MCP); turning it off is one click and disables scripted packages; both
  are recorded in History, and the look lock covers it. The safe look
  turns it off (its undo turns it back on). A restore or bundle brings
  scripted packages back disabled while it is off. The capability set
  (§ 12.3) says whether a launcher runs scripts. On Linux, where
  approval is a convenience (§ 4), only the sandbox protects.
- Scripts never run on locked surfaces (§ 4). The engine lives in its own
  crate, used by the launcher and the CLI's validator.
- The studio edits scripts as text with checks; an agent-written script
  is untrusted like any package content.
- The language (Rhai, as #79 uses, or WASM) and its cost on the Philips
  are decided before the slice (§ 16 Q19); #79's Rhai sandbox is a
  starting point.

### 5.9 Themes with runtime code (owner request 14)

- A `runtime: code` theme carries its Home (and later its other
  surfaces) as `.slint` files in `code/`, compiled on the TV by
  `slint-interpreter` when the theme loads. It draws through a fixed host
  API (a `@rusttv/theme.slint` module, starting from #79's
  `@rusttv/skin.slint`): the launcher passes in the same values data
  themes get (apps, tiles, focus, clock, knobs, tokens) and gets back only
  callbacks the launcher handles (open the focused item, move focus,
  open the tile menu). The host API is versioned like the format.
- It loads only when "Allow packages with code" is on (§ 5.8: off by
  default, the code and hold to turn on), and the review says "contains
  code".
- It can't reach files, the network, Settings, History, pairing or any
  locked surface (§ 4). When the theme is read, its `.slint` source is
  checked against an allowlist: it imports only the host API module and
  its own `code/` files, and has no `Window`, `PopupWindow`, `TextInput`
  or file loader; images come only from its `assets/` through the host.
  Size caps (bytes, elements, nesting) stand in for § 5.1's node caps.
- **Floors, enforced by the launcher around it**: keys reach the launcher
  first (reserved keys and the all-apps key never reach the code, § 5.7);
  the all-apps key opens the launcher-drawn list of every app whatever
  the theme draws; Back and the Settings way in always work; the
  "custom Home" mark, the name of the app OK will open, and the
  launcher's notices are drawn above the theme. Locked surfaces are
  separate screens a theme can't reach, so a look-alike drawn by a theme
  approves nothing; a hostile-code-theme test (§ 4) covers a fake dialog,
  trapped focus and a hidden app. A theme that fails a floor, misses
  frames again and again or fails to compile is disabled with its reason
  and the emergency Home loads.
- **Versions**: a code theme declares `host_api` (and the Slint language
  version it was checked with); a launcher refuses a newer one at
  install, compiles every code theme at the first start after an update,
  and disables one that fails, with its reason. A removed host API
  feature is deprecated for one release first.
- **Review facts**: "contains code"; each `.slint` and script file with
  its size and hash (never its content); the host API version; "draws
  your whole Home and sees your app names and icons, the clock and its
  knobs"; no network and no files; its cost class.
- Cost: measured at about 50% more per frame than compiled code for the
  same Home (§ 6.7); each code theme is measured on the Philips in its
  PR. Scripts (§ 5.8) are Rhai first (Q19).
- `slint-interpreter` is a new launcher dependency, justified in its PR
  (size, licence, maintenance).

## 6. The TV side

### 6.1 Runtime renderer

- A **fixed set of generic Slint components** driven by models from Rust
  (not the Slint interpreter: not a dependency, unmeasured cost, more
  power than data needs; measured in § 6.7 at about 50% more per frame).
  Slint can't
  recurse, so the tree is drawn by **typed nested repeaters to the depth
  limit (8)**; group effects (clip, opacity, rotation) belong to their
  level. The step 1 spike measures per-node cost and sets the node limit.
- **Layout**: v2 maps onto **Slint's own layouts** (row, column, stack,
  fixed position, fill/hug/fixed, gap, padding, alignment, min/max; grid
  and wrap through nested layouts), so text measurement stays in Slint's
  shaper and results match today's hand-written UI. Full Figma auto
  layout semantics beyond that are later and measured first.
- **Built-in panels move to it** (step 2e), so there is one path.
- **Function faces** (rows, corner glyphs, banners, idle lines) keep
  launcher-owned slot components (§ 4): a package fills their slots (icon,
  title, value, attention) rather than drawing free nodes.
- **Platforms**: Android (Skia) and Linux and desktop (software renderer)
  run the same components. The software renderer draws **no box shadows
  and no rotation** in Slint 1.18, so the tiers below are per renderer and
  reported per device (§ 12.3).
- **The studio's preview** must match the TV: either a Skia-backed
  desktop launcher in a child process streaming frames to the studio, or
  the software renderer labelled "approximate" with live preview (§ 11) as
  the real check. Spike 1 decides.

### 6.2 What the TV can draw (support tiers)

From Slint 1.18 (checked in its source; confirmed by the step 1 spike):

| Tier (Skia, Android) | Properties |
|---|---|
| **Native** | per-corner radius; linear and radial gradients; one drop shadow per element; opacity; clip; rotation; paths with fill rule, caps and joins; letter spacing, line height, max lines, italic, text stroke; image fit, tiling, colorize; transitions |
| **Unverified until step 1** | conic gradients; inner shadow and shadow spread (not Slint element properties as far as we know; maybe drawn as layers) |
| **Costly** (allowed, counted in the budget) | several shadows (stacked layers, each an offscreen blur); cached layers; large images; keyframes and stagger (step 2f) |
| **Not on the TV in v2** | layer blur; background blur; blend modes; dashed strokes; per-side or outside/centre strokes; corner smoothing; text gradient |

On the software renderer (Linux, desktop) shadows and rotation move to
"not drawn". The studio greys out what the selected target can't draw; on export,
anything not drawable is either refused or (on request) flattened into a
PNG within the image limits.

### 6.3 Fonts and text fit

- **System fonts** (owner decision Q6, 2026-11-24): RustTV uses the TV's
  own fonts with fallback and bundles none; packages never carry fonts and
  pick a family by role (regular, bold, mono) from what the TV has. Looks
  may differ slightly per TV, which is why fit is measured on the target.
- **Fit**: the TV truncates and clips every text; the studio checks fit
  using **metrics captured from the target TV** (a one-off read of the
  real fonts' widths), so the check is advice, not a guarantee.

### 6.4 Sources

Each source has a type, a refresh trigger and a **privacy tier**; a
binding reports `loading | value | empty | unavailable | error` and has a
fallback; a panel can switch variants on that state. Only declared
sources can be bound (the runtime refuses others). Sources are versioned;
a removed one is deprecated for one release first.

| Source | Type | Refresh | Tier |
|---|---|---|---|
| clock, date, time zone, 12/24 h | time | minute tick | 0 |
| theme in use, mode | string | on change | 0 |
| updater state, version | enum, string | on change | 0 |
| device model, uptime | string, duration | on resume | 1 |
| storage free | bytes | on resume | 1 |
| network connected (yes/no; needs `ACCESS_NETWORK_STATE`, a SECURITY.md change) | bool | on change | 1 |
| app list (visible apps only), app icon by package | collection, image | on change | 2 |
| paired phones (`{"count": "phones.list"}`; names only on the TV) | number, collection | on change | 2 |
| History summary (age, waiting count) | duration, number | on change | 2 |

- Not in v2, because each needs an Android permission RustTV doesn't
  hold (and a SECURITY.md change): the network name (location or nearby
  Wi-Fi permission), recent apps (usage access), the focused app (undefined
  while Home is in front), now playing (a notification listener).
- Binding states also include `stale` with its age ("updated 12 min
  ago") for sources that can go out of date.
- Tier 2 sources are shown with a warning on the review. Serial numbers, MAC and IP addresses, accounts
  and hidden apps are never sources.
- **The phone mirror** gets a PNG the TV rendered from the `phone` face
  (§ 8.1); the phone evaluates nothing, and the TV renders that face with
  only the sources the phone's Home mirror already shows; the **preview and studio screenshots use mock data only** (TV
  data never reaches the computer or an agent).
- **Expressions** (formatting, comparisons, `if`, joins, plural choice)
  are total functions with fuel per evaluation and per frame, depth and
  output-length caps, cycle detection at validation, no panics, evaluated
  off the UI thread when they get large.
- **As built (step 2a, expressions slice, 2026-11-24; owner chose a JSON
  tree over a text language)**: an expression is a literal (string,
  number, yes/no) or one of `{"source": "group.field"}`,
  `{"msg": id, "args": {name: expr}}`, `{"if": [c, a, b]}`,
  `{"call": f, "args": [...]}`, with `f` one of not, and, or, eq, ne, lt,
  le, gt, ge, add, sub, mul, div, min, max, round, concat, text,
  format_time (numeric patterns `HH H hh h mm ss dd d MM M yyyy a`; `'`
  reserved for quoting; day and month names come with locale formatting).
  Types (yes/no, number, text, colour, time) are checked when the package
  is read; a string literal is a colour only where one is wanted. A node's
  `bindings` set text, color, fill, tint, value, opacity or visible, each
  where its literal property is allowed, and the literal property is the
  fallback while a binding has no value (≤ 100 bindings a variant).
  `meta.sources` lists groups (clock, theme, updater, device, storage,
  network, phones, history); an expression may read only listed groups,
  and a group's tier is shown on review. Evaluation is total: a missing
  source, a division by 0, an out-of-range number or spent fuel gives no
  value; the host passes one fuel budget per frame. Texts an expression
  makes are cut to 500 characters and cleaned again. No variables, so no
  cycles. Messages bindings use must exist in the default table with the
  placeholders they fill. Binding states beyond value/no value, `stale`,
  variant switching on state, tokens and locals come with later
  slices; knobs came with the knobs slice (§ 7). Number literals are at most 2^53 in size (`OUT_OF_RANGE`); a
  colour literal that doesn't parse is `BAD_COLOR`. Codes:
  `BAD_EXPRESSION`, `UNKNOWN_FUNCTION`, `TYPE_MISMATCH`,
  `UNKNOWN_SOURCE`, `UNDECLARED_SOURCE`, `EXPRESSION_TOO_LARGE`,
  `TOO_MANY_BINDINGS`.
- **For full customisation** (§ 15 step 2f):
  - launch counts: how often RustTV opened each app in the last 30 days,
    kept by the launcher itself (so no usage-access permission; launches
    from elsewhere don't count), hidden apps excluded, never in History,
    the Mac backup or MCP tools, tier 2. Counting is off until the person
    turns on "Count app launches" or approves a package that reads them
    (the review shows it in amber); turning it off or resetting clears the
    store, and both are under the look lock;
  - an app icon's palette and monochrome icon, made by the launcher, tier
    2 like the app list;
  - `ui.text_scale`, `ui.reduced_motion`, `ui.high_contrast`, `ui.rtl`,
    `ui.language` and the window size, tier 0;
  - `wallpaper.blurred` (§ 10.4), tier 0.
- **Later sources**, each with its own SECURITY.md section: web data
  through one RustTV fetcher with fixed endpoints and no package-supplied
  parameters; data from an app's content provider the person allows; data
  pushed from the computer (labelled as such).

### 6.5 Budget, watchdog and performance

- Per-face cost estimate (nodes, layers, shadows, images, animations,
  bindings) checked against the caps (§ 5.1); the studio shows it.
- **Watchdog**: a face that misses frames repeatedly loses effects first,
  then animations, then pauses ("panel paused"), recorded in History (a
  Home document isn't paused: the emergency Home loads instead, § 10.4). With
  several user faces visible, the one with the highest cost estimate is
  degraded first.
- **BENCHMARK_PLAN.md** gets a synthetic-panel fixture set (small,
  medium, at the limits) and a named baseline commit; every renderer step
  reports cold start, PSS, first draw and frame times on the Philips. No
  claim without that data.

### 6.6 Spike 1 result (2026-11-24)

Measured on the Philips with a throwaway APK (own applicationId, debug key,
no HOME filter, uninstalled afterwards); code and full numbers in
`spikes/tv-renderer/`. The stage moved every frame, so every node was
redrawn each frame.

| nodes | generic (JSON) render p50 | compiled render p50 | PSS generic / compiled |
|---|---|---|---|
| 180 (Home, 18 tiles) | 7.4 ms | 6.3 ms | 41.5 / 41.1 MB |
| 500 | 17.5 ms | 14.8 ms | 43.3 / 42.2 MB |
| 1000 | 34.4 ms | 28.7 ms | 45.9 / 43.5 MB |

What it settles:

- **Renderer route: generic Slint components fed by JSON.** They cost
  about 18–20% more per frame than hand-written Slint (≈ 0.031 vs
  0.026 ms per node), with the same cold start (`am start -W` 212–246 vs
  208–219 ms) and about 0.4–2.4 MB more memory. Parsing is cheap (0.5 ms
  for 180 nodes). The slint-interpreter route was measured later
  (§ 6.7).
- **Node budget.** A full redraw passes 16.7 ms at about 450 generic
  nodes, so the per-face cap of 500 (§ 5.1) stays, and a new **visible
  total** for user faces is added: about 250 nodes on screen at once,
  tuned in step 2b against BENCHMARK_PLAN.md; the watchdog (§ 6.5) works
  from this cost per node.
- **Images.** Decoding a 1080p PNG takes about 290 ms on this TV, so
  decoding always runs off the UI thread (a placeholder until it's done),
  and a single image is capped at 1280×720 decoded (tile art needs far
  less). The decoder adds about 16 KB.
- **Phone mirror.** `Window::take_snapshot` of the whole window takes
  33–45 ms; a face's image is cropped from it, so no second renderer is
  needed (§ 8.1, Q12).
- **TalkBack: no.** Slint 1.18's `accessible-label` doesn't reach
  Android's accessibility tree on this TV, so screen-reader support for
  panels waits for Slint (§ 5.3 already doesn't promise it); spoken
  approvals (Q13) use Android's text-to-speech directly, not TalkBack.
- **Staging over `content write` works**: 1 MiB and 4 MiB arrive byte
  exact in about 2 s, and the cap stops a larger file. The shell closes its
  end of the pipe without a clean status, so the CLI first sends the
  expected size and SHA-256 (§ 9.1), and the provider keeps the bytes only
  if both match.
- **No file picker on the Philips**: no DocumentsUI, nothing handles
  "open a document". The USB route (Q11) can't use the system picker here;
  on such TVs it would need our own browser and a storage permission, so
  it is offered only where a picker exists, and the Panels page says so
  (§ 9.1). Phone and computer installs are unaffected.
- **Not settled here**: the studio preview route (§ 6.1, Skia helper vs
  "approximate") is checked with the studio in step 6; the safe-look chord
  in step 3.

### 6.7 Runtime `.slint` measured (2026-11-27)

PR #79 proposed skins as `.slint` files run by `slint-interpreter`, plus
Rhai scripts. The step 1 spike gained an `interp-home` mode that builds
the same 18-tile Home as `compiled-home` from source at run time; both
drew the same screen (checked by screenshot) on the Philips:

| Home, 18 tiles | render p50 / p95 | PSS |
|---|---|---|
| compiled (today) | 6.2 / 7.1 ms | about 51 MB |
| `slint-interpreter` | 9.4 / 10.2 ms | about 51 MB |

- Spike numbers, not a BENCHMARK_PLAN.md measurement. The interpreter
  costs about 50% more per frame than compiled code (the JSON route of
  § 6.6: about 18%); both kept a frame every 16.7 ms on this screen. PSS is
  about 51 MB in both modes because this APK links the interpreter (§ 6.6's
  41 MB is the APK without it).
  Compiling the `.slint` source took 30 ms; the spike's library grew to
  21 MB with the interpreter.
- So request 13 is met with data drawn by compiled components first
  (§ 5.7, § 10.4); request 14 adds `slint-interpreter` themes (§ 5.9) at
  about 50% more per frame, measured per theme on the Philips (T5);
  scripts (§ 5.8) stay value-only.

## 7. Knobs and settings (no canvas needed)

- A package declares **knobs**: typed settings with a label, default and
  range — colour, number, choice, toggle, short text, date, time,
  duration, unit choice, time zone, icon (from the built-in set), image
  (a PNG under the image rules), app, list of apps, list of short items —
  with groups and conditional visibility. Bindings read them.
- Knobs are edited on the TV (the panel's row in Settings → Panels, or
  long press on its tile), on the phone (RustTV Remote page), in the
  studio's simple form. Only knobs that bring in nothing new (toggle,
  choice, number, colour, time zone, unit) are one-click look settings,
  on the TV and from the phone alike, because an injected key press is as
  strong as the phone; image, text, app and list knobs need the code on
  the TV each time they change, and are never set from the phone. Over the MCP an
  agent edits a project's knob defaults, never live values on a TV. Changing a knob on the TV is
  RustTV's own look: one click, undone by another, recorded in History
  (owner decision Q3, 2026-11-24).
- **Look lock** (optional, for families): with it on, knob, placement,
  theme and named-look changes, schedules, quiet hours and instances, the
  Panels and Themes pages, and every write in § 9 (install, update,
  re-enable, disable, remove, delete permanently, roll back, restore,
  copy my setup, import, bundled installs) all need a PIN — except the
  accessibility tweaks and the quiet switch, which only make the TV easier
  to use and stay open, like the safe look — (or an unlock from the one
  paired phone the PIN holder names when setting the lock) besides the
  usual approval; the safe look stays open because it
  only takes things away. The PIN is stored hashed with a per-device salt,
  with the wrong-code lockout, stricter than the code's (it is a static
  secret an adb peer could guess blind): PIN entry is `FLAG_SECURE`, and
  after 5 wrong tries the lock waits an hour, doubling each time. A forgotten PIN is reset on the TV — the
  code and hold, then a 24 h wait (`elapsedRealtime`, which counts standby, summed across boots and stored,
  so changing the date does nothing and nightly power-offs only pause it) with a notice on Home that is also sent
  to paired phones, then the code again. The CLI route is a plan that
  starts the same 24 h wait and the same on-TV approval (no route skips the
  TV or the wait). Because the code proves presence, not who, the TV reset
  is **off by default** while the lock is on; the PIN holder may switch it
  on. The CLI route always works (it needs the owner's computer), so a
  forgotten PIN is never a dead end; the last resort is clearing the
  launcher's data, which loses packages, knob values and local History
  (a synced copy on the computer can bring them back). A pending reset is
  cancelled with the PIN or from the designated phone. After a reset the
  lock is off until a new PIN is set (§ 14, § 16 Q16).
- **As built (step 2a, knobs slice, 2026-11-27)**: `meta.knobs` lists
  up to 32 knobs `{name, type, label, group?, visible_if?, default}`
  (each an object); `name` is `[a-z0-9-]{1,32}`, not starting or ending
  with `-`, and distinct. `label`, `group` and option labels are message
  ids that must be in the default table (a package with knobs needs one)
  with no placeholders; messages inside `visible_if` must exist with the
  placeholders they fill. Types so far: `toggle` (yes/no), `number`
  (`min` < `max`; optional `step` > 0, no wider than the range and at
  most 10 000 steps across it, only a hint for the editor; default in
  range), `choice` (2 to 32 `options` `{id, label}` with distinct ids;
  the value is the id, as text), `color`, `text` (`max_length` 1…200
  characters, default 200; the default may be empty and keeps its spaces
  at the ends, like text literals). `min`, `max`, `step`, `options` or
  `max_length` on a knob of another type is `NOT_FOR_KIND`. Bindings read
  `{"knob": name}` with the knob's type, and colours compare as colours;
  `visible_if` is a yes/no expression over knobs declared before it (so
  no cycles) and no sources. Of the types built so far, only text knobs
  aren't one-click. `Knob::accepts` says whether a stored value still
  fits (type, range, options, length), and `Knob::resolve` gives the
  default for one that doesn't; the host stores values (4c). Date, time,
  duration, unit, time zone, icon, image, app and list knobs come with
  their slices (formatting, assets, app sources, `repeat`). Codes:
  `BAD_KNOB` (an empty range; a step that is zero, negative, wider than
  the range or finer than 10 000 steps; fewer than two options; a default
  that isn't an option), `UNKNOWN_KNOB`.
- **Knobs never switch surfaces**: a knob may switch a face's variants,
  but not turn on a surface or a banner declaration; that is placement,
  under the first-activation rule (§ 8.2).
- **Knob values are stored in History** and synced to the computer; the
  knob editor says so for text knobs ("don't put passwords here").
- **Dangling references**: a schedule, named look, quiet rule or
  instance that names a package or theme that is removed, disabled,
  quarantined or restored as "won't draw" shows "unavailable", never
  switches anything on, and is listed on the removal review.
- **Named looks**: a theme, placement and knob values saved under a name
  ("kids", "movie night") and switched from the Themes panel, the phone
  or a schedule; the look lock covers the switch. Looks and schedules are
  created on the TV only; a look can only hold surfaces already activated
  (§ 8.2), so switching to one never activates anything new.
- **Schedules**: a theme pick or a face's placement can follow a time of
  day or a date range (a seasonal theme, a kids' corner only in the
  afternoon); a **quiet** switch silences package banners (never the
  launcher's own notices: a pending PIN reset, a waiting package, the
  safe-look notice), and quiet hours can be set per surface. Schedules need a trusted clock: with no network time
  they pause and the current look stays.
- **Start from a built-in**: "Duplicate as user panel" on any built-in
  panel copies its look into a user package (its built-in-only sources and
  actions dropped); the built-ins ship as editable sample projects in the
  studio and offer their own knobs, so simple changes need no copy.
- With instances (§ 5.3) this is how one panel varies (a clock's time
  zone, a weather city later).
- **Theme tweaks**: on top of the active theme, a small user layer —
  accent colour, text scale, corner radius, grid columns, reduced effects,
  and for low vision: focus-frame thickness and colour, focus zoom, bold
  text, how long banners stay (or until dismissed), and a font choice
  among the TV's own fonts where it has more than one (§ 16 Q6) — that survives
  switching themes where it still applies.

## 8. Placement: every face is the person's choice

### 8.1 Surfaces

| Surface | Faces | Size | Focus | Rules |
|---|---|---|---|---|
| Home grid | tile | 1×1 | yes | move, hide, folders (as today) |
| Hero | hero | the hero area | no | the focused tile's hero |
| Settings list | row | one row | yes | in Settings → Panels' section; never between RustTV's own rows |
| Top-right menu | row | one row | yes | after RustTV's rows |
| Status corner | corner | ≤ 2 glyphs total | no | priority order set by the person |
| Banners | banner | capped card | no | only on a source change the package declares; one at a time; ≤ 1 per package per 10 min and ≤ 1 per 2 min overall; never over pages or locked surfaces |
| Idle screen | ambient | one line or a capped block | no | OLED rules below |
| Home layout | home (themes only) | the window | yes | § 10.4's floors; the first pick needs the code |
| Phone mirror | phone | a tile image | — | rendered by the TV as a PNG and sent as an image (no package JSON reaches the phone page, so no second renderer and no injection) |

**OLED** (the Philips is OLED): ambient faces have a luminance cap, no
static bright areas over a size, a slow pixel shift drawn by the host, and
dim after a set time.

### 8.2 Choosing

- A **Panels** page in Settings lists every panel with its faces and an
  on/off per surface, plus its knobs; long press on a panel tile offers the
  same.
- **Defaults**: built-in panels keep today's placement. A new user panel
  starts with **its Home tile only**; every other surface (Settings, menu,
  status corner, banners, hero, idle screen, phone) is switched on by the
  person. "Off on every surface" is a one-click way to hide a panel without
  disabling it.
- **Approval**: a key press can be injected over adb (SECURITY.md), so
  the **first activation** of a panel on each surface other than its Home
  tile (a banner, the status corner, the idle screen, the menu) needs the
  code — per surface and per instance. Creating a schedule that would
  switch it on there counts as that activation. A new instance, or a
  surface an update adds, starts over. A restore — and a reinstall that
  inherits old placement (§ 9.3) — never trusts approvals recorded in a
  snapshot (the computer can edit it): its review carries the full § 4
  facts for every package, every surface and author key it would bring
  back, and its non-Home surfaces need the first-activation code — one
  code per package covers every surface of it the review lists (that is
  the per-surface activation: each surface is shown, then approved).
  Image, text, app and list knob values come back at their defaults
  unless listed on that review. Under the look lock the PIN is needed too. After
  that, switching it off and on again, reordering, one-click knob values
  and picks among installed themes are look settings: one click (owner
  decision Q3, 2026-11-24). A placement listed on the install review is approved with the
  install.
- Placement survives theme changes (the Home order is a flat list; a
  theme decides how it is laid out, § 10.4, but never reorders or hides
  it, and tweaks that don't apply to a Home document, such as grid
  columns, are ignored). The order file and History keep it
  (§ 9.5).

## 9. Installing and managing packages

### 9.1 Getting a package onto the TV

All routes end the same way: the bytes are **staged inert** in the
launcher's private storage (copied, then hashed after the copy; nothing
parsed beyond the meta, nothing drawn), one staged package at a time,
rate-limited per source (N per hour, with the wrong-code lockout logic);
a newer staging replaces the older one, and a staged package expires
after 24 h of the device's monotonic clock or at reboot, whichever is
first. Staging needs no approval on
the computer on purpose: the bytes are inert until the person on the TV
approves. A review is pinned to the hash it opened with; if the slot
changed meanwhile, apply is refused and the review says "replaced".
The waiting row shows only facts the launcher derived — where it came
from ("computer over adb", "phone: <paired name>", "file"), its size and
a hash prefix — nothing parsed from the file, so a crafted staged file
can't crash Home at start. The rate limit also counts replacements of the
slot, so a peer can't keep the slot busy. The person opens it
in Settings → Panels, which runs the validator **under a crash marker**
(§ 9.6) and shows the review.

The first install release ships every route at once (owner decision Q11,
2026-11-24): the computer (4b), the phone and a file on the TV (4e), so
people without a computer can install from day one. Steps 4b–4e are
released together, not one by one; if the step 1 spike finds no file
picker on the Philips, the USB route ships where a picker exists and the
Panels page says so elsewhere.

- **From the computer**: `rusttv panel install <file>` / `rusttv theme
  install <file>` (also the studio's Install button and the MCP's plan
  tool, which returns the command a person runs). A new typed door
  command stages the bytes, streamed through the content provider's file
  descriptor (no pushed path, so no SELinux or shared-storage questions;
  new risk class, § 14). The provider checks the caller is the shell uid,
  enforces the size cap while streaming, and keeps the bytes only if their
  length and SHA-256 match what the command sent first (the shell doesn't
  report a clean close, § 6.6). Whether
  `content write` over `adb exec-in` (binary-safe stdin) works on the
  Philips is checked in the step 1 spike; the fallback is the same typed
  command sending numbered chunks (under the binder size limit) through
  `content call`, reassembled, capped and hashed by the provider
  (`/data/local/tmp` is not an option: apps can't read it under SELinux).
- **From the phone**: RustTV Remote's page can upload a package (a new
  message type for the phone role with its own caps), staged the same
  way. The phone page is plain HTTP, so it can't be an Android share
  target: the person opens the page and picks the file (a native share
  target would be a new app with its own SECURITY.md entry, later).
- **Without any file**: the APK ships a few optional themes and panels,
  off until the person installs them from Settings → Panels. Installing
  copies them into the package store, so they load from storage with user
  privilege and follow every user-package rule (the code, the
  first-activation rule). A newer bundled version in an APK update is
  offered as an update with the code and hold, never applied silently.
- **From a file on the TV** (USB stick): only the picker in Settings →
  Panels; no "Open with" or other exported entry point in v2, so no other
  app on the TV can fill the staging slot.
- **Linux**: the CLI writes into the launcher's data folder's staging
  area; the desktop launcher shows the same review.

### 9.2 Approving

- **Install** and **update**: the review (§ 4) with the code and hold.
  "Approve only" is not enough: one key press can be injected over adb.
- **Update**: strictly higher version only, always with the code and
  hold; the review shows what changed, and a new source, face, surface,
  action or a different author key is shown in red.
- **Remove** and **disable**: Approve on the review, no code: they only
  take things away and are undone by reinstalling from History (§ 9.5);
  a removed package's bytes, placement and knob values are kept for at
  least 30 days and the review says "can be reinstalled until <date>"
  ("Delete permanently now" is offered too; it needs the code and also
  drops the side-store copy; the next `history sync` deletes the
  computer's copy only for hashes the TV-written `[launcher]` section
  lists as deleted and no pinned snapshot needs),
  so an injected press is a nuisance, not a risk (like the safe look).
  SECURITY.md has no such tier today, so this is a dated decision (§ 14,
  § 16 Q18).
  Removing the theme in use loads the emergency Home until the person
  picks another.
- The approval binds to the package hash, its author key, its declared
  sources, faces, surfaces and actions; apply re-hashes the staged copy
  first.
- **Apply** follows HISTORY.md § 4.1: a TV journal first, each step
  verified, rolled back or forward on failure; the lockout and
  debuggable-build display-only rules are inherited.
- **Re-enabling** a disabled package and **rolling back** to the previous
  version take the full approval (they bring content back).

### 9.3 Disable, remove, roll back

- **Disable** keeps the package and its placement, hides it everywhere.
- **Remove** deletes it; its slots close up and come back on reinstall
  (any version) only when the id and author key match (History's hash is
  used only to find the bytes; unsigned packages never inherit placement);
  otherwise
  the package starts Home-tile-only with default knobs. The previous version is kept for one **roll back** step.
- An update that drops a face, surface or knob that is in use lists it
  on the update review; its placement closes up, and schedules or named
  looks that name it show "unavailable" (like dangling references, § 7).
- Knob values migrate by name across versions; a value the knob no
  longer accepts (new type, range, options or length) resets to its
  default, with a note on the update review.

### 9.4 Themes on the TV

The **Themes panel** (built in): installed themes with previews (rendered
by the launcher), pick one, its modes (manual, follow the system, or a
clock range), theme tweaks (§ 7), remove. Themes with runtime code are
marked "contains code" and shown disabled, with the reason, while "Allow
packages with code" is off; turning it off while one is in use loads the
emergency Home. With nothing installed it says so and how to install a
theme from the computer.

### 9.5 History

- Separate format bumps per feature, each written only when used, all
  under the TV-written `[launcher]` section (the door already refuses
  `[launcher]` in observations, so an adb peer can't forge them):
  `[launcher.panels]` (id, version, hash, author key, enabled, instances),
  `[launcher.themes]` (id, version, hash, mode, in use), placement, knobs,
  schedules.
- **Package bytes live in a side store** next to the history, not in the
  git tree (`state.toml` stays the only file per commit). `history sync`
  copies the side store to the computer with the rest (an R0 read, size
  capped, re-hashed on receipt); `restore-tv` puts it back, and an import
  fills the side store but never installs or enables anything.
- **Restore** (and "copy my setup" to another TV) re-validates stored
  bytes under the current validator **and the target's capability set**
  (§ 12.3), and hashes before using them, and lists them on the review; a
  package that doesn't fit is restored disabled with the reason ("won't
  draw: rotation"). Pinned author keys travel only through the full
  review. A missing hash
  restores everything else and shows "needs reinstall: name".
- **A new or reset TV without a computer** starts fresh: every restore
  route needs the synced copy on a computer. The plan says so on the
  Panels page ("back up to a computer to keep your setup"); a phone
  backup is a later decision (§ 16 Q11).
- Retention is a convenience, not a promise (§ 9.2 "Delete permanently
  now"). Removed packages are pruned after 30 days at the earliest
  (§ 9.2) unless a pinned snapshot needs them; the side store has a quota,
  and a remove that would have to evict a package within its 30 days
  needs the code instead.
- **Downgrading the launcher** below a format it needs: the old launcher
  refuses the new snapshot (as today), keeps the files, and starts with
  the emergency Home and a notice.

**After a launcher update** every stored package is validated again at
the first start; one that fails a tightened limit or uses a removed
source is disabled with a reason (not deleted, not quarantined) and the
Panels page says why. The launcher reads format 2 for at least the next
two major format versions; the studio migrates project files forward.

### 9.6 Crash quarantine

The launcher keeps a marker naming each user package that is **at risk**
(being validated, loaded, opened as a page, or changed or switched on
since the last clean start — not every tile that merely sits on Home) and
clears it when that is over. A cheap "last package drawing or
evaluating" slot, written by the panic hook before the abort, names the
package that was busy (an expression, a source change, a banner, an event
handler, a script call) even if
it only sat on Home; the launcher runs `panic = "abort"`, so a crash at any
of those moments leaves the marker. At the next start the marked packages
are disabled one at a time (newest change first, then highest cost
estimate). One rule on every Android version: an exit the panic hook
named counts at once; otherwise two exits in a row **while RustTV was in
front or a user banner or source evaluation was running** with the same
package marked count (a foreground marker, so the
usual background low-memory kills never count, and a single power cut
disables nothing). On Android 11+ `ApplicationExitInfo` refines it: a
force stop never counts, and a foreground low-memory kill counts only
three times in a row. A
crash during staged validation deletes the staged file,
and after two crashes in a row every user package is disabled and the
emergency Home loads (the safe-look notice says what happened). Same for a
theme. A code theme's compile and every frame it draws keep its marker
set (the theme in use is always at risk); compiling runs off the UI
thread under a time and size cap, and two marked exits in a row disable
it.

### 9.7 Profiles and several TVs

Packages, placement, knobs and theme are per launcher data (one set per
Android user). The studio's device list shows what is installed on each
TV to the person, read by its own door command in a new **private
read** class (like RL, above R0, so the MCP can never run it; shell uid
only; knob values live in the same store, so they stay on the
person's screen too); the MCP sees only the renderer feature set, limits,
fit metrics and launcher version (§ 12.3). History's "copy my setup"
between TVs carries packages too.

### 9.8 Later

Signed packages and a catalog (ROADMAP.md "Phase 7+"), **setup bundles**
(`.rtvsetup`: panels, a theme, placement and knobs in one review), sound
for navigation (host-owned clips in a theme), icon packs (§ 10.3), phone
remote layouts.

## 10. Themes

### 10.1 Tokens

A theme is a package with its Home (§ 10.4) and **tokens** in layers (the W3C Design Tokens
format, DTCG): primitives (colour ramps, sizes) → semantic tokens (`bg`,
`surface`, `text`, `text-dim`, `accent`, `focus`, `danger`, spacing,
radii, elevation and shadows, type styles, motion durations and easings,
focus zoom, sway, glow, page transition) → optional component tokens
(tile, row, page, banner, folder). Semantic tokens only alias primitives.
Each token says where it is used, `since` and `deprecated`.

- **Partial overlay**: a theme sets what it wants; a missing token takes
  the fallback alias every new token must declare (an existing semantic
  token, so a light theme stays light), else the launcher's compiled
  token default (not a theme); the contrast
  floors run on the merged result at each load and after a launcher
  update, and a theme that fails is disabled with a reason and the
  emergency Home loads; an unknown token (from a newer launcher) is ignored with a
  warning, so themes move between versions.
- **Modes**: light, dark, high contrast, night; picked manually, following
  the system, or by a clock range.
- **Floors enforced on the TV**: contrast of text pairs and of the focus
  indicator (WCAG AA), minimum text size, ranges for every layout token
  of today's compiled Home (a Home document has § 10.4's floors instead)
  (a theme can't hide the grid or the focus). A theme below a floor is
  refused at install.
- **Ranges for every token**, motion included (focus zoom, sway, glow,
  durations), and no alias cycles (checked like expressions).
- **Themes carry tokens, assets and a Home document (§ 10.4)** — no panel overrides; a theme
  can't change what a panel says, only how everything looks. Its assets
  go into fixed slots: Home background, folder background, focus frame
  (nine-slice), tile mask; a reduced-motion alternative applies.
- **Import**: DTCG token files and Figma variables become a theme in the
  studio; an unsupported brand font is flagged with the nearest font from
  the set.

### 10.2 What a theme reaches

| Surface | Theme can |
|---|---|
| Home screen | tokens and the Home document (§ 10.4), within its floors; a `runtime: code` theme draws it with `.slint` (§ 5.9) |
| No theme installed | the emergency Home; Settings and the launcher's pages use the compiled token defaults |
| Folders, tile menu, top-right menu, banners, idle screen | tokens, and the slots § 10.4 lists (a `folder` document, the `menu` slot, banner tokens, the `ambient` face) |
| Settings, History, Optimizer, Remote pages | tokens only (layout stays the launcher's) |
| User and built-in panels | tokens (panels read tokens) |
| Review screen, code entry, pairing and trust screens, Recovery, safe-look notice, update confirmation | **nothing** (locked) |
| Android's own UI, other apps, the on-screen keyboard | nothing (not ours) |

### 10.3 Wallpaper, art and later items

- The wallpaper is the person's; a theme may suggest one. The accent can
  follow the wallpaper (the launcher already derives colours with
  `material-colors`).
- **Tile art overrides** (a different banner for one app) and **icon
  packs**: later (format 2.x), as theme assets with the same image rules.
- **A photo frame** in v2 is a studio-built package of up to N images; a
  "your photos" source is later, with its own tier and review.
- The launcher's literals become tokens before themes exist (§ 15 step
  3).

### 10.4 Home as a document (owner request 13)

- **Every theme carries `home`** (owner request 14): a document for the
  face `home` (themes only; its root fills the window, § 5.7) with the
  same nodes, bindings, states, events, navigation and motion as panels,
  or for a `runtime: code` theme its `.slint` Home (§ 5.9).
- **No theme is built in** (owner request 14): every theme, the
  default-looking one included, is installed like any package (through
  `rusttv-mcp` and approved on the TV); none ships inside the APK. The
  launcher keeps only a plain **emergency Home**: the apps and Settings,
  drawn with the compiled token defaults and the person's accessibility
  tweaks (text scale, high contrast), used on first start until a theme
  is installed, when no theme loads, when a theme fails a floor or
  crashes, for the safe look and for Recovery. Until data themes exist
  (step 2f4), today's compiled Home stays the look.
- **Slots** the theme places and styles, never changes the content of:
  - the app list (the windowed host list, § 5.7) and folders (a `folder`
    document, or tokens);
  - the panel tiles the person placed (§ 8) sit in the app order as items
    of kind `panel`, drawn through a `panel` slot in the item template;
    the theme sizes that slot and the face is picked by size (a dock icon
    uses `meta.icon`, an icon from the launcher's built-in set, never a
    package image, else a generated icon and name), but it never changes
    what the panel draws, its user badge or its focus look, and never takes
    panels out of the person's order;
  - the hero (the focused tile's, § 8.1), the clock (the minute tick;
    seconds only while visible and not idle, at most once a second) and
    the status corner (the person's priority and its 2-glyph cap stay);
  - the tile menu and top-right menu: a `menu` slot that repeats the
    launcher's entries, placed and styled, never remapped or removed;
    banners (tokens for how they enter, where and for how long, within
    § 8.1's caps) and the idle screen (the `ambient` face, OLED rules).
- **Layout samples**, not modes: grid, shelf, dock, top tabs, carousel,
  floating pills, sidebar, blade and zen (several from PR #79) ship as
  editable starting points; step 2f's acceptance table lists each with
  the primitives it uses and a Philips screenshot test.
- A theme may declare knobs (§ 7) that drive its tokens, variants and
  list options (columns, density, labels, hero on or off), and named
  images (within § 5.1's caps) that image nodes pick by expression (a
  wallpaper per time of day). `wallpaper.blurred` (blurred once per
  wallpaper by the host) gives frosted glass where background blur isn't
  drawn (§ 6.2).
- **Floors, enforced while Home runs**, not only at install:
  - every variant of a `home` document has one plain `apps.list`
    repeat: no `sort`, `filter`, `from`, `count`, `group_by` or `mask`, and
    a variant switch that would remove it is refused;
  - the floors are minimums, not bans: an item may dim (opacity at least
    0.3, the focused one 1.0) and scale (a `transform` scale counts as
    size) but stays at least the minimum size and on screen, and an idle
    fade dims the list to the minimum, never hides it; the focus frame and
    the Settings way in ignore `visible`, opacity and size bindings;
  - the host draws the app list, the focus frame and the Settings way in
    above any node the document draws over them; focus lands only on a
    node that is drawn, on screen and at least the minimum size, and the
    host scrolls it into view;
  - the all-apps key opens a launcher-drawn list of every app the person
    hasn't hidden, whatever the document does;
  - a contrast guard remaps text colours at run time when bound colours
    fail the floor (as in high contrast, § 5.3);
  - a launcher-drawn "custom Home · name" mark and the launcher's own
    notices (pending PIN reset, a waiting package, the safe-look notice)
    sit in a slot the document can't cover or move;
  - Back follows § 5.7.

  If a floor fails, or the document goes over budget, crashes or trips
  the watchdog, the emergency Home loads with a notice, recorded in
  History (§ 9.6).
- **First pick**: picking an installed theme for the first time needs
  the code, like a first activation (§ 8.2): installing approved the
  package, the pick approves it taking over Home (on a fresh TV the
  install review offers "use it now", so one code covers both); a schedule or
  named look that would switch to it counts as that pick. Later picks
  among installed themes stay one click.
- **Budget**: until step 2f measures it (18, 50 and 100 apps with the
  three heaviest samples), § 5.1's caps apply to the Home document.
- Accessibility: the label is an expression (an item's position, "3 of
  18"); TalkBack support stays as § 5.3 says. RustTV profiles (a kids'
  Home behind a PIN) are a separate decision (§ 16 Q20).

## 11. Live preview on the TV (optional)

- The studio streams the document; the TV draws it with the real renderer
  in a **preview layer** marked by the launcher ("Preview from <computer>",
  un-coverable), never over locked surfaces, keeps nothing (memory only:
  no files, History or settings), and ends on Back, after inactivity or at
  a hard time cap.
- **Every session is started on the TV by a person**: a request always
  waits as a row in Settings → Panels (one pending at a time, rate-limited
  per peer, expires after 10 minutes, the computer's name under the
  review-name rules with its key fingerprint) (the TV never pops a prompt for an
  outside request, § 4), and opening it shows a locked,
  `FLAG_SECURE` prompt ("Allow preview from <computer> for 10 minutes?")
  confirmed with the 4-arrow code (keys over adb prove nothing); a standing
  pairing alone is not enough. The session is bound to a one-time nonce
  shown on that prompt; streams from any other peer are refused.
- Preview content runs inert: no actions, mock data only, the full
  validator and limits; per-message and per-session byte caps and a rate
  limit.
- **Transport** (§ 16 Q5): over adb (a CLI-only typed command; never an
  MCP tool) or a **preview-only role** on RustTV Remote's listener (the
  role is fixed and shown at pairing, checked per message; a preview key
  can do nothing else).
- Themes and placement can be previewed the same way.
- SECURITY.md gets its section before any of this is built.

## 12. The studio

### 12.1 Architecture

- **One typed command registry** for every edit (menus, shortcuts, the
  palette, the CLI, the MCP, plugins), with batches as one undo step, a
  persistent document with stable ids, atomic saves and autosave, no
  `unwrap`/`expect`, panicking commands rolled back.
- **Commands that reach a TV are not in the registry**: install, preview
  and device commands live only in our own binary's UI and CLI; a test
  enumerates the registry, MCP tools and plugin API and fails if any of
  them can reach those.
- **Crates**, layered: `rusttv-panels` (format, validator, layout model,
  expressions, shared by TV, CLI and studio), `rusttv-panels-ui` (the Slint
  components of § 6), `studio-engine`, `studio-mcp`, `studio-ui`. The
  launcher's dependency tree must not contain the studio's (egui,
  FigCraft, and wasmi unless Q19 picks WASM for scripts): a CI check enforces it.

### 12.2 Workspace and canvas

- **Docking**: tool windows (layers, assets, components, inspector,
  tokens and themes, knobs, sources and mock data, preview, devices,
  problems, MCP activity, history) dock, split, tab, float in their own
  window (also on another monitor), minimise to a side bar; layouts are
  saved, named, switched and reset. Lead candidate: egui with egui_dock
  (FigCraft's toolkit); gpui if egui falls short (spike, step 0).
- **Canvas**: infinite, zoom and pan, frames per face at real TV size,
  selection, resize and rotate handles, smart guides and snapping (pixel,
  `u`, layers, grid), align and distribute, rulers and measurements,
  copy/paste between panels, drag from libraries, nudging, layer lock and
  hide, undo, find, comments; overlays for the TV safe area, the focus
  zoom footprint and a page's D-pad order.

### 12.3 Devices and targets

A Devices window: the desktop launcher (built in), and TVs (adb or a
preview pairing), with the launcher version and its **renderer feature
set**, limits and fit metrics (read by an R0 door command, § 14), cached
with their age so the studio and the MCP can check a panel without the
TV connected; the installed-package list is shown to the person only,
never to an MCP tool. The studio checks a panel
against each target and shows "won't draw on <TV>: background blur".

### 12.4 Scenarios and checks

- **Scenarios**: named mock-data sets (update available, no phones,
  three phones, long text, empty, error) and a matrix preview: face ×
  language × mode × data state × text scale × reduced effects × theme.
  The MCP can export the matrix as screenshots.
- **Problems** (lint codes, machine-readable so agents can fix them):
  unsupported on target, over a limit or the budget, text fit (advice),
  contrast, missing focus look, focus order holes, RTL issues, literal
  colours, unused assets, missing translations and plural forms, unbound
  or undeclared sources, a Back trap, apps or focus islands out of reach,
  a timer faster than 1 s, a flash, a script over budget, an app item whose
  name and image come from different items, id collision, version not bumped, licence
  missing, metadata leaks (author paths), placement conflicts.

### 12.5 Project files

A project is a **folder of JSON and assets** (one file per panel or theme,
git-friendly, stable ids); the zip is only the export. A package can be
opened and edited by anyone (it contains the whole document), so a shared
file is never a dead end.

### 12.6 MCP

- stdio; to drive a running studio, a stdio bridge that spawns the engine
  (no open port), or loopback with a per-launch token in a 0600 file,
  Host/Origin checks and no CORS.
- **Typed tools**: open, inspect (trimmable), schema, supported
  properties per target, examples, create, set properties, batch, undo,
  redo, validate, export (paths confined to the project, opened once),
  screenshot (mock data), scenarios, translate drafts; an **allowlist** of
  further registry commands (not "everything").
- **History tools** in `rusttv-mcp` skip every `[launcher.panels]` and
  `[launcher.themes]` section and everything that names user packages or
  holds their values (knobs, placement, schedules, named looks, instances,
  theme tweaks and modes, the side store) — so the private read (§ 9.7) isn't undone through
  History; the files stay on the person's computer, and an agent with
  plain file access can still read them (the MCP is the boundary, not the
  disk).
- **Plan tools** live in `rusttv-mcp` (MCP.md), not in the studio's MCP:
  `panel_install_plan` and `theme_install_plan` return a dry run from the
  package file alone; plans for update, remove, disable, placement and
  theme pick only return the CLI command a person runs, and the dry run
  from installed state is shown by the CLI to the person, never to the
  MCP (no installed-state oracle). The studio's MCP has no device access at all; no
  install, preview, approval or TV write through any MCP.
- Paths: symlinks refused, files opened without following them.
- **No input injection** (no simulated mouse or keys). Document revision
  checks on writes, so an agent and a person don't overwrite each other.

### 12.7 Imports and exports

- **In**: our packages; a FigCraft document (frames named by face, a
  published mapping spec); Figma JSON and Penpot via FigCraft if we take
  its importers; SVG → path nodes. Imports run under the same limits and
  never read paths named inside a document.
- **Out**: our package; PNGs of every face; the theme as DTCG JSON.

### 12.8 First run and the studio itself

Installer per platform (signed and notarised on macOS, signed on
Windows, AppImage or Flatpak on Linux); updates through
the same minisign-verified channel as `rusttv update` (the request is the
only thing sent); a sample project and the built-in panels as samples; the
desktop launcher as the first target; no internet needed ("translate
drafts" is offline or says which service it calls). UI in English first,
then the launcher's languages, with a mirrored UI for Arabic; text nodes
and string tables need correct bidi and shaping (Arabic, Devanagari,
CJK input), which is on the step 0 pass/fail list; keyboard use
everywhere; high DPI. The
canvas isn't screen-reader friendly; the form (6a) and the MCP are the
accessible routes.

## 13. Design system and design file (request 11)

### 13.1 Tokens and components

- One DTCG token file as the source; primitives → semantic aliases;
  each token describes its use; generated outputs (the studio's Rust
  theme, a TOKENS.md table) are never hand-edited, and CI fails when they
  are stale.
- Components each have a spec (rules, states, focus, contrast) and a
  gallery window with screenshot tests.
- Light and dark modes from the start.

### 13.2 The design file

The studio's screens are designed before coding in a design file
**generated from the tokens by scripts** (FigCraft, headless): variables
from the tokens, components from the specs, screens built in rounds and
reviewed by the owner. The file is rebuilt, never hand-edited.

### 13.3 The same system for RustTV themes

The default RustTV theme is expressed as a theme package from the same
token structure (§ 15 step 3).

### 13.4 Option B limits

If FigCraft is the editor: FigCraft's own MCP (which has input injection)
is not connected to an agent alongside ours; plugins start with no
capabilities; install and preview never go through FigCraft.

## 14. SECURITY.md changes (before the steps that need them)

- A new **staging** door command and risk class (inert bytes, caps, one at
  a time, nothing parsed or drawn before the person opens the review), and
  the install/update/remove/disable plans with their approvals.
- Level 2 lifted from "parked" by a dated owner decision (PANELS.md
  decision 11).
- Locked surfaces, review-screen content rules, privilege by load
  location, user-face marking, crash quarantine, the safe look.
- Sources and tiers, and any new Android permission (`ACCESS_NETWORK_STATE`).
- The phone upload and TV-file install routes (a paired phone can't stage
  packages today), with their channel labels and caps.
- Linux: approval as a convenience, validation at every load.
- An "Approve only, no code" tier for actions that only take things away
  (remove, disable, the safe-look CLI), and the safe-look chord exception.
- An R0 door command for the renderer feature set, limits and fit
  metrics (built in 2c, listed in the door tests); the private-read class
  and its installed-list command (built in 4b, in its door tests); the look lock's PIN storage, lockout and reset; first
  activation of non-tile surfaces needing the code.
- Sources and tiers; the phone and preview get mock or mirror-only data.
- The preview session and its transport; the studio MCP's scope.
- Full customisation (step 2f): the Home document, its run-time floors,
  the reserved keys and the all-apps and Settings ways in, events and
  timers, launch counts as an opt-in launcher-kept store (and their
  exclusions), the icon palette source, `open_item` and the launcher-drawn
  name beside the "custom Home" mark.
- Level 3 by the owner's dated decision (2026-11-27, Q9 and request
  14): themes with runtime code (§ 5.9: the interpreter, the host API and
  its import allowlist, the floors around it) and scripts (§ 5.8), the
  "Allow packages with code" setting and its tier; the emergency Home and
  first start with no theme. Before T5.
- Supply chain: FigCraft and new decoders pinned, vendored with notices,
  `cargo deny` source rules, audited for network calls; each new
  dependency justified in its PR.

## 15. Build order

Each step: its own PRs, reviews split by dimension (round 1 in parallel
with checklists, at most two rounds), required checks, BENCHMARK_PLAN.md
measurements where it touches the TV, a real-TV check where the TV is
involved, and the safety gates listed. Every step that connects to the
Philips or installs anything on it (spikes, measurements, real-TV checks,
the measurement APK) starts with the owner's go-ahead in that session.

0. **Spike: the editor** (in parallel with 1; blocks steps 5 and 6).
   FigCraft as the base against the § 3 pass/fail list; egui_dock (split,
   tab, float into a second OS window); our preview beside it. Output: A,
   B or C.
1. **Spike: the TV renderer** (and staging bytes over `content write`,
   and whether the Philips has a file picker for a USB stick — 4e depends
   on it). It runs on a throwaway spike APK (its own
   applicationId, the debug key, no HOME filter) with a typed spike
   provider that only accepts fixture bytes, driven by a typed CLI test
   command — never a free-form adb string; nothing from it ships. Being
   no Home app, it gives per-node and per-effect costs and memory, not
   Home cold start or key handling: the safe-look chord is proven in
   step 3 on the real launcher, and Home-level numbers come from the
   release APK in 2e. Generic components drawing a v2 tile and
   page from JSON vs the compiled panel, on the Philips: cold start, PSS,
   first draw, frame times, per-node cost; the support tiers (§ 6.2)
   confirmed per renderer; the `png` decoder's size and cost; TalkBack
   through `accessible-*`; offscreen rendering of a face to a PNG (for the
   phone mirror; until it works the mirror shows a launcher-drawn tile with
   the name, or the thumbnail); the studio preview route (§ 6.1); the
   slint-interpreter route for comparison (measured later, § 6.7). Output: renderer route, node and
   effect limits, the preview route.
2. **Format and renderer**, in slices:
   **Owner decisions before step 2**: Q4 (lift "parked"), Q7 (open an
   app), Q14 (instances) and Q15 (decoded images) change the format and
   the renderer, so they are answered and dated before 2a starts
   (all four answered 2026-11-24); Q6 (fonts), Q13 (accessible
   approvals), Q17 and Q18 before step 3; Q3, Q11 and Q16 before step 4
   (all answered 2026-11-24); Q12 before 4e; Q5 before step 7.
   - 2a `rusttv-panels`: types, validator, JSON Schema, expressions,
     message tables, zip reader, fuzz targets (no UI; meta, document and
     message tables, expressions and the zip reader done 2026-11-24,
     knobs and repeat 2026-11-27); `rusttv panel
     pack` for people, so packages can be made and tested before the
     studio (unsigned until 4b). Until 4b a package reaches only the
     desktop launcher, or a debug-build TV loader that is compiled out of
     release APKs and never run on the Philips without the owner's
     go-ahead. Release-build renderer measurements (steps 2b–2d; step 1 has its own
     spike APK) use a
     measurement variant of the release APK whose loader reads only the
     fixed test fixtures from BENCHMARK_PLAN.md, never a file from
     outside the APK; it reports to its own log, and gives renderer costs
     only. Home-level parity (cold start from Home, memory as Home, key
     latency) is measured in 2e on the release APK itself, whose built-in
     panels in v2 load from inside the APK (no outside loader), and 2c's
     door command is checked on a release candidate of the real launcher.
     It has its own applicationId and the debug key, no HOME intent
     filter and no exported door provider
     (never the release key, so it can't update the real app), and the
     release workflow never builds or publishes it.
   - 2b renderer for tile and page faces, behind a flag; built-ins
     unchanged; it reads today's five colours through a small shim until
     step 3's tokens replace it (so 2b–2d don't wait for step 3).
   - 2c strings and fallback at runtime; fit metrics capture; the R0 door
     command that reads the capability set, limits and fit metrics (its
     SECURITY.md entry lands with it, ahead of 4a).
   - 2d function faces (rows, corner, banner, idle line, hero) as
     launcher slots.
   - 2e (after step 3) built-in panels moved to v2 with no visible change.
     The release APK carries both paths behind a flag; BENCHMARK_PLAN.md
     measures each as Home on the Philips (cold start from Home, PSS,
     first draw, key-to-frame latency, sway CPU), and v2 becomes the
     default only when every metric is within the noise band of the
     compiled path. `convert.rs`, schema 1 and the compiled path are
     retired only after install ships (4b).
   - 2f (after step 3 and 2e) full customisation (owner request 13, § 5.7,
     § 10.4), in slices: 2f1 layout, scroller, pages, `repeat` options and
     item values; 2f2 states, events, navigation and the reserved-key
     filter; 2f3 keyframes, stagger, masks, transforms and the other
     effects; 2f4 the Home document, its floors and slots, the layout
     samples with their acceptance table, and Home's node budget measured
     on the Philips (18, 50 and 100 apps). Its SECURITY.md entries (§ 14)
     land first.
3. **Tokens**, in slices with the screenshot checks byte-identical after
   each, starting with the SECURITY.md entries for locked surfaces, the
   safe look, its CLI plan and the Approve-only tier — blocked on the
   owner's dated answers to Q17 and Q18 (ahead of 4a, like the capability
   command;
   disabling user packages is tested from 4b, and pausing schedules,
   named looks and tweaks is added with them in 4c and 4d);
   first the locked surfaces on `LockedTheme` with the hostile-theme
   test, the safe look with its key chord proven on the Philips and one
   other remote; then colours → type scale and radii → spacing and sizes → motion →
   Home layout tokens (for today's compiled Home, the default look until
   data themes exist); the Settings way in and
   the all-apps key, decided with the safe-look chord; `Theme` driven from Rust
   (system fonts, Q6). Step 3 ends with today's compiled Home and the
   emergency Home; the first data theme package comes after 2f4 and 4b,
   with T3.
4. **Install and placement**:
   - 4a the remaining SECURITY.md sections (§ 14) not already written in
     2c and 3, with the owner's dated decisions.
   - 4b staging, panel install/update/remove/disable, review rules,
     History side store and formats, crash quarantine, CLI commands, MCP
     plan tools, the author key and `rusttv panel sign` (it asks for the
     key's passphrase in an interactive terminal; keychain keys are
     passphrase-protected too. That is a speed bump, not a guarantee — an
     agent with a shell or a CI file key can still sign — which is why
     § 5.2 says a signature means only "same author"), a minimal Panels page (the waiting row and
     the review), tests (every door command above R0 listed).
   - 4c the full Panels page, placement per surface, knobs, look lock,
     named looks.
   - 4d themes: install, the Themes panel, floors, theme tweaks,
     `theme_install_plan`.
   - 4e install from the phone and from a file on the TV; the `phone`
     face as a TV-rendered image (after Q12 and the step 1 offscreen
     result).
5. **Design system and design file** for the studio (§ 13), after step 0
   (headless FigCraft only under A or C; under B the design file is our
   own JSON).
6. **The studio**, per the step 0 decision, in usable slices:
   - 6a open a package, edit its text, colours and knobs in a form (under B
     an egui form in our own shell),
     desktop preview, scenarios, problems, export, install; the test that
     no registry command, MCP tool or plugin can reach install or preview;
   - 6b canvas and inspectors;
   - 6c docking workspace;
   - 6d components, variants, tokens and themes editing;
   - 6e sources, bindings, motion;
   - 6f MCP.
7. **Live preview** (§ 11), after its SECURITY.md section.

   **Theme track with #79** (owner request 14), its own PRs, alongside
   steps 2–4, in this order: T1, T4, T3, T2, then T5 (after 2f4's floors
   and the level 3 SECURITY.md entry; it builds the "Allow packages with
   code" setting), T6, T7:
   - T1 theme format: `runtime` in meta, `code/` and `scripts/` entries
     (us).
   - T2 Settings → Theme listing installed theme packages, translated
     (#79's author; after T3, though its UI can be reviewed earlier).
   - T3 install over MCP: `theme_install_plan`, staging, the review on the
     TV, History (us; parts of 4b and 4d). Owner's split (2026-11-27): T3a
     the SECURITY.md rules (section "Packages: staging and install", risk
     class RS), T3b `rusttv theme install` and the inert staging slot with
     its waiting row, T3c the review with the code, the package store and
     History, T3d the MCP tool.
   - T4 safety net: the emergency Home, the safe-look key (us; with the
     step 3 safe look and its SECURITY.md entries).
   - T5 the runtime-code engine and host API behind "Allow packages with
     code", measured on Android (#79's author; the level 3 SECURITY.md
     entry first).
   - T6 #79's themes repacked as packages, outside the launcher.
   - T7 Rhai scripts (after Q19).
8. **Later, each with its own decision**: scripts (§ 5.8, after step 2f
   and Q19), web data sources, settings actions, input switching and
   global keys, signed sharing and a catalog, setup bundles, larger
   tiles, shared transitions, icon packs and tile art, sound, more faces.

## 16. Open questions for the owner

1. **Editor (§ 3):** A, B or C, after the step 0 spike. — Spike done
   (§ 3.1); **recommended: C**, a pinned fork of FigCraft. — **Decided
   2026-11-24: C.**
2. **Name and home:** "Panel Studio"? In this repository until it has its
   own release cycle?
3. **One click for look settings** (from the PR #59 discussion): may knob
   changes, placement switches of installed panels and theme picks apply
   on one click (undone by another, recorded in History)? Install,
   update, remove and anything new keep the code and hold. — **Decided
   2026-11-24: one click.**
4. **Level 2:** lift PANELS.md decision 11 ("parked") with a dated
   decision, so user packages can be built? — **Decided 2026-11-24: lift
   it.**
5. **Preview transport:** adb first (no new network surface), the
   RustTV Remote preview role, or both?
6. **Fonts:** keep the system fonts, or bundle a RustTV font set covering
   the 21 languages (APK size)? Fonts in packages: never (recommended). —
   **Decided 2026-11-24: system fonts; no fonts in packages.**
7. **"Open an app" from a panel** (reverses PANELS.md § 5): allowed, from
   the launcher's visible app list only, after a key press, excluding
   Settings, SystemUI, installers and the launcher itself (§ 5.5)? —
   **Decided 2026-11-24: allowed, and only apps named on the install
   review.**
8. **Tiles bigger than 1×1** (PANELS.md decision 3): stay 1×1 in v2?
9. **Code panels (level 3) and settings actions:** later, each its own decision? — **Scripts decided 2026-11-27: later, opt-in, on top of
   data-first full customisation (§ 5.8, Q19).** Settings actions stay
   open.
10. **Web data sources** (weather…): later, each its own review?
11. **Install from the phone and from a file on the TV:** in the first
    install release (step 4e) or later? — **Decided 2026-11-24: in the
    first install release.**
12. **Phone mirror of user tiles as an image** rendered by the TV (no
    second renderer, no package JSON on the phone)?
13. **Accessible approvals**, designed with SECURITY.md: an alternative to
    the 2 s hold for people who can't hold a key, and a non-visual way to
    get the code (spoken by the TV) for blind people? — **Decided
    2026-11-24: both, opt-in (OK three times; code spoken by TTS on the
    secure screen).**
14. **Panel instances** (amends PANELS.md decision 9): allow up to 4 per
    panel, each with its own knobs and placement? — **Decided 2026-11-24:
    unlimited, within the shared budget.**
15. **Decoded images on the TV** (reverses PANELS.md § 7): PNG only, with a
    new runtime decoder, after the step 1 measurements? — **Decided
    2026-11-24: PNG now; the measurements only tune the limits.**
16. **Look lock** for families (a PIN over look changes and installs):
    in the first install release, or later? — **Decided 2026-11-24: in
    the first install release (off by default).**
17. **Safe-look exception**: may the safe-look key chord apply without
    plan and Approve (it only takes things away, is recorded in History,
    and can be undone), as a dated SECURITY.md exception? — **Decided
    2026-11-24: allowed.**
18. **Approve-only tier**: may remove, disable and the safe-look CLI
    take a plain Approve with no code, since they only take things away
    and can be undone (a new SECURITY.md tier)? — **Decided 2026-11-24:
    plain Approve.**
19. **Script language** (§ 5.8): Rhai (as #79 uses) or WASM (`wasmi`)?
    — **Leaning 2026-11-27: Rhai first** (readable on the review, no
    toolchain, #79 already uses it), behind a language-neutral host API so
    WASM (scripts in Rust) can be added later; confirmed by a Philips
    measurement before the scripts slice.
20. **RustTV profiles** (a kids' Home with its own hidden apps, theme and
    knobs, switched with a PIN): wanted, and when? They need a SECURITY.md
    decision, since Settings behind a PIN meets § 10.4's floors.

## 17. Research

Read-only, 2026-11-20 and 2026-11-24: FigCraft's architecture, MCP and
components docs and its sibling apps; docking in Rust (egui_dock,
egui_tiles; none for Slint); Slint 1.18's rendering hooks, element
properties and live preview; RustTV's PANELS.md, SECURITY.md, MCP.md,
REMOTE.md, HISTORY.md, ROADMAP.md, BENCHMARK_PLAN.md and
`codegen/convert.rs`. Unverified details are kept in the private research
notes, not here.

## Review log

**Round 1 (2026-11-24)**, three reviewers in parallel, each with a
checklist:

- *Coverage* (85 findings): locked surfaces; knobs and panel settings;
  theme tweaks; install without a computer; crash recovery; capability
  per target; editable packages; data-source states; format and token
  versioning; lists, widgets, transforms, accessibility, remote keys;
  OLED; a cost model; the Home order vs theme layout; the phone renderer;
  scenarios and mock data; contradictions with PANELS.md (apps, tile
  size, images, two copies).
- *Safety* (46): the door can't stage a package without a new class; the
  TV must not open reviews by itself; one-click placement and theme picks
  depend on the owner's decision; themes can't override panels or reach
  locked surfaces; no package content on the review screen; zip, image,
  font, SVG, `.po` and expression attack surfaces; privilege by load
  location; crash quarantine; source tiers; the phone page and preview
  must not receive TV data or raw package JSON; preview sessions started
  on the TV; the studio MCP's port, allowlist and no input injection;
  supply chain.
- *Feasibility* (44): what Slint can and can't draw; nested repeaters
  instead of a flat list; layout through Slint's own layouts; the desktop
  preview isn't Skia; runtime strings can't be `@tr`; History can't keep
  package bytes in the git tree; step 2 too big; option B needs pass/fail
  checks; limits need numbers; BENCHMARK_PLAN.md fixtures.

All findings were folded into the second version.

**Round 2 (2026-11-24)**, two reviewers (coverage; safety and
feasibility), checking round 1's fixes and looking for new issues:

- *Coverage* (31): the blind reset wasn't defined; locked surfaces and
  low vision; duplicating built-ins; runtime contrast for panels; text
  scale and size classes; following Android's accessibility settings; no
  flashing; bidi marks must be kept for Arabic; locale formatting;
  instances (two clocks); more knob types; schedules and quiet hours;
  theme asset slots; lock look settings for families; Linux aspect
  ratios and pointer input; token import; translated errors; defaults
  (Home tile only); MCP knob edits vs no TV writes.
- *Safety and feasibility* (21): the software renderer draws no shadows
  or rotation, so tiers are per renderer and the studio preview needs a
  decision; Android has no runtime PNG decoder; phone-mirror images need
  offscreen rendering (spike); crash quarantine for every active moment
  and for staged validation; lineage needs an author key; the preview
  prompt needs the code; phone and file routes in SECURITY.md; Linux is
  not a trust boundary; review-name rules; `LockedTheme` first in step 3;
  sources that need Android permissions dropped; flash caps; nested
  repeats; zip details; History under `[launcher]`; the apply journal;
  plan tools in `rusttv-mcp`; studio updates; step 2e gated on data.

All findings were folded into the third version.

**Round 3 (2026-11-24)**, two fresh reviewers, new issues only:

- *Coverage* (15): Home isn't a key event, so the reset chord must
  change; the code proves presence, not who (the look lock covers
  installs); author-key lifecycle (backup, rotation, no keychain);
  bidi in the studio; re-validation after a launcher update; per-device
  limits; low-memory kills aren't crashes; named looks; low-vision
  tweaks; a global banner cap and per-surface quiet hours; input switching
  and global keys out of scope; packages for people with no computer;
  studio packaging on Linux and Windows; clock trust; contradictions.
- *Safety and feasibility* (15): first activation of a non-tile surface
  needs the code; Home chord not detectable; minimal Panels page in 4b;
  2b before tokens via a shim; phone edits only harmless knobs; paired
  names sanitised; review pinned to its hash; the MCP never sees the
  installed list; signing is a person's action; staging is approval-free
  by design; preview nonce; inner shadow and conic unverified;
  `ed25519-compact` is a new runtime dependency; "tweaks" meant two
  things; step 5 and 6a under option B.

All findings were folded into the fourth version.

**Round 4 (2026-11-24)**, two fresh reviewers, high and medium only:

- *Safety and feasibility* (8 + 3 low): `ApplicationExitInfo` is API 30+
  and low-memory kills could hide a bad package; the waiting row must not
  parse the staged file; image, text, app and list knobs need the code;
  bundled optional packages follow user rules; the PIN reset can't skip
  the TV; computer names are untrusted text; the phone page can't be a
  share target; the capability door command had no step.
- *Coverage* (5 + 5 low): quarantine marked every Home tile; remove's
  undo could be pruned; a forgotten PIN with no computer; new tokens need
  fallback aliases and floors on the merged theme; restore to a weaker TV;
  one update rule; step 0 blocks step 5; non-visual approval; RTL focus
  order; a font choice.

All findings were folded into the fifth version.

**Round 5 (2026-11-24)**, two fresh reviewers, high and medium only:

- *Coverage* (5 + 3 low): package actions could change code-only knobs;
  the PIN reset needed a cancel and a "no TV reset" option; removed bytes
  needed "delete now"; the ASCII name rule broke Ukrainian and Arabic
  names; crashes in a panel only sitting on Home weren't attributed; knob
  type changes; passwords in text knobs; per-panel language.
- *Safety and feasibility* (6 + 1 low): bundled packages loaded from the
  APK would get built-in privilege; reinstall inherited placement without
  a key match; the CLI PIN reset skipped the wait; any paired phone could
  unlock; the preview prompt popped by itself; no way to pack and sign
  before the studio; power cuts below Android 11.

All findings were folded into the sixth version.

**Round 6 (2026-11-24)**, two fresh reviewers, high and medium only (none
high):

- *Safety and feasibility* (3 + 3 low): the safe-look CLI needed a TV
  Approve; `panel sign` came before the key existed; native crashes below
  Android 11 never reach the panic hook; sync deletions limited to
  TV-listed hashes; the capability command's SECURITY.md entry; name-rule
  dependencies.
- *Coverage* (4 + 3 low): "one script" refused Japanese and Korean names;
  reinstall of a newer version lost its placement; the PIN reset let a
  child defeat the lock; preview request rows had no limits; the review's
  surfaces for a new install; layout direction of a per-panel language.

All findings were folded into the seventh version.

**Round 7 (2026-11-24)**, two fresh reviewers, high and medium only (none
high):

- *Coverage* (4 + 1 low-medium): a forgotten PIN with no way out; the
  look lock missed disable; the crash thresholds disagreed and below
  Android 11 a low-memory kill looked like a crash; mixed
  Ukrainian–English names were refused; keychain keys and the passphrase.
- *Safety and feasibility* (3 + 2 low): the SECURITY.md entries for safe
  look and locked surfaces came after step 3; the CLI reset vs "TV reset
  off"; the passphrase was oversold; Latin with Japanese and the order of
  cleaning and checking names; staged expiry across reboots.

All findings were folded into the eighth version.

**Round 8 (2026-11-24)**, two fresh reviewers, high and medium only (none
high):

- *Coverage* (2 + 4 low): safe look stripped low-vision settings; the
  look lock missed re-enable, restore, delete and schedules; a broken
  reference and a garbled name paragraph; a new TV without a computer;
  background native crashes.
- *Safety and feasibility* (3 + 3 low): the safe-look chord skips the
  CLAUDE.md pipeline and needs a dated exception; safe look and
  accessibility tweaks; the review must be computed from the document,
  not `meta`; step 3 can't test disabling packages; unsigned packages
  inheriting placement.

All findings were folded into the ninth version.

**Round 9 (2026-11-24)**, two fresh reviewers, high and medium only (none
high):

- *Coverage* (3): the look lock blocked low-vision settings; safe-look
  undo vs the lock and quarantine; an unsigned update over a signed one.
- *Safety and feasibility* (3 + 1 low): the QR literal on the review;
  the first-activation rule per surface and instance; safe look stripping
  high contrast; unsigned-over-unsigned updates.

All findings were folded into the tenth version.

**Round 10 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (4): QR facts missing from the review's list; signed over
  unsigned; schedules and the first-activation rule; a second safe-look
  press losing the record, and what undo restores.
- *Safety and feasibility* (2 + 2 low): quiet must never hide the
  launcher's own notices; the undo record bound to hashes; punycode hosts;
  how staged bytes reach the TV.

All findings were folded into the eleventh version.

**Round 11 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (3): safe look didn't pause schedules and named looks;
  dangling references to removed packages; the chord vs undo wording.
- *Safety and feasibility* (3): a pre-4b loader must be debug-only;
  streaming bytes over `content write` needs a spike and a fallback; QR
  codes limited to `https:`.

All findings were folded into the twelfth version.

**Round 12 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (2): the debug-only loader vs release measurements on the
  Philips; schedules staying paused after a manual re-enable.
- *Safety and feasibility* (2): remove and disable without the code need
  a dated SECURITY.md tier; "Open with" would let any app fill the
  staging slot.

All findings were folded into the thirteenth version.

**Round 13 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (2): the measurement APK needs its own id and the debug
  key; resuming schedules brings content back, so it takes approval.
- *Safety and feasibility* (3 + 1 low): the Approve-only tier's entry
  came after step 3; every Philips step needs the owner's go-ahead, not
  only spikes; knobs must not switch surfaces on; the USB file picker
  is unproven.

All findings were folded into the fourteenth version.

**Round 14 (2026-11-24)**, two fresh reviewers, high and medium only:

- *Safety and feasibility*: **nothing new** (3 low: the installed-list
  read needs its own door command; the measurement APK has no HOME filter
  or door provider; "open an app" excludes system packages).
- *Coverage* (2): step 1 needs its own throwaway APK and typed test
  command; Q17 and Q18 block step 3, and 4a only covers what's left.

All findings were folded into the fifteenth version.

**Round 15 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (2 + 1 low): owner decisions weren't gated before the steps
  that need them; the measurement APK can't measure Home-level numbers;
  the installed-list command had no SECURITY.md entry.
- *Safety and feasibility* (3): the installed-list read is not R1 (the
  MCP may run R1) but a new private-read class; the spike APK can't prove
  the Home chord; 2c's door command can't be checked on the measurement
  APK.

All findings were folded into the sixteenth version.

**Round 16 (2026-11-24)**, two fresh reviewers, high and medium only:

- *Safety and feasibility*: **nothing new** (3 low: the two APKs'
  scopes overlapped in wording; when `convert.rs` retires; Q6 and Q13
  gates).
- *Coverage* (4): § 4 still said the chord is proven in step 1; Q6 and
  Q13 weren't gated; the 2e parity gate was circular; the PIN-reset wait
  had no clock rule.

All findings were folded into the seventeenth version.

**Round 17 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (3): no step built the hero and phone faces (and Q12 was
  ungated); a PIN-reset wait that reboots restart never ends; the phone
  mirror was described two ways.
- *Safety and feasibility* (2): History hands the MCP what the private
  read hides; a restore trusted approvals in a computer-editable
  snapshot.

All findings were folded into the eighteenth version.

**Round 18 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (4): placement, schedules and named looks leak the
  installed list to MCP History tools; a reinstall skipped the restore
  rule; the hero face wasn't marked; updates that drop a placed face.
- *Safety and feasibility* (2 + 2 low): the restore review needs the
  full § 4 facts and code-only knob values; the same MCP skip list; the
  PIN wait uses `elapsedRealtime`; safe-look pausing hooks land in 4c/4d.

All findings were folded into the nineteenth version.

**Round 19 (2026-11-24)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (2): themes missing from the MCP History skip list; a
  restore needing dozens of codes (now one per package).
- *Safety and feasibility* (1 + 2 low): MCP plan tools for installed
  packages were an installed-state oracle; system apps excluded in the
  text, not only in Q7; who creates named looks and schedules.

All findings were folded into the twentieth version.

**Round 20 (2026-11-24)**, two fresh reviewers: **both found nothing new
at high or medium severity**, which ends the loop. Their low items were
folded in: adb peers have no fingerprint; the PIN is `FLAG_SECURE` with a
stricter lockout; Q5 gates step 7; "system packages" defined (YouTube,
Netflix and the Play Store stay allowed); `theme_install_plan` lands in
4d.

**Round 21 (2026-11-27)**, owner request 13 (full customisation, scripts
later), two fresh reviewers:

- *Coverage* (70): no auto-layout, scroller or pages; the 50-item cap
  vs every app reachable; no item or focus values, per-item states or
  gliding focus; regions, folders, Zen's idle timer; edge, focus-scope,
  logical keys, key repeat, focus targets, double-press delay; launch
  counts' rules; multi-key sort vs the person's order; time, colour and
  `select` functions; masks, transforms, particles, image and text
  effects; per-surface rules; accessibility sources; profiles; a `home`
  face; `open_item`; Home's fallback; scripts in themes; step 2f too big.
- *Consistency and safety* (37): Back and reserved keys; actions from
  non-key events; keyframes still "not on the TV"; the interpreter "not
  measured"; PSS labels; budgets; "local" vs "state"; themes over
  panels; first pick of a Home-replacing theme; 2f's place; wasmi; Q9;
  floors only static; focus and Back traps; fake locked screens; timers
  behind locked surfaces; script steps checked at run time; the code
  setting's tier; § 14 entries; crash marker and watchdog for Home.

All findings were folded into this version (§ 5.7, § 5.8, § 10.4 rewritten).

**Round 22 (2026-11-27)**, two fresh reviewers, high and medium only
(none high):

- *Coverage* (9): floors banned the dimming and scaling carousels need;
  no list for "most used" rows; no sway loop; geometry bindings and RTL
  transforms; default navigation and key fall-through; Home modes and
  item states; panel items in the app order; focused-item art;
  `group_by` vs nested repeats.
- *Consistency and safety* (8): `open_item` vs § 5.5; the same-item lint
  enforced at run time; one plain app list per variant, drawn above
  overlaps; knob writes from handlers vs the look lock (now a `remember`
  slot); the compiled Home kept as the fallback; `meta.icon`; `home` and
  `folder` faces; launch counts opt-in.

All findings were folded in. The owner's two-round cap ends this loop
here; a round 23 runs if the owner wants the "nothing new" proof.

**Round 23 (2026-11-27)**, owner request 14 (one theme system, two
runtimes, nothing built in), one fresh reviewer, high and medium only:
the default look's bytes and first start (no bundled theme: the
emergency Home until one is installed); every "default theme" fallback
(now the emergency Home); § 6.7 still saying "not a runtime UI
language"; build order (step 3 vs 2f4 and 4b, T-track order, where the
code setting is built); the first-pick rule; the Themes panel with code
themes; § 10.1/10.2 wording; § 14 entries; how § 5.9's floors hold for
`.slint` (keys first, launcher-drawn list and marks, import allowlist);
host API versions; the review facts; the crash marker. All folded in.

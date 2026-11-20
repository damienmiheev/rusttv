# Changelog

All notable changes to RustTV. Versions follow [Semantic Versioning](https://semver.org);
before 1.0.0 a minor version may break the CLI/JSON contract (the `schema`
field in `--json` output says which contract you get, CLI.md).

## Unreleased

### Launcher

- The About tile is the first panel (PANELS.md): the RustTV logo and
  version, and "Update available", "Downloading" or "Installing" in place
  of the update dot, drawn from a JSON template (`templates/about_tile.json`)
  that the build turns into Slint. On Android, Settings keeps its dot (with
  a top-right menu, as on Linux, neither tile is shown). The colours and
  reading direction moved to `ui/theme.slint`, shared by every screen and
  template. `scripts/launcher-pot.sh` extracts template texts for
  translators. The desktop build takes `RUSTTV_UPDATE=available` and
  `RUSTTV_NO_DEVICE=1` (the Android layout). Captions are short enough to
  fit the tile in every language.
- The About page and Settings' RustTV row are About's page and row faces,
  drawn from templates (`templates/about_page.json`, `about_row.json`)
  made of page blocks (title, paragraph, mark, scroll box, hint;
  `ui/blocks.slint`). They work as before (OK installs or checks for an
  update, Up and Down scroll the notices, Slint's attribution stays) and
  look the same, pixel for pixel.
- The RustTV Remote tile is a panel too (`templates/remote_tile.json`):
  the Remote badge, how many phones are paired ("No phones",
  "Phones: 2"), and "Pairing" in the accent while pairing is open. It
  changes only when pairing opens or closes or a phone is paired or
  removed; Home's tile list isn't redrawn. Three new texts in all 20
  translations, measured to fit the tile.
- The RustTV Remote page is drawn from a template
  (`templates/remote_page.json`) with new general blocks: a list page's
  header (title and status line), a picture and a focusable list of rows
  with pointer hover and click. It looks and works as before, except that
  "Pointer in other apps" is now dim where there is no pointer service,
  and shows the accent dot while it is on.
- A version inside right-to-left text (Arabic, Hebrew) is an isolated
  left-to-right run: on the About tile and page, and Settings' RustTV and
  Updates rows (templates: `{"ltr": prop}`).
- History records where the RustTV Remote tile is, so undo brings it back
  (HISTORY.md § 1). New snapshots are therefore at least state format 5
  on Android too (6 with History's tile, below), which released builds
  (0.3.2 and older) can't read. Older
  snapshots still restore, with the tile at the end.
- History has its own tile, a panel (`templates/history_tile.json`, the
  first `panel:<id>` tile, `panel:history`): the History badge, "Last
  change" and its age ("Just now", "2 hours ago"), or "Waiting" in the
  accent while an approval waits for the computer. OK opens Settings on its
  History section, whose entry in the section list now shows the same age,
  or "Waiting" and the accent dot. The age comes from a summary the history
  thread keeps after its own work and is formatted on resume and focus
  changes, never by a timer; until the history has been read once since
  start the tile shows just "History". A first run puts it last, after
  RustTV Remote; it moves like the other built-in tiles. Its place is in the
  history, so new snapshots with a launcher order are state format 6; older
  snapshots still restore, with the tile at the end.
- Panel templates take plural forms for any text with a number
  (`"plural"` and `"count"`), so each language has its own forms ("1
  hour", "2 hours", Ukrainian's three, Arabic's six). New texts in all 20
  translations, measured to fit the tile. The desktop build takes
  `--demo-history <dir>` and `RUSTTV_HISTORY_PENDING=1` (a TV without the
  settings grant, so an approved change waits for the computer).
- The About, Settings, RustTV Remote and History tiles can be hidden and
  put in folders like apps (PANELS.md § 6): their menu is Move and Hide,
  and holding a moving tile over another makes a folder of them. A folder
  shows their banners small; the open folder draws them as their tiles and
  OK opens their page. Hidden ones are listed on the Hidden apps page
  (after the apps), whose tile is always on Home while something is
  hidden, so a hidden Settings is two presses away. While About is hidden,
  Settings gains its About section, so the licence notices and Slint's
  attribution stay reachable. The history records them, so a snapshot
  with a built-in tile hidden or in a folder is state format 7, which
  builds before it refuse; older snapshots still restore. A change list
  names hiding a tile only as hidden, no longer also as removed.
- The top-right menu (Linux) has a RustTV entry: a quick panel with the
  panels' rows, "RustTV <version>" (with "Update available",
  "Downloading" or "Installing" while there is an update, else
  "Third-party notices"), RustTV Remote (the paired phones, or
  "Pairing") and History (the last change's age, or "Waiting"), each from
  the same view as its tile; OK or a click opens its page. Settings'
  RustTV row shows the same (its long licence line, which didn't fit the
  panel in most languages, became "Third-party notices"). No new texts.
  The desktop build takes `--demo-panels <dir>`.
- Settings → Effects: Reduced turns the tile shadows and the backdrop glow
  off, for a weak GPU. The choice is in the TV's history and kept as the
  `effects-reduced` flag file next to `order.toml`, so an image can preset
  it. `RUSTTV_FPS=1` shows the frame rate in a corner.
- A mouse or air mouse works on every screen: hovering focuses (tiles, the
  open folder, the tile menu, Hidden apps, Settings, RustTV Remote, the
  review screen), a click is OK, the secondary button opens a tile's menu,
  and the wheel moves a row per notch. The pointer hides after 4 s at rest
  and as soon as a key is pressed. Moving apps and making folders stay on
  the arrow keys as before.

- A Linux home screen (feature `linux`, binary `rusttv-launcher-linux`):
  the apps of the machine's desktop entries in a Wayland compositor such as
  labwc. It has a top-right menu (Modify, Settings and the quick panels the
  device has: Audio, Network, KDE Connect, Power), device settings,
  an on-screen keyboard for other apps and game controllers over USB or
  Bluetooth. Settings has an About tab there. Android keeps its About and
  Settings tiles and has no top-right menu (`Platform::top_right_menu`). A
  history snapshot with Linux tile names (`*.desktop`) is written as
  format 5.

- `rusttv-portal`, a remote desktop portal backend for compositors without
  one: it serves only `xdg-desktop-portal` and gives remote input only to
  KDE Connect (the phone as touchpad and keyboard); every other app is
  refused.

- Linux keeps at most `max_apps` apps running and closes background apps
  when memory runs low (`~/.config/rusttv/linux.toml`). Stopping background
  apps (`freeze_background`) is off unless the config turns it on.

### RustTV Remote

- The phone's Home mirror shows About's caption too: "Update available"
  (in the accent), "Downloading" or "Installing" on the tile's picture, in
  the phone's language. The TV sends it as the tile's optional `caption`
  (REMOTE.md § 4), from the same panel view as the TV's tile, only when
  the update state changes it. Older pages ignore it.
- History's tile shows its age or "Waiting" (in the accent) on the phone
  too: new `caption` values (`waiting`, `just_now`, `minutes`, `hours`,
  `days`) with an optional `count`, the age in the phone's language and
  plural forms (REMOTE.md § 4). The phone may tap, drag or ask the
  picture of `panel:history`.
- The phone's mirror shows built-in tiles in folders and on the hidden
  apps page, with their pictures and captions; its menu offers Hide for
  them, and a tap on the hidden apps page puts one back (REMOTE.md § 4).
  On a built-in page (About, RustTV Remote, Settings, or Settings opened
  on History), `focused` names the tile that opened it, wherever it
  was.

### CLI

- Schema `rusttv.v11`: history changes have the new kind `effects`, and
  `history show` may include `launcher.reduced_effects` (CLI.md).
- TV history snapshots in state format 6 hold History's tile,
  `panel:history`, in `launcher.order`; `moved` may name it, never `added`
  or `removed` (CLI.md). The schema stays `rusttv.v11`.
- TV history snapshots in state format 7 may hold built-in and panel
  tiles (`about`, `settings`, `remote`, `panel:history`) in
  `launcher.hidden` and in folders' `apps`; `hidden` and `in_folder` may
  name them, and hiding a tile (or showing it again) is reported only as
  `hidden`, no longer also as `removed` (or `added`). The schema stays
  `rusttv.v11` (CLI.md).

## 0.3.2 (2026-11-02)

Fixes the armeabi-v7a launcher, which crashed at startup on Android 11 and
12. If your TV runs 0.3.1's armeabi-v7a APK, update to this one. The
arm64-v8a and x86_64 APKs and the CLI behave as in 0.3.1.

### Launcher

- The armeabi-v7a APK installed and the launcher then died at startup on an
  Android 11 TV: `dlopen failed: cannot locate symbol
  "_ZNSt6__ndk113__hash_memoryEPKvj"`. The armv7 Skia is built by
  skia-armv7.yml with NDK 30, whose libc++ holds that function, but
  android/build.sh took an inherited `ANDROID_NDK_HOME` ahead of its own pin
  and GitHub's ubuntu runners export one for their older preinstalled NDK, so
  the launcher was linked against a libc++ without the symbol. A shared
  library may keep an undefined symbol, so the link, the build and
  `pm install` all passed, and the reference fell to the platform's libc++:
  Android 14 exports it, Android 11 does not. build.sh now refuses any NDK but
  the pinned one, Android APK points `ANDROID_NDK_HOME` at it, and
  android/check-so.sh fails the build when an APK leaves a C++ runtime symbol
  undefined. The arm64-v8a and x86_64 APKs of 0.3.1 are unaffected. Found
  and fixed by @Capka3i (#30).

### Dependencies

- pulldown-cmark 0.13.6 (through Slint), for RUSTSEC-2026-0433.

## 0.3.1 (2026-10-29)

The 0.3.0 release was published without its files, and its tag can't take
another release, so this is 0.3.0 again under a new version. No code changes
since 0.3.0; what's new is in the 0.3.0 section of
[CHANGELOG.md](https://github.com/damienmiheev/rusttv/blob/v0.3.1/CHANGELOG.md).

## 0.3.0 (2026-10-27, never published with files)

### RustTV Remote (new)

- A phone becomes the TV's remote, keyboard and touchpad: a web page served
  by the launcher (no app to install), paired once by scanning a QR code on
  the TV and pressing OK there; every message encrypted end to end
  (REMOTE.md, SECURITY.md § RustTV Remote). RustTV Remote has its own Home
  tile and page.
- On RustTV, the phone mirrors Home: tap to open, hold for options, hold and
  drag to move apps and make folders, tile pictures, hidden apps, typing
  into the TV's text fields.
- HTTPS with the TV as its own certificate authority (opt-in trust with a
  check code compared on the phone), Add to Home Screen on iPhone and
  Android, and Home Screen icons that start paired.
- Magic remote in other apps through RustTV's own accessibility service
  "RustTV Pointer", turned on from the RustTV Remote page: a cursor moved by
  the touchpad, clicks and two-finger scrolls on the element under it,
  arrows / OK / Back / Home, play/pause and volume (held keys repeat),
  typing into the focused field with the TV's keyboard hidden, and whole
  searches for apps with their own keyboard (YouTube). The phone picks
  pointer or arrows per app and remembers the person's choice.
- Paired phones are recorded in the TV's history (names only, never keys);
  a restore can only remove phones.

### Launcher

- Folders, like on a phone's Home, worked with the remote: in move mode,
  leave an app on top of another app (or a folder) for 0.6 s to make a
  folder (or put it in); the folder opens over a dimmed Home with pages of
  12, Up reaches its name and OK renames it with the TV's keyboard, Back
  closes it. Apps move out past the folder's edge; an empty folder goes. The
  first name comes from the apps' category, in all 21 languages. Folders are
  kept in `order.toml` and in the TV's history.

### CLI

- Schema `rusttv.v9`: history changes have the new kinds `folder_added`,
  `folder_removed`, `folder_renamed` and `in_folder`, and `history show` may
  include `launcher.folders` (CLI.md).
- Schema `rusttv.v10`: history changes have the new kinds `phone_paired` and
  `phone_removed`, and `history show` may include `launcher.phones` (CLI.md).

### Releases

- Every release file has a GitHub build provenance attestation
  (`gh attestation verify <file> --repo damienmiheev/rusttv`), and every
  binary a CycloneDX SBOM: two in each CLI archive, one per launcher ABI
  next to the APKs (RELEASE.md).

## 0.1.0 (2026-10-24)

First public release (ROADMAP Phase 6).

### CLI (`rusttv`)

- `device list`, `device connect`, `device inspect`; `apps list`;
  `performance snapshot` and `performance benchmark`; `compat profile`.
- `launcher status`, `launcher install` / `uninstall`, and the opt-in Home
  takeover / restore (SECURITY.md; the takeover wakes the TV first, since a
  screensaver swallows the HOME press, and stops while a TV overlay has
  focus; plans show this as the `awake` check), all planned, shown as a dry
  run, approved, journaled and verified by the safety engine.
- `optimize analyze`, `optimize apply privacy` and `optimize restore`: disable
  (never uninstall) packages from a shared package catalog (`compat/packages/`:
  telemetry, ads and extras by default; casting, smart-home, voice and
  streaming apps opt-in), chosen in a local choices file (`keep`, `also`,
  `extra`), with live protected-package checks. Only packages tested on the
  model are planned unless `--allow-untested`, which marks each one UNTESTED
  (OPTIMIZER.md, SECURITY.md).
- `optimize apply reduced-animations` and `optimize restore
  [privacy|reduced-animations]`: put the three animation scales at 0.5 and
  back, leaving any setting changed since as it is. Refused on models whose
  profile doesn't record it as tested unless `--allow-untested`; tested on
  the Philips 65OLED818 (OPTIMIZER.md § 1–6).
- `update check`, `update launcher` and `update cli`: install newer RustTV
  releases from GitHub (or `RUSTTV_UPDATE_DIR`), the launcher on a TV through
  the normal install plan and the CLI on this computer with `.previous`
  copies and automatic put-back. Only files whose `SHA256SUMS` is signed
  with the owner's minisign key (`SHA256SUMS.minisig`, key id
  `C1DB81DC9704FCB0`); never a downgrade (CLI.md § Updates, SECURITY.md §
  Updates).
- `launcher upgrade`: install or upgrade the launcher also while a Home
  takeover is active, as restore, install and takeover again, three separate
  plans approved by typing `upgrade` once; stops at the first failing stage
  and says how to continue.
- `approve --for <duration> --only launcher`, `--status`, `--revoke`: a
  time-limited (at most 2 h) delegation, granted at a terminal by typing
  `delegate`, that lets launcher install, upgrade, takeover and restore
  plans for one device and firmware be applied without a terminal (e.g. by
  an agent). Journals record how each plan was approved (`approved_by`)
  (owner decision 2026-10-24, SECURITY.md § Delegated approval).
- `history sync | log | show | diff`: a full copy of each TV's history on
  this computer (`<state dir>/history/<id>.git`, plain git), fetched in
  verified parts and linked to the TV's `ro.serialno`; `sync` lists the
  approvals waiting on the TV and warns about sensitive settings changed
  since the last sync (accessibility services, keyboard, wireless debugging,
  unknown sources, Play Protect, proxy, private DNS, the launcher's grant).
- `history restore <entry>` (and `--journal` to undo it): put the animation
  settings and catalog packages back as an entry recorded them, through a
  `history-restore` plan with the privacy rules; refused while an optimizer
  or privacy change holds the same keys or packages, and the other way
  round.
- `history restore-tv`: put this computer's copy back on the TV after a
  reinstall, only into an empty history. `launcher uninstall` copies the
  history here first.
- `history apply-pending`: apply what a person approved on the TV, at a
  terminal, after claiming the approval through the door; nothing to type
  when the plan is exactly what was approved. Refused for test builds of
  the launcher.
- `history allow-tv-settings` / `revoke-tv-settings`: let the launcher
  change its animation settings itself (`WRITE_SECURE_SETTINGS`).
- Plan-applying commands record an observation before and a note after each
  plan in the TV's history (risk class RL, launcher-private; warnings only).
- Stable `--json` output, schema `rusttv.v8`, and documented error codes (CLI.md).

### MCP server (`rusttv-mcp`)

- Stdio MCP server with RustTV's read-only commands as tools, and plan tools
  that return a dry run plus the CLI command to apply it (MCP.md).

### Launcher (`ai.rusttv.launcher`)

- Rust + Slint home screen for Android TV 8+: app grid with banners, D-pad
  navigation, move mode, hide and show apps from the hold-OK menu, focus
  kept on the last app you opened, About page with the third-party licence
  notices.
- One APK per ABI: arm64-v8a, armeabi-v7a, x86_64.
- In the TV's language: English plus 20 translations (Arabic, Chinese
  Simplified and Traditional, Dutch, French, German, Hebrew, Hindi,
  Indonesian, Italian, Japanese, Korean, Polish, Portuguese (Brazil),
  Russian, Spanish, Thai, Turkish, Ukrainian, Vietnamese), drafted by AI and
  awaiting native review; Arabic and Hebrew mirror the layout
  (`crates/rusttv-launcher/translations/`).
- Updates itself from GitHub releases: once a day (on by default) it checks
  the latest release; About shows the result, and OK installs a newer
  version after checking its size, SHA-256, package, version and signing
  key. Android asks the user to confirm every install; never a downgrade
  (SECURITY.md § Launcher updates).
- Settings page (a tile after About; HISTORY.md § 8): Animations (seven
  speeds: Off, Fastest, Faster, Fast, Normal, Slower, Slowest), the update check on or off, the TV's history (who changed
  what, and "Restore this state"), and approvals waiting for the computer.
  Changes are reviewed and approved on the TV: Approve, a random 4-arrow
  code shown only on the screen (`FLAG_SECURE`), then OK held for 2 s;
  wrong codes lock approvals for longer each time; test builds can't
  approve. With `WRITE_SECURE_SETTINGS` (granted from the computer) the TV
  applies the animation scales itself; everything else waits for
  `rusttv history apply-pending`.
- History: every change RustTV manages (the launcher's order and hidden
  apps, the animation scales, catalog packages, firmware) as git commits in
  the launcher's storage, packed in the background, read by the computer
  through a door that answers only adb (SECURITY.md § The history door).
  Moving, hiding and showing apps are recorded too.
- Skia's GPU resource cache is capped at 24 MB (patched Slint renderer,
  `third_party/README.md`): opening About and the menu repeatedly no longer
  grows native memory (49.4 → 37.4 MB after three rounds on the Philips).
- Two more patches to Slint's Android renderer (`third_party/README.md`):
  the GL context is reused across windows, so returning to Home no longer
  leaks GPU driver memory, and app banners are kept as GPU textures without
  an extra copy.

### Compatibility and benchmarks

- Device profiles: Philips 65OLED818, and the stock Android emulator (phone
  image, read-only commands; the tests' reference device)
  (`compat/devices/`, schema 5). Benchmark data from the Philips in `benchmarks/raw/`.
- Package catalog (`compat/packages/`, 279 entries): 25 from our own checks on
  the Philips 65OLED818 (22 tested there), the rest derived from cleanyourtv's
  per-model lists (MIT).

### Licences

- Third-party notices (Rust crates, Slint, Skia, LLVM runtimes, the Rust
  standard library, cleanyourtv's package data) ship in every APK and in every
  CLI archive.

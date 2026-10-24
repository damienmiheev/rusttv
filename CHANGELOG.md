# Changelog

All notable changes to RustTV. Versions follow [Semantic Versioning](https://semver.org);
before 1.0.0 a minor version may break the CLI/JSON contract (the `schema`
field in `--json` output says which contract you get, CLI.md).

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
  with the owner's minisign key (`SHA256SUMS.minisig`); never a downgrade.
  Until the key is compiled in, both install paths refuse
  (`UPDATE_VERIFY_FAILED`) (CLI.md § Updates, SECURITY.md § Updates).
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

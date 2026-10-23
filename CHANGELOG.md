# Changelog

All notable changes to RustTV. Versions follow [Semantic Versioning](https://semver.org);
before 1.0.0 a minor version may break the CLI/JSON contract (the `schema`
field in `--json` output says which contract you get, CLI.md).

## 0.1.0 (unreleased)

First public release (ROADMAP Phase 6).

### CLI (`rusttv`)

- `device list`, `device connect`, `device inspect`; `apps list`;
  `performance snapshot` and `performance benchmark`; `compat profile`.
- `launcher status`, `launcher install` / `uninstall`, and the opt-in Home
  takeover / restore (SECURITY.md), all planned, shown as a dry run, approved,
  journaled and verified by the safety engine.
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
  profile doesn't record it as tested unless `--allow-untested`; no model
  does yet (OPTIMIZER.md § 1–6).
- Experimental `tools airplay status`, `enable` and `disable`: turn on, until
  the TV restarts, a MediaTek AirPlay receiver that some TVs ship installed
  but disabled, through a trigger activity in the launcher; only on models
  with a bundled compat profile and the vendor packages. Not certified by
  Apple for these models (TOOLS.md).
- `update check`, `update launcher` and `update cli`: install newer RustTV
  releases from GitHub (or `RUSTTV_UPDATE_DIR`), the launcher on a TV through
  the normal install plan and the CLI on this computer with `.previous`
  copies and automatic put-back. Only files whose `SHA256SUMS` is signed
  with the owner's minisign key (`SHA256SUMS.minisig`); never a downgrade.
  Until the key is compiled in, both install paths refuse
  (`UPDATE_VERIFY_FAILED`) (CLI.md § Updates, SECURITY.md § Updates).
- Stable `--json` output, schema `rusttv.v7`, and documented error codes (CLI.md).

### MCP server (`rusttv-mcp`)

- Stdio MCP server with RustTV's read-only commands as tools, and plan tools
  that return a dry run plus the CLI command to apply it (MCP.md).

### Launcher (`ai.rusttv.launcher`)

- Rust + Slint home screen for Android TV 8+: app grid with banners, D-pad
  navigation, move mode, About page with the third-party licence notices.
- One APK per ABI: arm64-v8a, armeabi-v7a, x86_64.
- `tools.HiddenFeatureActivity`: no UI, startable only by the adb shell or
  the system (it requires `android.permission.DUMP`), used by `rusttv tools`
  (TOOLS.md).

### Compatibility and benchmarks

- Device profiles: Google TV emulator (arm64) and Philips 65OLED818
  (`compat/devices/`, schema 4). Benchmark data from the Philips in `benchmarks/raw/`.
- Package catalog (`compat/packages/`, 279 entries): 25 from our own checks on
  the Philips 65OLED818 (12 tested there), the rest derived from cleanyourtv's
  per-model lists (MIT).

### Licences

- Third-party notices (Rust crates, Slint, Skia, LLVM runtimes, the Rust
  standard library, cleanyourtv's package data) ship in every APK and in every
  CLI archive.

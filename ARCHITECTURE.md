# Architecture

## Overview

```text
   Human (terminal)        AI agent
          |                    |
     rusttv CLI           rusttv-mcp   (Phase 5)
          \                    /
           +-- rusttv-core ---+         runs on the user's computer
                    |
             Adb trait → system `adb` binary
                    |   (USB or Wi-Fi ADB)
   ---------------- | ----------------------------- TV (stock firmware)
                    v
           RustTV Launcher APK  (independent; never needs the CLI)
           Slint UI + Rust logic  ⇄  thin Kotlin shim (Android APIs)
```

- The launcher and the host tools are **independent**. The launcher must boot to a working home screen with no computer, network or CLI.
- They share Rust *libraries* where useful (e.g. data types), never a runtime connection.
- No background daemon or service on the TV.

## Repository layout (monorepo)

```text
rusttv/
├── Cargo.toml              # workspace
├── rust-toolchain.toml     # pinned stable
├── deny.toml               # cargo-deny: advisories, licenses, bans
├── crates/
│   ├── rusttv-core/        # device model, Adb trait, typed commands, inspect, perf, (later) safety engine
│   ├── rusttv-cli/         # binary `rusttv`: clap, output formatting
│   ├── rusttv-mcp/         # Phase 5
│   └── rusttv-launcher/    # Phase 1b: cdylib for Android + desktop dev mode
├── android/                # Phase 1b: Gradle project, Kotlin shim, manifest
├── compat/
│   ├── schema.json         # validates profiles in CI
│   ├── protected.toml      # static protected-package deny-list (SECURITY.md)
│   └── devices/*.toml      # one file per device model/firmware family
└── *.md
```

Only `rusttv-core` and `rusttv-cli` exist in Milestone 1. Other directories are created when their phase starts.

## rusttv-core

Modules (initial):

| Module | Responsibility |
|---|---|
| `adb` | `Adb` trait + `SystemAdb` impl (spawns `adb`), `FakeAdb` for tests (canned outputs) |
| `command` | Typed `DeviceCommand` enum. Each variant renders to argv and carries a `RiskClass`. **The only way to reach a device** |
| `device` | `DeviceInfo` (manufacturer, model, Android/API, fingerprint, ABI list, RAM, storage) parsed from `getprop` etc. |
| `packages` | Package listing/inspection |
| `perf` | Snapshots (meminfo, cpu) and benchmark runners |
| `error` | `thiserror` error enum with stable string codes (`ADB_NOT_FOUND`, `DEVICE_UNAUTHORIZED`, `PARSE_ERROR`, …) |

Parsers are pure functions over `&str` → unit-tested with real captured output from emulator and Philips TV (fixtures in `crates/rusttv-core/tests/fixtures/`).

### Adb trait (sketch)

```rust
pub trait Adb {
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError>;
}
```

There is intentionally no `shell(&str)`. Values that end up in `adb shell` arguments are validated newtypes (e.g. `PackageName`), because the device shell re-parses them. Host-side operations (`devices`, `connect`) are added with step 3. Synchronous (`std::process`) in Milestone 1; no tokio until MCP needs it. `FakeAdb` is available under the `test-util` feature.

## rusttv-cli

- `clap` derive. Commands map 1:1 to core functions; no logic in the CLI beyond formatting.
- Default output: human text. `--json`: envelope `{"schema": "rusttv.v1", "ok": bool, "data": …, "error": {"code", "message"}}`.
- Exit codes: 0 ok, 1 operation error, 2 usage error, 3 device not reachable/unauthorized.
- Logging via `tracing` to stderr (`-v`/`RUST_LOG`); stdout reserved for results.

## Launcher (Phase 1b)

### Android bridge

```text
LauncherActivity (Kotlin, extends GameActivity)
   │ loads librusttv_launcher.so (android-activity + Slint)
   │
   ├─ PlatformBridge.kt  (static methods, called from Rust via `jni`)
   │     listLaunchables(): Array<AppEntry>   label, package, activity, banner/icon as RGBA + w/h
   │     launch(package, activity): Boolean
   │     registerPackageListener()            → calls native onPackagesChanged()
   │
Rust: trait Platform { list_apps(), launch(), … }
   ├─ AndroidPlatform  — only module with JNI/unsafe; one coarse call per operation; checks Java exceptions
   └─ FakePlatform     — desktop dev mode + tests (fake app list, icons from PNGs)
```

- Banner (320×180, Leanback) preferred; fall back to icon.
- Manifest: intent filters `MAIN`+`HOME`+`DEFAULT` and `MAIN`+`LEANBACK_LAUNCHER`; `<queries>` for `LEANBACK_LAUNCHER` and `LAUNCHER` intents (no `QUERY_ALL_PACKAGES`); `leanback` and `touchscreen` features `required="false"`.
- ABIs: `arm64-v8a` **and** `armeabi-v7a` (many TVs run 32-bit userspace on 64-bit SoCs), plus `x86_64` for the emulator.

### Minimal proof-of-concept design

- One screen: horizontal-scrolling grid of app tiles, clock, nothing else.
- Selection index lives in a Rust model; a root `FocusScope` maps D-pad keys → index arithmetic; DPAD_CENTER/Enter → `launch()`. Back is consumed at home.
- On `onResume`: refresh list if the package listener flagged changes. No polling, no services.
- Icons decoded once and cached in memory; disk cache only if measurements justify it.
- About screen with Slint attribution (license requirement).
- Must run on desktop with `FakePlatform` so UI work does not need a TV.

## MCP (Phase 5)

`rusttv-mcp` exposes core functions as tools (`tv_get_device_info`, `tv_list_packages`, …) over local stdio. Tools call the same core functions as the CLI; modifying tools can only submit plans for human approval (SECURITY.md).

## Initial dependencies (Milestone 1)

| Crate | Use | Why |
|---|---|---|
| `clap` (derive) | CLI parsing | Standard, discoverable help |
| `serde`, `serde_json` | JSON output | Standard |
| `thiserror` | core errors | Zero runtime cost |
| `anyhow` | CLI only | Ergonomic top-level errors |
| `tracing`, `tracing-subscriber` | Structured logs | Standard |
| `toml` | Compat profiles | Chosen format |

Dev/CI tools: `cargo-deny` (covers RustSec advisories + licenses, so `cargo-audit` is redundant), `cargo-nextest` optional. Added later only when needed: `proptest` (parsers), `criterion` (host-side micro-benchmarks), `tokio` + `rmcp` (MCP), `slint` + `jni` + `android-activity` (launcher).

## Decision log

| # | Date | Decision | Alternatives | Revisit if |
|---|---|---|---|---|
| D1 | 2026-10-23 | Monorepo, Cargo workspace | Multi-repo | Release cadences diverge strongly |
| D2 | 2026-10-23 | Rust CLI is Milestone 1, launcher second | Launcher first | — |
| D3 | 2026-10-23 | Thin Kotlin shim for Android APIs | Pure Rust JNI | Shim grows beyond ~300 lines of logic |
| D4 | 2026-10-23 | Wrap system `adb` behind `Adb` trait | Pure-Rust ADB client | Users can't install `adb`, or process spawn latency distorts benchmarks |
| D5 | 2026-10-23 | Min Android API 26 | 28 / 30 | Slint raises its minimum |
| D6 | 2026-10-23 | CLI hosts: macOS + Linux first | + Windows | A Windows tester joins |
| D7 | 2026-10-23 | Compat profiles: TOML in repo, schema-checked | JSON / separate repo | Profile volume needs its own release cadence |
| D8 | 2026-10-23 | No free-form shell in the API | Allow `shell(&str)` | Never for agent paths |
| D9 | 2026-10-23 | Slint under Royalty-free 2.0 with attribution | GPL / commercial | Slint's terms change |
| D10 | 2026-10-23 | Sync core, no tokio in M1 | tokio from day one | MCP or parallel device ops need it |

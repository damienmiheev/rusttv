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
│   ├── rusttv-mcp/         # binary `rusttv-mcp`: read-only MCP server over stdio (rmcp + tokio)
│   └── rusttv-launcher/    # home screen: model, Slint UI, JNI bridge; cdylib + desktop dev mode
├── android/                # Gradle app ai.rusttv.launcher: Kotlin shim, manifest, build.sh
├── compat/
│   ├── README.md           # how to contribute a profile
│   ├── protected.toml      # Phase 4: static protected-package deny-list (SECURITY.md)
│   └── devices/*.toml      # one file per device model/firmware family
└── *.md
```

`compat/protected.toml` does not exist yet; it is created when its phase starts.

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

Parsers are pure functions over `&str` → unit-tested with real captured output (fixtures in `crates/rusttv-core/src/fixtures/<device>/`, each with a README naming the commands and any redactions). Inspection fields are optional: missing vendor properties become `null`, never an error.

### Adb trait (sketch)

```rust
pub trait Adb {
    fn run(&self, serial: &str, command: &DeviceCommand) -> Result<String, AdbError>;
    fn devices(&self) -> Result<Vec<DeviceEntry>, AdbError>;
    fn connect(&self, address: &DeviceAddress) -> Result<ConnectOutcome, AdbError>;
}
```

There is intentionally no `shell(&str)`. Values that end up in `adb shell` arguments are validated newtypes (e.g. `PackageName`), because the device shell re-parses them. `adb connect` exits 0 even on failure, so its stdout is parsed to detect errors. `SystemAdb` uses `adb` from `PATH`, or the executable named by the `RUSTTV_ADB` environment variable. Synchronous (`std::process`); tokio exists only inside `rusttv-mcp` (D13). `FakeAdb` is available under the `test-util` feature.

## rusttv-cli

- `clap` derive. Commands map 1:1 to core functions; no logic in the CLI beyond formatting.
- Default output: human text; `--json` gives the versioned `rusttv.v3` envelope. The output contract, exit codes and every error code are in [CLI.md](CLI.md); `rusttv_core::ERROR_CODES` is the canonical list, and tests keep code, list and CLI.md in sync.
- Device-targeting commands pick the only connected device, or require `--serial` when there are several (exit code 2). With `--json`, errors are also a JSON envelope on stdout.
- CLI tests run the real binary against `tests/fixtures/fake-adb` (selected with `RUSTTV_ADB`), so they need no device.
- Logging via `tracing` to stderr (`-v`/`RUST_LOG`); stdout reserved for results.

## Launcher

### Crate `rusttv-launcher`

| Module | Role |
|---|---|
| `model.rs` | Pure state machine `Home`: tile order, D-pad focus, shelf/grid scroll, move mode, launch in flight. Time is passed in; returns `Effect`s. No Slint, no platform. |
| `order_file.rs` | Tile order in `order.toml` (app files dir): atomic write, unreadable file kept as `.bak`. |
| `platform.rs` | `Platform` trait (`list_apps`, `launch`, `take_apps_changed`), `PlatformError`, `Bitmap` (shrink, blur, hero backdrop glow), `FakePlatform` (feature `test-util`). |
| `ui.rs` + `ui/home.slint` | Rust owns state and timers and pushes properties; Slint draws, animates and forwards keys. `Lifecycle` reports pause/resume. |
| `android.rs` | `android_main` + `AndroidPlatform` over JNI. The only module with `unsafe` (crate lint `unsafe_code = "deny"`, module `allow`, a `SAFETY:` comment per block). |
| `bin/desktop.rs` | Desktop dev mode with `FakePlatform` (Slint winit backend + software renderer, no Skia); `--order <file>`, `--demo <dir>` snapshots. |

Targets: on Android Slint's `backend-android-activity-06` (Skia, D11); elsewhere `backend-winit` + `renderer-software`. `crate-type = ["lib", "cdylib"]`; the APK's library is built with the size-tuned `android-release` profile.

### Android bridge

```text
LauncherActivity (Kotlin, extends NativeActivity, D11)
   │ loads librusttv_launcher.so (android-activity + Slint)
   │
   ├─ Bridge.kt  (object with @JvmStatic methods, called from Rust via `jni`)
   │     listApps(context): Array<String>          "component\nlabel", LEANBACK_LAUNCHER, without itself
   │     banner/icon(context, component, w, h): IntArray?   ARGB pixels, null if none
   │     launch(context, component): Boolean       false if the activity is gone
   │     takeAppsChanged(): Boolean                set by the package receiver; cleared on read
   │
Rust: trait Platform { list_apps(), launch(), take_apps_changed() }
   ├─ AndroidPlatform  — class loaded once via the activity's class loader; JNI calls in local
   │                     frames; Java exceptions cleared and returned as PlatformError
   └─ FakePlatform     — desktop dev mode + tests (generated banners)
```

- Apps are identified by component name (`package/activity`). Banner (320×180, Leanback) preferred; icon (96×96) as fallback.
- Manifest: `MAIN`+`LEANBACK_LAUNCHER` and `MAIN`+`HOME`+`DEFAULT`; `<queries>` for `LEANBACK_LAUNCHER` (no `QUERY_ALL_PACKAGES`); `leanback` and `touchscreen` features `required="false"`; `allowBackup="false"`.
- Lifecycle: Slint's Android event listener forwards `Pause`/`Resume` to `Lifecycle`. `LauncherActivity` registers a runtime receiver for package added/changed/removed/replaced (manifest receivers no longer get these since Android 8); it sets a flag in `Bridge`, and resume re-reads the app list only when `Platform::take_apps_changed` reports it.
- ABIs: `arm64-v8a`, `armeabi-v7a` (32-bit userspace TVs) and `x86_64` (emulator), one APK per ABI (Gradle ABI splits, `app-<abi>-release.apk`).
- Build: `android/build.sh` (cargo-ndk, then Gradle). Skia is a prebuilt binary per ABI, fetched by `build.sh` and checked against a pinned SHA-256: rust-skia's for arm64-v8a and x86_64; for armeabi-v7a, one built by our manual workflow `skia-armv7.yml` and hosted as a release of this repository (SECURITY.md § Supply chain). Release builds are signed with the debug key until release signing exists (Phase 6). `build.sh` runs `scripts/licenses.sh`, and Gradle packs the result into every APK as `assets/licenses/THIRD_PARTY_NOTICES.txt` (Rust crates, Slint, Skia and its bundled libraries, the NDK's LLVM runtimes, the Rust standard library); the build fails if the file is missing.

### Still open

- About page with Slint attribution exists; app details and settings do not.
- Banner cache is in memory only; a disk cache only if measurements justify it.
- Device checks on the Philips TV (focus timings, backdrop look, label readability at 3 m) and benchmarks per BENCHMARK_PLAN.md.

## MCP (Phase 5)

`rusttv-mcp` exposes the core's read-only functions as MCP tools over local stdio: `device_list`, `device_inspect`, `apps_list`, `launcher_status`, `performance_snapshot`, and the plan tools `launcher_install_plan`, `launcher_uninstall_plan`, `launcher_restore_plan` (contract in [MCP.md](MCP.md)).

- Built on the official Rust SDK `rmcp` (D13). `RustTvServer<A: Adb>` is generic so tests use `FakeAdb`; the binary uses `SystemAdb`.
- The core stays sync (D10): each tool call runs the core function on tokio's blocking pool, holding a mutex around the `Adb`, so calls run one at a time and `Adb` need not be `Sync`.
- Results are the CLI's `rusttv.v3` envelope as `structuredContent`; core errors are tool errors (`isError`) with the CLI's error codes, bad arguments are rejected before any device access.
- No tool runs above R0 and none approves anything. Plan tools only return a dry run and the `rusttv` command a person runs to apply it; they read journals from the CLI's state directory (`rusttv_core::journals_dir`) and write none (SECURITY.md § AI agents).

## Initial dependencies (Milestone 1)

| Crate | Use | Why |
|---|---|---|
| `clap` (derive) | CLI parsing | Standard, discoverable help |
| `serde`, `serde_json` | JSON output | Standard |
| `thiserror` | core errors | Zero runtime cost |
| `anyhow` | CLI only | Ergonomic top-level errors |
| `tracing`, `tracing-subscriber` | Structured logs | Standard |
| `toml` | Compat profiles | Chosen format |

Dev/CI tools: `cargo-deny` (covers RustSec advisories + licenses, so `cargo-audit` is redundant), `cargo-nextest` optional. Added later only when needed: `proptest` (parsers), `criterion` (host-side micro-benchmarks), MCP (Phase 5, `rusttv-mcp` only): `rmcp` 3.7 (features `server`, `macros`, `transport-io`; Apache-2.0) and `tokio` (current-thread runtime). Launcher (Phase 1c): `slint`/`slint-build` (=1.18.1), `log`, `material-colors` (backdrop colour), and on Android `android-activity`, `jni`, `android_logger`.

## Decision log

| # | Date | Decision | Alternatives | Revisit if |
|---|---|---|---|---|
| D1 | 2026-10-23 | Monorepo, Cargo workspace | Multi-repo | Release cadences diverge strongly |
| D2 | 2026-10-23 | Rust CLI is Milestone 1, launcher second | Launcher first | — |
| D3 | 2026-10-23 | Thin Kotlin shim for Android APIs | Pure Rust JNI | Shim grows beyond ~300 lines of logic |
| D4 | 2026-10-23 | Wrap system `adb` behind `Adb` trait | Pure-Rust ADB client | Users can't install `adb`, or process spawn latency distorts benchmarks |
| D5 | 2026-10-23 | Min Android API 26 | 28 / 30 | Slint raises its minimum |
| D6 | 2026-10-23 | CLI hosts: macOS + Linux first | + Windows | A Windows tester joins |
| D7 | 2026-10-23 | Compat profiles: TOML in repo; the Rust types (`deny_unknown_fields`) are the schema, and a test loads every profile | JSON Schema file / separate repo | Non-Rust tools need to validate profiles, or profile volume needs its own release cadence |
| D8 | 2026-10-23 | No free-form shell in the API | Allow `shell(&str)` | Never for agent paths |
| D9 | 2026-10-23 | Slint under Royalty-free 2.0 with attribution | GPL / commercial | Slint's terms change |
| D10 | 2026-10-23 | Sync core, no tokio in M1 | tokio from day one | MCP or parallel device ops need it |
| D11 | 2026-10-23 | Launcher activity: Kotlin subclass of `NativeActivity`; Slint's Android backend (Skia renderer, ~10 MB) with a size-tuned release profile (spike) | GameActivity; custom platform with Slint's software renderer (no Skia) | Text input/IME needs GameActivity, or APK size becomes a real problem on TVs |
| D12 | 2026-10-23 | Home on Google TV: opt-in Home takeover that disables (never uninstalls) the stock launcher, with automatic rollback and `rusttv launcher restore` | App + remote-button mapping; accessibility-service Home override | Google allows third-party HOME priority, or takeover proves unreliable on tested firmware |
| D13 | 2026-10-23 | MCP server on the official Rust SDK `rmcp` with tokio confined to `rusttv-mcp`; core stays sync and tools run it on the blocking pool | Hand-written sync JSON-RPC over stdio on `serde_json`; `rust-mcp-sdk` | `rmcp` major versions break us often, or tokio leaks into the core |
| D14 | 2026-10-23 | Home takeover also disables any other system package with a Home activity that outranks RustTV, when the device profile lists it (`disable_home_packages`; on the Philips 65OLED818 the setup wizard `setupwraith`, whose `RecoveryActivity` has priority 1) | Making RustTV the Home role holder (tested on the Philips: Home still opened the stock launcher, because Android picks a strictly higher-priority Home activity without consulting the preferred one); disabling only `RecoveryActivity` (refused on Android 14: the adb shell may not change a single activity of a system app); an accessibility-service Home override; parking takeover | Disabling the extra package breaks something the owner needs, or it turns itself back on |

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
│   └── rusttv-launcher/    # home screen: model, Slint UI, JNI bridge; cdylib + desktop dev mode
├── android/                # Gradle app ai.rusttv.launcher: Kotlin shim, manifest, build.sh
├── compat/
│   ├── README.md           # how to contribute a profile
│   ├── protected.toml      # Phase 4: static protected-package deny-list (SECURITY.md)
│   └── devices/*.toml      # one file per device model/firmware family
└── *.md
```

`rusttv-mcp` and `compat/protected.toml` do not exist yet; they are created when their phase starts.

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

There is intentionally no `shell(&str)`. Values that end up in `adb shell` arguments are validated newtypes (e.g. `PackageName`), because the device shell re-parses them. `adb connect` exits 0 even on failure, so its stdout is parsed to detect errors. `SystemAdb` uses `adb` from `PATH`, or the executable named by the `RUSTTV_ADB` environment variable. Synchronous (`std::process`) in Milestone 1; no tokio until MCP needs it. `FakeAdb` is available under the `test-util` feature.

## rusttv-cli

- `clap` derive. Commands map 1:1 to core functions; no logic in the CLI beyond formatting.
- Default output: human text; `--json` gives the versioned `rusttv.v1` envelope. The output contract, exit codes and every error code are in [CLI.md](CLI.md); `rusttv_core::ERROR_CODES` is the canonical list, and tests keep code, list and CLI.md in sync.
- Device-targeting commands pick the only connected device, or require `--serial` when there are several (exit code 2). With `--json`, errors are also a JSON envelope on stdout.
- CLI tests run the real binary against `tests/fixtures/fake-adb` (selected with `RUSTTV_ADB`), so they need no device.
- Logging via `tracing` to stderr (`-v`/`RUST_LOG`); stdout reserved for results.

## Launcher

### Crate `rusttv-launcher`

| Module | Role |
|---|---|
| `model.rs` | Pure state machine `Home`: tile order, D-pad focus, shelf/grid scroll, move mode, launch in flight. Time is passed in; returns `Effect`s. No Slint, no platform. |
| `order_file.rs` | Tile order in `order.toml` (app files dir): atomic write, unreadable file kept as `.bak`. |
| `platform.rs` | `Platform` trait (`list_apps`, `launch`), `PlatformError`, `Bitmap` (shrink, blur, hero backdrop glow), `FakePlatform` (feature `test-util`). |
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
   │
Rust: trait Platform { list_apps(), launch() }
   ├─ AndroidPlatform  — class loaded once via the activity's class loader; JNI calls in local
   │                     frames; Java exceptions cleared and returned as PlatformError
   └─ FakePlatform     — desktop dev mode + tests (generated banners)
```

- Apps are identified by component name (`package/activity`). Banner (320×180, Leanback) preferred; icon (96×96) as fallback.
- Manifest: `MAIN`+`LEANBACK_LAUNCHER` and `MAIN`+`HOME`+`DEFAULT`; `<queries>` for `LEANBACK_LAUNCHER` (no `QUERY_ALL_PACKAGES`); `leanback` and `touchscreen` features `required="false"`; `allowBackup="false"`.
- Lifecycle: Slint's Android event listener forwards `Pause`/`Resume` to `Lifecycle`; resume re-reads the app list. A package-change listener (refresh only when flagged) is a later step.
- ABIs: `arm64-v8a`, `armeabi-v7a` (32-bit userspace TVs) and `x86_64` (emulator), all in one APK.
- Build: `android/build.sh` (cargo-ndk, then Gradle). For arm64-v8a and x86_64, Skia is a prebuilt binary per ABI, fetched by `build.sh` and checked against a pinned SHA-256. skia-binaries has no armeabi-v7a prebuilt, so `build.sh` builds Skia from source for it: the source archive is pinned by commit and SHA-256, and Skia's own `git-sync-deps` fetches its dependencies at the git commits in its `DEPS` file (gn and ninja are fetched by version, without a hash check) (SECURITY.md § Supply chain). Release builds are signed with the debug key until release signing exists (Phase 6).

### Still open

- About page with Slint attribution exists; app details and settings do not.
- Banner cache is in memory only; a disk cache only if measurements justify it.
- Device checks on the Philips TV (focus timings, backdrop look, label readability at 3 m) and benchmarks per BENCHMARK_PLAN.md.

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

Dev/CI tools: `cargo-deny` (covers RustSec advisories + licenses, so `cargo-audit` is redundant), `cargo-nextest` optional. Added later only when needed: `proptest` (parsers), `criterion` (host-side micro-benchmarks), `tokio` + `rmcp` (MCP). Launcher (Phase 1c): `slint`/`slint-build` (=1.18.1), `log`, `material-colors` (backdrop colour), and on Android `android-activity`, `jni`, `android_logger`.

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

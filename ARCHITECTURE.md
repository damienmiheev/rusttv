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
│   ├── rusttv-history/     # TV history in git (gix): snapshots, store, door protocol, TV side (HISTORY.md)
│   └── rusttv-launcher/    # home screen: model, Slint UI, JNI bridge; cdylib + desktop dev mode
├── android/                # Gradle app ai.rusttv.launcher: Kotlin shim, manifest, build.sh, check-dex.sh
├── benchmarks/raw/         # measurements per device and date (BENCHMARK_PLAN.md)
├── spikes/                 # throwaway experiments, kept for their README numbers
├── third_party/            # patched upstream crates (third_party/README.md), not linted
├── compat/
│   ├── README.md           # how to contribute a profile
│   ├── protected.toml      # static protected-package deny-list (SECURITY.md), compiled in
│   ├── packages/*.toml     # privacy package catalog (OPTIMIZER.md § Generic design), compiled in
│   ├── optimizer/*.toml    # Phase 4: optimizer profiles (OPTIMIZER.md)
│   └── devices/*.toml      # one file per device model/firmware family
└── *.md
```

**Naming and boundaries.** Every crate is `crates/rusttv-<name>`, one
feature area each (`rusttv-history` today; `rusttv-remote` for the phone
remote next; later add-ons such as `rusttv-weather`, ROADMAP § Panels). A
shared crate is declared once in the workspace's `[workspace.dependencies]`
and used as `<name>.workspace = true`. Dependencies point one way: the
binaries (`rusttv-cli`, `rusttv-mcp`) use `rusttv-core` and the feature
crates; the launcher uses feature crates but **never** `rusttv-core` (the
safety engine, adb and the update verifier stay off the TV); a feature crate
depends on neither the launcher nor `rusttv-core`, so both sides share it.
Platform code (JNI, Kotlin) stays in the launcher and `android/`.

`compat/optimizer/` holds the optimizer profiles (so far `reduced-animations.toml`), compiled into the binary by `crates/rusttv-core/src/optimizer.rs`. The Phase 4 optimizer (typed `SetSetting`/`GetSetting` commands and the `SettingIs` check on the existing plan engine) is described in [OPTIMIZER.md](OPTIMIZER.md).

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
- Default output: human text; `--json` gives the versioned `rusttv.v5` envelope. The output contract, exit codes and every error code are in [CLI.md](CLI.md); `rusttv_core::ERROR_CODES` is the canonical list, and tests keep code, list and CLI.md in sync.
- Device-targeting commands pick the only connected device, or require `--serial` when there are several (exit code 2). With `--json`, errors are also a JSON envelope on stdout.
- CLI tests run the real binary against `tests/fixtures/fake-adb` (selected with `RUSTTV_ADB`), so they need no device.
- Logging via `tracing` to stderr (`-v`/`RUST_LOG`); stdout reserved for results.

## Launcher

### Crate `rusttv-launcher`

| Module | Role |
|---|---|
| `model.rs` | Pure state machine `Home`: tile order, D-pad focus, shelf/grid scroll, long-press menu, move mode, hidden apps, launch in flight. Time is passed in; returns `Effect`s. No Slint, no platform. |
| `i18n.rs` | Which bundled translation fits the device's language tag (Chinese by script or region, Portuguese to `pt_BR`, else English), and whether it is right-to-left. Translations: `translations/<lang>/LC_MESSAGES/rusttv-launcher.po`, compiled in by `build.rs` (Slint bundled translations, `@tr` in `home.slint`; texts Rust puts together go through the `Strings` global). Right-to-left mirrors positions through the `Dir` global and swaps Left/Right in the model. |
| `order_file.rs` | Tile order, hidden apps and last focused tile in `order.toml` (app files dir): atomic write, unreadable file kept as `.bak`. The history's `[launcher]` section is read from and written to it; the update switch is the `update-check-off` flag file next to it (also read by `Updater.kt`). |
| `history.rs` | The one history thread per process (`History::global`): owns `rusttv_history::tv::TvHistory`; the UI and the door send it jobs. `HistoryPlatform` is what it needs from the platform (desktop: in-memory settings; Android: `android/history.rs`). Packing at background priority, 5 s after a pause, interrupted on resume. |
| `settings.rs`, `review.rs` | Pure state machines for the Settings page (sections, panel rows, what OK asks for) and the review screen (read, Approve, the 4-arrow code, hold OK 2 s). |
| `ui/settings_view.rs` | Draws them and runs the approval: page data and approved changes run on worker threads, picked up by a short timer; `FLAG_SECURE` while the review is open. |
| `platform.rs` | `Platform` trait (`list_apps`, `launch`, `take_apps_changed`, `update_status`, `check_for_update`, `install_update`), `PlatformError`, `UpdateStatus`, `Bitmap` (shrink, blur, hero backdrop glow), `FakePlatform` (feature `test-util`). |
| `ui.rs` + `ui/home.slint` | Rust owns state and timers and pushes properties; Slint draws, animates and forwards keys. `Lifecycle` reports pause/resume. |
| `android.rs`, `android/history.rs` | `android_main` + `AndroidPlatform` over JNI; the history's Android platform and the door's native entry points (`History.native*`). The only modules with `unsafe` (crate lint `unsafe_code = "deny"`, module `allow`, a `SAFETY:` comment per block). |
| `bin/desktop.rs` | Desktop dev mode with `FakePlatform` (Slint winit backend + software renderer, no Skia); `--order <file>`, `--demo <dir>` snapshots, `RUSTTV_LANG=<tag>` for a translation. |

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
   │     language(context): String                 BCP 47 tag, read once at start (i18n.rs)
   │     updateStatus(context): String             "kind" or "kind\tdetail" (Updater.kt, D18)
   │     checkForUpdate/installUpdate(context)     start background work, return at once
   │     setSecure(activity, on)                   FLAG_SECURE + hidden overlays for the review screen
   │     randomCode(): IntArray                    SecureRandom, the review code
   │
   ├─ History.kt (helpers for the Rust history thread, and the door's natives)
   │     setting/putSetting(context, index)        through SecureSettings only (three keys, by index)
   │     bootCount, elapsedMs, randomBytes, testBuild, lowerPriority
   │     external nativeExport/nativePending/nativeCommit   (android/history.rs)
   ├─ SecureSettings.kt  the only settings writer in the APK (check-dex.sh enforces it)
   ├─ HistoryProvider.kt the door: uid 2000 only, onCreate does nothing, loads the library on use
   │
Rust: trait Platform { list_apps(), launch(), take_apps_changed(), update_status(), check_for_update(), install_update() }
   ├─ AndroidPlatform  — class loaded once via the activity's class loader; JNI calls in local
   │                     frames; Java exceptions cleared and returned as PlatformError
   └─ FakePlatform     — desktop dev mode + tests (generated banners)
```

- Apps are identified by component name (`package/activity`). Banner (320×180, Leanback) preferred; icon (96×96) as fallback.
- Manifest: `MAIN`+`LEANBACK_LAUNCHER` and `MAIN`+`HOME`+`DEFAULT`; `<queries>` for `LEANBACK_LAUNCHER` (no `QUERY_ALL_PACKAGES`); `leanback` and `touchscreen` features `required="false"`; `allowBackup="false"`. `INTERNET` and `REQUEST_INSTALL_PACKAGES` for launcher updates only.
- Updates (D18, SECURITY.md § Launcher updates): `Updater.kt` reads GitHub's latest release on resume when the last check is over a day old, and on OK in About; the APK and `SHA256SUMS` are downloaded only when the user installs. Network work runs on one background thread; Rust polls a coarse status string (`UpdateStatus`) every 500 ms only while About is open or a step runs, so the home screen gets no extra work.
- Lifecycle: Slint's Android event listener forwards `Pause`/`Resume` to `Lifecycle`. `LauncherActivity` registers a runtime receiver for package added/changed/removed/replaced (manifest receivers no longer get these since Android 8); it sets a flag in `Bridge`, and resume re-reads the app list only when `Platform::take_apps_changed` reports it.
- ABIs: `arm64-v8a`, `armeabi-v7a` (32-bit userspace TVs) and `x86_64` (emulator), one APK per ABI (Gradle ABI splits, `app-<abi>-release.apk`).
- Build: `android/build.sh` (cargo-ndk, then Gradle). Skia is a prebuilt binary per ABI, fetched by `build.sh` and checked against a pinned SHA-256: rust-skia's for arm64-v8a and x86_64; for armeabi-v7a, one built by our manual workflow `skia-armv7.yml` and hosted as a release of this repository (SECURITY.md § Supply chain). Release builds are signed with the project release key when it is configured (env vars or the git-ignored `android/keystore.properties`, RELEASE.md), else with the debug key. The APK version is the Cargo workspace version. `build.sh` runs `scripts/licenses.sh`, and Gradle packs the result into every APK as `assets/licenses/THIRD_PARTY_NOTICES.txt` (Rust crates, Slint, Skia and its bundled libraries, the NDK's LLVM runtimes, the Rust standard library); the build fails if the file is missing.

### Still open

- About page with Slint attribution and a Settings page (HISTORY.md § 8) exist; app details don't.
- Banner cache is in memory only; a disk cache only if measurements justify it.
- Device checks on the Philips TV (focus timings, backdrop look, label readability at 3 m) and benchmarks per BENCHMARK_PLAN.md.

## MCP (Phase 5)

`rusttv-mcp` exposes the core's read-only functions as MCP tools over local stdio: `device_list`, `device_inspect`, `apps_list`, `launcher_status`, `performance_snapshot`, and the plan tools `launcher_install_plan`, `launcher_uninstall_plan`, `launcher_restore_plan` (contract in [MCP.md](MCP.md)).

- Built on the official Rust SDK `rmcp` (D13). `RustTvServer<A: Adb>` is generic so tests use `FakeAdb`; the binary uses `SystemAdb`.
- The core stays sync (D10): each tool call runs the core function on tokio's blocking pool, holding a mutex around the `Adb`, so calls run one at a time and `Adb` need not be `Sync`.
- Results are the CLI's `rusttv.v6` envelope as `structuredContent`; core errors are tool errors (`isError`) with the CLI's error codes, bad arguments are rejected before any device access.
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

Dev/CI tools: `cargo-deny` (covers RustSec advisories + licenses, so `cargo-audit` is redundant), `cargo-nextest` optional. Added later only when needed: `proptest` (parsers), `criterion` (host-side micro-benchmarks), MCP (Phase 5, `rusttv-mcp` only): `rmcp` 3.7 (features `server`, `macros`, `transport-io`; Apache-2.0) and `tokio` (current-thread runtime). Launcher (Phase 1c): `slint`/`slint-build` (=1.18.1), `log`, `material-colors` (backdrop colour), and on Android `android-activity`, `jni`, `android_logger`. Updates (D17, `rusttv-core`): `minisign-verify` (signature of `SHA256SUMS`), `sha2` (SHA-256 of downloads), `serde_json` (release JSON); test-only, behind `test-util`: `ed25519-compact` and `blake2` (signing fake releases).

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
| D15 | 2026-10-23 | Vendor Slint 1.18.1's `i-slint-renderer-skia` and `i-slint-backend-android-activity` with a patch that reuses one GL context and Skia context across Android windows (`third_party/`, `[patch.crates-io]`); report upstream | Living with a ~1.6 MB driver leak per return to Home; restarting the process periodically; waiting for an upstream fix | Slint ships a fix (then drop the vendored copy), or a Slint upgrade is needed |
| D16 | 2026-10-23 | Optimizer setting steps carry their original (`SetSetting { key, to, from }`), so the inverse and journaled original come from the command alone; `plan::restore` (not rollback) skips a setting that is neither `to` nor `from` right before its undo and returns those "left as is" notes | A separate per-setting original store in the journal; overwriting drifted settings on restore | A setting kind needs an original that can't be validated as a typed value, or the owner wants restore to overwrite drift |
| D17 | 2026-10-24 | `rusttv update`: HTTP by spawning `curl` with a fixed argument list (no shell; `RUSTTV_CURL`, like `RUSTTV_ADB`), HTTPS only with a size cap; trust only from a minisign signature of `SHA256SUMS` by a compiled-in owner key, verified in the core sans-IO (`minisign-verify`, `sha2`); `RUSTTV_UPDATE_DIR` as an offline/test source | `ureq` + `rustls` (pure Rust, but a TLS stack and ~20 crates in every build for a rarely used command); `reqwest` (tokio and hyper in a sync core, D10); trusting HTTPS and GitHub alone (an account or CDN compromise would ship code) | `curl` is missing on a supported host, or its behaviour differs enough between versions to break downloads |
| D18 | 2026-10-24 | Launcher self-update in the Kotlin shim: `HttpsURLConnection` + `org.json` on a single-thread executor, redirects followed by hand with every hop checked against a GitHub host allow-list, the APK checked (size, SHA-256 from `SHA256SUMS`, package, versionCode, signing certificates) and handed to a `PackageInstaller` session; Android's own confirmation and same-key rule are the trust root, `SHA256SUMS.minisig` is not checked on the TV | Updating in Rust (a TLS + HTTP stack such as `ureq`/`rustls` in the APK, plus JNI for PackageInstaller anyway); `DownloadManager` (no redirect host check, needs a public download dir or extra plumbing); OkHttp or another Android dependency; `ACTION_VIEW`/`ACTION_INSTALL_PACKAGE` intents (deprecated, no result); verifying the minisign signature on the TV | The release key is rotated (then signer checks must accept the rotation lineage), GitHub changes its release download hosts, or the launcher gains a settings page (add an off switch) |
| D19 | 2026-10-24 | Delegated approval: `rusttv launcher upgrade` runs restore, install and takeover as three unchanged plans under one typed `upgrade` (an in-memory, per-run grant approves each, since later plans are built against the device after the stage before); `rusttv approve` stores a time-limited grant (one device and firmware, scope enum with only `launcher`, at most 2 h) that `Plan::approve_by_grant` checks for runs without a terminal; journals record `approved_by` (SECURITY.md § Delegated approval) | A `--yes` flag (rejected by the owner); one merged upgrade plan (would break "irreversible step last" and plan against a stale device); signing or encrypting the grant (an agent that can write the file can also read any key on the same machine) | Agents misuse the grant file, or a scope beyond the launcher is wanted |
| D20 | 2026-10-24 | Owner decision: drop the experimental AirPlay tool (`rusttv tools airplay`, the launcher's trigger activity, its typed commands, setting keys and error code) as a dead end for now; no experimental `tools` command group remains. The research notes stay private, outside this repository. Package-catalog entries for vendor AirPlay receivers stay: the privacy optimizer can still disable them | Keeping the tool behind its experimental gate | The owner reopens it with a new dated decision |
| D21 | 2026-10-24 | TV history (HISTORY.md, owner decisions): real git in pure Rust (`gix`, sha1 only, packing in v1) in a shared `rusttv-history` crate with no `rusttv-core` dependency, one store on the TV (launcher-private) and a full bare copy per history on the Mac; the TV and the Mac meet only through a door, a uid-2000-only `ContentProvider`, read with `adb exec-out content read` and written with `adb exec-in content write` + `content call … commit` (new binary `Adb::run_binary`); a new risk class RL (launcher-private, between R0 and R1) for the door's writes, sent around plan-applying commands without an approval of their own; TV approvals (code + hold OK) that the Mac applies only through `history apply-pending` at a terminal, after consuming them through the door, when the live plan is exactly the approved list (`approve_by_tv`, `approved_by = tv`) | A custom store (more code to get right than git); the Mac writing the TV's store directly (`run-as` needs a debuggable build); a separate approval for every door write (a note after each plan would need a second confirmation); silent application of TV approvals on the next connection (rejected by both reviews) | The door's provider is reachable by another uid on some firmware, or gix's size or speed budgets (HISTORY.md § 9) are missed |
| D22 | 2026-10-24 | Launcher side of the history (HISTORY.md § 3–9): one history thread per process owns the store (no locks; the door's binder threads wait for answers, the UI never waits on the key path); the door is a `ContentProvider` in the launcher's process that loads the native library on first use; approvals on the TV are review → Approve → a random 4-arrow code under `FLAG_SECURE` → hold OK 2 s, with doubling lockouts after wrong codes, and are off in debuggable or profileable builds; `WRITE_SECURE_SETTINGS` is used only by `SecureSettings.kt`, enforced by a DEX scan in `build.sh` and a Rust source test; `profileable` only in builds made with `-Prusttv.profileable=true` | A `Mutex<Store>` shared by threads (round 3: a race between a TV entry and a Mac observation); "physical remote only" (measured on the Philips: the adb shell can write `/dev/input` and `/dev/uinput`, so injected keys look physical); a provider in its own process (two writers to one store); no grant in v1 (owner decision 5) | Slint or Android starts exposing window contents in a way `FLAG_SECURE` doesn't cover (e.g. the accessibility tree), or the history thread's work shows up in Home's frame times |

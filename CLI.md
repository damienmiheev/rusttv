# `rusttv` CLI reference

The contract for scripts and AI agents. All commands are **read-only** except
`performance benchmark`, which stops and starts the apps it measures (risk
class R1, transient), `launcher takeover` / `launcher restore`, which
disable or re-enable the stock launcher, and `launcher install` /
`launcher uninstall`, which install or remove the RustTV launcher, and
`optimize apply privacy` / `optimize restore`, which disable or re-enable the
packages your privacy choices select (all R3),
only after a person approves the plan at a terminal (see [Changing a device](#changing-a-device)). Run
`rusttv <command> --help` for every option. Agents can use the read-only
commands, and the `--dry-run` of `launcher install` / `uninstall` / `restore`
and `optimize apply privacy`, as MCP tools too: see [MCP.md](MCP.md).

## Commands

| Command | What it does | `data` in `--json` |
|---|---|---|
| `device list` | Devices known to the local adb server | array of [Device](#device) |
| `device connect <host[:port]>` | `adb connect` (port 5555 by default); only changes state on this computer | [ConnectReport](#connectreport) |
| `device inspect` | Identity, firmware, CPU, memory, storage | [DeviceInfo](#deviceinfo) |
| `apps list` | Installed packages | array of [App](#app) |
| `performance snapshot [--sample-ms N]` | CPU (N ms sample, default 1000), memory, launcher memory, top processes | [PerformanceSnapshot](#performancesnapshot) |
| `performance benchmark --app <package/.Activity>...` | Repeated, interleaved cold start, resume, idle memory and CPU per app (BENCHMARK_PLAN.md); progress on stderr. **Stops and starts the apps**, and wakes the TV if its screensaver comes on. Resume needs two or more apps | [BenchmarkReport](#benchmarkreport) |
| `launcher status` | Which activity the Home button opens, every Home candidate with its priority, whether the RustTV launcher is installed / is Home. Read-only | [LauncherStatus](#launcherstatus) |
| `launcher takeover [--dry-run]` | Make RustTV the Home screen by **disabling** (never uninstalling) the stock launcher (SECURITY.md § Home takeover). Only on devices whose bundled compat profile records a tested takeover (`TAKEOVER_NOT_TESTED` otherwise, even with `--dry-run`). Shows the plan, asks for typed confirmation, verifies Home opens RustTV, presses HOME and waits up to 10 s for RustTV to be in front and running, rolls back otherwise. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher install [--apk <file>] [--dry-run]` | Install the RustTV launcher (`ai.rusttv.launcher`) with `adb install --pkg ai.rusttv.launcher`, which makes Android refuse an APK of any other package. The APK is `android/app/build/outputs/apk/release/app-<abi>-release.apk` (from `android/build.sh`, relative to the current directory, so run from the repository root) for the device's primary ABI, the first of `ro.product.cpu.abilist` (`arm64-v8a`, `armeabi-v7a` or `x86_64`; `INVALID_APK` otherwise), or `--apk <file>` (any directory; file name of ASCII letters, digits, `.`, `_`, `-` ending in `.apk`). Checks RustTV is installed and can handle Home; a fresh install is uninstalled again otherwise. If RustTV is already installed it is an **upgrade** (`adb install -r`, keeps its data) that can't be undone: the plan says so, and it is refused while Home depends on RustTV (as for `uninstall`). Typed confirmation. Doesn't change which app Home opens. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher uninstall [--dry-run]` | Uninstall the RustTV launcher and its data (`pm uninstall ai.rusttv.launcher`; it can remove no other package). Refused (`PRECONDITION_FAILED`) while a Home takeover is active: a takeover journal for this firmware that isn't restored, a journal file that can't be read (it may be one; check it, then move it out of the journal directory), or no system launcher other than RustTV handling Home. Run `launcher restore` first, so the TV is never left with only Android's fallback Home. Then checks RustTV is gone and Home opens the launcher it opened before (or the system launcher, if it opened RustTV). Can't be undone; reinstall with `launcher install`. Typed confirmation. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher restore [--journal <file> \| --from-profile] [--dry-run]` | Undo `launcher takeover`: re-enable the stock launcher (and any other package the takeover disabled) from the takeover's journal (by default the newest one for the device's firmware; each journal file that can't be read is named in a warning on stderr, also with `--json`, and skipped). Without a journal (lost, or the takeover ran on another computer), `--from-profile` rebuilds one from the device's compiled-in profile: it re-enables the packages that profile's takeover disables, only where they are disabled now, and saves the result as a new `<ms>-home-takeover-from-profile.toml` journal. Typed confirmation. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `optimize analyze [--choices <file>]` | Every package of the catalog (`compat/packages/`, OPTIMIZER.md § Generic design) installed on the device, and every `extra` of your [choices](#privacy-choices), in plan order: category (default or opt-in), what it does, side effects, whether it was tested on this model (listed in the matching compat profile's `[[privacy.tested]]`), whether your choices select it, whether it is enabled / disabled, and whether RustTV refuses to touch it (protected). Works on any device; without a matching profile nothing is tested. Read-only | [OptimizeAnalysis](#optimizeanalysis) |
| `optimize apply privacy [--only <categories>] [--choices <file>] [--allow-untested] [--dry-run]` | **Disable** (never uninstall) the packages your [choices](#privacy-choices) select that are installed and enabled: catalog packages in the chosen categories (by default `telemetry`, `ads`, `extras`) or in `also`, plus `extra` ones, minus `keep`. `--only` narrows the categories (comma-separated: `telemetry`, `ads`, `extras`, `casting`, `smart-home`, `voice`, `streaming-apps`); it never adds one, and with it `extra` packages are left out. Order: category, then the order of the profile's tested list, then catalog order, `extra` last. Packages not tested on this model are left out and named in the notes, unless `--allow-untested`: then they are planned and each note says "UNTESTED on this model". With nothing tested or allowed left, `PROFILE_NOT_TESTED` (even with `--dry-run`). Every chosen package is checked live first (SECURITY.md § Protected package policy): if one handles or can handle Home, provides an input method or an enabled accessibility service, or is on `compat/protected.toml`, nothing is planned (`PACKAGE_PROTECTED`). Packages not installed or already disabled are skipped and named in the notes. The protected-package check runs again right before each step, for that step's package; if it fails, the steps done so far are undone (`ROLLED_BACK`). After the last step Home must open the same activity as before, the Home candidates must be unchanged, and the TV input services bound when planning must still be bound (except those of packages the plan disables), otherwise everything is rolled back. Typed confirmation `privacy`. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `optimize restore [--journal <file>] [--dry-run]` | Undo `optimize apply privacy`: re-enable what the privacy journal disabled (by default the newest one for the device's firmware; unreadable journal files are named in a warning on stderr and skipped). Packages already enabled again (e.g. in Settings) are skipped and named in the notes; if every one is, `PRECONDITION_FAILED`, and (except with `--dry-run`) the journal is marked `restored`, so the next `optimize restore` finds the journal before it. Only re-enables: a journal asking for anything else is refused. Typed confirmation `restore`. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `compat profile` | Compatibility profile for `compat/devices/` (TOML on stdout; `--json` wraps it) | `{"path": string, "profile": object}` |

Commands that target a device take `--serial <serial>`. Without it, the only
connected device is used; with several devices connected, `--serial` is
required (`MULTIPLE_DEVICES`).

## Changing a device

Commands that change a device persistently follow SECURITY.md § Change
lifecycle:

1. **Plan** (read-only): what will run, at what risk, what is recorded first,
   what is checked after, and how each step is undone. `--dry-run` stops here
   and prints it (`--json`: a [DryRun](#dryrun)).
2. **Approve**: the plan is applied only after a person types its
   `confirmation` word (`takeover`, `privacy`, `restore`, `install`, `uninstall`) at an
   interactive terminal.
   There is no `--yes` flag. Without a terminal on stdin (scripts, agents,
   MCP) the command fails with `NOT_APPROVED` before contacting the device.
   This proves a terminal, not a person: never run `rusttv` for an agent in
   a pseudo-terminal.
3. **Apply**: the device's `ro.build.fingerprint` must match the plan and each
   step's original state must hold; then the original state is written to a
   journal, and each step is journaled before it runs and verified after.
   If a step or a check fails, every step that ran is undone in reverse
   (`ROLLED_BACK`, or `ROLLBACK_FAILED` with the commands to run by hand).
   Transient steps (pressing HOME) need no undo. Upgrading and uninstalling
   the RustTV launcher can't be undone (putting the old version back needs
   its APK): such a step is always a plan's last, its `undo` is null, the
   plan's notes say so, and a failed check after it is `ROLLBACK_FAILED`.
4. **Restore** later from the journal (`launcher restore`, `optimize restore`). Restoring is never
   rolled back; a failed step is reported with its manual command. If the
   firmware changed since the journal was written, the plan warns and restore
   still runs.

Journals are TOML files in `<state dir>/journals/`, named
`<unix ms>-<plan>.toml`. `launcher restore` only restores takeover
journals and `optimize restore` only privacy journals (`<ms>-privacy.toml`),
so either can be undone without the other; install and uninstall journals are
a record (undo an install with `launcher uninstall`). Each `optimize apply
privacy` writes its own journal (a second run only disables what the first
left enabled); `optimize restore` restores the newest, so run it once per
journal. A journal file that can't be read (edited, or
written by an incompatible build) is never ignored silently: `launcher restore`
warns about it, and `launcher install` (upgrade) and `launcher uninstall` refuse
with `PRECONDITION_FAILED` until it is moved out of the directory. The state directory is `RUSTTV_STATE_DIR` if set,
otherwise `~/Library/Application Support/rusttv` on macOS and
`$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`) on Linux.
Recovery is never guaranteed.

### Privacy choices

`optimize analyze` and `optimize apply privacy` read your choices from
`--choices <file>` or else `<config dir>/privacy.toml`, where the config dir is
`RUSTTV_CONFIG_DIR` if set, otherwise `~/Library/Application Support/rusttv`
on macOS and `$XDG_CONFIG_HOME/rusttv` (default `~/.config/rusttv`) elsewhere.
Without a file the defaults apply. The file is yours; it is never in the
repository:

```toml
categories = ["telemetry", "ads", "extras", "casting"]  # default: telemetry, ads, extras
keep = ["com.google.android.apps.mediashell"]          # never disable these; wins over everything
also = ["org.droidtv.amazonalexa"]                     # catalog packages outside the chosen categories
extra = ["com.example.preinstalled"]                   # packages not in the catalog; always untested
```

Every field is optional; unknown fields and invalid package names are
refused (`INVALID_CHOICES`). `casting`, `smart-home`, `voice` and
`streaming-apps` are features some people use, so they are planned only if
the file chooses them (or names a package in `also`).

## Output

Without `--json`: human-readable text on stdout, errors and hints on stderr.

With `--json`, stdout carries exactly one JSON object, success or failure:

```json
{"schema":"rusttv.v5","ok":true,"data":{}}
{"schema":"rusttv.v5","ok":false,"error":{"code":"DEVICE_UNAUTHORIZED","message":"device `192.168.1.20:5555` is unauthorized; accept the debugging prompt on the TV"}}
```

- `schema` identifies the contract version. Within `rusttv.v5`, fields are only
  added, never renamed, removed or changed in meaning; anything else bumps it.
  - `rusttv.v5`: [OptimizeAnalysis](#optimizeanalysis) lists catalog packages
    (`privacy` with its `tested` flag and `packages` is gone; `choices`,
    `categories` and `packages` with `source`, `default`, `tested`,
    `chosen` are new); categories `casting`, `smart-home`, `voice`,
    `streaming-apps` are new; new error code `INVALID_CHOICES`;
    `PROFILE_NOT_TESTED` now means no tested or allowed package is left.
    `optimize apply privacy` takes `--choices` and `--allow-untested`.
    Everything else is unchanged.
  - `rusttv.v4`: new error codes `PROFILE_NOT_TESTED` and `PACKAGE_PROTECTED`
    (`optimize apply privacy`); new commands `optimize analyze`, `optimize
    apply privacy`, `optimize restore`. Everything else is unchanged.
  - `rusttv.v3`: in [BenchmarkReport](#benchmarkreport), `apps[].idle_pss_kib`
    is renamed `idle_pss_kib_after_resumes` (same measurement), and
    `apps[].idle_pss_kib_fresh`, `apps[].idle_pss_growth_kib` and
    `options.fresh_settle_ms` are new. Everything else is unchanged.
  - `rusttv.v2`: `pm list packages` printing an error is now the
    `PACKAGE_LIST_FAILED` error. In `rusttv.v1` it read as an empty list
    (e.g. `apps list` returned `[]`); success output is unchanged.
- `error.code` is stable and meant for programs. `error.message` is for people
  and may change.
- Absent values are `null`, never omitted. Sizes are in KiB (`*_kib`),
  durations in milliseconds (`*_ms`).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | The operation failed (adb missing, command failed, unexpected output) |
| 2 | Usage error: bad arguments, or several devices and no `--serial` |
| 3 | The device can't be reached: not found, unauthorized, offline, connection failed |

## Error codes

| Code | Exit | Meaning |
|---|---|---|
| `ADB_NOT_FOUND` | 1 | `adb` isn't installed or not on `PATH` (set `RUSTTV_ADB`) |
| `ADB_SPAWN_FAILED` | 1 | `adb` exists but couldn't be started |
| `NO_DEVICE` | 3 | No device is connected |
| `MULTIPLE_DEVICES` | 2 | Several devices are connected; pass `--serial` |
| `DEVICE_NOT_FOUND` | 3 | The requested serial isn't connected |
| `DEVICE_UNAUTHORIZED` | 3 | The TV hasn't accepted this computer's debugging key yet |
| `DEVICE_OFFLINE` | 3 | adb knows the device but it isn't responding |
| `DEVICE_NOT_READY` | 3 | The device is in another state (e.g. recovery) |
| `CONNECT_FAILED` | 3 | `adb connect` failed (refused, unknown host, ...) |
| `COMMAND_FAILED` | 1 | An adb command failed for another reason; the message has its stderr |
| `INVALID_OUTPUT` | 1 | adb printed something that isn't UTF-8 |
| `PACKAGE_LIST_FAILED` | 1 | `pm list packages` printed an error (e.g. `cmd: Can't find service: package`, `Exception occurred ...`) instead of a package list; the message has the line. An empty list is never this error; nothing changed |
| `LAUNCH_FAILED` | 1 | `am start` couldn't start a benchmarked activity (wrong component, app not installed) |
| `INCOMPLETE_PROFILE` | 1 | The device didn't report a value a compatibility profile needs |
| `INVALID_PLAN` | 1 | A plan or journal has a step that can't be undone, or doesn't match its journal (a bug or an edited journal) |
| `PRECONDITION_FAILED` | 1 | The device isn't in the state the plan needs (e.g. RustTV not installed, Home isn't a stock launcher, firmware changed since planning); nothing changed |
| `NOT_APPROVED` | 1 | No interactive terminal, or the confirmation wasn't typed; nothing changed |
| `JOURNAL_FAILED` | 1 | A journal couldn't be read or written; if a change had started, it was rolled back |
| `TAKEOVER_NOT_TESTED` | 1 | No bundled compat profile matches this device's model and firmware family, or it doesn't record a tested takeover of the stock launcher Home opens; nothing changed |
| `PROFILE_NOT_TESTED` | 1 | `optimize apply`: none of the chosen packages that are enabled on the device is tested on this model (no compat profile matches, or its `[[privacy.tested]]` doesn't list them), and `--allow-untested` wasn't given; the message names them; nothing changed |
| `PACKAGE_PROTECTED` | 1 | `optimize apply`: your choices select a package RustTV never disables (it handles or can handle Home, provides an input method or an enabled accessibility service, or is on `compat/protected.toml`); the message names each one and why (add it to `keep`, or remove it from `extra`); nothing changed |
| `INVALID_CHOICES` | 1 | `optimize`: the [choices file](#privacy-choices) can't be read or parsed (a file given with `--choices` must exist), `also` names a package not in the catalog, or `extra` one that is; nothing changed |
| `INVALID_APK` | 1 | `launcher install` found no APK to install: no APK is built for the device's primary ABI, the file is missing or not a file, or its path isn't absolute UTF-8 with a plain `.apk` file name; nothing changed |
| `NO_JOURNAL` | 1 | `restore` found no journal to restore for this device; pass `--journal <file>` |
| `ROLLED_BACK` | 1 | A step or its check failed; every step that ran was undone and verified |
| `ROLLBACK_FAILED` | 1 | A step failed **and** undoing failed; the message has the commands to run by hand |
| `RESTORE_FAILED` | 1 | `restore` couldn't undo a step; the message has the commands to run by hand |
| `INTERNAL` | 1 | Output couldn't be serialized (a bug; please report it) |

## Environment

| Variable | Effect |
|---|---|
| `RUSTTV_ADB` | Path of the `adb` executable to use instead of the one on `PATH` |
| `RUSTTV_STATE_DIR` | Directory for journals instead of the platform default ([Changing a device](#changing-a-device)) |
| `RUSTTV_CONFIG_DIR` | Directory of the privacy choices file `privacy.toml` instead of the platform default ([Privacy choices](#privacy-choices)) |
| `RUST_LOG` | Reserved for logging (not used yet) |

## Data types

### Device

`serial` (string), `state` (`device` \| `unauthorized` \| `offline` \| other
adb state, e.g. `no permissions`), `model`, `product`, `device` (string or null).

### ConnectReport

`address` (`host:port`), `outcome` (`connected` \| `already_connected`),
`state` (as in [Device](#device), or null if adb doesn't list it yet).

### DeviceInfo

`serial`; `manufacturer`, `brand`, `model`, `device`, `android_version`,
`security_patch`, `build_fingerprint`, `build_characteristics` (string or
null); `api_level` (integer or null); `abis` (array of strings, may be empty);
`memory` (`{total_kib, available_kib}` or null; `available_kib` may be null);
`data_storage` (`{total_kib, available_kib}` or null).

### App

`package`, `apk_path` (string or null), `kind` (`system` \| `third_party`),
`enabled` (boolean).

### PerformanceSnapshot

`serial`; `taken_at_unix_ms` (host clock); `build_fingerprint` (string or
null); `memory` (as in [DeviceInfo](#deviceinfo)); `cpu` (`{busy_percent,
sample_ms}` or null when no CPU time elapsed); `home_app` (`{package,
activity, pss_kib, rss_kib}` or null; memory values are null when the
launcher isn't running); `top_processes` (up to 10 `{pid, name, rss_kib}`,
largest first).

### BenchmarkReport

`rusttv_version`; `device` (`serial`, `manufacturer`, `model`, `api_level`,
`build_fingerprint`); `options` (`runs`, `warmup`, `pause_ms`, `settle_ms`,
`fresh_settle_ms`, `cpu_window_ms`, `idle_samples`); `apps`: one per `--app`,
with `component`, `cold_start_ms`, `resume_ms`, `idle_pss_kib_fresh`
(after a fresh cold start and `fresh_settle_ms`, before any series),
`idle_pss_kib_after_resumes` (after the cold-start and resume series and
`settle_ms`; with one app there is no resume series), `idle_cpu_percent`
(after the series; each a [Stats](#stats) or null), `idle_pss_growth_kib`
(median after resumes minus median fresh; positive when the series left the
app bigger; null when either is missing) and `discarded_runs`; `runs`: every launch with
`component`, `kind` (`cold` \| `resume`), `round`, `launch_state` (Android's
`COLD`/`WARM`/`HOT`/...), `total_time_ms`, `wait_time_ms`, `counted`,
`discarded_because` (string or null).

### LauncherStatus

`home` (`package/activity` or null); `candidates` (`{component, priority}`,
highest priority first: Android opens the highest without asking, so on
Google TV the stock launcher's priority 2 beats every other launcher's 0);
`rusttv_installed`, `rusttv_is_home` (booleans; package `ai.rusttv.launcher`).

### DryRun

`plan` (e.g. `home-takeover`, `home-takeover-restore`, `launcher-install`,
`launcher-uninstall`, `privacy`, `privacy-restore`); `serial`;
`fingerprint` (`ro.build.fingerprint` the plan is for); `max_risk` (`R0` to
`R3`); `confirmation` (the word to type); `steps`: in order, each with
`risk`, `command` (exact adb command line), `records` (original state checked
and journaled first, or null), `verify` (array of checks after the command),
`undo` (adb command line, or null), `poll_ms` (how long the checks are
retried before the step fails, or null); `notes` (array of strings for people:
recovery hints, warnings).

### OptimizeAnalysis

`serial`; `profile` (file name of the bundled compat profile matching the
device, e.g. `tpv-ph1m-ww-9972.toml`, or null: then nothing is tested);
`choices` (path of the choices file read, or null for the defaults);
`categories` (the categories the choices apply); `packages`: every catalog
package installed on the device and every `extra` of the choices (installed
or not), in plan order, each with `package`, `source` (`catalog` \|
`extra`), `category` (`telemetry` \| `ads` \| `extras` \| `casting` \|
`smart-home` \| `voice` \| `streaming-apps`, or null for an `extra`),
`default` (boolean: the category is on unless opted out), `what` and
`side_effects` (null for an `extra`), `tested` (boolean: listed in the
profile's `[[privacy.tested]]`), `checked` (date it was tested on this
model, or null), `chosen` (boolean: the choices select it; `optimize apply
privacy` plans it if it is enabled and tested, or with `--allow-untested`),
`state` (`absent` \| `enabled` \| `disabled`) and `protected` (why RustTV
refuses to disable it, or null).

### ChangeReport

`journal` (path of the journal file); `status` (`applied` \| `restored`).

### Stats

`n`, `min`, `p10`, `median`, `p90`, `max`, `iqr` (p75 − p25). Percentiles use
the nearest-rank method.

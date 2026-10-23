# `rusttv` CLI reference

The contract for scripts and AI agents. All commands are **read-only** except
`performance benchmark`, which stops and starts the apps it measures (risk
class R1, transient), and `launcher takeover` / `launcher restore`, which
disable or re-enable the stock launcher (R3) only after a person approves the
plan at a terminal (see [Changing a device](#changing-a-device)). Run
`rusttv <command> --help` for every option.

## Commands

| Command | What it does | `data` in `--json` |
|---|---|---|
| `device list` | Devices known to the local adb server | array of [Device](#device) |
| `device connect <host[:port]>` | `adb connect` (port 5555 by default); only changes state on this computer | [ConnectReport](#connectreport) |
| `device inspect` | Identity, firmware, CPU, memory, storage | [DeviceInfo](#deviceinfo) |
| `apps list` | Installed packages | array of [App](#app) |
| `performance snapshot [--sample-ms N]` | CPU (N ms sample, default 1000), memory, launcher memory, top processes | [PerformanceSnapshot](#performancesnapshot) |
| `performance benchmark --app <package/.Activity>...` | Repeated, interleaved cold start, resume, idle memory and CPU per app (BENCHMARK_PLAN.md); progress on stderr. **Stops and starts the apps.** Resume needs two or more apps | [BenchmarkReport](#benchmarkreport) |
| `launcher status` | Which activity the Home button opens, every Home candidate with its priority, whether the RustTV launcher is installed / is Home. Read-only | [LauncherStatus](#launcherstatus) |
| `launcher takeover [--dry-run]` | Make RustTV the Home screen by **disabling** (never uninstalling) the stock launcher (SECURITY.md § Home takeover). Only on devices whose bundled compat profile records a tested takeover (`TAKEOVER_NOT_TESTED` otherwise, even with `--dry-run`). Shows the plan, asks for typed confirmation, verifies Home opens RustTV, presses HOME and waits up to 10 s for RustTV to be in front and running, rolls back otherwise. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher restore [--journal <file>] [--dry-run]` | Undo `launcher takeover`: re-enable the stock launcher from the takeover's journal (by default the newest one for the device's firmware). Typed confirmation. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
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
   `confirmation` word (`takeover`, `restore`) at an interactive terminal.
   There is no `--yes` flag. Without a terminal on stdin (scripts, agents,
   MCP) the command fails with `NOT_APPROVED` before contacting the device.
   This proves a terminal, not a person: never run `rusttv` for an agent in
   a pseudo-terminal.
3. **Apply**: the device's `ro.build.fingerprint` must match the plan and each
   step's original state must hold; then the original state is written to a
   journal, and each step is journaled before it runs and verified after.
   If a step or a check fails, every step that ran is undone in reverse
   (`ROLLED_BACK`, or `ROLLBACK_FAILED` with the commands to run by hand).
   Transient steps (pressing HOME) need no undo.
4. **Restore** later from the journal (`launcher restore`). Restoring is never
   rolled back; a failed step is reported with its manual command. If the
   firmware changed since the journal was written, the plan warns and restore
   still runs.

Journals are TOML files in `<state dir>/journals/`, named
`<unix ms>-<plan>.toml`. The state directory is `RUSTTV_STATE_DIR` if set,
otherwise `~/Library/Application Support/rusttv` on macOS and
`$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`) on Linux.
Recovery is never guaranteed.

## Output

Without `--json`: human-readable text on stdout, errors and hints on stderr.

With `--json`, stdout carries exactly one JSON object, success or failure:

```json
{"schema":"rusttv.v1","ok":true,"data":{}}
{"schema":"rusttv.v1","ok":false,"error":{"code":"DEVICE_UNAUTHORIZED","message":"device `192.168.1.20:5555` is unauthorized; accept the debugging prompt on the TV"}}
```

- `schema` identifies the contract version. Within `rusttv.v1`, fields are only
  added, never renamed, removed or changed in meaning; anything else bumps it.
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
| `LAUNCH_FAILED` | 1 | `am start` couldn't start a benchmarked activity (wrong component, app not installed) |
| `INCOMPLETE_PROFILE` | 1 | The device didn't report a value a compatibility profile needs |
| `INVALID_PLAN` | 1 | A plan or journal has a step that can't be undone, or doesn't match its journal (a bug or an edited journal) |
| `PRECONDITION_FAILED` | 1 | The device isn't in the state the plan needs (e.g. RustTV not installed, Home isn't a stock launcher, firmware changed since planning); nothing changed |
| `NOT_APPROVED` | 1 | No interactive terminal, or the confirmation wasn't typed; nothing changed |
| `JOURNAL_FAILED` | 1 | A journal couldn't be read or written; if a change had started, it was rolled back |
| `TAKEOVER_NOT_TESTED` | 1 | No bundled compat profile matches this device's model and firmware family, or it doesn't record a tested takeover of the stock launcher Home opens; nothing changed |
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
`cpu_window_ms`, `idle_samples`); `apps`: one per `--app`, with `component`,
`cold_start_ms`, `resume_ms`, `idle_pss_kib`, `idle_cpu_percent` (each a
[Stats](#stats) or null) and `discarded_runs`; `runs`: every launch with
`component`, `kind` (`cold` \| `resume`), `round`, `launch_state` (Android's
`COLD`/`WARM`/`HOT`/...), `total_time_ms`, `wait_time_ms`, `counted`,
`discarded_because` (string or null).

### LauncherStatus

`home` (`package/activity` or null); `candidates` (`{component, priority}`,
highest priority first: Android opens the highest without asking, so on
Google TV the stock launcher's priority 2 beats every other launcher's 0);
`rusttv_installed`, `rusttv_is_home` (booleans; package `ai.rusttv.launcher`).

### DryRun

`plan` (e.g. `home-takeover`, `home-takeover-restore`); `serial`;
`fingerprint` (`ro.build.fingerprint` the plan is for); `max_risk` (`R0` to
`R3`); `confirmation` (the word to type); `steps`: in order, each with
`risk`, `command` (exact adb command line), `records` (original state checked
and journaled first, or null), `verify` (array of checks after the command),
`undo` (adb command line, or null), `poll_ms` (how long the checks are
retried before the step fails, or null); `notes` (array of strings for people:
recovery hints, warnings).

### ChangeReport

`journal` (path of the journal file); `status` (`applied` \| `restored`).

### Stats

`n`, `min`, `p10`, `median`, `p90`, `max`, `iqr` (p75 − p25). Percentiles use
the nearest-rank method.

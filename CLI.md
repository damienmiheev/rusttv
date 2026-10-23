# `rusttv` CLI reference

The contract for scripts and AI agents. All commands are **read-only**: none
changes anything on the device. Run `rusttv <command> --help` for every option.

## Commands

| Command | What it does | `data` in `--json` |
|---|---|---|
| `device list` | Devices known to the local adb server | array of [Device](#device) |
| `device connect <host[:port]>` | `adb connect` (port 5555 by default); only changes state on this computer | [ConnectReport](#connectreport) |
| `device inspect` | Identity, firmware, CPU, memory, storage | [DeviceInfo](#deviceinfo) |
| `apps list` | Installed packages | array of [App](#app) |
| `performance snapshot [--sample-ms N]` | CPU (N ms sample, default 1000), memory, launcher memory, top processes | [PerformanceSnapshot](#performancesnapshot) |
| `compat profile` | Compatibility profile for `compat/devices/` (TOML on stdout; `--json` wraps it) | `{"path": string, "profile": object}` |

Commands that target a device take `--serial <serial>`. Without it, the only
connected device is used; with several devices connected, `--serial` is
required (`MULTIPLE_DEVICES`).

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
| `INCOMPLETE_PROFILE` | 1 | The device didn't report a value a compatibility profile needs |
| `INTERNAL` | 1 | Output couldn't be serialized (a bug; please report it) |

## Environment

| Variable | Effect |
|---|---|
| `RUSTTV_ADB` | Path of the `adb` executable to use instead of the one on `PATH` |
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

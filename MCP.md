# `rusttv-mcp` reference

`rusttv-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io)
server that gives AI agents RustTV's **read-only** commands as tools, plus
tools that **plan** a launcher or privacy change and return its dry run. It talks over
stdin/stdout (the stdio transport) and calls the same `rusttv-core` functions
as the CLI. It is a local process on your computer; it opens no network port.

**It cannot change a device.** No tool runs a command above risk class R0,
none writes a journal, and none can approve or apply a plan. A plan tool
returns the command a person runs at a terminal; the change happens only
there, after they read the plan and type its confirmation (SECURITY.md § AI
agents, CLI.md § Changing a device).

## Run it

```bash
cargo build --release -p rusttv-mcp
```

Then register `target/release/rusttv-mcp` with your MCP client as a stdio
server with no arguments. For example, in Claude Code:

```bash
claude mcp add rusttv -- /path/to/rusttv/target/release/rusttv-mcp
```

It uses `adb` from `PATH`, or the executable named by `RUSTTV_ADB` (as the
CLI does). Start the adb server and connect the TV first (`rusttv device
connect <address>`); the MCP server does not connect devices.

## Tools

| Tool | Arguments | Same as | `data` |
|---|---|---|---|
| `device_list` | none | `rusttv device list --json` | array of [Device](CLI.md#device) |
| `device_inspect` | `serial?` | `rusttv device inspect --json` | [DeviceInfo](CLI.md#deviceinfo) |
| `apps_list` | `serial?` | `rusttv apps list --json` | array of [App](CLI.md#app) |
| `launcher_status` | `serial?` | `rusttv launcher status --json` | [LauncherStatus](CLI.md#launcherstatus) |
| `performance_snapshot` | `serial?`, `sample_ms?` (100–60000, default 1000) | `rusttv performance snapshot --json` | [PerformanceSnapshot](CLI.md#performancesnapshot) |
| `launcher_install_plan` | `serial?`, `apk?` | `rusttv launcher install --dry-run --json` | [PlanProposal](#planproposal) |
| `launcher_uninstall_plan` | `serial?` | `rusttv launcher uninstall --dry-run --json` | [PlanProposal](#planproposal) |
| `launcher_restore_plan` | `serial?`, `from_profile?` (default false) | `rusttv launcher restore --dry-run --json` | [PlanProposal](#planproposal) |
| `optimize_analyze` | `serial?` | `rusttv optimize analyze --json` | [OptimizeAnalysis](CLI.md#optimizeanalysis) |
| `privacy_plan` | `serial?`, `only?` (array of `telemetry` \| `ads` \| `extras`; default all) | `rusttv optimize apply privacy [--only ...] --dry-run --json` | [PlanProposal](#planproposal) |

`serial` picks the device like the CLI's `--serial`: without it, the only
connected device is used; with several connected, it is required
(`MULTIPLE_DEVICES`). Every tool is marked `readOnlyHint`.

Not exposed: `device connect` (changes adb state on the computer),
`performance benchmark` (stops and starts apps, R1), `launcher takeover`
(not even as a plan: SECURITY.md § Home takeover rule 1, MCP cannot request
it), `optimize restore` (not even as a plan, for now), applying or approving
any plan, and `compat profile`. An apply or approve tool never will be.

## Plan tools

`launcher_install_plan`, `launcher_uninstall_plan`, `launcher_restore_plan`
and `privacy_plan` build the same plan as the CLI command with `--dry-run` and change nothing:
they run only R0 commands, write no journal, and create no approval. The
plan's preconditions are checked as in the CLI, so a plan the CLI would
refuse fails here with the same code (`PRECONDITION_FAILED` for uninstalling
during a Home takeover, `NO_JOURNAL`, `INVALID_APK`, ...).

- `launcher_install_plan`'s `apk` is an absolute path on the computer running
  the server, with a file name of ASCII letters, digits, `.`, `_`, `-` ending
  in `.apk` (the CLI's `--apk` rule). Its shape is checked before the device
  is contacted, so an agent can't probe arbitrary files. Without `apk`, the
  APK `android/build.sh` builds for the device's primary ABI, relative to
  the server's working directory (`INVALID_APK` if it isn't there).
- `launcher_restore_plan` uses the newest takeover journal for the device's
  firmware in the CLI's state directory (`RUSTTV_STATE_DIR`, CLI.md
  § Changing a device). It takes no journal path. `.toml` files there that
  can't be read are not skipped silently: each is named, with the reason, in
  an extra text content item after the envelope (one per file), or in the
  `NO_JOURNAL` message when no readable journal is left. Any of them may be a
  takeover that still needs restoring.
- `launcher_restore_plan` with `from_profile: true` is the CLI's
  `--from-profile`, for when the journal is lost: it reads no journal and
  plans re-enabling what the device's compiled-in compatibility profile says a
  takeover disables, of the packages disabled now. `PRECONDITION_FAILED` if
  none are; `NO_JOURNAL` if no profile with a `[launcher]` section matches the
  device. `apply_command` is then `rusttv launcher restore --serial <s>
  --from-profile`. It still writes no journal; the CLI saves one when the
  person applies it.

- `privacy_plan` plans the device's privacy profile from its bundled compat
  profile (`PROFILE_NOT_TESTED` unless it is marked tested on this model;
  `PACKAGE_PROTECTED` if it lists a package RustTV never disables). It takes
  no package names: `only` is a closed list of categories, and anything else
  is rejected before the device is contacted. `apply_command` is `rusttv
  optimize apply privacy --serial <s>`, plus `--only <categories>` if given.

### PlanProposal

`dry_run`: a [DryRun](CLI.md#dryrun), exactly what the CLI prints with
`--dry-run --json`. `apply_command`: the `rusttv` command line that applies
this plan, for a person to run at an interactive terminal, with
`--serial` and the planned APK (`--apk`) or journal (`--journal`), or
`--from-profile`, spelled out so it plans the same change; arguments are quoted for a POSIX shell. For
example:

```json
{"dry_run": {"plan": "launcher-uninstall", "serial": "emulator-5554", "...": "..."},
 "apply_command": "rusttv launcher uninstall --serial emulator-5554"}
```

Running it re-plans against the device as it is then, shows the plan, and
asks for the `confirmation` word; without an interactive terminal it fails
with `NOT_APPROVED`. An agent should show the dry run and the command to the
user, not run it: never give an agent a terminal running `rusttv`
(SECURITY.md § Change lifecycle, step 4).

## Results

A tool result's `structuredContent` is the CLI's `rusttv.v4` envelope
(CLI.md § Output), and its first text content item is the same JSON as a
string (`launcher_restore_plan` may add warnings as further text items, see
§ Plan tools):

```json
{"schema":"rusttv.v4","ok":true,"data":{}}
{"schema":"rusttv.v4","ok":false,"error":{"code":"NO_DEVICE","message":"no devices found; connect one with `rusttv device connect <address>`"}}
```

- Success: `isError` is `false` and `data` is exactly what the CLI prints.
- Device or adb failure: `isError` is `true` and `error.code` is one of the
  stable codes in CLI.md § Error codes (`NO_DEVICE`, `MULTIPLE_DEVICES`,
  `DEVICE_UNAUTHORIZED`, `COMMAND_FAILED`, ...).
- Bad arguments (an unknown tool, an unknown or mistyped argument,
  `sample_ms` out of range) are rejected before anything runs: either a
  JSON-RPC error, or `isError: true` with a plain-text message and no
  envelope.

Tool calls run one at a time.

## Implementation

- Crate `crates/rusttv-mcp`: library `RustTvServer<A: Adb>` plus the
  `rusttv-mcp` binary. Built on the official Rust SDK `rmcp` with tokio,
  which only this crate uses; the core stays synchronous and each call runs on
  tokio's blocking pool (ARCHITECTURE.md D10, D13).
- Tests (`crates/rusttv-mcp/tests/server.rs`) drive the server over an
  in-memory pipe with raw JSON-RPC and `FakeAdb`, and check that each tool's
  `data` equals the core result the CLI prints, that plan tools issue no
  command above R0 and leave the journal directory unchanged (including
  `launcher_restore_plan` with `from_profile`, against the Philips capture
  and the bundled profile), that unreadable journal files are reported, and that
  invalid arguments and APK paths are rejected. No device or adb is needed.
- Checked with a real MCP client on 2026-10-23: MCP Inspector 2.12.0 in CLI
  mode (`npx @modelcontextprotocol/inspector --cli target/debug/rusttv-mcp
  ...`) against the CLI tests' `fake-adb` script as `RUSTTV_ADB`: tools listed,
  the plan tools returned the same dry run as the CLI, and adb saw only
  `devices`, `getprop`, `pm list packages`, `cmd package` queries. Not yet
  run against a real device.

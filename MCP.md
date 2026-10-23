# `rusttv-mcp` reference

`rusttv-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io)
server that gives AI agents RustTV's **read-only** commands as tools. It talks
over stdin/stdout (the stdio transport) and calls the same `rusttv-core`
functions as the CLI. It is a local process on your computer; it opens no
network port.

**It cannot change a device.** No tool runs a command above risk class R0, and
none can approve a plan. Device changes are planned and approved by a person
with the `rusttv` CLI (SECURITY.md § AI agents, CLI.md § Changing a device).

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

`serial` picks the device like the CLI's `--serial`: without it, the only
connected device is used; with several connected, it is required
(`MULTIPLE_DEVICES`). Every tool is marked `readOnlyHint`.

Not exposed: `device connect` (changes adb state on the computer),
`performance benchmark` (stops and starts apps, R1), `launcher takeover` and
`launcher restore` (R3), and `compat profile`. Plan tools that return only a
dry run may come later (ROADMAP.md Phase 5); an approve tool never will.

## Results

A tool result's `structuredContent` is the CLI's `rusttv.v2` envelope
(CLI.md § Output), and its text content is the same JSON as a string:

```json
{"schema":"rusttv.v2","ok":true,"data":{}}
{"schema":"rusttv.v2","ok":false,"error":{"code":"NO_DEVICE","message":"no devices found; connect one with `rusttv device connect <address>`"}}
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
  `data` equals the core result the CLI prints. No device or adb is needed.

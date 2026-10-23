# Security and Device Safety

## Reporting vulnerabilities

Until a dedicated address exists, report privately via GitHub Security Advisories on this repository. Do not open public issues for vulnerabilities.

## Absolute restrictions

RustTV never:

- uses root, flashes firmware, unlocks bootloaders, or modifies system partitions;
- uninstalls the stock launcher, or disables it outside the opt-in [Home takeover](#home-takeover-opt-in) flow;
- removes packages (`pm uninstall --user 0` is not supported in the MVP);
- executes free-form shell strings — device access only through typed `DeviceCommand`s;
- lets an AI agent apply a change without a human approving the specific plan;
- sends data off the user's machine (no telemetry).

## Risk classes

Every `DeviceCommand` declares one:

| Class | Meaning | Examples | Who may run it |
|---|---|---|---|
| R0 Read | No state change | `getprop`, `pm list packages`, `dumpsys meminfo` | CLI, MCP freely |
| R1 Transient | Changes runtime state only; reboot or app restart undoes it | `am start`, `am force-stop`, `input keyevent` | CLI; MCP for allowlisted targets |
| R2 Setting | Persistent, trivially reversible setting | `settings put global animator_duration_scale 0.5`, set default home | Only via an approved plan |
| R3 Package state | Enable/disable an app for user 0 | `pm disable-user --user 0` | Only via an approved plan, Phase 4+, known-device profile only |
| R4 Forbidden | Not implementable | uninstall, stock-launcher changes outside Home takeover, system partition writes | Nobody |

So far only R0 commands exist, plus two R1 commands for benchmarking (`am force-stop`, `am start`); a unit test fails if anything else appears. R2/R3 wait for the plan/approve/journal engine.

## Protected package policy

A package is protected (no R3 action) if **any** of these is true:

1. It resolves `HOME`, or is the current/previous default home.
2. It provides an input method, accessibility service, TV input (HDMI/tuner), setup wizard, system UI, settings, package installer, network stack, Bluetooth/remote pairing, or Google Play services.
3. It is on the static deny-list in `compat/protected.toml`.
4. The device has no profile marking the package as tested-safe for that model **and** firmware family.

Rules 1–2 are computed live on the device each time, not cached. Unknown devices get R0–R1 only.

## Change lifecycle (R2–R3)

1. **Analyze** (read-only): collect device info and current state.
2. **Plan**: write a plan file (TOML) with device fingerprint (`ro.build.fingerprint`), steps, risk class per step, and the original value for each step.
3. **Dry-run**: render the plan as human-readable text with exact commands.
4. **Approve**: a human confirms in the CLI (typed confirmation for R3). MCP tools can create plans but not approve them.
5. **Apply**: precondition check (same fingerprint, device reachable, protected-package check re-run). Write a journal entry *before* each step; run one step; verify its effect; stop on the first failure.
6. **Verify**: re-read state; confirm HOME still resolves to a launchable activity.
7. **Restore**: `rusttv optimize restore` replays recorded originals from the journal in reverse order.

Plans and journals live in a local state directory (`~/.local/state/rusttv/` on Linux, the platform equivalent on macOS) and are plain text for auditability.

## Recovery scenarios

| Scenario | Design response | Residual risk |
|---|---|---|
| RustTV launcher crashes on boot | Without Home takeover the stock launcher still owns Home. With it: `rusttv launcher restore` (re-enables the stock launcher over ADB); manual path documented for Settings → Apps → System apps → enable | Some firmware may loop; user can still uninstall via ADB |
| ADB connection lost mid-apply | Journal shows the last completed step; `restore` resumes when reconnected | If Wi-Fi ADB is disabled by a reset, USB or manual settings needed |
| Device reboots mid-apply | Same as above; R2/R3 states persist predictably | — |
| Firmware update re-enables/changes packages | `optimize analyze` detects fingerprint change and marks plan stale | Behavior of new firmware is unknown until tested |
| Network ADB unavailable | All TV-side functionality (launcher) works without it | — |
| Restore fails | Print exact manual commands and Settings paths per step | **Recovery is never guaranteed**; we say so in docs and CLI output |

## Home takeover (opt-in)

*Owner decision 2026-10-23 (ARCHITECTURE D12).* On Google TV the stock launcher declares a higher HOME priority than any third-party app can (RESEARCH §8), so RustTV can only become the Home screen if the stock launcher is disabled. This is the **only** permitted change to the stock launcher, and it is R3 with these extra rules:

1. **Opt-in only.** Never part of an optimization profile, never suggested by default; a separate command (`rusttv launcher takeover`) with typed confirmation. MCP cannot request it.
2. **Known devices only.** Offered only when the device's compat profile names its stock launcher package and records a successful takeover and restore on that firmware family.
3. **Preconditions.** RustTV launcher installed, launchable, and verified drawing frames; ADB connected; the stock launcher package is the current HOME resolver.
4. **Disable, never uninstall.** `pm disable-user --user 0 <stock launcher>` only; original state recorded in the journal first.
5. **Verify or roll back automatically.** After disabling, HOME must resolve to RustTV and RustTV must come to the foreground on a HOME key event within a timeout; otherwise re-enable the stock launcher immediately.
6. **Restore is one command.** `rusttv launcher restore` re-enables the stock launcher; `rusttv launcher uninstall` restores *before* removing RustTV, so the TV is never left with only Android's fallback Home.
7. **Documented manual recovery** in the user docs, for when ADB is unavailable (Settings → Apps → show system apps → enable).

Residual risk: firmware updates may re-enable the stock launcher or change its package; `optimize analyze` must detect and report this. Recovery is never guaranteed.

## AI agents

- Agent output is untrusted input. MCP tools validate arguments against schemas and the protected-package policy, exactly like CLI input.
- No MCP tool accepts commands, shell strings, or arbitrary package lists for R3.
- MCP can read, analyze, and propose plans; approval is a separate human CLI step.

## ADB hygiene (user guidance)

- Legacy Wi-Fi ADB (TCP 5555) relies only on the one-time RSA key approval and is unencrypted; enable it only on trusted networks and disable it after use.
- Prefer `adb pair` (API 30+) where available.
- Revoke USB debugging authorizations when finished.

## Supply chain

- `cargo deny check` in CI: RustSec advisories, license allowlist (permissive + MPL + Slint's royalty-free license, D9), banned/duplicate crates. Ignored advisories need a reason in `deny.toml`; so far only unmaintained crates used by the desktop dev mode, never by the APK.
- `Cargo.lock` committed; dependencies pinned; Slint pinned with `=`.
- Release APKs signed with a project key kept offline; checksums published with each release. Reproducible builds investigated in Phase 6.
- No running of unvetted third-party tools (see RESEARCH.md §4).

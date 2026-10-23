# Security and Device Safety

## Reporting vulnerabilities

Until a dedicated address exists, report privately via GitHub Security Advisories on this repository. Do not open public issues for vulnerabilities.

## Absolute restrictions

RustTV never:

- uses root, flashes firmware, unlocks bootloaders, or modifies system partitions;
- disables, uninstalls, or replaces the package that provides the stock launcher;
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
| R4 Forbidden | Not implementable | uninstall, anything touching the stock launcher package, system partition writes | Nobody |

Milestone 1 contains only R0 commands; a unit test enforces this.

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
| RustTV launcher crashes on boot | Never disable the stock launcher, so Android offers it; document "Settings → Apps → RustTV → Clear defaults" and `adb uninstall ai.rusttv.launcher` | Some firmware may loop; user can still uninstall via ADB |
| ADB connection lost mid-apply | Journal shows the last completed step; `restore` resumes when reconnected | If Wi-Fi ADB is disabled by a reset, USB or manual settings needed |
| Device reboots mid-apply | Same as above; R2/R3 states persist predictably | — |
| Firmware update re-enables/changes packages | `optimize analyze` detects fingerprint change and marks plan stale | Behavior of new firmware is unknown until tested |
| Network ADB unavailable | All TV-side functionality (launcher) works without it | — |
| Restore fails | Print exact manual commands and Settings paths per step | **Recovery is never guaranteed**; we say so in docs and CLI output |

## AI agents

- Agent output is untrusted input. MCP tools validate arguments against schemas and the protected-package policy, exactly like CLI input.
- No MCP tool accepts commands, shell strings, or arbitrary package lists for R3.
- MCP can read, analyze, and propose plans; approval is a separate human CLI step.

## ADB hygiene (user guidance)

- Legacy Wi-Fi ADB (TCP 5555) relies only on the one-time RSA key approval and is unencrypted; enable it only on trusted networks and disable it after use.
- Prefer `adb pair` (API 30+) where available.
- Revoke USB debugging authorizations when finished.

## Supply chain

- `cargo deny check` in CI: RustSec advisories, license allowlist (permissive + MPL), banned/duplicate crates.
- `Cargo.lock` committed; dependencies pinned; Slint pinned with `=`.
- Release APKs signed with a project key kept offline; checksums published with each release. Reproducible builds investigated in Phase 6.
- No running of unvetted third-party tools (see RESEARCH.md §4).

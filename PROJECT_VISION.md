# RustTV — Project Vision

**Mission: make every Android TV faster, safely, for free.**

RustTV is an open-source, rootless toolkit for stock Android TV devices: a fast replacement home screen, a safe optimizer, and CLI/MCP tooling for humans and AI agents, all built around one shared Rust core.

## Who it is for

- People with slow or cluttered Android/Google TVs who want a faster, cleaner home screen without rooting.
- Tinkerers and developers who manage TVs over Wi-Fi ADB and want scriptable, safe tooling.
- AI-agent users who want structured, permission-checked access to their TV.

## What we are building first (MVP)

| Component | What it does | Runs on |
|---|---|---|
| `rusttv` CLI | Read-only device inspection, performance snapshots, benchmark harness; later, safe optimization plans | User's computer (macOS, Linux) |
| RustTV Launcher | Minimal Rust + Slint home screen: app grid, D-pad navigation, app launching | The TV |
| RustTV MCP | The CLI's capabilities as structured agent tools | User's computer |
| Compatibility data | Versioned TOML device profiles, contributed via PRs | Repo |

Everything else in the long-term vision (RustCast, RustBrowse, RustPlay, AI Concierge, Cloud, B2B fleet management, etc.) is **out of scope** until the MVP is shipped, measured, and used. Each future component must justify itself independently with real demand.

## Principles

1. **Measure before claiming.** Every speed claim comes from published, reproducible benchmarks.
2. **Device safety above everything.** Rootless, reversible where possible, honest when not.
3. **Local-first, no telemetry.** Works offline; no network calls of its own, except the launcher's update check (owner decision 2026-10-24, below); no data leaves the machine unless the user exports it.
4. **Rust-first, not Rust-only.** Rust for logic, UI, CLI, MCP; a thin Kotlin shim where Android APIs require Java.
5. **Agent-native.** Every capability is a CLI command with stable `--json` output; MCP exposes the same core.
6. **LLM-independent.** No model is needed for any normal operation.
7. **Vendor-neutral.** Detect capabilities; never assume behavior from a brand name.
8. **Small and modular.** No background daemons, no services, no dependency without a reason.

## Product decisions (owner, 2026-10-23)

| Decision | Choice |
|---|---|
| License | Apache-2.0 OR MIT (dual), all components |
| Launcher UI | Rust + Slint |
| Working name | RustTV — check Rust Foundation trademark policy and TV trademarks before first public release; don't use the official Rust logo |
| App ID namespace | `ai.rusttv.*` (owner holds `rusttv.ai`) — permanent once published |
| Telemetry | None. Users export diagnostics manually, review/redact, then submit |
| CLI output | Human-readable by default; `--json` for stable versioned schema |
| Prior art | Inspired by CleanYourTV (MIT): clean-room Rust implementation; data files may be reused with attribution |
| Primary test device | Owner's Philips TV running Google TV, plus the Android TV emulator |

## Product decisions (owner, 2026-10-24)

| Decision | Choice |
|---|---|
| Launcher updates | The launcher checks GitHub for a newer release once a day, **on by default**, and can install it on the TV itself; Android asks the user to confirm each install. There is no off switch until the launcher has a settings page. What it fetches and why the TV trusts it: SECURITY.md § Launcher updates |

Technical decisions are logged in ARCHITECTURE.md.

## Success looks like

- A user can install the launcher next to the stock one and use it daily. If they opt in to making it their Home screen, one command restores the stock launcher, and removing RustTV restores it automatically.
- Benchmarks published in the repo show where RustTV is faster, where it is not, and how it was measured.
- A contributor can add a device profile with a single TOML file and a CI check.

## Non-goals

- Root, custom ROMs, firmware modification.
- Bypassing DRM, access controls, or vendor licensing.
  *Owner decision 2026-10-23, one exception:* the experimental, opt-in
  `rusttv tools airplay` (TOOLS.md) turns on a receiver feature the TV maker
  installed but left disabled on some models, on the owner's own TV, with a
  restore path. It doesn't touch DRM or system partitions, is never on by
  default, and the TV maker's setting comes back on every reboot. Not a
  precedent for other features: each needs its own dated decision here.
- Universal cross-streaming-service content search.
- A cloud service, account system, or billing in the MVP.

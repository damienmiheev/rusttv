# RustTV

**Make every Android TV faster, safely, for free.**

RustTV is an open-source, rootless toolkit for stock Android TVs: a fast Rust + Slint launcher, a safe ADB-based optimizer, and CLI/MCP tools for humans and AI agents. Local-first, no telemetry, and every performance claim backed by published benchmarks.

> **Status: Milestone 1 in progress.** The CLI can list, connect to and inspect devices and list installed apps (all read-only). See [ROADMAP.md](ROADMAP.md) for what comes next.

## What it will be

| Component | What it does | Runs on |
|---|---|---|
| `rusttv` CLI | Inspect devices, take performance snapshots, run benchmarks, and (later) apply safe, reversible optimizations over ADB | Your computer (macOS, Linux) |
| RustTV Launcher | Minimal, fast home screen built with Rust + Slint: app grid, D-pad navigation, app launching | The TV (Android 8+) |
| RustTV MCP | The CLI's capabilities as structured tools for AI agents | Your computer |
| Compatibility data | Community-maintained, versioned device profiles | This repo |

## Principles

- **Rootless.** No root, no bootloader unlocking, no firmware flashing, no system-partition changes.
- **Safety first.** The stock launcher is never disabled. Every change is planned, shown as a dry run, approved by a human, recorded, and reversible where possible. See [SECURITY.md](SECURITY.md).
- **Measured, not claimed.** No "X% faster" without reproducible data. See [BENCHMARK_PLAN.md](BENCHMARK_PLAN.md).
- **Local-first, no telemetry.** Nothing leaves your machine unless you export it yourself.
- **Agent-native, LLM-independent.** Everything is scriptable with stable `--json` output; no AI model is needed for normal use.

## Documentation

| Doc | Contents |
|---|---|
| [PROJECT_VISION.md](PROJECT_VISION.md) | Mission, scope, product decisions, non-goals |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Repo layout, crates, Android bridge, decision log |
| [SECURITY.md](SECURITY.md) | Safety rules, risk classes, recovery, vulnerability reporting |
| [BENCHMARK_PLAN.md](BENCHMARK_PLAN.md) | How performance is measured and reported |
| [RESEARCH.md](RESEARCH.md) | Prior art and technology evaluation |
| [RISKS.md](RISKS.md) | Risks and open technical questions |
| [ROADMAP.md](ROADMAP.md) | Phases, milestones, success criteria |
| [CLAUDE.md](CLAUDE.md) | Rules for AI coding agents working on this repo |

## Getting started

Requirements: [Rust via rustup](https://rustup.rs) (the pinned toolchain in `rust-toolchain.toml` installs automatically) and, for device commands later, Android platform-tools (`adb`).

```bash
cargo run -p rusttv-cli -- device list                 # devices known to adb
cargo run -p rusttv-cli -- device connect 192.168.1.20 # connect over Wi-Fi (port 5555 by default)
cargo run -p rusttv-cli -- device inspect               # model, Android version, CPU, memory, storage (read-only)
cargo run -p rusttv-cli -- apps list                    # installed packages: system/third-party, enabled/disabled
cargo run -p rusttv-cli -- device list --json          # machine-readable output
```

To connect over the network, first enable Developer options and network/wireless debugging on the TV. The first connection shows a prompt on the TV asking you to allow this computer. If `adb` is not on your `PATH`, set `RUSTTV_ADB=/path/to/adb`.

Before sending a change, run the same checks as CI:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check   # cargo install --locked cargo-deny
```

## Contributing

The project is at an early stage. The most useful contributions right now are feedback on the design docs (open an issue) and, once the CLI exists, device reports from your TV. Please report security issues privately via GitHub Security Advisories, not public issues.

## Acknowledgements

- [CleanYourTV / tv-vibehack](https://github.com/Gathin23/tv-vibehack) (MIT) — inspiration for the safety and recovery design.
- [Slint](https://slint.dev) — UI toolkit for the launcher.

## License

Dual-licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

"RustTV" is a working name and may change before the first public release. RustTV is not affiliated with the Rust Foundation or the Rust project.

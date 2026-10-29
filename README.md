# RustTV

**Make every Android TV faster, safely, for free.**

RustTV is an open-source, rootless toolkit for stock Android TVs: a fast Rust + Slint launcher, a safe ADB-based optimizer, and CLI/MCP tools for humans and AI agents. Local-first, no telemetry, and every performance claim backed by published benchmarks.

> **Status: 0.3.1.** Tested on one real TV, the Philips 65OLED818 (Google TV, Android 14); [compat/devices/](compat/devices/) lists what was tested there. The launcher is also developed on the Google TV emulator. Other TVs are untested: RustTV refuses changes a device's profile doesn't record as tested unless you pass `--allow-untested`. See [CHANGELOG.md](CHANGELOG.md) for what each version does and [ROADMAP.md](ROADMAP.md) for what comes next.

**To install and use RustTV, follow [GETTING_STARTED.md](GETTING_STARTED.md).**

## What it is

| Component | What it does | Runs on |
|---|---|---|
| `rusttv` CLI | Inspect devices, run benchmarks, install the launcher and make it your Home screen, disable telemetry and ads packages, reduce animations, sync and restore the TV's history, update RustTV: every change planned, approved and reversible | Your computer (macOS, Linux) |
| RustTV Launcher | Minimal, fast home screen built with Rust + Slint: app row, D-pad navigation, move and hide apps, Settings with a history of changes approved on the TV, English and 20 other languages | The TV (Android 8+) |
| RustTV MCP | The CLI's capabilities as structured tools for AI agents | Your computer |
| Compatibility data | Community-maintained, versioned device profiles | This repo |

## Principles

- **Rootless.** No root, no bootloader unlocking, no firmware flashing, no system-partition changes.
- **Safety first.** The stock launcher is never uninstalled, and only disabled if you opt in to making RustTV your Home screen (with one-command restore). Every change is planned, shown as a dry run, approved by a human, recorded, and reversible where possible. See [SECURITY.md](SECURITY.md).
- **Measured, not claimed.** No "X% faster" without reproducible data. See [BENCHMARK_PLAN.md](BENCHMARK_PLAN.md).
- **Local-first, no telemetry.** Nothing leaves your machine unless you export it yourself.
- **Agent-native, LLM-independent.** Everything is scriptable with stable `--json` output; no AI model is needed for normal use.

## Documentation

| Doc | Contents |
|---|---|
| [GETTING_STARTED.md](GETTING_STARTED.md) | Install, connect, use, update and remove RustTV (release files or a build from source); notes for AI agents |
| [PROJECT_VISION.md](PROJECT_VISION.md) | Mission, scope, product decisions, non-goals |
| [CLI.md](CLI.md) | Commands, `--json` output contract, exit and error codes |
| [MCP.md](MCP.md) | MCP server for AI agents: tools, results, setup |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Repo layout, crates, Android bridge, decision log |
| [SECURITY.md](SECURITY.md) | Safety rules, risk classes, recovery, vulnerability reporting |
| [BENCHMARK_PLAN.md](BENCHMARK_PLAN.md) | How performance is measured and reported |
| [RESEARCH.md](RESEARCH.md) | Prior art and technology evaluation |
| [RISKS.md](RISKS.md) | Risks and open technical questions |
| [ROADMAP.md](ROADMAP.md) | Phases, milestones, success criteria |
| [RELEASE.md](RELEASE.md) | Versioning, APK signing, release checklist |
| [CHANGELOG.md](CHANGELOG.md) | Changes per release |
| [CLAUDE.md](CLAUDE.md) | Rules for AI coding agents working on this repo |

## Development

Installing and using RustTV: [GETTING_STARTED.md](GETTING_STARTED.md). For working on the code:

Requirements: [Rust via rustup](https://rustup.rs) (the pinned toolchain in `rust-toolchain.toml` installs automatically) and Android platform-tools (`adb`) for the device commands.

```bash
cargo run -p rusttv-cli -- device list                 # devices known to adb
cargo run -p rusttv-cli -- device connect 192.168.1.20 # connect over Wi-Fi (port 5555 by default)
cargo run -p rusttv-cli -- device inspect               # model, Android version, CPU, memory, storage (read-only)
cargo run -p rusttv-cli -- apps list                    # installed packages: system/third-party, enabled/disabled
cargo run -p rusttv-cli -- performance snapshot         # CPU, memory, launcher memory, biggest processes
cargo run -p rusttv-cli -- compat profile               # compatibility profile for compat/devices/
cargo run -p rusttv-cli -- launcher status              # which launcher the Home button opens, and why
cargo run -p rusttv-cli -- performance benchmark --app <package/.Activity> --app <other> --json > results.json
cargo run -p rusttv-cli -- device list --json          # machine-readable output
```

Launcher, without a TV (desktop dev mode with made-up apps; arrows move, Return is OK, hold it for a tile menu to move or hide it, Escape is Back):

```bash
cargo run -p rusttv-launcher --features test-util --bin rusttv-launcher-desktop
```

Launcher APKs (arm64-v8a, armeabi-v7a, x86_64): `android/build.sh`. Needs the Android SDK (platform 34), NDK 30, JDK 17, `cargo install cargo-ndk` and `cargo install cargo-about --version 0.9.2 --locked --features cli` (for the third-party notices packed into the APK).

To connect over the network, first enable Developer options and network/wireless debugging on the TV. The first connection shows a prompt on the TV asking you to allow this computer. If `adb` is not on your `PATH`, set `RUSTTV_ADB=/path/to/adb`.

Before sending a change, run the same checks as CI:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check   # cargo install --locked cargo-deny
```

## Contributing

The project is at an early stage. The most useful contributions right now are feedback on the design docs (open an issue) and a compatibility profile for your TV (see [compat/README.md](compat/README.md)). Please report security issues privately via GitHub Security Advisories, not public issues.

## Acknowledgements

- [CleanYourTV / tv-vibehack](https://github.com/Gathin23/tv-vibehack) (MIT) — inspiration for the safety and recovery design and the launcher's hero backdrop.
- [material-color-utilities](https://github.com/material-foundation/material-color-utilities) via the Rust port [material-colors](https://crates.io/crates/material-colors) — backdrop colour extraction.
- [Slint](https://slint.dev) — UI toolkit for the launcher.

## License

Dual-licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

RustTV is an independent project. It is not affiliated with or endorsed by the Rust Foundation, the Rust Project or Google. Rust is a trademark of the Rust Foundation; Android TV and Google TV are trademarks of Google LLC.

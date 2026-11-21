# RustTV

**Make every Android TV faster, safely, for free.**

RustTV is an open-source, rootless toolkit for stock Android TVs: a fast Rust + Slint launcher, a safe ADB-based optimizer, and CLI/MCP tools for humans and AI agents. The same launcher is also a TV home screen for Linux boxes (see [RustTV on Linux](#rusttv-on-linux)). Local-first, no telemetry, and every performance claim backed by published benchmarks.

> **Status: 0.3.1.** Tested on one real TV, the Philips 65OLED818 (Google TV, Android 14); [compat/devices/](compat/devices/) lists what was tested there. The launcher is also developed on the Google TV emulator. Other TVs are untested: RustTV refuses changes a device's profile doesn't record as tested unless you pass `--allow-untested`. See [CHANGELOG.md](CHANGELOG.md) for what each version does and [ROADMAP.md](ROADMAP.md) for what comes next.

**To install and use RustTV, follow [GETTING_STARTED.md](GETTING_STARTED.md).**

## What it is

| Component | What it does | Runs on |
|---|---|---|
| `rusttv` CLI | Inspect devices, run benchmarks, install the launcher and make it your Home screen, disable telemetry and ads packages, reduce animations, sync and restore the TV's history, update RustTV: every change planned, approved and reversible | Your computer (macOS, Linux) |
| RustTV Launcher | Minimal, fast home screen built with Rust + Slint: app row, D-pad navigation, move and hide apps, Settings with a history of changes approved on the TV, English and 20 other languages | The TV (Android 8+), or a Linux box in a Wayland compositor |
| RustTV MCP | The CLI's capabilities as structured tools for AI agents | Your computer |
| Compatibility data | Community-maintained, versioned device profiles | This repo |

## RustTV on Linux

With the `linux` feature, the launcher is the home screen of a Linux TV box: run `rusttv-launcher-linux` full screen as the first program of a Wayland session (labwc, cage or another wlroots compositor). It is experimental and developed on one ARM projector running Debian.

- **Apps:** every `Type=Application` desktop entry (system packages and Flatpaks), with icons from the icon themes. Apps start as child processes and cover the home screen; when one ends, Home is back. At most `max_apps` run at once and background apps close when memory runs low (`~/.config/rusttv/linux.toml`; freezing background apps, `freeze_background`, is off unless turned on).
- **Every input:** remote (IR or Bluetooth), mouse and air mouse (hover focuses, click is OK, the wheel scrolls), trackpad, keyboard, and game controllers over USB or Bluetooth (D-pad and left stick are the arrows, A is OK, B is Back, Guide is Home; while an app is in front only Home is taken, so games keep the pad).
- **On-screen keyboard** for text fields in any app (`input-method-unstable-v2`); typing on a real keyboard, or a phone's, passes straight through.
- **Top-right menu:** Modify, Settings and quick panels for what the device has: Audio, Network, KDE Connect, Power.
- **Device settings** through the system's own tools, each only where it is installed: `wpctl` (PipeWire) for audio, `nmcli` (NetworkManager) for Wi-Fi and Ethernet, `bluetoothctl` for Bluetooth, `systemctl` to restart or power off. About shows facts from `/etc/os-release`, `/proc` and `/sys`.
- **Over apps:** holding Home opens a panel at the right edge (app size, volume, close the app) and the volume keys show a volume bar, both as layer-shell overlays.
- **Phone as remote:** `rusttv-portal`, a remote desktop portal backend for compositors without one, gives remote input only to KDE Connect (the phone as touchpad and keyboard) and refuses every other app.
- **Updates** are the package manager's job: the launcher's updater does nothing on Linux.

Build and run (on the Linux machine):

```bash
cargo build --release -p rusttv-launcher --features linux --bin rusttv-launcher-linux --bin rusttv-portal
```

Bind the remote's keys in the compositor: Home to `rusttv-launcher-linux --home` (plus raising the window titled `RustTV`), held Home (`XF86Launch1`) to `--app-menu`, Power to `--power`, and the volume keys to `--volume up|down|mute`. Tile order, hidden apps and history live in `$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`); `RUSTTV_LANG=<tag>` overrides the language. Details: [`src/bin/linux.rs`](crates/rusttv-launcher/src/bin/linux.rs) and [ARCHITECTURE.md](ARCHITECTURE.md).

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

## Contributors

[![Contributors](https://contrib.rocks/image?repo=damienmiheev/rusttv)](https://github.com/damienmiheev/rusttv/graphs/contributors)

## Acknowledgements

- [CleanYourTV / tv-vibehack](https://github.com/Gathin23/tv-vibehack) (MIT) — inspiration for the safety and recovery design and the launcher's hero backdrop.
- [material-color-utilities](https://github.com/material-foundation/material-color-utilities) via the Rust port [material-colors](https://crates.io/crates/material-colors) — backdrop colour extraction.
- [Slint](https://slint.dev) — UI toolkit for the launcher.

## License

Dual-licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.

RustTV is an independent project. It is not affiliated with or endorsed by the Rust Foundation, the Rust Project or Google. Rust is a trademark of the Rust Foundation; Android TV and Google TV are trademarks of Google LLC.

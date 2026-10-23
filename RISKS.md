# Risks

Likelihood / Impact: H / M / L. "Validation" is how we will find out; "Remaining uncertainty" is what we can't eliminate.

| # | Risk | L | I | Evidence | Mitigation | Validation | Remaining uncertainty |
|---|---|---|---|---|---|---|---|
| 1 | Slint renders incorrectly/slowly on TV GPUs | M | H | Spike renders correctly with FemtoVG on the Google TV emulator (host GPU); real TV GPUs untested | Test on emulator + Philips early; keep UI simple | Launcher spike on device | Fragmented low-end GPUs |
| 2 | D-pad keys not delivered/mapped to Slint | **L** | H | Spike: arrows, OK and BACK all arrive (RESEARCH §8) | Root `FocusScope` + own key mapping; fallback via Kotlin `dispatchKeyEvent` forwarding | Spike task #1 | Vendor remotes with odd keycodes |
| 3 | Google TV blocks third-party HOME | **H (confirmed)** | H | Spike (RESEARCH §8): stock launcher HOME priority 2 beats any third-party preference; role/preferred changes are ignored on the Google TV emulator | Owner decision (D12): opt-in Home takeover by disabling the stock launcher, with verify/auto-rollback and one-command restore (SECURITY.md) | Confirm on Philips Google TV; check classic Android TV | Future Google updates; behaviour of other vendors |
| 4 | Limited ADB permissions on some vendors | M | M | CleanYourTV needed its own ADB client in some setups; vendor restrictions known | Probe capabilities; degrade gracefully; record in compat profiles | Compat profile per device | Unknown vendors |
| 5 | Package visibility restrictions (API 30+) | L | M | Documented Android behavior | `<queries>` for launcher intents | Emulator API 34 test | — |
| 6 | JNI integration complexity | M | M | Slint's documented path is NativeActivity without Java | Thin Kotlin shim, coarse calls, single JNI module, `FakePlatform` tests | Spike | GameActivity + Slint compatibility |
| 7 | GPU/driver fragmentation | M | M | General Android TV knowledge | Software-renderer fallback evaluated if Skia GL fails | Community device reports | High |
| 8 | Rust/Slint launcher not faster than lightweight Kotlin launchers | M | H | Java launcher baseline is ~0.5 MB; Slint adds native libs + own GPU context | Measure; publish honest results; optimize where data points | Phase 2 benchmarks | Until measured |
| 9 | Package disabling breaks a device | M | H | Common debloat failure mode | Protected package policy, R3 only for profiled devices, journaled restore | Phase 4 tests on emulator first | Model-specific dependencies |
| 10 | Rootless rollback fails | L | H | If ADB is lost, nothing rootless can restore | Journal, manual instructions, never touch HOME/setup/network packages | Fault-injection tests (kill ADB mid-apply) | Cannot be eliminated |
| 11 | Firmware updates break compatibility | H | M | Firmware updates re-enable/rename packages | Fingerprint-bound plans; `analyze` detects drift | Re-test after Philips updates | Ongoing |
| 12 | Background execution limits affect launcher | L | L | Launcher is foreground; no services planned | No background work | — | — |
| 13 | Browser DRM limitations (future) | H | M | Servo has no Widevine | Native apps for protected playback | Phase 7 research | — |
| 14 | Casting protocol compatibility (future) | H | M | AirPlay/Miracast are closed or fragmented | DLNA first | Phase 7 | — |
| 15 | An agent approves a device change | L | H | Approval is a word typed at an interactive terminal (SECURITY.md § Change lifecycle); an agent driving a pseudo-terminal could type it | No `--yes` flag; CLI refuses without a TTY on stdin; MCP has no approve tool; never give agents/MCP a pseudo-terminal running `rusttv` (documented) | CLI test: no TTY → `NOT_APPROVED` before any device access | A process with a PTY is indistinguishable from a person to the CLI |
| 15 | Third-party licensing | M | M | GPL prior art (FLauncher, UAD-ng); Slint attribution requirement | Clean-room; `cargo deny` license check; attribution on About screen | CI | Slint terms may change |
| 16 | Supply-chain attack | M | H | Suspicious popularity metrics seen (RESEARCH §4); `skia-bindings` downloads a prebuilt Skia binary at build time | Minimal deps, `cargo deny`, lockfile, no unvetted tools | CI, review | Transitive deps |
| 17 | LLM-generated defects | H | M | Owner new to Rust; AI-assisted | CLAUDE.md rules, tests first, clippy `-D warnings`, independent review, small PRs | CI + review | Subtle logic bugs |
| 18 | Maintenance burden | M | M | Ambitious ecosystem vision | MVP scope only; defer ecosystem apps | Roadmap gates | — |
| 19 | Few contributors | M | M | Typical for new OSS | Easy compat/benchmark contributions; good-first-issues | Track contributions | — |
| 20 | Scope creep | H | H | Vision lists 12+ future apps, AI, cloud, B2B | Phase gates with exit criteria (ROADMAP.md) | Review at each phase | — |
| 22 | Launcher APK much larger than Java launchers | H | L | Spike: 11.5 MB APK after tuning; ~10 MB is Skia, required by Slint's Android backend. Stock Google TV launcher is 44.0 MB, a minimal Java launcher ~0.5 MB | Size-tuned release profile; track size in CI; revisit software renderer only if TVs show a problem | Size report in CI; install time on the Philips TV | Low-storage TVs |
| 21 | Many TVs use 32-bit userspace | H | M | Common on low-cost SoCs | Build `armeabi-v7a` too; CLI reports ABI list | Compat data | Rust/Skia on armv7 perf |

## Top 10 technical uncertainties

1. Do D-pad and DPAD_CENTER events reach Slint reliably on the Philips remote?
2. Can RustTV be set as (and stay) the HOME app on Philips Google TV, and does the remote's Home button go to it?
3. Slint launcher idle memory and APK size vs a ~0.5 MB Java launcher.
4. Does Slint's Android backend work with GameActivity / a custom Activity so the Kotlin shim can live in the same APK?
5. Skia/GL rendering stability and frame times on low-end TV GPUs at 1080p/4K UI.
6. Which rootless ADB operations (`pm disable-user`, `set-home-activity`, `settings put`) the Philips firmware allows.
7. Repeatability of measurements on a TV (thermal, background updates, ambient-mode jobs) — how many runs are needed.
8. Whether `am start -W` / logcat timings are reliable for HOME activities on Google TV builds.
9. `rmcp` API stability across versions: Phase 5 started on 3.7 (2026-10-23); the SDK had three major versions in its first ~19 months.
10. How Philips (TP Vision) firmware updates treat a non-stock default home.

# Security and Device Safety

## Reporting vulnerabilities

Until a dedicated address exists, report privately via GitHub Security Advisories on this repository. Do not open public issues for vulnerabilities.

## Absolute restrictions

RustTV never:

- uses root, flashes firmware, unlocks bootloaders, or modifies system partitions;
- uninstalls the stock launcher, or disables it outside the opt-in [Home takeover](#home-takeover-opt-in) flow;
- removes packages, except its own launcher (`ai.rusttv.launcher`) through `rusttv launcher uninstall`; the uninstall command takes no package name, so it can remove nothing else;
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
| R3 Package state | Enable/disable an app for user 0; install, upgrade or uninstall the RustTV launcher (its own package only) | `pm disable-user --user 0`, `adb install --pkg ai.rusttv.launcher <apk>` | Only via an approved plan; disabling: Phase 4+, known-device profile only |
| R4 Forbidden | Not implementable | uninstalling anything but RustTV's launcher, stock-launcher changes outside Home takeover, system partition writes | Nobody |

So far: R0 commands, two R1 commands for benchmarking (`am force-stop`, `am start`), two R3 commands used only by Home takeover (`pm disable-user --user 0`, `pm enable --user 0`), and three R3 commands used only by `launcher install`/`uninstall`, fixed to package `ai.rusttv.launcher` (`adb install --pkg ai.rusttv.launcher <apk>`, `adb install -r --pkg ...`, `pm uninstall ai.rusttv.launcher`), all run only through the safety engine (`rusttv-core` `plan` module). A unit test fails if anything else above R0 appears. Every R2/R3 command must have an inverse, except upgrading and uninstalling RustTV's own launcher, which can't be undone without the previous APK: plans accept those only as their last step, show "no undo", and a failure after them is reported as a failed rollback with the manual fix. `adb install` runs on the host, not through `adb shell`; its only free argument is a validated absolute APK path whose file name is limited to `[A-Za-z0-9._-]+.apk`, because older adb versions pass the file name to `pm` on the device.

## Protected package policy

A package is protected (no R3 action) if **any** of these is true:

1. It resolves `HOME`, or is the current/previous default home.
2. It provides an input method, accessibility service, TV input (HDMI/tuner), setup wizard, system UI, settings, package installer, network stack, Bluetooth/remote pairing, or Google Play services.
3. It is on the static deny-list in `compat/protected.toml`.
4. The device has no profile marking the package as tested-safe for that model **and** firmware family.

Rules 1–2 are computed live on the device each time, not cached. Unknown devices get R0–R1 only.

One exception, inside Home takeover only: a system package with a Home activity of higher priority than RustTV may be disabled (never uninstalled) when the device's tested profile lists it in `disable_home_packages`. On the Philips 65OLED818 that is the setup wizard, `com.google.android.tungsten.setupwraith` (owner decision 2026-10-23, D14): while it is disabled, its deferred-setup and reset screens are off too. Disabling only its Home activity would be narrower, but Android 14 doesn't let the adb shell change a single activity of a system app. See [Home takeover](#home-takeover-opt-in), rule 4.

## Change lifecycle (R2–R3)

1. **Analyze** (read-only): collect device info and current state.
2. **Plan**: device fingerprint (`ro.build.fingerprint`), steps, risk class per step, and the original value for each step. The plan is stored inside its journal: one TOML file per change, no separate plan file.
3. **Dry-run**: render the plan as human-readable text with exact commands.
4. **Approve**: a human confirms in the CLI (typed confirmation for R3). MCP tools can create plans but not approve them. The CLI accepts the confirmation only when stdin is an interactive terminal, and has no `--yes` flag. That proves a terminal, not a person: an agent driving a pseudo-terminal could type the word. Agents and MCP servers must therefore never be given a pseudo-terminal running `rusttv` (RISKS.md #15).
5. **Apply**: precondition check (same fingerprint, device reachable, protected-package check re-run). Write a journal entry *before* each step; run one step; verify its effect; stop on the first failure.
6. **Verify**: re-read state; confirm HOME still resolves to a launchable activity.
7. **Restore**: one journal-based restore for every kind of change. It undoes, last first, every step the journal says may have run (started or done), verifies each undo, and keeps going past a failure, printing the manual command for it. A restore is never rolled back. If the device's fingerprint changed since the journal was written, restore warns and still runs: undoing is the safe direction, and a firmware update is a likely reason to restore. The command is `rusttv launcher restore` for now (Home takeover is the only kind of change); when optimization profiles land, the same engine backs a general `rusttv restore [--journal <file>]`, and `rusttv launcher restore` stays as a shortcut for takeover journals.

Plans and journals live in a local state directory (`~/.local/state/rusttv/` on Linux, `~/Library/Application Support/rusttv/` on macOS, `RUSTTV_STATE_DIR` to override) and are plain text for auditability. The journal (TOML, `journals/<unix ms>-<plan>.toml`) holds the plan's steps, the original state checked before any step, and each step's progress. It is written atomically before the first step, when each step starts and ends, and at the end; if it can't be written, the change is rolled back. A journal is untrusted input when restoring: Home takeover restore refuses any step other than re-enabling a package. A journal file that can't be read is never skipped silently: `launcher restore` warns about each one on stderr (suggesting `--journal <file>` or `--from-profile`), and uninstall and upgrade refuse while one is there (rule 6).

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
2. **Known devices only.** Offered only when the device's compat profile names its stock launcher package and records a successful takeover and restore on that firmware family. *As built:* profiles from `compat/devices/` are compiled into the binary (so dropping a file somewhere can't satisfy the gate); one must match manufacturer, model and codename, and the firmware family (fingerprint without build ID and incremental: same Android version and build type), and its `[launcher]` section must name the package Home opens and set `takeover_tested = true`. Otherwise `TAKEOVER_NOT_TESTED`. Emulator profiles are never marked tested. The first takeover on a new device therefore needs a development build with that profile edited locally; commit `takeover_tested = true` only after takeover **and** restore succeeded on it.
3. **Preconditions.** RustTV launcher installed, launchable, and verified drawing frames; ADB connected; the stock launcher package is the current HOME resolver. *As built:* before the change, RustTV must be a HOME candidate and the HOME resolver must be a system app that outranks RustTV's priority. "Launchable and drawing frames" is checked after the change, as part of rule 5 (resumed activity with a live process); frame drawing itself isn't measured.
4. **Disable, never uninstall.** `pm disable-user --user 0 <stock launcher>` only; original state recorded in the journal first. *As built:* Android opens the highest-priority Home activity without asking and ignores the preferred Home (the Home role) when one outranks the rest, so every other system activity above RustTV's priority would win Home once the stock launcher is gone. Each one's package must be listed in the profile's `disable_home_packages`, or the plan is refused with `TAKEOVER_NOT_TESTED`; the plan disables those packages first (`pm disable-user --user 0`), while the stock launcher still wins Home, then the stock launcher. Restore re-enables them last.
5. **Verify or roll back automatically.** After disabling, HOME must resolve to RustTV and RustTV must come to the foreground on a HOME key event within a timeout; otherwise re-enable the stock launcher immediately. *As built:* `input keyevent KEYCODE_HOME` (R1), then for up to 10 s the resumed activity (`dumpsys activity activities`) must be `ai.rusttv.launcher` and its process must be in `ps`.
6. **Restore is one command.** `rusttv launcher restore` re-enables the stock launcher; RustTV is never removed while the stock launcher is disabled, so the TV is never left with only Android's fallback Home. *As built:* `rusttv launcher uninstall` (and an upgrade by `launcher install`) refuses, with `PRECONDITION_FAILED` telling the person to run `rusttv launcher restore` first, when a takeover journal for this firmware isn't restored, when a `.toml` file in the journal directory can't be read (it may be an unrestored takeover; the error names each file, and the person checks it, restores if needed, then moves it out of the directory), or when no system launcher other than RustTV can handle Home (catches a takeover from another computer or by hand). It doesn't restore inside the same plan: the restore then runs through its own verified plan and marks the takeover journal restored, and the uninstall journal never contains a step whose undo would disable the stock launcher again after RustTV is gone. After uninstalling, Home must open the stock launcher.
7. **Documented manual recovery** in the user docs, for when ADB is unavailable (Settings → Apps → show system apps → enable). Re-enabling the stock launcher there brings the stock Home back even with the other disabled packages still off (it outranks them); enable those the same way, or with `rusttv launcher restore`.

Restore without a journal (journal lost, or the takeover ran from another computer): `rusttv launcher restore --from-profile` rebuilds the takeover's journal from the device's compiled-in profile, which, unlike a journal file, is trusted. It lists only the packages that profile's takeover disables (`disable_home_packages`, then the stock launcher) that are disabled on the device now, and restores them through the same never-rolled-back restore. A device without a matching profile gets `NO_JOURNAL` and the Settings path. The takeover plan also shows the undo commands and the Settings path, and a failed undo prints its command again.

Residual risk: firmware updates may re-enable the stock launcher or change its package; `optimize analyze` must detect and report this. Recovery is never guaranteed.

## AI agents

- Agent output is untrusted input. MCP tools validate arguments against schemas and the protected-package policy, exactly like CLI input.
- No MCP tool accepts commands, shell strings, or arbitrary package lists for R3.
- MCP can read, analyze, and propose plans; approval is a separate human CLI step. *As built:* the MCP plan tools (`launcher_install_plan`, `launcher_uninstall_plan`, `launcher_restore_plan`) return the dry run and the `rusttv` command a person runs at a terminal; they run only R0 commands and write no journal. There is no MCP tool to plan a Home takeover, or to approve or apply anything (MCP.md).
- Typed confirmation proves only an interactive terminal. Never give an agent or MCP server a pseudo-terminal that runs `rusttv` (see Change lifecycle, step 4).

## ADB hygiene (user guidance)

- Legacy Wi-Fi ADB (TCP 5555) relies only on the one-time RSA key approval and is unencrypted; enable it only on trusted networks and disable it after use.
- Prefer `adb pair` (API 30+) where available.
- Revoke USB debugging authorizations when finished.

## Supply chain

- `cargo deny check` in CI: RustSec advisories, license allowlist (permissive + MPL + Slint's royalty-free license, D9), banned/duplicate crates. Ignored advisories need a reason in `deny.toml`; so far only unmaintained crates used by the desktop dev mode, never by the APK.
- `Cargo.lock` committed; dependencies pinned; Slint pinned with `=`.
- Skia (linked by Slint's Android renderer) is a prebuilt binary per ABI that `android/build.sh` fetches and checks against a pinned SHA-256. arm64-v8a and x86_64 come from rust-skia's skia-binaries releases. rust-skia has no armeabi-v7a build, so ours is built from source by the manual workflow `.github/workflows/skia-armv7.yml` ([run 39956273511](https://github.com/damienmiheev/rusttv/actions/runs/39956273511)) and hosted as the release `skia-0.153.3-armv7` of this repository. It must be rebuilt, re-released and re-pinned whenever Slint (and so skia-bindings) is upgraded.
- Release APKs signed with a project key kept offline; checksums published with each release. Reproducible builds investigated in Phase 6.
- No running of unvetted third-party tools (see RESEARCH.md §4).

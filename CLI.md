# `rusttv` CLI reference

The contract for scripts and AI agents. All commands are **read-only** except
`performance benchmark`, which stops and starts the apps it measures (risk
class R1, transient), `launcher takeover` / `launcher restore`, which
disable or re-enable the stock launcher, and `launcher install` /
`launcher upgrade` / `launcher uninstall`, which install, upgrade or remove the RustTV launcher, and
`optimize apply privacy` / `optimize restore`, which disable or re-enable the
packages your privacy choices select (all R3), and `optimize apply
reduced-animations` / `optimize restore`, which change three global
animation settings and put them back (R2), and the experimental `tools
airplay enable` / `disable`, which turn on a vendor AirPlay receiver until
the TV restarts and turn it off again (R2, TOOLS.md), and `update launcher`,
which installs or upgrades the RustTV launcher from a verified release (R3),
only after a person approves the plan at a terminal, or, for launcher plans,
by a time-limited delegation a person granted there with `rusttv approve`
(see [Changing a device](#changing-a-device)).
`update cli` replaces programs on this computer, not the device, and also
asks for a typed confirmation ([Updates](#updates)). Run
`rusttv <command> --help` for every option. Agents can use the read-only
commands, and the `--dry-run` of `launcher install` / `uninstall` / `restore`,
`optimize apply` and `optimize restore`, as MCP tools too: see [MCP.md](MCP.md).

## Commands

| Command | What it does | `data` in `--json` |
|---|---|---|
| `device list` | Devices known to the local adb server | array of [Device](#device) |
| `device connect <host[:port]>` | `adb connect` (port 5555 by default); only changes state on this computer | [ConnectReport](#connectreport) |
| `device inspect` | Identity, firmware, CPU, memory, storage | [DeviceInfo](#deviceinfo) |
| `apps list` | Installed packages | array of [App](#app) |
| `performance snapshot [--sample-ms N]` | CPU (N ms sample, default 1000), memory, launcher memory, top processes | [PerformanceSnapshot](#performancesnapshot) |
| `performance benchmark --app <package/.Activity>...` | Repeated, interleaved cold start, resume, idle memory and CPU per app (BENCHMARK_PLAN.md); progress on stderr. **Stops and starts the apps**, and wakes the TV if its screensaver comes on. Resume needs two or more apps | [BenchmarkReport](#benchmarkreport) |
| `launcher status` | Which activity the Home button opens, every Home candidate with its priority, whether the RustTV launcher is installed / is Home. Read-only | [LauncherStatus](#launcherstatus) |
| `launcher takeover [--dry-run]` | Make RustTV the Home screen by **disabling** (never uninstalling) the stock launcher (SECURITY.md § Home takeover). Only on devices whose bundled compat profile records a tested takeover (`TAKEOVER_NOT_TESTED` otherwise, even with `--dry-run`). Shows the plan, asks for typed confirmation, verifies Home opens RustTV, wakes the TV (ends a screensaver, which would swallow the HOME press; up to 5 s), presses HOME and waits up to 10 s for RustTV to be in front and running, rolls back otherwise. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher install [--apk <file>] [--dry-run]` | Install the RustTV launcher (`ai.rusttv.launcher`) with `adb install --pkg ai.rusttv.launcher`, which makes Android refuse an APK of any other package. The APK is `android/app/build/outputs/apk/release/app-<abi>-release.apk` (from `android/build.sh`, relative to the current directory, so run from the repository root) for the device's primary ABI, the first of `ro.product.cpu.abilist` (`arm64-v8a`, `armeabi-v7a` or `x86_64`; `INVALID_APK` otherwise), or `--apk <file>` (any directory; file name of ASCII letters, digits, `.`, `_`, `-` ending in `.apk`). Checks RustTV is installed and can handle Home; a fresh install is uninstalled again otherwise. If RustTV is already installed it is an **upgrade** (`adb install -r`, keeps its data) that can't be undone: the plan says so, and it is refused while Home depends on RustTV (as for `uninstall`). Typed confirmation. Doesn't change which app Home opens. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher upgrade [--apk <file>] [--dry-run]` | Install or upgrade the RustTV launcher, also while a Home takeover is active. Without an active takeover it is `launcher install` (same plan, refusals, checks and journal) with the confirmation `upgrade`. With one (a restorable takeover journal for this firmware, as `launcher install` detects it), three stages, each the existing plan with its own journal, rollback and checks, in this order: `launcher restore` of that journal, then `launcher install` (an upgrade), then `launcher takeover`. Each later plan is made only when its stage starts, against the device as the stage before left it, so the dry run shows the restore plan in full and describes the other two (the APK, which is checked now; the packages the journal recorded as disabled). A person types `upgrade` once at a terminal; that approves each stage's plan for this device and firmware, for this run only (or a [delegation](#delegated-approval) approves each one). Refused before anything changes if the APK is missing (`INVALID_APK`) or a journal file can't be read (`JOURNAL_FAILED`). Stops at the first stage that fails; the error keeps that stage's code and says how the TV is left and what to run: restore failed: nothing installed, finish with `launcher restore`; install failed: Home opens the stock launcher, RustTV still installed (the old version unless the error says otherwise), run `launcher takeover` to go back; takeover rolled back: Home opens the stock launcher with the new RustTV, run `launcher takeover` again. R3 | array of [StageReport](#stagereport); with `--dry-run`: [LauncherUpgrade](#launcherupgrade) |
| `launcher uninstall [--dry-run]` | Uninstall the RustTV launcher and its data (`pm uninstall ai.rusttv.launcher`; it can remove no other package). Refused (`PRECONDITION_FAILED`) while a Home takeover is active: a takeover journal for this firmware that isn't restored, a journal file that can't be read (it may be one; check it, then move it out of the journal directory), or no system launcher other than RustTV handling Home. Run `launcher restore` first, so the TV is never left with only Android's fallback Home. Then checks RustTV is gone and Home opens the launcher it opened before (or the system launcher, if it opened RustTV). Can't be undone; reinstall with `launcher install`. Typed confirmation. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `launcher restore [--journal <file> \| --from-profile] [--dry-run]` | Undo `launcher takeover`: re-enable the stock launcher (and any other package the takeover disabled) from the takeover's journal (by default the newest one for the device's firmware; each journal file that can't be read is named in a warning on stderr, also with `--json`, and skipped). Without a journal (lost, or the takeover ran on another computer), `--from-profile` rebuilds one from the device's compiled-in profile: it re-enables the packages that profile's takeover disables, only where they are disabled now, and saves the result as a new `<ms>-home-takeover-from-profile.toml` journal. Typed confirmation. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `optimize analyze [--choices <file>]` | Every package of the catalog (`compat/packages/`, OPTIMIZER.md § Generic design) installed on the device, and every `extra` of your [choices](#privacy-choices), in plan order: category (default or opt-in), what it does, side effects, whether it was tested on this model (listed in the matching compat profile's `[[privacy.tested]]`), whether your choices select it, whether it is enabled / disabled, and whether RustTV refuses to touch it (protected). Works on any device; without a matching profile nothing is tested. Also every [optimizer profile](OPTIMIZER.md#3-profiles) with its status on the device (`suggested`, `available`, `applied`, `drifted`, `stale`, `not_tested`, `already_set`, `unknown`; reads the journals) and the current value of each of its settings (`settings get global <key>`). Read-only | [OptimizeAnalysis](#optimizeanalysis) |
| `optimize apply privacy [--only <categories>] [--choices <file>] [--allow-untested] [--dry-run]` | **Disable** (never uninstall) the packages your [choices](#privacy-choices) select that are installed and enabled: catalog packages in the chosen categories (by default `telemetry`, `ads`, `extras`) or in `also`, plus `extra` ones, minus `keep`. `--only` narrows the categories (comma-separated: `telemetry`, `ads`, `extras`, `casting`, `smart-home`, `voice`, `streaming-apps`); it never adds one, and with it `extra` packages are left out. Order: category, then the order of the profile's tested list, then catalog order, `extra` last. Packages not tested on this model are left out and named in the notes, unless `--allow-untested`: then they are planned and each note says "UNTESTED on this model". With nothing tested or allowed left, `PROFILE_NOT_TESTED` (even with `--dry-run`). Every chosen package is checked live first (SECURITY.md § Protected package policy): if one handles or can handle Home, provides an input method or an enabled accessibility service, or is on `compat/protected.toml`, nothing is planned (`PACKAGE_PROTECTED`). Packages already disabled are skipped and named in the notes; catalog packages not installed are only counted (ones named in `also` or `extra` are named). The protected-package check runs again right before each step, for that step's package; if it fails, the steps done so far are undone (`ROLLED_BACK`). After the last step Home must open the same activity as before, the Home candidates must be unchanged, and the TV input services bound when planning must still be bound (except those of packages the plan disables), otherwise everything is rolled back. Typed confirmation `privacy`. R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `optimize apply reduced-animations [--allow-untested] [--dry-run]` | Put `window_animation_scale`, `transition_animation_scale` and `animator_duration_scale` (global settings) at `0.5`: one `settings put global` step per key, each recording the value found first (`settings get global` printing `null` = unset, undone with `settings delete global`) and verified by reading it back. Refused with `OPTIMIZE_NOT_TESTED` (even with `--dry-run`) unless the matching compat profile has `[[optimizer]] profile = "reduced-animations", version = 1, tested = true`; `--allow-untested` plans it anyway for a first test on a device and the notes say "UNTESTED on this model". Refused (`PRECONDITION_FAILED`) while an optimizer journal of this model and firmware family is restorable (one at a time: restore it first), if a current value isn't a decimal like `1.0` (it couldn't be put back), or if every key already has its value, or while a `.toml` file in the journal directory can't be read (it may be an unrestored optimizer journal: check it, restore it if needed, then move it out of the directory; the error names each file); keys already at `0.5` are skipped and named. After the last step Home must still open the activity it opened before, otherwise everything is rolled back. Typed confirmation `optimize`. Journal `<ms>-optimize-reduced-animations-v1.toml`. R2 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `optimize restore [privacy \| reduced-animations] [--journal <file>] [--dry-run]` | Undo `optimize apply`. Without a name, the newest restorable journal for the device's firmware of the one kind that has one; if both a privacy and an optimizer journal are restorable, `PRECONDITION_FAILED` naming `optimize restore privacy` and `optimize restore reduced-animations`. With `--journal` and no name, the journal's plan says which kind it is. Unreadable journal files are named in a warning on stderr and skipped. **privacy**: re-enable what the privacy journal disabled; packages already enabled again (e.g. in Settings) are skipped and named in the notes. Only re-enables. **reduced-animations**: put back each setting's value from before (or delete it if it was unset); a key already back is skipped, and a key that is neither the value RustTV set nor the original (changed since, e.g. in Developer options) is left as is and named, both in the dry run and when it changes between plan and restore (named on stderr). Only `settings` steps. Each refuses the other kind's journal. If nothing is left to undo, `PRECONDITION_FAILED`, and (except with `--dry-run`) the journal is marked `restored`, so the next `optimize restore` finds the journal before it. Typed confirmation `restore`. R3 (privacy) / R2 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `tools airplay status [--check-bonjour]` | **Experimental** (TOOLS.md). Whether `enable` runs on this device (a bundled compat profile matches it, and MediaTek's `com.mediatek.partner.airplay_sys`, `com.mediatek.airplaydaemon` and `com.mediatek.AirplayAPK` are installed) and why not; whether the RustTV launcher (its trigger activity) is installed; with all three packages, also `setup_airplay_enable` and `show_airplay_setting_ui` (`settings get global`), whether the vendor `BootupService` runs (`dumpsys activity services com.mediatek.partner.airplay_sys`), the last 20 log lines of the AirPlay components (`logcat -d -t 2000`, filtered on this computer), listening TCP ports (`ss -ltnp`; null where `ss` is missing) and a restorable `enable` journal. `--check-bonjour` also browses `_airplay._tcp` from this computer for 5 s (`dns-sd -B` on macOS, `avahi-browse -t -p` on Linux; skipped with a note if neither is installed). Prints a caveat on stderr. Read-only | [AirplayStatus](#airplaystatus) |
| `tools airplay enable [--dry-run]` | **Experimental** (TOOLS.md). Two steps: `settings put global setup_airplay_enable 1`, recording the value found (must be `0` or `7`; the TV maker writes `7` at boot), then `am start -n ai.rusttv.launcher/.tools.HiddenFeatureActivity --es feature airplay --es action start`, which starts MediaTek's `BootupService`; it must run within 20 s, otherwise everything is undone. Refused with `FEATURE_NOT_SUPPORTED` (even with `--dry-run`) without a matching compat profile or the three packages; `PRECONDITION_FAILED` without the RustTV launcher, while an `enable` journal for this firmware is restorable (run `disable` first, also after a restart), when the switch is already `1` or holds another value, or when the service already runs. Lasts until the TV restarts. Typed confirmation `airplay`. Journal `<ms>-tools-airplay.toml`. R2 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `tools airplay disable [--journal <file>] [--dry-run]` | Undo `tools airplay enable` from its journal (by default the newest restorable one for the device's firmware; unreadable journal files are named in a warning on stderr and skipped): `am force-stop com.mediatek.partner.airplay_sys` (Android restarts the persistent service without AirPlay), then put the switch back to the value recorded. A switch changed since (neither `1` nor the original) is left as is. Only these two steps: any other journal is refused. Restarting the TV has the same effect. Typed confirmation `restore`. R2 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `update check` | The latest [release](#updates) (version, page) and whether this `rusttv` is older; with a device, the installed RustTV launcher's `versionName` (`dumpsys package ai.rusttv.launcher`) and primary ABI, and whether the release has a newer APK for it. Without `--serial` the only connected device is checked, and no device (or several, or adb missing) only skips the launcher part, with the reason in `launcher_note`; with `--serial` the device must be there (errors as for other device commands). Doesn't download or verify release files. Read-only | [UpdateCheck](#updatecheck) |
| `update launcher [--dry-run]` | Download the latest release's `rusttv-launcher-<version>-<abi>.apk` for the device's primary ABI into a private directory under `<state dir>/updates/`, [verify](#updates) it, then the `launcher install` plan with that APK, unchanged (fresh install or upgrade, its refusals, typed confirmation `install`, verify, journal `<ms>-launcher-install.toml`). Refused (`PRECONDITION_FAILED`) when the installed launcher is that version or newer, or reports a version that isn't `<major>.<minor>.<patch>`; `UPDATE_CHECK_FAILED` when the release has no APK for the ABI. The download is removed when the command ends, also after `--dry-run` (whose plan names it). R3 | [ChangeReport](#changereport); with `--dry-run`: [DryRun](#dryrun) |
| `update cli [--dry-run]` | Replace the running `rusttv` (and `rusttv-mcp` if it is in the same directory) with the latest release's: download `rusttv-<version>-<target>.tar.gz` for this build's target triple, [verify](#updates) it, unpack it with `tar`, show which files it replaces, ask for the typed confirmation `update`, then for each file keep the current one as `<name>.previous` and rename the new one over it (same directory, so each replacement is atomic). Then runs the new `rusttv --version`; if it doesn't print `rusttv <version>`, every file is put back (`ROLLED_BACK`). Refused (`PRECONDITION_FAILED`) when the release isn't newer than this build, when the directory isn't writable (installed by another user or a package manager: update it that way), or when `rusttv` runs from a cargo `target/debug` or `target/release` directory (update with git and cargo). Same terminal rule as device changes (`NOT_APPROVED` without one). No device needed; no journal (the `.previous` files are the record; undo with `mv rusttv.previous rusttv`) | [CliUpdate](#cliupdate) (`applied` false with `--dry-run`) |
| `approve --for <duration> --only launcher` | Grant a [delegation](#delegated-approval): for `<duration>` (`<n>m` or `<n>h`, more than 0 and at most `2h`; default `30m`), launcher plans for this device and firmware are applied without a terminal. Shows the device (serial, model, `ro.build.fingerprint`), the scope and the expiry, then asks for the typed confirmation `delegate`; needs an interactive terminal (`NOT_APPROVED` otherwise). Replaces any earlier delegation. Changes only `<state dir>/approval-grant.toml` on this computer | [GrantStatus](#grantstatus) |
| `approve --status` | The delegation, if any, and whether it is valid now. Read-only; no device needed | [GrantStatus](#grantstatus) |
| `approve --revoke` | Delete the delegation; no terminal or device needed | `{"path": string, "revoked": boolean}` (`revoked` false when there was none) |
| `compat profile` | Compatibility profile for `compat/devices/` (TOML on stdout; `--json` wraps it) | `{"path": string, "profile": object}` |

Commands that target a device take `--serial <serial>`. Without it, the only
connected device is used; with several devices connected, `--serial` is
required (`MULTIPLE_DEVICES`).

## Changing a device

Commands that change a device persistently follow SECURITY.md § Change
lifecycle:

1. **Plan** (read-only): what will run, at what risk, what is recorded first,
   what is checked after, and how each step is undone. `--dry-run` stops here
   and prints it (`--json`: a [DryRun](#dryrun)).
2. **Approve**: the plan is applied only after a person types its
   `confirmation` word (`takeover`, `privacy`, `optimize`, `airplay`, `restore`, `install`, `uninstall`, `upgrade`; `update` for `update cli`; `delegate` for `approve`) at an
   interactive terminal.
   There is no `--yes` flag. Without a terminal on stdin (scripts, agents,
   MCP) the command fails with `NOT_APPROVED` before contacting the device,
   unless a valid [delegation](#delegated-approval) covers it.
   This proves a terminal, not a person: never run `rusttv` for an agent in
   a pseudo-terminal.
3. **Apply**: the device's `ro.build.fingerprint` must match the plan and each
   step's original state must hold; then the original state is written to a
   journal, and each step is journaled before it runs and verified after.
   If a step or a check fails, every step that ran is undone in reverse
   (`ROLLED_BACK`, or `ROLLBACK_FAILED` with the commands to run by hand).
   Transient steps (pressing HOME) need no undo. Upgrading and uninstalling
   the RustTV launcher can't be undone (putting the old version back needs
   its APK): such a step is always a plan's last, its `undo` is null, the
   plan's notes say so, and a failed check after it is `ROLLBACK_FAILED`.
4. **Restore** later from the journal (`launcher restore`, `optimize restore`). Restoring is never
   rolled back; a failed step is reported with its manual command. If the
   firmware changed since the journal was written, the plan warns and restore
   still runs.

Journals are TOML files in `<state dir>/journals/`, named
`<unix ms>-<plan>.toml`. `launcher restore` only restores takeover
journals and `optimize restore` only privacy journals (`<ms>-privacy.toml`)
and optimizer journals (`<ms>-optimize-<profile>-v<version>.toml`),
so each can be undone without the others; install and uninstall journals are
a record (undo an install with `launcher uninstall`). Each `optimize apply
privacy` writes its own journal (a second run only disables what the first
left enabled); `optimize restore` restores the newest, so run it once per
journal. Only one optimizer journal can be restorable per model and firmware
family at a time. Optimizer journals contain the `set_setting` command, which
builds before `rusttv.v6` can't read: such a build lists them as unreadable
(and so refuses `launcher install`/`uninstall` until they are restored or
moved). A journal file that can't be read (edited, or
written by an incompatible build) is never ignored silently: `launcher restore`
warns about it (as does `optimize restore`), and `launcher install` (upgrade), `launcher uninstall` and `optimize apply reduced-animations` refuse
with `PRECONDITION_FAILED` until it is moved out of the directory. The state directory is `RUSTTV_STATE_DIR` if set,
otherwise `~/Library/Application Support/rusttv` on macOS and
`$XDG_STATE_HOME/rusttv` (default `~/.local/state/rusttv`) on Linux.
Recovery is never guaranteed.

Each journal records how its plan was approved in `approved_by` (and a
restored journal how its restore was, in `restore_approved_by`): `{ by =
"terminal" }`, `{ by = "delegation", granted_at_unix_ms = <ms> }` or `{ by =
"upgrade", confirmed_at_unix_ms = <ms> }` (a `launcher upgrade` stage).
Journals without them (written before, or rebuilt by `--from-profile`) still
load. Builds before this field (earlier `rusttv.v7` builds and older) refuse
unknown journal fields, so they list such journals as unreadable, like
optimizer journals before `rusttv.v6` (and so refuse `launcher install`
(upgrade) / `uninstall` until they are moved).

### Delegated approval

*Owner decision 2026-10-24 (SECURITY.md § Delegated approval).* A person at
an interactive terminal may let launcher plans be applied without a terminal
for a while, e.g. by an agent in their session:

```sh
rusttv approve --for 30m --only launcher [--serial <s>]   # asks for `delegate`
rusttv approve --status [--json]
rusttv approve --revoke
```

The delegation is a TOML file, `<state dir>/approval-grant.toml`
(`schema_version`, `serial`, `fingerprint`, `scope`, `granted_at_unix_ms`,
`expires_at_unix_ms`); one at a time, a new one replaces it. It lasts at most
2 h (default 30 min). Scope `launcher` covers exactly the plans
`launcher-install` (`launcher install`, `launcher upgrade`, `update
launcher`), `home-takeover` and `home-takeover-restore` (`launcher takeover`,
`launcher upgrade`, `launcher restore`), not `launcher uninstall`,
`optimize`, `tools` or `update cli`. When such a command runs **without** a
terminal and the delegation is valid (now before its expiry, same serial and
same `ro.build.fingerprint` as the plan, which the apply checks again on the
device, scope covering the plan), the plan is approved by it and stderr says
"Approved by the delegation granted at HH:MM UTC, valid until HH:MM UTC".
With a terminal a person is present, so the command asks as usual. Without
either: `NOT_APPROVED`, naming `rusttv approve`. A delegation file that
doesn't parse, or claims more than 2 h, approves nothing. Agents and MCP
servers must never create or edit it: this is a rule, not a lock, since
anything that can write files as you can write it.

### Updates

`update check`, `update launcher` and `update cli` read the latest release
of `damienmiheev/rusttv` from
`https://api.github.com/repos/damienmiheev/rusttv/releases/latest` with
`curl` (`RUSTTV_CURL`; HTTPS only, also for redirects, TLS 1.2 or newer, a
size cap per file), or from `RUSTTV_UPDATE_DIR` without any network. Drafts,
pre-releases and tags that aren't `v<major>.<minor>.<patch>` are refused
(`UPDATE_CHECK_FAILED`); only a strictly newer version is installed, never
an older one. Download URLs must be `https://` on `github.com` or
`objects.githubusercontent.com`.

Before anything is installed or replaced, `SHA256SUMS` must carry a valid
minisign signature (`SHA256SUMS.minisig`) by the release signing key compiled
into `rusttv`, and the file's size and SHA-256 must match the release and
`SHA256SUMS`; otherwise `UPDATE_VERIFY_FAILED` and nothing changes
(SECURITY.md § Updates). **This build has no release signing key yet**, so
`update launcher` and `update cli` refuse with `UPDATE_VERIFY_FAILED` ("this
build has no release signing key"), also with `--dry-run`; `update check`
works and reports `can_verify: false`. Not available as MCP tools.

### Privacy choices

`optimize analyze` and `optimize apply privacy` read your choices from
`--choices <file>` or else `<config dir>/privacy.toml`, where the config dir is
`RUSTTV_CONFIG_DIR` if set, otherwise `~/Library/Application Support/rusttv`
on macOS and `$XDG_CONFIG_HOME/rusttv` (default `~/.config/rusttv`) elsewhere.
Without a file the defaults apply. The file is yours; it is never in the
repository:

```toml
categories = ["telemetry", "ads", "extras", "casting"]  # default: telemetry, ads, extras
keep = ["com.google.android.apps.mediashell"]          # never disable these; wins over everything
also = ["org.droidtv.amazonalexa"]                     # catalog packages outside the chosen categories
extra = ["com.example.preinstalled"]                   # packages not in the catalog; always untested
```

Every field is optional; unknown fields and invalid package names are
refused (`INVALID_CHOICES`). `casting`, `smart-home`, `voice` and
`streaming-apps` are features some people use, so they are planned only if
the file chooses them (or names a package in `also`).

## Output

Without `--json`: human-readable text on stdout, errors and hints on stderr.

With `--json`, stdout carries exactly one JSON object, success or failure:

```json
{"schema":"rusttv.v7","ok":true,"data":{}}
{"schema":"rusttv.v7","ok":false,"error":{"code":"DEVICE_UNAUTHORIZED","message":"device `192.168.1.20:5555` is unauthorized; accept the debugging prompt on the TV"}}
```

- `schema` identifies the contract version. Within `rusttv.v7`, fields are only
  added, never renamed, removed or changed in meaning; anything else bumps it.
  - `rusttv.v7`: new commands `update check`, `update launcher` and `update
    cli` ([UpdateCheck](#updatecheck), [CliUpdate](#cliupdate); `update
    launcher` reuses the `launcher-install` [DryRun](#dryrun)); new error
    codes `UPDATE_CHECK_FAILED` and `UPDATE_VERIFY_FAILED`; new confirmation
    word `update`. Also new in `rusttv.v7` (before its release): the
    commands `launcher upgrade` ([LauncherUpgrade](#launcherupgrade),
    [StageReport](#stagereport)) and `approve` ([GrantStatus](#grantstatus);
    [delegated approval](#delegated-approval)), the confirmation words
    `upgrade` and `delegate`, and the journal fields `approved_by` /
    `restore_approved_by`. Everything else is unchanged.
  - `rusttv.v6`: [OptimizeAnalysis](#optimizeanalysis) has a new `optimizer`
    array (status and current settings of each optimizer profile); new
    command `optimize apply reduced-animations`; `optimize restore` takes an
    optional `privacy` \| `reduced-animations` and also restores optimizer
    journals (without a name it refuses with `PRECONDITION_FAILED` when both
    kinds are restorable); new error code `OPTIMIZE_NOT_TESTED`; new
    [DryRun](#dryrun) plan names `optimize-<profile>-v<n>` and `…-restore`
    with `R2` steps. Also new in `rusttv.v6` (before its release): the
    experimental commands `tools airplay status`, `enable` and `disable`
    ([AirplayStatus](#airplaystatus), [DryRun](#dryrun) plan names
    `tools-airplay` and `tools-airplay-restore`) and the error code
    `FEATURE_NOT_SUPPORTED`. Everything else is unchanged.
  - `rusttv.v5`: [OptimizeAnalysis](#optimizeanalysis) lists catalog packages
    (`privacy` with its `tested` flag and `packages` is gone; `choices`,
    `categories` and `packages` with `source`, `default`, `tested`,
    `chosen` are new); categories `casting`, `smart-home`, `voice`,
    `streaming-apps` are new; new error code `INVALID_CHOICES`;
    `PROFILE_NOT_TESTED` now means no tested or allowed package is left.
    `optimize apply privacy` takes `--choices` and `--allow-untested`.
    Everything else is unchanged.
  - `rusttv.v4`: new error codes `PROFILE_NOT_TESTED` and `PACKAGE_PROTECTED`
    (`optimize apply privacy`); new commands `optimize analyze`, `optimize
    apply privacy`, `optimize restore`. Everything else is unchanged.
  - `rusttv.v3`: in [BenchmarkReport](#benchmarkreport), `apps[].idle_pss_kib`
    is renamed `idle_pss_kib_after_resumes` (same measurement), and
    `apps[].idle_pss_kib_fresh`, `apps[].idle_pss_growth_kib` and
    `options.fresh_settle_ms` are new. Everything else is unchanged.
  - `rusttv.v2`: `pm list packages` printing an error is now the
    `PACKAGE_LIST_FAILED` error. In `rusttv.v1` it read as an empty list
    (e.g. `apps list` returned `[]`); success output is unchanged.
- `error.code` is stable and meant for programs. `error.message` is for people
  and may change.
- Absent values are `null`, never omitted. Sizes are in KiB (`*_kib`),
  durations in milliseconds (`*_ms`).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | The operation failed (adb missing, command failed, unexpected output) |
| 2 | Usage error: bad arguments, or several devices and no `--serial` |
| 3 | The device can't be reached: not found, unauthorized, offline, connection failed |

## Error codes

| Code | Exit | Meaning |
|---|---|---|
| `ADB_NOT_FOUND` | 1 | `adb` isn't installed or not on `PATH` (set `RUSTTV_ADB`) |
| `ADB_SPAWN_FAILED` | 1 | `adb` exists but couldn't be started |
| `NO_DEVICE` | 3 | No device is connected |
| `MULTIPLE_DEVICES` | 2 | Several devices are connected; pass `--serial` |
| `DEVICE_NOT_FOUND` | 3 | The requested serial isn't connected |
| `DEVICE_UNAUTHORIZED` | 3 | The TV hasn't accepted this computer's debugging key yet |
| `DEVICE_OFFLINE` | 3 | adb knows the device but it isn't responding |
| `DEVICE_NOT_READY` | 3 | The device is in another state (e.g. recovery) |
| `CONNECT_FAILED` | 3 | `adb connect` failed (refused, unknown host, ...) |
| `COMMAND_FAILED` | 1 | An adb command failed for another reason; the message has its stderr |
| `INVALID_OUTPUT` | 1 | adb printed something that isn't UTF-8 |
| `PACKAGE_LIST_FAILED` | 1 | `pm list packages` printed an error (e.g. `cmd: Can't find service: package`, `Exception occurred ...`) instead of a package list; the message has the line. An empty list is never this error; nothing changed |
| `LAUNCH_FAILED` | 1 | `am start` couldn't start a benchmarked activity (wrong component, app not installed) |
| `INCOMPLETE_PROFILE` | 1 | The device didn't report a value a compatibility profile needs |
| `INVALID_PLAN` | 1 | A plan or journal has a step that can't be undone, or doesn't match its journal (a bug or an edited journal) |
| `PRECONDITION_FAILED` | 1 | The device isn't in the state the plan needs (e.g. RustTV not installed, Home isn't a stock launcher, firmware changed since planning); nothing changed |
| `NOT_APPROVED` | 1 | No interactive terminal and no valid [delegation](#delegated-approval) covering the plan, or the confirmation wasn't typed; nothing changed |
| `JOURNAL_FAILED` | 1 | A journal (or the delegation file) couldn't be read or written; if a change had started, it was rolled back |
| `TAKEOVER_NOT_TESTED` | 1 | No bundled compat profile matches this device's model and firmware family, or it doesn't record a tested takeover of the stock launcher Home opens; nothing changed |
| `PROFILE_NOT_TESTED` | 1 | `optimize apply`: none of the chosen packages that are enabled on the device is tested on this model (no compat profile matches, or its `[[privacy.tested]]` doesn't list them), and `--allow-untested` wasn't given; the message names them; nothing changed |
| `OPTIMIZE_NOT_TESTED` | 1 | `optimize apply <profile>` (not `privacy`): the matching compat profile doesn't record this optimizer profile and version as tested (`[[optimizer]] … tested = true`), or no profile matches, and `--allow-untested` wasn't given; nothing changed |
| `PACKAGE_PROTECTED` | 1 | `optimize apply`: your choices select a package RustTV never disables (it handles or can handle Home, provides an input method or an enabled accessibility service, or is on `compat/protected.toml`); the message names each one and why (add it to `keep`, or remove it from `extra`); nothing changed |
| `INVALID_CHOICES` | 1 | `optimize`: the [choices file](#privacy-choices) can't be read or parsed (a file given with `--choices` must exist), `also` names a package not in the catalog, or `extra` one that is; nothing changed |
| `FEATURE_NOT_SUPPORTED` | 1 | `tools airplay enable`: no bundled compat profile matches this device, or it lacks MediaTek's AirPlay packages; the message says which; nothing changed |
| `INVALID_APK` | 1 | `launcher install` found no APK to install: no APK is built for the device's primary ABI, the file is missing or not a file, or its path isn't absolute UTF-8 with a plain `.apk` file name; nothing changed |
| `NO_JOURNAL` | 1 | `restore` found no journal to restore for this device; pass `--journal <file>` |
| `ROLLED_BACK` | 1 | A step or its check failed; every step that ran was undone and verified |
| `ROLLBACK_FAILED` | 1 | A step failed **and** undoing failed; the message has the commands to run by hand |
| `RESTORE_FAILED` | 1 | `restore` couldn't undo a step; the message has the commands to run by hand |
| `UPDATE_CHECK_FAILED` | 1 | `update`: the release couldn't be fetched or read (curl or `RUSTTV_UPDATE_DIR` failed, a draft or pre-release, a tag that isn't `v<major>.<minor>.<patch>`, a download URL off GitHub or not HTTPS, a file missing from the release, e.g. no APK for the device's ABI or no archive for this computer), or the archive couldn't be unpacked; nothing changed |
| `UPDATE_VERIFY_FAILED` | 1 | `update launcher` / `update cli`: this build has no release signing key, `SHA256SUMS` isn't signed by it (another key, a bad or legacy signature), a file isn't listed in `SHA256SUMS`, or its size or SHA-256 doesn't match; nothing was installed or replaced |
| `INTERNAL` | 1 | Output couldn't be serialized (a bug; please report it) |

## Environment

| Variable | Effect |
|---|---|
| `RUSTTV_ADB` | Path of the `adb` executable to use instead of the one on `PATH` |
| `RUSTTV_STATE_DIR` | Directory for journals and the delegation file instead of the platform default ([Changing a device](#changing-a-device)) |
| `RUSTTV_CONFIG_DIR` | Directory of the privacy choices file `privacy.toml` instead of the platform default ([Privacy choices](#privacy-choices)) |
| `RUSTTV_CURL` | Path of the `curl` executable `update` uses instead of the one on `PATH` |
| `RUSTTV_UPDATE_DIR` | Read the latest release from this directory instead of GitHub, with no network: `release.json` (the subset of GitHub's release JSON RustTV reads: `tag_name`, `draft`, `prerelease`, `html_url`, `assets` of `{name, browser_download_url, size}`) and each asset as a file named after it. Files are verified exactly like downloads, so they must be signed with the release signing key ([Updates](#updates)); for tests and offline installs |
| `RUST_LOG` | Reserved for logging (not used yet) |

## Data types

### Device

`serial` (string), `state` (`device` \| `unauthorized` \| `offline` \| other
adb state, e.g. `no permissions`), `model`, `product`, `device` (string or null).

### ConnectReport

`address` (`host:port`), `outcome` (`connected` \| `already_connected`),
`state` (as in [Device](#device), or null if adb doesn't list it yet).

### DeviceInfo

`serial`; `manufacturer`, `brand`, `model`, `device`, `android_version`,
`security_patch`, `build_fingerprint`, `build_characteristics` (string or
null); `api_level` (integer or null); `abis` (array of strings, may be empty);
`memory` (`{total_kib, available_kib}` or null; `available_kib` may be null);
`data_storage` (`{total_kib, available_kib}` or null).

### App

`package`, `apk_path` (string or null), `kind` (`system` \| `third_party`),
`enabled` (boolean).

### PerformanceSnapshot

`serial`; `taken_at_unix_ms` (host clock); `build_fingerprint` (string or
null); `memory` (as in [DeviceInfo](#deviceinfo)); `cpu` (`{busy_percent,
sample_ms}` or null when no CPU time elapsed); `home_app` (`{package,
activity, pss_kib, rss_kib}` or null; memory values are null when the
launcher isn't running); `top_processes` (up to 10 `{pid, name, rss_kib}`,
largest first).

### BenchmarkReport

`rusttv_version`; `device` (`serial`, `manufacturer`, `model`, `api_level`,
`build_fingerprint`); `options` (`runs`, `warmup`, `pause_ms`, `settle_ms`,
`fresh_settle_ms`, `cpu_window_ms`, `idle_samples`); `apps`: one per `--app`,
with `component`, `cold_start_ms`, `resume_ms`, `idle_pss_kib_fresh`
(after a fresh cold start and `fresh_settle_ms`, before any series),
`idle_pss_kib_after_resumes` (after the cold-start and resume series and
`settle_ms`; with one app there is no resume series), `idle_cpu_percent`
(after the series; each a [Stats](#stats) or null), `idle_pss_growth_kib`
(median after resumes minus median fresh; positive when the series left the
app bigger; null when either is missing) and `discarded_runs`; `runs`: every launch with
`component`, `kind` (`cold` \| `resume`), `round`, `launch_state` (Android's
`COLD`/`WARM`/`HOT`/...), `total_time_ms`, `wait_time_ms`, `counted`,
`discarded_because` (string or null).

### LauncherStatus

`home` (`package/activity` or null); `candidates` (`{component, priority}`,
highest priority first: Android opens the highest without asking, so on
Google TV the stock launcher's priority 2 beats every other launcher's 0);
`rusttv_installed`, `rusttv_is_home` (booleans; package `ai.rusttv.launcher`).

### DryRun

`plan` (e.g. `home-takeover`, `home-takeover-restore`, `launcher-install`,
`launcher-uninstall`, `privacy`, `privacy-restore`,
`optimize-reduced-animations-v1`, `optimize-reduced-animations-v1-restore`,
`tools-airplay`, `tools-airplay-restore`); `serial`;
`fingerprint` (`ro.build.fingerprint` the plan is for); `max_risk` (`R0` to
`R3`); `confirmation` (the word to type); `steps`: in order, each with
`risk`, `command` (exact adb command line), `records` (original state checked
and journaled first, or null), `verify` (array of checks after the command),
`undo` (adb command line, or null), `poll_ms` (how long the checks are
retried before the step fails, or null); `notes` (array of strings for people:
recovery hints, warnings).

### OptimizeAnalysis

`serial`; `profile` (file name of the bundled compat profile matching the
device, e.g. `tpv-ph1m-ww-9972.toml`, or null: then nothing is tested);
`choices` (path of the choices file read, or null for the defaults);
`categories` (the categories the choices apply); `packages`: every catalog
package installed on the device and every `extra` of the choices (installed
or not), in plan order, each with `package`, `source` (`catalog` \|
`extra`), `category` (`telemetry` \| `ads` \| `extras` \| `casting` \|
`smart-home` \| `voice` \| `streaming-apps`, or null for an `extra`),
`default` (boolean: the category is on unless opted out), `what` and
`side_effects` (null for an `extra`), `tested` (boolean: listed in the
profile's `[[privacy.tested]]`), `checked` (date it was tested on this
model, or null), `chosen` (boolean: the choices select it; `optimize apply
privacy` plans it if it is enabled and tested, or with `--allow-untested`),
`state` (`absent` \| `enabled` \| `disabled`) and `protected` (why RustTV
refuses to disable it, or null). `optimizer`: every bundled optimizer
profile, each with `profile` (e.g. `reduced-animations`), `version`,
`description`, `status` (`suggested`: tested here with a committed report of
a measured benefit, not applied; `available`: tested here, so `apply` is
allowed, but no measured benefit, so not suggested; `applied`: a restorable
journal for this firmware exists and every key still has the value it set;
`drifted`: such a journal exists but a key has another value now; `stale`: a
restorable journal from another firmware of this model, restore it with
`--journal` and apply again; `not_tested`: `apply` refuses it here;
`already_set`: every key already has the profile's value; `unknown`: a key
the status depends on can't be read now, see `read_error`), `tested`
(boolean: the compat profile records this version as tested), `evidence`
(path of the report, or null), `journal` (path behind `applied` / `drifted` /
`stale`, or null) and `settings`: `{key, value, current, read_error}` per
key, `value` being the profile's, `current` the value `settings get global`
prints now (null when unset, or when it can't be read) and `read_error` why
it can't be read (adb failed, or the output is neither `null` nor a decimal
like `1.0`), or null. A key that can't be read never fails `optimize
analyze`; the privacy part is listed as usual.

### AirplayStatus

`serial`; `supported` (boolean: `tools airplay enable` would plan here);
`reason` (why not, or null); `profile` (file name of the matching bundled
compat profile, or null); `packages` (`{package, installed}` for each of the
three MediaTek packages); `launcher_installed` (boolean); and, null unless
all three packages are installed: `setup_airplay_enable`,
`show_airplay_setting_ui` (what `settings get global` prints, null when
unset), `service_running` (boolean), `log` (array of log lines, newest last,
or null if the log can't be read), `listening_ports` (sorted array of
integers, or null if `ss` is missing or fails), `journal` (path of a
restorable `enable` journal for this firmware, or null). `bonjour`: null
without `--check-bonjour`, else `{tool, instances, note}`: `tool`
(`dns-sd` \| `avahi-browse`, or null when neither is installed),
`instances` (names of the `_airplay._tcp` services seen, any device on the
network) and `note` (string or null).

### UpdateCheck

`latest` (version of the latest release), `page` (its GitHub page),
`can_verify` (boolean: this build has a release signing key; without one
`update cli` / `update launcher` refuse); `cli`: `current` (this build's
version), `target` (its Rust target triple), `update_available` (boolean:
the release is newer), `archive` (the release's archive for `target`, or
null when it has none); `launcher`: null when no device was checked, else
`serial`, `installed` (`versionName`, or null when not installed), `abi`
(primary ABI, or null), `update_available` (boolean: `update launcher` would
install `apk`), `apk` (file name, or null); `launcher_note` (why no device
was checked, or why there is no launcher update; or null).

### CliUpdate

`from` (this build's version), `to` (the release's), `archive` (the verified
archive), `dir` (directory of the running `rusttv`), `files` (`{path,
previous}` for each file replaced, `previous` being where the current one is
kept), `confirmation` (`update`), `applied` (false for `--dry-run`).

### ChangeReport

`journal` (path of the journal file); `status` (`applied` \| `restored`).

### LauncherUpgrade

`takeover_active` (boolean: the upgrade has the three stages);
`confirmation` (`upgrade`); `stages`: in order, each with `stage`
(`restore` \| `install` \| `takeover`), `plan` (a [DryRun](#dryrun), or null
for a stage planned only when it runs: the install and takeover of an active
takeover) and `description` (what the stage does, for people). Without an
active takeover there is one `install` stage, whose plan's `confirmation` is
`upgrade`.

### StageReport

`stage` (`restore` \| `install` \| `takeover`), and `journal`, `status` as in
[ChangeReport](#changereport): one per stage, in order (only `install`
without an active takeover).

### GrantStatus

`path` (the delegation file); `grant` (null when there is none, or it can't
be read; else `schema_version`, `serial`, `fingerprint`, `scope`
(`launcher`), `granted_at_unix_ms`, `expires_at_unix_ms`); `valid` (boolean:
it approves plans now, within its scope, device and firmware); `reason` (why
not, or null).

### Stats

`n`, `min`, `p10`, `median`, `p90`, `max`, `iqr` (p75 − p25). Percentiles use
the nearest-rank method.

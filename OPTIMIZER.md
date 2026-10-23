# Safe optimizer (Phase 4) — design

Status (2026-10-23): § 1–6 (R2 animation settings) are **design, not built**.
§ Privacy profiles (R3) is **built** in its generic form (catalog, device
profiles, choices; § Generic design) and tested against fakes; twelve
packages were tested on the owner's Philips 65OLED818 (2026-10-23).
It builds on the safety engine that Home takeover already uses
(`crates/rusttv-core/src/plan.rs`, `journal.rs`; SECURITY.md § Change
lifecycle) and changes only what the optimizer needs.

Status legend as in RESEARCH.md: **[verified]** = checked against the cited
source on 2026-10-23; **[unverified]** = confirm before relying on it.

## Goals and limits

- `rusttv optimize analyze` (R0) → `optimize apply <profile> [--dry-run]` (R2,
  approved plan) → `optimize restore`.
- R2 only in this phase: global `settings` keys from a short allowlist that
  is compiled into the binary. Package changes (R3 debloat) are a later,
  separate design.
- No suggestion without data: a profile is suggested for a device only when
  that device's profile records a measured benefit made with the
  BENCHMARK_PLAN method (§ Profiles).
- Excluded: anything needing root, `setprop`, system partitions, free-form
  setting keys or values, or anything not reliably reversible over ADB.

## 1. Scope: candidate settings

All keys are in `Settings.Global`, readable and writable by the adb shell
(it holds `WRITE_SECURE_SETTINGS`; RESEARCH §5) **[unverified on the
Philips]**. The `settings` shell command (`SettingsService.java`, Android 14)
takes `get|put|delete global KEY [VALUE]`; `get` prints the literal `null`
when the key is unset **[verified]**.

### In scope (v1)

| Key | Source | Effect | Values we write |
|---|---|---|---|
| `window_animation_scale` | `Settings.Global.WINDOW_ANIMATION_SCALE`, "Scaling factor for normal window animations … 0.0f will disable window animations" **[verified]** | Window open/close animations | `0.5` |
| `transition_animation_scale` | `Settings.Global.TRANSITION_ANIMATION_SCALE`, "activity transition animations" **[verified]** | Activity transitions (launching an app, returning Home) | `0.5` |
| `animator_duration_scale` | `Settings.Global.ANIMATOR_DURATION_SCALE`, "Animator-based animations … start delay and duration", default 1.0 **[verified]** | `ValueAnimator` in every app; `0` ends animations immediately | `0.5` |

All three take effect at once: `WindowManagerService` observes their URIs
and posts `UPDATE_ANIMATION_SCALE`, which also pushes the new animator scale
to running apps (`WindowManagerService.java` `SettingsObserver`,
`dispatchNewAnimatorScaleLocked`, Android 14) **[verified]**. No reboot.
Values are clamped by `WindowManager.fixScale` **[verified]**.

Why `0.5` and not `0`: `0` makes animator-driven UI (spinners, focus
animations) jump or vanish in some apps **[unverified, common report]**.
`0.5` is the conservative choice; `0` is an owner decision (Open question 1).
RustTV's own launcher animates in Slint, which doesn't read these scales
**[unverified]**, so any benefit is system-wide, reported separately from
launcher numbers (BENCHMARK_PLAN § What we compare).

### Considered, deferred

| Key | Why deferred |
|---|---|
| `disable_window_blurs` (API 31+, `@hide @TestApi`, boolean) **[verified]** | Helps only if the firmware uses cross-window blur. `analyze` reports `ro.surface_flinger.supports_background_blur` first; add a profile only if a device uses blur and a measurement shows a benefit |
| `cached_apps_freezer` (`enabled`/`disabled`/`device_default`, `@hide`) **[verified]**; observed live by `CachedAppOptimizer` on Android 14 **[verified]** | Memory/CPU of cached apps, not responsiveness; vendor kernels may lack freezer support; turning it off may cost memory. Needs its own measurement design |

### Excluded

| Candidate | Reason |
|---|---|
| Background process limit (Developer options) | Not a setting: the Settings app calls `IActivityManager.setProcessLimit` directly (`BackgroundProcessLimitPreferenceController`, Android 14) **[verified]**; not persisted, no `settings` key, and no typed shell command for it **[unverified that `am` lacks one]**. Limiting processes also makes app starts colder |
| `always_finish_activities` **[verified key]** | Destroys every activity when left: makes switching slower, and breaks apps |
| `activity_manager_constants` **[verified key, `@hide`]** | A free-form `key=value,…` string, overridden by DeviceConfig; not a fixed value we can validate or reliably restore |
| `setprop debug.*`, `persist.*` | `persist.*` needs root; `debug.*` doesn't survive reboot and isn't a setting |
| Package disabling ("debloat") | R3; see § Privacy profiles |

## 2. `analyze` (R0 only)

Reads, with existing R0 commands plus one new one (`GetSetting`):

- Device: `getprop` (fingerprint, model, API, `ro.config.low_ram`,
  `ro.surface_flinger.supports_background_blur`), `/proc/meminfo`.
- Each allowlisted key's current value (`settings get global <key>`).
- The matching bundled device profile and its `[[optimizer]]` entries.
- Journals in the state directory: restorable `optimize-*` and
  `home-takeover` journals for this device.

Reports, per optimizer profile:

| Status | Meaning |
|---|---|
| `suggested` | Device profile lists it as tested on this firmware family, with evidence; not applied now |
| `applied` | A restorable journal exists and every key still has the value it set |
| `drifted` | A restorable journal exists but some key has another value now (user changed it, accessibility "Remove animations" **[unverified that it writes these keys]**, or a firmware update) |
| `stale` | Applied on another fingerprint (firmware updated since); suggests `restore`, then a fresh `apply` |
| `not_tested` | No measured benefit on this device; shown, never suggested, and `apply` refuses it |
| `already_set` | Current values already equal the profile's |

The `suggested` entry quotes the evidence (metric, median and IQR before and
after, device, date, path to raw data), never a general claim. No profile has
evidence today, so `analyze` will suggest nothing until the Philips run in
§ 6 produces some.

## 3. Profiles

Two kinds of TOML, both compiled into the binary like device profiles today
(so dropping a file can't widen what runs), both with `deny_unknown_fields`
and a test loading every file.

**Optimizer profile**, `compat/optimizer/<name>.toml`: what to change.

```toml
schema_version = 1
name = "reduced-animations"
version = 1                     # bump on any change to settings or values
description = "Halve window, transition and animator animation durations."

[[settings]]
key = "window_animation_scale"  # must be a SettingKey variant, else parse error
value = "0.5"                   # must pass that key's value validator

[[settings]]
key = "transition_animation_scale"
value = "0.5"

[[settings]]
key = "animator_duration_scale"
value = "0.5"
```

**Device gating**, in the existing device profile (`compat/devices/*.toml`,
`PROFILE_SCHEMA_VERSION` 4 → 5; 3 added `[privacy]`, 4 made it
`[[privacy.tested]]`), like `[launcher] takeover_tested`:

```toml
[[optimizer]]
profile = "reduced-animations"
version = 1                     # must equal the optimizer profile's version
tested = true                   # apply + restore both succeeded here
evidence = "benchmarks/raw/philips-65oled818/2026-11-xx/reduced-animations/report.md"
```

- `apply` refuses with a new `OPTIMIZE_NOT_TESTED` unless a device profile
  matching model and firmware family (`DeviceProfile::matches`) has
  `tested = true` for that profile **and version**. This follows "unknown
  devices get R0–R1 only" (SECURITY.md). The first run on a new device needs
  a development build with the entry edited locally, as for takeover.
- `evidence` must name a committed report (a test checks the file exists).
  `analyze` suggests only entries with evidence; `tested = true` without
  evidence allows `apply` but not a suggestion.
- A version bump invalidates every device entry for the old version.
- The plan's name carries the version: `optimize-reduced-animations-v1`, so
  journals say exactly what was applied without a journal schema change.

## 4. Plan, apply, restore on the existing engine

### New typed pieces (rusttv-core)

```rust
/// Allowlisted global settings. The namespace is fixed per key: no
/// namespace argument, no free-form key.
enum SettingKey { WindowAnimationScale, TransitionAnimationScale, AnimatorDurationScale }

/// A value validated for its key: for the three scales, a decimal
/// `[0-9]{1,2}(\.[0-9]{1,3})?` (no shell metacharacters possible).
struct SettingValue(String);

enum SettingState { Unset, Value(SettingValue) }

// DeviceCommand
GetSetting(SettingKey),                         // R0: settings get global <key>
SetSetting { key: SettingKey, to: SettingState, from: SettingState },
    // R2: to=Value(v) → settings put global <key> <v>
    //     to=Unset    → settings delete global <key>
    // inverse() = SetSetting { key, to: from, from: to }

// Check
SettingIs(SettingKey, SettingState),            // via GetSetting; "null" = Unset
```

Carrying the original (`from`) inside the command keeps the engine as it is:
`inverse()` is still derived from the command alone, `Check::effect_of` gives
`SettingIs(key, to)` as the step's verification, and `Step::original_state`
gives `SettingIs(key, from)`, which `apply` already checks before any step and
writes to the journal. `SetSetting` with `from == to` is refused at plan time
(`already_set`).

Reading the original: `settings get` prints `null` both for an unset key and
for a stored string `"null"`. Both read as the default for these float keys,
so treating `null` as `Unset` (restored by `delete`) is behaviourally the same
**[verified: `Settings.Global.getFloat` returns the default for a missing or
non-numeric value, `parseFloatSettingWithDefault`, Android 14]**. An original
that fails `SettingValue` validation (e.g. a vendor wrote something odd)
makes `plan` refuse that profile (`PRECONDITION_FAILED`), never restore an
unvalidated string.

### Plan shape

1. One `SetSetting` step per key, in profile order; R2; confirmation word
   `optimize` (typed, TTY only, no `--yes`, as today).
2. Extra check on the last step: `HomeIs(<package Home opened at plan time>)`
   (SECURITY.md lifecycle step 6: Home still resolves). This reuses the
   existing check and also holds with a takeover active (Home = RustTV).
3. `Plan::new` already rejects R2 steps without an inverse. An extra builder
   check: optimizer plans contain only `SetSetting` steps (no package steps).

### Journals and restore

- Same journal format and directory; plan name `optimize-<profile>-v<n>`.
  `JOURNAL_SCHEMA_VERSION` stays 1 (a new enum variant; old journals still
  load). Older `rusttv` binaries can't read the new journals and will list
  them as unreadable, which blocks `launcher uninstall` there (SECURITY.md
  rule 6) — acceptable, noted in CLI.md.
- `optimize restore [--journal <file>] [--dry-run]`: newest restorable
  `optimize-*` journal for the device. The journal is untrusted: the restore
  plan must contain only `SetSetting` steps (keys and values are re-validated
  by deserialization), mirroring `takeover_restore_plan`'s check.
- **Drift on restore (engine change):** before each undo, read the key. If it
  is neither the value we set (`to`) nor the original (`from`), someone
  changed it since; skip that step and report it ("left as is"), instead of
  overwriting the person's choice (Open question 3). Restore is still never
  rolled back.
- **One optimizer journal at a time:** `apply` refuses while a restorable
  `optimize-*` journal exists for the device; otherwise a second plan would
  record the first plan's values as "original", and restoring in the wrong
  order would leave changes behind. To switch profiles: restore, then apply.
- General `rusttv restore --journal <file>` (promised in SECURITY.md step 7)
  dispatches on the plan name to the takeover or optimizer validator; without
  `--journal` it refuses if more than one kind is restorable and lists them.

### Firmware changes

- `apply` already refuses if the fingerprint changed since the plan.
- Gating uses the firmware family, so a new Android version needs a new
  tested entry.
- `restore` warns and still runs on a changed fingerprint (existing rule).
- `analyze` reports `stale`/`drifted` (§ 2). Whether OTA updates keep global
  settings is **[unverified]**; the Philips run should check after the next
  update.

### Interaction with Home takeover

- Independent changes, independent journals. Optimizer plans never touch
  packages; `takeover_restore_plan` already refuses any step but
  `EnablePackage`, and the optimizer validator refuses any step but
  `SetSetting`, so neither restore can run the other's journal.
- `launcher restore` stays takeover-only; `optimize restore` optimizer-only.
- Home takeover stays out of every optimizer profile (SECURITY.md rule 1).
- The `HomeIs` check (plan step 2) uses whatever Home opens now, so it is
  correct with or without a takeover.

## 5. MCP

Agents may analyze and plan, never apply or restore (SECURITY.md § AI agents):

| Tool | Arguments | Same as |
|---|---|---|
| `optimize_analyze` | `serial?` | `rusttv optimize analyze --json` |
| `optimize_plan` | `serial?`, `profile` (enum of bundled profile names) | `rusttv optimize apply <profile> --dry-run --json` → PlanProposal |
| `optimize_restore_plan` | `serial?` | `rusttv optimize restore --dry-run --json` → PlanProposal |

No tool takes a setting key, value, journal path or command. All run R0 only
and write no journal.

## 6. Test plan

1. **Unit, fakes** (`FakeAdb`, `MemoryJournal`): render/parse of every new
   command and check; `SettingValue` rejects metacharacters and junk; profile
   and gating parse (unknown key, bad value, version mismatch, missing
   evidence file); plan refuses untested device, `from == to`, unvalidated
   original, a second restorable journal; restore refuses a journal with a
   non-`SetSetting` step; risk test updated (R2 = `SetSetting` only).
2. **Fault injection, fakes**: adb fails at every step index (rollback
   restores all earlier keys); verification fails (vendor clamps the value);
   journal write fails before/after each step; fingerprint changes between
   plan and apply; process "dies" mid-apply (journal left `applying` →
   restore undoes `started` and `done` steps); drift before restore; tampered
   journal (unknown key, `0.5; reboot` value) → deserialization error.
3. **Emulator** (Android TV API 34, opt-in `device-tests`): the same
   scenarios with real `adb` and `settings`, plus: kill the adb server
   mid-apply, `adb reboot` mid-apply then restore, values survive reboot,
   restore with a takeover journal also present. No performance numbers.
4. **Philips, only with the owner's approval for that session** (takeover is
   live there): `analyze` → `apply --dry-run` → owner applies → look at the
   TV → `restore` → `analyze` shows nothing applied. Then the benefit
   measurement (Open question 2), commit raw data and report, set `tested`
   and `evidence`.

Exit criteria (ROADMAP Phase 4): steps 1–3 pass; recovery docs (manual
path: Developer options → animation scales, or `adb shell settings delete
## Privacy profiles (R3): telemetry and ads

*Owner decision 2026-10-23: build this on the engine.* A privacy profile
disables, never uninstalls, packages that collect telemetry, show ads or run
for nothing, and optional features a person chooses to turn off. It is the
route SECURITY.md § Protected package policy already plans for (rule 4:
"tested-safe for that model and firmware family"), with one explicit,
per-plan override (`--allow-untested`, § Generic design).

**History.** The first build (commit bc04d9a, profile schema 3) listed each
package with its description inside the device profile and gated the whole
list on one `[privacy] tested = true`. The Philips 65OLED818 list of twelve
packages (five telemetry, three ads, four extras) was applied, used, rebooted
and restored on the owner's TV on 2026-10-23 with it. That form was then
replaced by the generic design below (*as built* 2026-10-23,
`crates/rusttv-core/src/{catalog,privacy}.rs`); the twelve became the
Philips profile's `[[privacy.tested]]` list and their descriptions moved to
the catalog. Journals did not change, so `optimize restore` restores
journals written by the first build.

**Checks before planning** (live, never cached; SECURITY.md § Protected
package policy *As built*): a chosen package is refused if Home opens it or
it can handle Home, if it provides an input method (`ime list -a -s`) or an
enabled accessibility service (`settings get secure
enabled_accessibility_services`), or if it is on `compat/protected.toml`
(compiled in; covers TV inputs, setup wizard, system UI, settings,
installers, network stack, Bluetooth, Play services and the launchers by
name). This applies to every chosen package, `extra` ones included; one
protected package refuses the whole plan (`PACKAGE_PROTECTED`; the message
suggests `keep`). **After the last step:** Home still opens the same activity
as before, and the set of Home candidates is unchanged; otherwise every step
is undone. Not built: a check that the TV input service is still running
after the last step (no typed command reads it yet), and re-running the
protected-package checks inside `apply` (the CLI plans right before the
confirmation).

**Separate from Home takeover.** The deny-list covers the stock launchers and
setupwraith, so privacy plans never list them; their journals are separate
(`launcher restore` and `optimize restore` each refuse the other's), so either
can be restored without the other.

**Restore.** `rusttv optimize restore [--journal <file>] [--dry-run]`: the
newest restorable privacy journal for the firmware. Like
`takeover_restore_plan`, the journal is untrusted: refused unless it is a
`privacy` journal whose every undo is `EnablePackage`. Packages enabled again
since are skipped (their steps marked undone) and named in the notes; if all
are, `PRECONDITION_FAILED`.

### Generic design: catalog, device profiles, your choices

*Owner decision 2026-10-23:* privacy profiles must work for anyone's TV and be
customisable, not just the owner's Philips. Three layers. **As built
2026-10-23**; tested against fakes and synthetic fixtures.

**1. Package catalog** (`compat/packages/*.toml`, compiled in,
`CATALOG_SCHEMA_VERSION` 1). One entry per package, written once and shared by
every device that has it, grouped by who ships it (`android.toml` for
AOSP/Google packages, `tp-vision.toml` for `org.droidtv.*`, `acr.toml` for
third-party content recognition such as Samba TV, one file per TV maker,
`streaming-apps.toml`):

```toml
schema_version = 1
[[packages]]
package = "tv.samba.ssm"
category = "telemetry"
what = "Samba TV / Samba Services Manager: automatic content recognition of what is on screen for ad targeting"
side_effects = "reported to cause problems when disabled on some Sony TVs; test carefully"
sources = ["rusttv: seen on a Philips 65OLED818, 2026-10-23", "cleanyourtv examples, 26 models"]
```

Categories: **default** `telemetry`, `ads`, `extras` (nobody needs them; on
unless a person opts out); **opt-in** `casting`, `smart-home`, `voice`,
`streaming-apps` (features some people use; never applied unless chosen).
Tests keep catalog entries valid, unique, off `compat/protected.toml`, with
non-empty `what`, `side_effects` and `sources`, and every tested package of a
bundled profile in the catalog.

*Seeded (2026-10-23), 279 entries:* 47 telemetry, 66 ads, 71 extras, 22
casting, 12 smart-home, 18 voice, 43 streaming-apps. 25 come from our own
checks on the owner's Philips (the twelve tested ones, plus thirteen seen
there and untested: Samba TV, the TP Vision auto-log service and demo mode,
Chromecast built-in, AirPlay, DLNA, Alexa, Control4, IP control, Netflix,
Prime Video, Rakuten TV); 23 of them also appear in cleanyourtv. The other
254 are derived from cleanyourtv (MIT, commit f9f0c53): packages on at least
three of its per-model `examples/*/debloat.txt` lists or in the `CATALOGUE`
of its `tools/privacy.py`, categorised by judgement, with `what` and
`side_effects` from its notes. Left out: anything on `protected.toml`, Home
screens, TV inputs and FAST channel inputs, tuners, HDMI, remote pairing and
remote helpers, firmware and OTA updaters, network and Wi-Fi helpers,
WebView, setup wizards, keyboards, TTS and accessibility, launcher partner
customisers that also supply the stock Home's Inputs button, and features
that fit no opt-in category (USB media players, galleries, browsers,
screensavers). Each derived file carries cleanyourtv's MIT notice;
`licenses/cleanyourtv/` has the licence, and `scripts/licenses.sh` ships it.

**2. Device profiles** (`compat/devices/*.toml`, `PROFILE_SCHEMA_VERSION` 4).
Only which catalog entries were tested on that model and firmware family,
and when:

```toml
[[privacy.tested]]
package = "org.droidtv.candeebug"
checked = "2026-10-23"
```

A package counts as tested on a device only if it is listed there (and in the
catalog). Tested means: disabled, the TV rebooted and used normally, and a
restore worked, on that model. Within a category, packages are disabled in
the order of this list, then untested ones in catalog order. A schema 3
`[privacy]` section still loads: with `tested = true`, each package with a
`checked` date counts as tested.

**3. Your choices** (a local file, never in the repo):
`<config dir>/privacy.toml`, where the config dir is `RUSTTV_CONFIG_DIR` if
set, else `~/Library/Application Support/rusttv` on macOS and
`$XDG_CONFIG_HOME/rusttv` (default `~/.config/rusttv`) elsewhere; or
`--choices <file>`.

```toml
categories = ["telemetry", "ads", "extras", "casting"]  # default: the three default ones
keep = ["com.google.android.apps.mediashell"]          # never disable these
also = ["org.droidtv.amazonalexa"]                     # catalog entries outside the chosen categories
extra = ["com.example.preinstalled"]                   # not in the catalog at all
```

Without a file, the default categories apply. `keep` wins over everything. A
file that exists but doesn't parse, a `--choices` file that doesn't exist, an
`also` package not in the catalog or an `extra` one that is, are refused
(`INVALID_CHOICES`), never silently replaced by the defaults.

**What gets planned.** The packages selected by the choices that are present
and enabled on the device. Each is one of:
- *tested*: in the catalog and tested on this device's profile;
- *untested*: in the catalog but not tested on this model, or in `extra`
  (not in the catalog). Planned only with `--allow-untested`; the dry run marks
  each one "UNTESTED on this model" with what is known about it (for an
  `extra`, that nothing is). Without the flag they are named in one note.
A plan with no tested and no allowed-untested package is refused
(`PROFILE_NOT_TESTED`). The live protected-package check applies to every
package, including `extra`; a protected package refuses the whole plan.
Absent or already disabled packages are skipped and named in the notes; the
last note says how many planned packages are tested on this model. Typed
confirmation `privacy`, journal (`<ms>-privacy.toml`), verify and rollback
as for every R3 plan.

**Commands.** `rusttv optimize analyze [--choices <file>]` lists every
catalog entry present on the device (state, category, default or opt-in,
tested on this model or not, chosen or not, protected or not) and every
`extra` from the choices file; it works without a matching profile (then
nothing is tested). `rusttv optimize apply privacy [--only <categories>]
[--choices <file>] [--allow-untested] [--dry-run]`. `rusttv optimize
restore` is unchanged (journals record what was disabled). `--only` narrows
the categories from the choices file; it never adds one, `also` counts only
within it, and `extra` packages are left out with it. JSON contract
`rusttv.v5` (CLI.md).

**Contributing a model.** Run `optimize analyze`, choose, apply with
`--allow-untested`, use the TV for a while, reboot, restore, then send a pull
request adding the catalog entries you learned about (with sources) and the
`[[privacy.tested]]` list for your model (`compat/README.md`).

**MCP.** `optimize_analyze` and `privacy_plan` read the same catalog, profiles
and choices file (on every call; no path parameter); `privacy_plan` takes
`only` and `allow_untested`, never applies, and returns the CLI command
(with `--allow-untested` when set). No tool takes a package name, applies or
restores.

applies, and returns the CLI command.

## Decisions (owner, 2026-10-23)

1. **Values**: one profile, `0.5` for all three animation scales.
2. **Measuring the benefit**: a Perfetto metric "key press → last frame of
   the transition", run as A–B–A blocks (baseline, applied, restored) of ≥ 30
   runs each; two typed approvals per measurement.
3. **Drift on restore**: skip keys changed since apply, and say so.
4. **Untested devices**: refuse `apply` (same rule as Home takeover).
5. **Command names**: `rusttv optimize analyze|apply|restore`, plus a
   general `rusttv restore --journal <file>`.
6. **`disable_window_blurs`**: a separate profile only if `analyze` finds the
   device supports and uses blur.

The owner's main interest beyond animations: turning off ads and telemetry
on the TV. That is mostly R3 (package state, protected-package policy), not
R2, and needs its own research and design.

## Risks

| Risk | Mitigation |
|---|---|
| No measurable benefit on the Philips | Then nothing is suggested; the feature still ships the safe path for later profiles |
| Vendor firmware clamps or ignores a key | Step verification fails → rollback; recorded in the device profile |
| The person or accessibility changes a key after apply | `drifted` in analyze; restore skips it |
| Settings reset by a firmware update | `stale`; restore is harmless (re-puts or deletes) |
| Apps misbehave at low animator scale | Conservative `0.5`; per-device `tested` gate |

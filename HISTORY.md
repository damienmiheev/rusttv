# Tweaks + History — design

Status (2026-10-24): **built** on the design below (v3 after three review
rounds, § Review log), tested on the desktop and with fakes; the run on the
Philips is recorded in § Spike results and benchmarks/raw. As-built notes are
marked *As built*. Replaces ROADMAP.md § Tweaks + History.

## Owner decisions (2026-10-24)

1. **Storage:** history in the launcher's private storage; the Mac keeps a
   full copy on every sync; after a reinstall the CLI puts it back.
2. **Approval on the TV** approves every kind of change. What the launcher
   can't do itself (packages) is approved on the TV and applied by the CLI,
   only if it is exactly what was approved and not expired.
3. **Door:** the launcher talks only to the adb shell (uid 2000).
4. **Store:** real git (`gix`) on both sides — "less window for bugs" than a
   custom store. **Packing is in v1.**
5. **`WRITE_SECURE_SETTINGS` is in v1** (contained, § 7).
6. **UI:** `emil-design-eng` + `design-taste-frontend`, clean and simple,
   the same app (§ 8).
7. **Fast:** the launcher must stay as fast as it is (§ 9 budgets).

## Goals and limits (v1)

- **One timeline** of the state RustTV manages, whoever changed it: the TV
  (Settings page, move/hide), the Mac CLI, or outside RustTV (drift). Each
  entry is a full snapshot; any two compare; any one can be restored.
- **Restore** through reviewed, approved, journaled, verified changes; the
  history itself only stores and compares.
- **Survives either side** losing its copy.

Not in v1: profiles, schedules, copy to another TV, presets, bisect, MCP
history tools. Never: the TV changing packages; anything but `rusttv
launcher takeover|restore` changing takeover packages; the Mac changing the
launcher's own state; history leaving the owner's TV and Mac; free-form
commands.

## 1. The snapshot

`state.toml`, the only file in each commit's tree:

```toml
format = 1

[launcher]                     # the launcher's own state; only the TV writes it
order = ["com.netflix.ninja/.MainActivity", "folder:1", "about", "settings"]
hidden = ["com.example/.Main"]
update_check = true

[launcher.folders."folder:1"]  # format 2; only when a folder exists
name = "Video"
apps = ["com.example.films/.Main", "com.example.series/.Main"]

[settings.global]              # SettingKey allowlist; absent key = unset
animator_duration_scale = "0.5"

[packages]                     # catalog packages only, observed by the Mac
"com.example.telemetry" = "disabled"

[device]                       # facts observed by the Mac, never restored
fingerprint = "Philips/PH1M_WW_9972/…"
takeover = "active"
```

- **Hashes are git's, over the stored bytes;** never re-serialized. State
  equality compares parsed values; all values are strings.
- **`[launcher]`:** tile names in order, hidden apps, the update switch; no
  focus. Uninstalled apps' names are kept (as the order file does).
- **Folders** (*added 2026-10-25*): a folder is the tile `folder:<number>`
  in `order`, and `[launcher.folders."folder:<number>"]` holds its name (1–40
  characters, no control characters; shown only, never run) and its apps in
  order. An app is in `order` or in one folder, never both; there are no
  folders in folders. A snapshot with a folder is written as `format = 2`;
  one without is still `format = 1`, so builds that read only format 1 keep
  reading a history until the first folder (then they say
  `HISTORY_FORMAT_NEWER`). The changes it adds: a folder made, removed or
  renamed, and an app put in or taken out of a folder; an app moving into a
  folder is not "removed from Home".
- **`[settings.global]`:** the `SettingKey` keys, values
  `[0-9]{1,2}(\.[0-9]{1,3})?`. For these three keys **absent ≡ "1"**
  (Android's default), so the TV restores "absent" by writing "1" and
  compares them as equal.
- **`[packages]`:** catalog packages only. **Takeover packages never**:
  `[device].takeover` records the takeover; a restore shows it and names
  `rusttv launcher takeover|restore`.
- **Privacy:** app names stay in the launcher's private storage and the
  Mac's state directory: never in this repository, logcat or MCP.

## 2. Entries

One commit on `main` per entry. Message: a one-line English summary (for
`git log` on the Mac), then trailers:

```
Settings: animations 0.5×

RustTV-Format: 1
RustTV-Author: tv          # tv | mac | drift | import
RustTV-Changes: <structured change list, TOML, base64url>
RustTV-Journal: 1792843149859-optimize.toml          # mac entries
RustTV-Approved-By: tv-code sha256:… | terminal | delegation | upgrade | none
```

- **Words come from the structured change list** at display time, in the
  current language, on the TV and in the CLI.
- **Order is the chain.** Times are the TV's clock; before 2024 shows as
  "time unknown".
- **History is never rewritten:** `main` only moves forward; nothing is
  coalesced or amended.
- **One writer:** the launcher. The Mac only fetches; it sends
  observations, notes and pending consumption through the door.
- **Caps:** 60 entries an hour for the TV's own edits: over the cap, the
  next entry waits and then records the full current state once (only
  in-between states are skipped; the edit itself is never lost). A separate
  30 an hour for everything through the door: over it, refused with
  `HISTORY_RATE_LIMITED`.

### What needs the code on the TV

| Change | On the TV |
|---|---|
| Move, hide, unhide, folders (make, rename, apps in and out; the launcher's direct edits, as today) | recorded as `tv` entries without a code |
| Settings, the update switch, any restore | review + code + hold OK (§ 6) |
| Packages | review + code + hold OK → pending for the Mac |

## 3. The store

- **gix** 0.90, `default-features = false, features = ["sha1"]`, plus
  `gix-pack` with `generate` and `streaming-input`; one crate,
  `rusttv-history` (no `rusttv-core` dependency), shared by the launcher and
  the CLI.
- **TV:** bare repository `files/history.git` (its id in `rusttv-id`, a
  UUID from Kotlin), opened lazily on the first history job. **One history
  thread owns the store** (process-wide, started on first use by the UI or
  the provider); the UI sends jobs without waiting, the provider sends a job
  and waits for its answer on its binder thread. So every entry is read
  head → build snapshot → commit in one job, and nothing races.
- **No panics on untrusted input:** the Android build aborts on panic
  (`panic = "abort"`), so a panic would kill Home; parsing and import are
  covered by property tests with random and damaged input. Crash recovery
  is the TV journal (§ 4.1).
- **Packing:** at ≥ 256 loose objects the launcher packs the objects of the
  commits after the newest packed one (gix-pack generator +
  `Bundle::write_to_directory`, which verifies), re-reads the history, then
  deletes those loose objects; at ≥ 8 packs, one full repack. On the history
  thread at background CPU priority, starting 5 s after the launcher pauses
  (so it doesn't compete with the app being opened); a resume interrupts it
  (gix's interrupt flag; temp files removed). Measured: 0.7 s per 256 objects; full repack of
  1,000 entries 3 s; 1,000 entries packed = 720 KB.
- **Mac:** `<state dir>/history/<history id>.git`, a normal bare repository
  (`git log`, `git diff`, any tool), written with gix — no system `git`.
- **Transfer = a git pack.** Export: a pack (whole objects, no deltas) of the
  commits after `since` plus the new head id. The receiver indexes it with
  `Bundle::write_to_directory` (which recomputes every object id), checks
  the chain from `since` and that every tree holds exactly one entry
  `state.toml` (blob, mode 100644), then moves its ref. Limits: 64 MB per
  pack, 200,000 objects (~66,000 entries, ~45 MB), 1 MB per object; an
  export over the limit is sent in parts by `since`. No retention: the
  Settings page shows the size.
- **Versions:** `RustTV-Format` per entry, `format` per snapshot, a door
  version; newer than the reader → `HISTORY_FORMAT_NEWER` (Mac: "run
  `rusttv update`"; TV: "update RustTV").

## 4. Flows

### 4.1 A change on the TV

1. Edit on the Settings page (a draft; Back discards).
2. **Review** (§ 8.5): each change, old → new, grouped "This TV changes now"
   / "Needs your computer" (packages, with their state "as last seen by your
   computer": the TV can't see catalog packages; the computer re-checks).
3. **Code + hold OK** (§ 6).
4. **TV journal first:** `files/history/journal.toml` (atomic): approval
   hash, each step's key, original value and target, status `applying`.
5. **Apply** step by step; verify by reading back; journal per step. On the
   first failure, undo the done steps **only where the value is still what
   the step set** (else leave it and name it), verify, status `rolled_back`
   or `rollback_failed`; the screen says which.
6. **Commit** a `tv` entry; remove the journal. Package steps become a
   pending approval (§ 6.3).
7. **Recovery** runs when the store first opens, whatever started the
   process (UI or provider): a journal still `applying` is rolled back by
   the same rule and recorded; one `applied` without its commit is rolled
   forward (committed). While a journal exists, door writes get `TV_BUSY`
   and the UI doesn't write `order.toml`. Launcher-state changes are the
   last step, one atomic rename of `order.toml`; when recovery changed it,
   the UI reloads it on resume (a flag, like the apps-changed flag).

### 4.2 A change from the Mac

Plans are unchanged. Only commands that apply a plan, with a TV that has
the launcher (never `inspect`, `sync` or MCP), send:

- **before:** an **observation** (`[settings]`, `[packages]`, `[device]`;
  never `[launcher]`) → a `drift` entry if it differs from head;
- **after** (applied, rolled back or restored): a **note** (plan, journal
  file, `approved_by`, outcome) and an observation → a `mac` entry.

A failing door is a warning, not an error; the next observation records the
difference.

### 4.3 Drift

Only the Mac observes device state; the TV records its own edits and what
the Mac sends. Resume does no history work. The sensitive-settings
watchlist baseline (§ 7) is kept **on the Mac**, never read back from TV
history.

### 4.4 Sync

`rusttv history sync`: export `since` the Mac's head (R0) → verify → fetch.
Also at the end of every plan-applying command. **History id:** a UUID
(Kotlin `UUID.randomUUID`) made with the first entry. The Mac's
`history/devices.toml` maps `ro.serialno` to history ids (an empty or
`unknown` serial: no mapping, `--history` required).

### 4.5 Reinstall: put the history back

- `rusttv launcher uninstall` (and any plan that reinstalls the launcher)
  runs a sync **before** its plan; if the sync fails it asks before going on.
- While the TV's history is empty and the Mac holds one for this serial, the
  CLI sends **no** observations or notes (they would start a new history);
  it offers `restore-tv` instead. Limit, stated in the CLI: once something
  is recorded on the TV first, the old history can't be put back on the TV
  but stays readable on the Mac.

*As built:* the whole history goes in one pack (`Store::export_whole`), as
the TV takes an import only whole (~0.7 MB per 1,000 entries).

`rusttv history restore-tv [--history <id>]` (`--history` required when more
than one fits): a plan, typed approval `history`, whose one step imports the
pack (class RL). The launcher accepts it only into an
empty store, keeps the old id, labels every imported entry "restored from
your computer", and commits an `import` entry for the live state. Imports
carry no pending approvals or journals.

### 4.6 Restore a snapshot (time machine)

- **From the TV:** History → entry → "Restore this state" → the diff to the
  live state becomes a draft → § 4.1. Launcher state and (with the grant)
  settings change now; packages become pending.
- **From the Mac:** `rusttv history restore <id>` → a new plan kind,
  `history-restore`, for **settings and packages only** (launcher state is
  restored only from the TV): `settings put/delete` and package
  enable/disable steps (protected rules 1–3 live, rule 4 as today, never
  `--allow-untested` from a TV approval). Dry run, typed approval `restore`,
  journal, verify, rollback. Refused while a restorable optimizer or privacy
  journal touches the same keys or packages, and vice versa. Its own
  journal-based undo: `rusttv history restore --journal <file>` (validator:
  only `settings put/delete` of allowlisted keys and enable/disable of
  catalog packages; the optimizer rule for values changed since).
- Pending approvals from the TV build the same plan kind.

## 5. The door

`ContentProvider`, authority `ai.rusttv.launcher.history`,
`exported="true"`, no permission, in the launcher's process. Every entry
point — `query`, `insert`, `update`, `delete`, `call`, `openFile`,
`openAssetFile`, `openTypedAssetFile`, `bulkInsert`, `applyBatch`,
`canonicalize`, `uncanonicalize`, `refresh` — first calls `checkShell()`
(`Binder.getCallingUid() == 2000`, literal, commented); `getType` returns a
constant; unknown URIs are refused; a test reflects over every overridden
method. It opens the store itself (recovery, § 4.1 step 7) and never starts
the UI.

**uid 2000 = anything with adb on this TV** (the Mac, wireless-debugging
peers, Shizuku-style servers). They can already change packages; through
the door they can only add observations and notes (labelled as such,
separately capped), consume a pending approval, or import into an empty
history. They can't change the launcher's state, create or approve
anything, or rewrite history. A forged observation is visible as a `drift`
entry from "your computer"; restoring to it from the TV still needs the
code and shows exactly what it changes.

Typed `DeviceCommand`s, fixed URIs plus validated hex:

| Command | adb | Class |
|---|---|---|
| `HistoryExport { since }` | `adb exec-out content read --uri content://ai.rusttv.launcher.history/export/<40-hex or "all">` | R0 |
| `HistoryPending` | `adb exec-out content read --uri …/pending` | R0 |
| `HistoryInbox { nonce, payload }` | `adb exec-in content write --uri …/inbox/<16-hex>`, payload on stdin | RL |
| `HistoryCommit { kind, nonce }` | `adb shell content call --uri content://ai.rusttv.launcher.history --method commit --arg <kind>:<16-hex>` | RL |

- **Class RL (new): launcher-private.** A `RiskClass` variant ordered between
  `Read` and `Transient`: above R0 (MCP never runs it; the "only listed
  commands above R0" test lists these four) and at most Transient (no
  inverse required). It only appends to RustTV's own history and never
  changes device state: SECURITY.md states this as the one exception to
  "every modifying operation is a plan" (door writes in plan-applying
  commands need no extra approval, like the Mac's own journal writes);
  `import` is still a step of the `history-restore-tv` plan.
- **Results:** `content write` returns nothing, so writes go to
  `inbox/<nonce>` — `openFile("w")` returns a real temp file
  (`ParcelFileDescriptor.open`), so the bytes are on disk when `content
  write` exits — and `content call … commit` validates and processes them
  and returns a bundle (`ok`, error code, new head) the CLI parses.
- **Export format:** one header line, then the pack:
  `rusttv-export 1 <history-uuid> <head|none> <more:0|1>\n`. Parts of at
  most 8 MB (by commit count); `more:1` means fetch again from the new head.
  An unknown `since` is its own error (`HISTORY_NOT_FOUND`): the CLI then
  fetches `all` into the repository for that history id.
- **Notes can't claim a TV approval:** the TV writes `tv-code` only for an
  approval it marked consumed for that note's consume nonce; other notes may
  say only `terminal | delegation | upgrade | none`.
- **The provider's `onCreate` does nothing** (no library load, no store
  open): it runs on Home's cold-start path. Its JNI calls pass the `Context`
  (a provider-only process has no activity).
- **Binary on stdout:** exports are a git pack, verified on arrival; any
  other bytes are an error. The `Adb` trait gets a binary method (`&[u8]` in,
  `Vec<u8>` out).
- `dataExtractionRules` exclude everything.

## 6. Approval on the TV

### 6.1 Threat model (measured on the Philips)

The adb shell is in groups `input` and `uhid`; it can create key events that
look exactly like the remote. What it can't do is **see** a `FLAG_SECURE`
window (`screencap`, `screenrecord` show black) *[spike: verify, also
UiAutomation screenshots]*. So the confirmation proves that someone **can
see the TV**, which a blind script, an agent or a remote-control app can't.
SECURITY.md states it this way: it stops accidents and blind automation; a
person looking at the TV with a remote is the person.

### 6.2 The confirmation

1. The review screen sets `FLAG_SECURE` and `setHideOverlayWindows(true)`
   (`HIDE_OVERLAY_WINDOWS`); the code is drawn only in the frame after the
   flag is set.
2. A **random 4-arrow code** (OS random source, `SecureRandom`), shown as
   four separate glyphs in a fixed left-to-right order in every language
   (right-to-left layouts don't mirror it; the arrow keys keep their
   physical meaning). Each arrow pressed turns accent.
3. Then **hold OK for 2 s** (owner's gesture): armed on a fresh key-down
   with repeat count 0, timed key-down to key-up, so an OK held from the
   previous screen can't confirm.
4. A wrong arrow closes the review ("Nothing changed"). **Three wrong codes
   in a row lock TV approvals for 1 hour of TV uptime, then 2, 4, 8 … hours
   for each further lockout** (a correct code resets it), persisted in the
   launcher's files (survives reboots: the remaining time counts down only
   while the TV runs).
5. Nothing about the code reaches logcat or accessibility: the code glyphs
   are not exposed as text *[spike: `uiautomator dump`]*.
6. **Release builds only:** `profileable` moves out of the release manifest
   into an opt-in build property for measurements; a debuggable launcher
   (or one built `profileable`) shows the review as display-only, and the
   CLI refuses its pending approvals.
7. The approval records each changed key's **old value**; applying re-checks
   each one live (not the head id, which a drift entry may have moved).

### 6.3 Pending approvals (for the Mac)

- Created only by the confirm path: `files/history/pending/<hash>.toml` with
  the change list (each with its old value), `boot_count`
  (`Settings.Global.BOOT_COUNT`) and `elapsedRealtime` at approval.
- **Valid for 2 h of TV uptime in the same boot**; the provider decides with
  its own clocks and returns only valid ones, with seconds left.
- **Applied by `rusttv history apply-pending`** in an interactive terminal
  (nothing to type: the approval happened on the TV). `history` commands and
  plan-applying commands mention waiting approvals and the time left; never
  `inspect`, `bench`, `perf` or MCP (they must not start the launcher's
  process). Never applied from non-interactive runs, `sync`, `inspect` or MCP.
- Order: the CLI **consumes it through the door** (`commit consume:<nonce>`; the TV marks it used atomically;
  nothing is applied without `ok`) → builds the `history-restore` plan live
  → applies it only if its steps are exactly the approved list with the same
  old values (otherwise: shows both and asks for typed approval at the
  terminal) → journal `approved_by = { by = "tv", approval }` → a note with
  the outcome.
- Never for takeover packages, never `--allow-untested`, never imported.

## 7. `WRITE_SECURE_SETTINGS`

- Declared in the manifest; granted from the Mac with `rusttv history
  allow-tv-settings`: a plan, one R2 step `pm grant ai.rusttv.launcher
  android.permission.WRITE_SECURE_SETTINGS` (inverse `pm revoke`), typed
  approval `allow`; `rusttv history revoke-tv-settings` revokes. Lost on
  uninstall (`sync` says so).
- **One writer:** `SecureSettings.put(key: SettingKey, value: String)` in
  Kotlin, the three keys as an enum, the value regex; Rust reaches it only
  through one JNI function taking the enum index.
- **Containment tests:**
  - DEX: scan the release APK's method references (build-tools `dexdump`);
    any reference to `Settings$Global|Secure|System.put*`,
    `ContentResolver.insert|update|delete|call|bulkInsert|applyBatch`
    anywhere in the DEX, or reflection (`Class.forName`, `Method.invoke`)
    from `ai.rusttv.launcher.*` (kotlin-stdlib uses reflection itself),
    outside `SecureSettings` fails the check.
  - Rust: no `android/provider/Settings` string outside the one JNI call.
- **Watchlist on every sync** (R0, baseline on the Mac):
  `enabled_accessibility_services`, `default_input_method`,
  `adb_wifi_enabled`, `install_non_market_apps`, `package_verifier_enable`,
  `http_proxy`, `private_dns_specifier`, and the grant itself; any change
  since the last sync is printed as a warning.
- Without the grant, settings approved on the TV become pending approvals.

## 8. UI — the same app

Reuse only what exists: `Theme` (bg `#0E1014`, surface `#1C1F26`, text
`#F2F3F5`, dim `#A4A9B4`, the one accent `#F5A524`); type sizes 57u (page
title), 29u (panel title), 27u (rows), 25u (dim text); 90u page margins;
the 1110u surface panel with 16u radius (About); the long-press menu's row
component (64u rows, light pill with dark text, 10u radius, 120 ms colour
change); the update dot; `Dir` mirroring; `@tr` in all 21 languages. No new
colours, fonts, shadows or effects.

**Motion** (emil-design-eng): nothing new moves on D-pad presses; pages
appear at once like About and Hidden apps (*As built*: neither fades; a
page opened by a key doesn't animate); the one deliberate animation
is the hold: the track fills **2 s linear while held, back in 200 ms
ease-out on release** (slow where the person decides, fast where the system
answers).

### 8.1 Entry
A **Settings** tile next to About, in About's quiet style. *As built:* after
About; the two stay together at the end, and new apps go before them.

### 8.2 Page
Hidden-apps header: "Settings" (57u) at 90u/90u, one dim hint line. Below,
two columns: left, the sections as menu rows — **Animations, Updates,
History, Waiting for your computer** (only when something waits); right, the
surface panel showing the selected section. Up/Down move; Right or OK enter
the panel; Left or Back return; Back on the list leaves Settings.

### 8.3 Sections
- **Animations:** seven rows, Off (0×), Fastest (0.25×), Faster (0.5×),
  Fast (0.75×), Normal (1×), Slower (1.5×), Slowest (2×); the current one has the
  update-dot accent dot. "Off" adds the dim line "Some apps may look odd
  without animations." Without the grant, a dim line "To change this on the
  TV: `rusttv history allow-tv-settings` on your computer"; choosing still
  works and becomes a pending approval.
- **Updates:** "Check for updates automatically", On / Off.
- **History:** newest first; each row: summary (27u), then time · who (dim
  25u: "This TV", "Your computer", "Changed outside RustTV", "Restored from
  your computer"). OK opens it: changes old → new, and **Restore this
  state**.
- **Waiting for your computer:** each approval, what it does, time left:
  "On your computer, run `rusttv history apply-pending` within 1 h 40 min."

### 8.4 States
- Empty history: "No changes yet. Changes made here or from your computer
  appear here."
- Loading on a cold start: dim "Reading history…", no spinner.
- Store unreadable: what failed; "Your computer's copy can put it back."
- Newer format: "Your computer has a newer RustTV. Update RustTV on this TV."
- Grant missing after a reinstall: the Animations line above.
- Busy (a change in progress from the TV or the computer): "Busy — try
  again in a moment."
- Approvals locked: "Too many wrong codes. Approvals are locked for 52 min."
- Debug or profiling build: "Approvals are off in test builds."
- Rollback failed: which setting, its value now, and "Fix it with `rusttv
  history restore --journal …` or in Android Settings."
- Pending expired: removed from the list, recorded in History.

### 8.5 Review and confirm
Hidden-apps header "Review changes"; the change list in two groups (§ 4.1);
each line `name: old → new` with the package id shown and bidi controls and
line breaks stripped from labels. Up/Down scroll the list; its last row is
**Approve**. OK on it replaces the list's focus with the confirm block, and
only then do arrows count as the code: "Press" + four arrow glyphs (fixed
order) + "then hold OK"; a 4u track under "Hold OK to apply". Back cancels
at any time. The result replaces the block in one line: "Applied",
"Saved for your computer", or "Nothing changed: …".

## 9. Performance budgets

History never runs on Home's hot paths: start-up to first frame, resume, key
handling and frames do no history I/O, gix or history JNI. Measured on the
Philips (BENCHMARK_PLAN.md) after each build step; a regression beyond
run-to-run noise blocks the step.

| Path | Budget |
|---|---|
| Cold start, resume, key → frame | no change beyond noise |
| Idle native heap / PSS | ≤ +1.5 MB (store opened on first use) |
| Commit after a confirmed change | off the UI thread |
| Packing | background, only while paused, never delays resume |
| History page | first rows ≤ 100 ms (walk: 19 ms per 1,000 packed entries) |
| APK | ≤ +1.5 MB. Measured (armeabi-v7a): +1.60 MB APK, +1.52 MB native library, of which gix and its pack writer +1.17 MB; slightly over, accepted for v1 (`benchmarks/raw/philips-65oled818/2026-10-24/history/`) |
| Provider-only process (Mac call, launcher not running) | measured; never starts the UI |

## 10. Build order

1. **Spike leftovers (Philips):** `FLAG_SECURE` blanks `screencap`,
   `screenrecord` and UiAutomation screenshots; `uiautomator dump` doesn't
   show the code; `content read/write/call` reach a provider as uid 2000 and
   the provider-only cold start time; `BOOT_COUNT` readable; whether a
   `FLAG_SECURE` toggle recreates the native surface.
2. **`rusttv-history` crate:** snapshot model, gix store (commit, log, show,
   diff, pack), pack export/import with limits and tree checks, formats;
   property tests for corrupt and truncated packs.
3. **Launcher:** lazy store, `tv` entries for move/hide/unhide, packing,
   the provider (`checkShell`, export, pending, inbox, commit), the Settings
   tile and page with History (read-only). Measure (§ 9).
4. **CLI:** `history sync | log | show | diff`, Mac repository,
   `devices.toml`, observations and notes, class RL, error codes, JSON
   schema bump. End to end on the emulator, then the Philips.
5. **Changes on the TV:** review + code + hold, lockout, TV journal and
   recovery, `SecureSettings` + containment tests, `allow-/revoke-tv-
   settings`, watchlist, pending approvals, `apply-pending`, `ApprovedBy::Tv`.
6. **Restore:** TV "Restore this state", `history-restore` (+ its journal
   restore), `restore-tv`.
7. **Docs:** SECURITY.md (owner decisions, door, class RL, confirmation
   threat model, pending contract, containment), CLI.md, ARCHITECTURE.md,
   RISKS.md (input injection, uid 2000), CHANGELOG, ROADMAP. The owner tests
   the code + hold on the Philips with the real remote.

## Spike results (Philips 65OLED818, 2026-10-24)

One-off spike runs to choose the design; their raw data was not kept, so
they are not performance claims (BENCHMARK_PLAN.md). The measured results of
the finished build are in `benchmarks/raw/philips-65oled818/2026-10-24/history/`.

- `gix` 0.90 (`sha1` only), armv7 release: **+1.09 MB** library.
- 1,000 commits of a ~5 KB `state.toml`: **1.9 ms** per commit (first
  4–8 ms), log walk **82 ms** loose, peak RSS **+1.3 MB**; **5.6 MB** loose.
- **Packing** (gix-pack generator + `Bundle::write_to_directory`, one
  thread): 256 entries 3 MB → **188 KB** in **0.7 s**; 1,000 entries (3,000
  objects, every state distinct) 12 MB → **720 KB** in **3.0 s**; walk
  **19 ms** packed; peak RSS 5.9 MB during packing; ~80 KB more code.
- Input: the shell (uid 2000) is in `input` and `uhid`; `/dev/input/event*`
  `root:input rw`, `/dev/uinput` `uhid rw`. Devices: `TPV_SMTRC`,
  `TPV_MutilRC`, `TPV_LKB`, `MTK PMU IR/KEYPAD`, `virtual-remote`,
  `virtual-search` (Google uinput), −1 (`input` command).
- `content read/write --uri` exist (stdin/stdout); `WRITE_SECURE_SETTINGS`
  has the `development` flag, so `pm grant` works.

## Review log

- **Round 1** (security; engineering): takeover packages out; Mac-only
  drift; inbox + `content call`; one writer with the provider in-process;
  `history-restore` plan kind; git hashes over stored bytes; formats;
  translated summaries; chain order; `ro.serialno`; empty-only imports
  without pending; pending contract; debug builds can't approve; TV journal
  first; new risk class; every provider entry gated; label spoofing;
  `dataExtractionRules`; no MCP history. Not adopted by owner decision:
  dropping gix, dropping `WRITE_SECURE_SETTINGS`. Measurement replaced
  "physical remote only" with `FLAG_SECURE` + a code.
- **Round 2** (combined): 4-arrow code from `SecureRandom`, lockout in
  uptime across reboots, code drawn after the flag, overlays hidden, no
  accessibility exposure, `profileable` out of release, fixed arrow order in
  RTL; pending consumed through the door before applying, bound to old
  values, extra fields and the Mac ledger cut; the Mac never writes launcher
  state; observations without `[launcher]`; separate door cap; RL placed
  between R0 and R1; no history rewriting; limits by pack size; recovery in
  any process, `TV_BUSY`, undo only unchanged values, absent ≡ "1",
  `rollback_failed`; DEX-level containment test and a Mac-side watchlist
  baseline; a journal restore for `history-restore`; which TV edits need the
  code; untested packages flagged in review; UI fade 250 ms, menu row reuse,
  all states listed. Kept by owner decision: hold OK after the code. Changed
  wording: "next time it connects" → `apply-pending` within 2 h with
  reminders (silent application was rejected by both reviews).
- **Round 3** (combined, verdict "ready to implement"): `launcher
  uninstall` syncs first and nothing is recorded into an empty TV history
  while the Mac holds one; an export header line with the history id, head
  and parts, `HISTORY_NOT_FOUND`; no `catch_unwind` (Android aborts on
  panic): no panics on untrusted input, property-tested; inbox files are
  real files; one history thread owns the store (read → build → commit in
  one job); the TV-edit cap defers instead of refusing; arrows scroll until
  **Approve**; pending reminders only in `history` and plan commands;
  DEX reflection rule only for our classes; recovery flag and no
  `order.toml` writes while a TV journal exists; notes can't claim a TV
  approval; doubling lockouts. Cut: the TV's untested-package flag (the
  Mac refuses those), live package values on the TV (shown as last seen),
  the CLI re-hashing the list, "irreversible, last" on the RL import.

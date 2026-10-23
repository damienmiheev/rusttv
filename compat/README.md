# Device compatibility profiles

One TOML file per device model and firmware family in `devices/`, named
`<manufacturer>-<codename>.toml`. Profiles record what a device is and what
was tested on it; they will later also record which optimizations are known
to be safe (Phase 4).

## Adding your TV

1. Enable Developer options and network debugging on the TV.
2. Connect: `rusttv device connect <tv-ip>` and accept the prompt on the TV.
3. Generate the profile (read-only):

   ```bash
   rusttv compat profile > compat/devices/<suggested-name>.toml
   ```

   The suggested file name is printed on stderr.
4. Describe in `tested.notes` what you tried and anything unusual.
5. Run `cargo test -p rusttv-core` and open a pull request.

## Home takeover

`rusttv launcher takeover` is refused unless a profile matching the device's
model and firmware family has:

```toml
[launcher]
stock_package = "com.google.android.apps.tv.launcherx"
takeover_tested = true
```

If another system app also handles Home with a higher priority than RustTV
(e.g. `com.google.android.tungsten.setupwraith`, the setup wizard, on some
Google TV models), Android would open it instead once the stock launcher is
disabled, and the takeover is refused. After checking what else that app does,
list its package to have the takeover disable it first (the adb shell may not
disable a single activity of a system app):

```toml
disable_home_packages = ["com.google.android.tungsten.setupwraith"]
```

Set `takeover_tested = true` only after `rusttv launcher takeover` **and**
`rusttv launcher restore` both succeeded on that device and firmware. Profiles
are compiled into `rusttv`, so add new ones to `BUNDLED_PROFILES` in
`crates/rusttv-core/src/compat.rs` too (a test checks). Emulator profiles are
never marked tested.

## Privacy profiles

`rusttv optimize apply privacy` disables (never uninstalls) the packages a
profile lists, and is refused unless the profile matching the device's model
and firmware family has `[privacy] tested = true` (OPTIMIZER.md § Privacy
profiles). Add one entry per package, in the order they should be disabled,
telemetry first, then ads, then extras:

```toml
[privacy]
tested = false

[[privacy.packages]]
package = "org.droidtv.candeebug"
category = "telemetry"          # telemetry | ads | extras
what = "TP Vision service that collects analytics; ..."
side_effects = "none expected"  # what stops working while it is disabled
checked = ""                    # date it was disabled and lived with, once it was
```

- Write `what` and `side_effects` from what you checked on the device
  yourself (`rusttv apps list`, `dumpsys package <name>`), in your own words;
  don't copy other projects' lists.
- Never list a package that handles Home, is a keyboard or accessibility
  service, or is on `compat/protected.toml`: the plan is refused
  (`PACKAGE_PROTECTED`). A test fails if a bundled profile lists one from
  `protected.toml`.
- Set `tested = true` only after `rusttv optimize apply privacy` on that
  model and firmware, a reboot, normal use (Home, HDMI inputs, apps, remote
  buttons, Settings) and `rusttv optimize restore` all worked; then fill in
  each `checked` date. Until then, testing needs a development build with
  `tested = true` edited locally, as for takeover.

`compat/protected.toml` is the static list of packages RustTV never disables
(SECURITY.md § Protected package policy, rule 3). It is compiled into
`rusttv` too; add to it freely, and remove an entry only with a reason.

## Format

The Rust types in `crates/rusttv-core/src/compat.rs` are the schema: unknown
fields are rejected, and CI loads every profile in `devices/`. The format is
versioned with `schema_version`.

## Privacy

Profiles contain only model-level data that is identical on every unit with
the same firmware. Never add serial numbers, MAC or IP addresses, account
names, or anything else that identifies one particular TV or person.
`rusttv compat profile` doesn't collect such data.

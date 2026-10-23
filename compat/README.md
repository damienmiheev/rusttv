# Device compatibility profiles

One TOML file per device model and firmware family in `devices/`, named
`<manufacturer>-<codename>.toml`. Profiles record what a device is and what
was tested on it (Home takeover, privacy packages). `packages/` is the
package catalog privacy profiles choose from; `protected.toml` is what RustTV
never disables.

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

`rusttv optimize apply privacy` disables (never uninstalls) packages in three
layers (OPTIMIZER.md § Generic design):

1. **The package catalog**, `packages/*.toml`: what each package is, written
   once for every device that has it.
2. **Device profiles**, `devices/*.toml`: which catalog packages were tested
   on that model and firmware family.
3. **Your choices**, a local `privacy.toml` that is never in the repository
   (CLI.md § Privacy choices).

Only tested packages are planned unless the person passes `--allow-untested`.

### Adding a catalog entry

Add the package to the file of whoever ships it (`android.toml` for
AOSP/Google packages, `tp-vision.toml` for `org.droidtv.*`, `sony.toml`,
`streaming-apps.toml` for third-party apps, ...; a new vendor gets a new file,
listed in `BUNDLED_CATALOG` in `crates/rusttv-core/src/catalog.rs`):

```toml
[[packages]]
package = "tv.samba.ssm"
category = "telemetry"
what = "Samba TV: recognises what is on screen for ad targeting"
side_effects = "reported to cause problems when disabled on some Sony TVs"
sources = ["rusttv: seen on a Philips 65OLED818, 2026-10-23"]
```

- `category`: `telemetry`, `ads` or `extras` for things nobody needs (on by
  default), `casting`, `smart-home`, `voice` or `streaming-apps` for features
  some people use (planned only if chosen). When unsure, pick an opt-in
  category, or leave the package out.
- `what` and `side_effects` in plain words; `side_effects` says what stops
  working ("the remote's mic button stops working"), or `none reported`.
- `sources`: where the facts come from: a model you checked it on, or another
  project (with its licence: data from cleanyourtv is MIT, and a file that
  uses it carries the notice at the top, see `licenses/cleanyourtv/`).
- Never add a package that is the Home screen, a TV input, tuner, HDMI,
  remote-pairing, firmware-update or OTA package, a keyboard or accessibility
  service, or anything on `protected.toml`; nothing whose known side effect
  breaks HDMI, inputs, the remote, the network or updates. A test fails if a
  catalog entry is on `protected.toml` or listed twice; the live checks
  refuse the whole plan (`PACKAGE_PROTECTED`) if a chosen one handles Home or
  is a keyboard or accessibility service on the device.

### Adding your model's tested list

1. Run `rusttv optimize analyze` and choose what to disable (your
   `privacy.toml`; `keep` anything you use).
2. `rusttv optimize apply privacy --allow-untested --dry-run`, read every
   UNTESTED note, then run it without `--dry-run`.
3. Use the TV for a while: Home, every HDMI input, apps, the remote's
   buttons (including voice), Settings; reboot and check again.
4. `rusttv optimize restore`, and check everything came back.
5. In your device profile, list each package that passed:

   ```toml
   [[privacy.tested]]
   package = "org.droidtv.candeebug"   # must be in the catalog
   checked = "2026-10-23"              # date you tested it, YYYY-MM-DD
   ```

   Within a category, packages are disabled in this order. Add catalog entries
   for packages you learned about (with `sources`) in the same pull request.

Schema 3 profiles (a `[privacy]` section with `tested = true` and
`[[privacy.packages]]` descriptions) still load: each package with a
`checked` date counts as tested; move the descriptions to the catalog.

`protected.toml` is the static list of packages RustTV never disables
(SECURITY.md § Protected package policy, rule 3). It is compiled into
`rusttv` too; add to it freely, and remove an entry only with a reason.

## Format

The Rust types in `crates/rusttv-core/src/compat.rs` (profiles) and
`catalog.rs` (catalog) are the schema: unknown fields are rejected, and CI
loads every file in `devices/` and `packages/`. Both formats are versioned
with `schema_version`.

## Privacy

Profiles contain only model-level data that is identical on every unit with
the same firmware. Never add serial numbers, MAC or IP addresses, account
names, or anything else that identifies one particular TV or person.
`rusttv compat profile` doesn't collect such data.

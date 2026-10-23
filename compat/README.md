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

## Format

The Rust types in `crates/rusttv-core/src/compat.rs` are the schema: unknown
fields are rejected, and CI loads every profile in `devices/`. The format is
versioned with `schema_version`.

## Privacy

Profiles contain only model-level data that is identical on every unit with
the same firmware. Never add serial numbers, MAC or IP addresses, account
names, or anything else that identifies one particular TV or person.
`rusttv compat profile` doesn't collect such data.

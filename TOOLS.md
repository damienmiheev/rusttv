# Experimental tools

> **Draft.** The launcher side (the trigger activity) is built; the `rusttv tools`
> commands described here are not implemented yet and nothing has run on a TV.

Some TVs ship features that are installed but turned off. `rusttv tools` turns
selected ones on, **experimentally**, only on models where RustTV knows exactly
what is installed. They are opt-in, never part of an optimization profile, and
go through the same safety engine as every other change (SECURITY.md § Change
lifecycle): plan, dry run, typed confirmation at a terminal, journal, apply,
verify, restore.

What these tools are not:

- **Not certified.** A feature the TV maker shipped disabled was not released
  for that model. It may be incomplete, behave differently from certified
  devices, or stop working after a firmware update.
- **Not permanent.** Each tool changes runtime state that the TV's own
  software resets at boot. After a restart the feature is off again.
- **Vendor software only.** RustTV starts the vendor's own, already installed
  software through the permissions that software declares. It patches nothing,
  installs no third-party implementation, does not change the TV maker's
  product configuration, and touches no DRM or system partition. No root.
  Whether a feature the maker switched off may be used is the TV owner's
  decision; RustTV only offers it behind a typed confirmation.

## AirPlay (MediaTek)

Some Philips Google TV models built on MediaTek's 2023 platform carry
MediaTek's AirPlay receiver (system service, daemon and TV input) installed but
switched off by the TV maker's settings app at every boot. AirPlay is Apple's
technology; Apple has not certified these models as AirPlay receivers.

| | |
|---|---|
| Commands | `rusttv tools airplay status`, `enable`, `disable` (CLI.md) |
| Tested on | Philips 65OLED818 (not yet) |
| Lasts | Until the TV restarts |
| Needs | The RustTV launcher installed (its trigger activity starts the vendor service) |

**Gate.** `enable` is refused unless the device's bundled compat profile allows
it, or all three MediaTek AirPlay packages are installed
(`com.mediatek.partner.airplay_sys`, `com.mediatek.airplaydaemon`,
`com.mediatek.AirplayAPK`). Every run prints the caveat above once.

**What `enable` does.**

1. Records the vendor flag `setup_airplay_enable` (Global settings), then sets
   it to 1 (on). The TV writes 7 (hidden) at boot.
2. Starts MediaTek's AirPlay bootup service through RustTV's trigger activity,
   `ai.rusttv.launcher/.tools.HiddenFeatureActivity`. The service requires a
   permission that the adb shell lacks but any app can declare; the trigger
   activity declares it, and only the adb shell or the system can start the
   activity (it requires `android.permission.DUMP`), never another app.
3. Verifies for up to about 20 s that the service runs.

**What `status` shows** (read-only): the flag, whether the service runs, the
service's own "setup UI" flag, recent log lines of the AirPlay components,
listening TCP ports, and with `--check-bonjour` whether this computer sees the
TV advertise `_airplay._tcp` (`dns-sd` on macOS, `avahi-browse` on Linux;
skipped with a note when neither is installed).

**Restore.** `rusttv tools airplay disable` sets the flag back to the value the
journal recorded and stops the service. Restarting the TV has the same effect.

**Possible later step.** Keeping AirPlay on across restarts would need RustTV's
launcher to re-apply the flag at boot, which needs
`pm grant ai.rusttv.launcher android.permission.WRITE_SECURE_SETTINGS`: a
persistent grant this project does not make without an explicit decision.

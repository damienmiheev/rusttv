# SYNTHETIC fixtures: experimental AirPlay tool

**None of these files were captured from a device.** They are hand-written
stand-ins so `rusttv tools airplay status|enable|disable` (TOOLS.md) can be
tested against `FakeAdb` before it runs on a TV. Replace each with a real
capture, and update this table, when one is taken. Real captures must keep
out device names, IP and MAC addresses, and serials.

They are meant to be used with the real Philips 65OLED818 captures in
`../philips-65oled818-api34/` (`getprop.txt`), so the bundled profile
`compat/devices/tpv-ph1m-ww-9972.toml` matches.

| File | Command | Stands for | Based on |
|---|---|---|---|
| `packages-f.txt` | `pm list packages -f` | The three MediaTek AirPlay packages installed, a few system packages and the RustTV launcher; APK paths made up | Format of `../emulator-pixel3a-api34/packages-f.txt` |
| `packages-f-no-airplayapk.txt` | `pm list packages -f` | Same without `com.mediatek.AirplayAPK`: the capability gate must refuse | — |
| `setting-setup-airplay-enable-7.txt` | `settings get global setup_airplay_enable` | Vendor value "hidden" written at every boot | Value seen on the Philips 2026-10-23 |
| `setting-setup-airplay-enable-1.txt` | same | "on" after `enable` | MediaTek's values: 0 off, 1 on, 7 hidden |
| `setting-show-airplay-setting-ui-null.txt` | `settings get global show_airplay_setting_ui` | Unset: the service never ran | `settings get` prints `null` for an unset key |
| `setting-show-airplay-setting-ui-1.txt` | same | Set by the service once it ran | Expected from the vendor service; not seen yet |
| `services-not-running.txt` | `dumpsys activity services com.mediatek.partner.airplay_sys` | No service of the package running | AOSP `ActiveServices` dump format |
| `services-running.txt` | same | `BootupService` started | AOSP `ServiceRecord` dump format; values made up |
| `am-start-trigger.txt` | `am start -n ai.rusttv.launcher/.tools.HiddenFeatureActivity --es feature airplay --es action start` | Trigger activity started | AOSP `am start` output |
| `am-force-stop.txt` | `am force-stop com.mediatek.partner.airplay_sys` | Prints nothing | AOSP `am force-stop` |
| `logcat-not-ready.txt` | `logcat -d -t <n>`, filtered to the AirPlay tags | Engine up, setup never done | Tag prefixes `AirPlaySystemService`, `AirPlayDaemon`, `AirPlayTIS` and the "Luna is not ready" message seen on the Philips; exact tags and lines made up |
| `logcat-started.txt` | same | RustTV's trigger log line, then the service and daemon starting | Trigger line as `HiddenFeatureActivity` logs it; vendor lines made up |
| `ss-ltnp-before.txt` | `ss -ltnp` | Listening ports before: adb, a cast port | iproute2 `ss` format; ports illustrative |
| `ss-ltnp-after.txt` | same | Plus AirPlay's usual ports 7000, 7100 and one RAOP port | Ports commonly used by AirPlay receivers; not seen on this TV yet |
| `dns-sd-airplay.txt` | `dns-sd -B _airplay._tcp local.` (macOS, stopped after a timeout) | The TV advertises AirPlay; instance name made up | macOS `dns-sd` browse output format |
| `dns-sd-none.txt` | same | Nothing found before the timeout | — |
| `avahi-browse-airplay.txt` | `avahi-browse -t -p _airplay._tcp` (Linux) | The TV advertises AirPlay | avahi `--parsable` format |

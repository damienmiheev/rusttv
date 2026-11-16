//! The device's settings on Linux (device.rs), through the system's own
//! tools, each only where it is installed: `wpctl` (PipeWire/WirePlumber)
//! for audio, `nmcli` (NetworkManager) for networks, `bluetoothctl` for
//! Bluetooth and `systemctl` to restart or power off. Facts for About come
//! from `/etc/os-release`, `/proc` and `/sys`.
//!
//! Every method blocks on a tool (at most [`TOOL_TIMEOUT`]); the launcher
//! calls them on worker threads only.

use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::program_exists;
use crate::device::{
    Control, ControlKind, DeviceSettings, Link, QuickPanel, Tab, Value, Volume, VolumeChange,
};

/// How long a tool may take before it is stopped. Listing Wi-Fi networks
/// can scan for a few seconds.
const TOOL_TIMEOUT: Duration = Duration::from_secs(15);

/// The default output, as wpctl names it.
const SINK: &str = "@DEFAULT_AUDIO_SINK@";

/// How much one volume key changes the volume, in percent.
const VOLUME_STEP: u32 = 5;

/// The most Wi-Fi networks a panel lists (strongest first).
const MAX_NETWORKS: usize = 8;

/// Control ids of Wi-Fi networks are this prefix and the SSID.
const CONNECT: &str = "connect:";

/// [`DeviceSettings`] through the tools found in `PATH` at start.
#[derive(Debug, Clone, Default)]
pub struct LinuxDevice {
    wpctl: bool,
    nmcli: bool,
    bluetoothctl: bool,
    systemctl: bool,
}

impl LinuxDevice {
    /// Looks for the tools in `PATH`.
    pub fn detect() -> Self {
        Self {
            wpctl: program_exists("wpctl"),
            nmcli: program_exists("nmcli"),
            bluetoothctl: program_exists("bluetoothctl"),
            systemctl: program_exists("systemctl"),
        }
    }

    /// Whether this panel can be shown (its tool is installed).
    pub(super) fn has(&self, panel: QuickPanel) -> bool {
        match panel {
            QuickPanel::Audio => self.wpctl,
            QuickPanel::Network => self.nmcli || self.bluetoothctl,
            QuickPanel::Power => self.systemctl,
        }
    }

    fn audio(&self) -> Option<Tab> {
        if !self.wpctl {
            return None;
        }
        let volume = run("wpctl", &["get-volume", SINK])
            .ok()
            .and_then(|text| parse_volume(&text));
        let mut controls = Vec::new();
        if let Some(volume) = volume {
            controls.push(control(
                "volume",
                "Volume",
                "",
                ControlKind::Slider {
                    value: percent(volume.level),
                    min: 0,
                    max: 100,
                    step: VOLUME_STEP as i32,
                },
            ));
            controls.push(control(
                "mute",
                "Mute",
                "",
                ControlKind::Toggle { on: volume.muted },
            ));
        }
        let sinks = run("wpctl", &["status"])
            .map(|text| parse_sinks(&text))
            .unwrap_or_default();
        if !sinks.is_empty() {
            let selected = sinks.iter().position(|sink| sink.default).unwrap_or(0);
            controls.push(control(
                "output",
                "Output",
                "",
                ControlKind::Choice {
                    options: sinks.into_iter().map(|sink| sink.name).collect(),
                    selected,
                },
            ));
        }
        if controls.is_empty() {
            controls.push(control(
                "none",
                "No sound output",
                "PipeWire is not running",
                ControlKind::Info,
            ));
        }
        Some(tab(QuickPanel::Audio, "Audio", controls))
    }

    fn network(&self) -> Option<Tab> {
        if !self.has(QuickPanel::Network) {
            return None;
        }
        let mut controls = Vec::new();
        if self.nmcli {
            if let Ok(devices) = run("nmcli", &["-t", "-f", "TYPE,STATE,CONNECTION", "device"]) {
                for fields in devices.lines().map(split_terse) {
                    if let [kind, state, connection] = fields.as_slice()
                        && kind == "ethernet"
                        && state == "connected"
                    {
                        controls.push(control(
                            "ethernet",
                            "Ethernet",
                            connection,
                            ControlKind::Info,
                        ));
                    }
                }
            }
            let wifi = run("nmcli", &["-t", "-f", "WIFI", "radio"])
                .is_ok_and(|text| text.trim() == "enabled");
            controls.push(control(
                "wifi",
                "Wi-Fi",
                "",
                ControlKind::Toggle { on: wifi },
            ));
            if wifi {
                let networks = run(
                    "nmcli",
                    &[
                        "-t",
                        "-f",
                        "IN-USE,SSID,SIGNAL,SECURITY",
                        "device",
                        "wifi",
                        "list",
                    ],
                )
                .map(|text| parse_networks(&text))
                .unwrap_or_default();
                let current = networks.iter().find(|network| network.in_use);
                controls.push(control(
                    "current",
                    "Connected to",
                    current.map_or("Not connected", |network| network.ssid.as_str()),
                    ControlKind::Info,
                ));
                for network in networks.iter().filter(|network| !network.in_use) {
                    let security = if network.security.is_empty() {
                        "open".to_owned()
                    } else {
                        network.security.clone()
                    };
                    controls.push(control(
                        &format!("{CONNECT}{}", network.ssid),
                        &network.ssid,
                        &format!("{} % · {security}", network.signal),
                        ControlKind::Action,
                    ));
                }
            }
        }
        if self.bluetoothctl
            && let Ok(text) = run("bluetoothctl", &["show"])
            && let Some(on) = parse_powered(&text)
        {
            controls.push(control(
                "bluetooth",
                "Bluetooth",
                "",
                ControlKind::Toggle { on },
            ));
        }
        Some(tab(QuickPanel::Network, "Network", controls))
    }

    fn power(&self) -> Option<Tab> {
        self.systemctl.then(|| {
            tab(
                QuickPanel::Power,
                "Power",
                vec![
                    control("reboot", "Restart", "", ControlKind::Action),
                    control("poweroff", "Power off", "", ControlKind::Action),
                ],
            )
        })
    }
}

impl DeviceSettings for LinuxDevice {
    /// Sound and Network, as in their quick panels.
    fn tabs(&self) -> Vec<Tab> {
        let mut tabs = Vec::new();
        if let Some(mut audio) = self.audio() {
            "Sound".clone_into(&mut audio.title);
            tabs.push(audio);
        }
        tabs.extend(self.network());
        tabs
    }

    fn panel(&self, panel: QuickPanel) -> Option<Tab> {
        match panel {
            QuickPanel::Audio => self.audio(),
            QuickPanel::Network => self.network(),
            QuickPanel::Power => self.power(),
        }
    }

    fn set(&self, _tab: &str, control: &str, value: Value) -> Result<(), String> {
        let on_off = |on: bool| if on { "on" } else { "off" };
        match (control, value) {
            ("volume", Value::Number(level)) => run(
                "wpctl",
                &["set-volume", SINK, &format!("{}%", level.clamp(0, 100))],
            ),
            ("mute", Value::Toggle(on)) => {
                run("wpctl", &["set-mute", SINK, if on { "1" } else { "0" }])
            }
            ("output", Value::Choice(index)) => {
                let sinks = parse_sinks(&run("wpctl", &["status"])?);
                let sink = sinks.get(index).ok_or("that output is gone")?;
                run("wpctl", &["set-default", &sink.id.to_string()])
            }
            ("wifi", Value::Toggle(on)) => run("nmcli", &["radio", "wifi", on_off(on)]),
            ("bluetooth", Value::Toggle(on)) => run("bluetoothctl", &["power", on_off(on)]),
            (control, Value::Action) if control.starts_with(CONNECT) => {
                let ssid = &control[CONNECT.len()..];
                // A saved network first; else a new one (open networks
                // only: there is no password entry yet).
                run("nmcli", &["connection", "up", "id", ssid])
                    .or_else(|_| run("nmcli", &["device", "wifi", "connect", ssid]))
            }
            ("reboot", Value::Action) => run("systemctl", &["reboot"]),
            ("poweroff", Value::Action) => run("systemctl", &["poweroff"]),
            (control, value) => Err(format!("unknown setting {control} = {value:?}")),
        }
        .map(drop)
    }

    fn about(&self) -> Vec<(String, String)> {
        let read = |path: &str| std::fs::read_to_string(path).ok();
        let mut facts = Vec::new();
        if let Some(name) = read("/etc/os-release")
            .or_else(|| read("/usr/lib/os-release"))
            .and_then(|text| os_name(&text))
        {
            facts.push(("Operating system".to_owned(), name));
        }
        if let Some(kernel) = read("/proc/sys/kernel/osrelease") {
            facts.push(("Linux kernel".to_owned(), kernel.trim().to_owned()));
        }
        if let Some(model) = read("/proc/device-tree/model")
            .or_else(|| read("/sys/class/dmi/id/product_name"))
            .map(|text| {
                text.trim_matches(|c: char| c == '\0' || c.is_whitespace())
                    .to_owned()
            })
            .filter(|model| !model.is_empty())
        {
            facts.push(("Device".to_owned(), model));
        }
        if let Some(memory) = read("/proc/meminfo").and_then(|text| memory_total(&text)) {
            facts.push(("Memory".to_owned(), memory));
        }
        facts
    }

    fn volume(&self, change: VolumeChange) -> Option<Volume> {
        if !self.wpctl {
            return None;
        }
        let step = format!("{VOLUME_STEP}%");
        let changed = match change {
            VolumeChange::Up => run(
                "wpctl",
                &["set-volume", "-l", "1.0", SINK, &format!("{step}+")],
            ),
            VolumeChange::Down => run("wpctl", &["set-volume", SINK, &format!("{step}-")]),
            VolumeChange::Mute => run("wpctl", &["set-mute", SINK, "toggle"]),
        };
        if let Err(error) = changed {
            log::warn!("changing the volume failed: {error}");
        }
        run("wpctl", &["get-volume", SINK])
            .ok()
            .and_then(|text| parse_volume(&text))
    }

    /// NetworkManager's Wi-Fi and Ethernet interfaces; the signal from its
    /// last scan (no new scan).
    fn links(&self) -> Vec<Link> {
        if !self.nmcli {
            return Vec::new();
        }
        let Ok(devices) = run("nmcli", &["-t", "-f", "DEVICE,TYPE,STATE", "device"]) else {
            return Vec::new();
        };
        let signals = run(
            "nmcli",
            &[
                "-t",
                "-f",
                "IN-USE,SIGNAL,DEVICE",
                "device",
                "wifi",
                "list",
                "--rescan",
                "no",
            ],
        )
        .unwrap_or_default();
        parse_links(&devices, &signals)
    }
}

/// The interfaces from `nmcli -t -f DEVICE,TYPE,STATE device`, with the
/// signal of each Wi-Fi's network in use from
/// `nmcli -t -f IN-USE,SIGNAL,DEVICE device wifi list`.
fn parse_links(devices: &str, signals: &str) -> Vec<Link> {
    let signal = |device: &str| {
        signals
            .lines()
            .map(split_terse)
            .find_map(|fields| match fields.as_slice() {
                [in_use, signal, name] if in_use.trim() == "*" && name == device => {
                    signal.parse::<u8>().ok()
                }
                _ => None,
            })
    };
    devices
        .lines()
        .map(split_terse)
        .filter_map(|fields| {
            let [device, kind, state] = fields.as_slice() else {
                return None;
            };
            let up = state.starts_with("connected");
            match kind.as_str() {
                _ if state == "unmanaged" => None,
                "wifi" => Some(Link::Wifi(up.then(|| {
                    // 1..4 bars, as most systems draw them; 4 when unknown.
                    signal(device).map_or(4, |signal| signal.div_ceil(25).clamp(1, 4))
                }))),
                "ethernet" => Some(Link::Wired(up)),
                _ => None,
            }
        })
        .collect()
}

fn tab(panel: QuickPanel, title: &str, controls: Vec<Control>) -> Tab {
    Tab {
        id: panel.id().to_owned(),
        title: title.to_owned(),
        controls,
    }
}

fn control(id: &str, label: &str, detail: &str, kind: ControlKind) -> Control {
    Control {
        id: id.to_owned(),
        label: label.to_owned(),
        detail: detail.to_owned(),
        kind,
    }
}

/// 0..1 as a whole percent.
fn percent(level: f32) -> i32 {
    // A volume is a small number; the cast cannot overflow.
    (level * 100.0).round() as i32
}

/// Runs a tool and returns its output; stops it after [`TOOL_TIMEOUT`].
/// `Err` is the tool's error message or why it didn't run.
fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    let deadline = Instant::now() + TOOL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{program} took too long"));
            }
            Err(error) => return Err(format!("{program}: {error}")),
        }
    };
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut output);
    }
    if status.success() {
        Ok(output)
    } else {
        let mut error = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut error);
        }
        let error = error.trim();
        Err(if error.is_empty() {
            format!("{program} failed ({status})")
        } else {
            error.lines().next().unwrap_or(error).to_owned()
        })
    }
}

/// `wpctl get-volume`: "Volume: 0.40" or "Volume: 0.40 [MUTED]".
fn parse_volume(text: &str) -> Option<Volume> {
    let rest = text.trim().strip_prefix("Volume:")?.trim();
    let level = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some(Volume {
        level,
        muted: rest.contains("[MUTED]"),
    })
}

/// An audio output in `wpctl status`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sink {
    id: u32,
    name: String,
    default: bool,
}

/// The "Sinks:" of the "Audio" section of `wpctl status`, e.g.
/// ` │  *   48. Built-in Audio Analog Stereo [vol: 0.40]`.
fn parse_sinks(text: &str) -> Vec<Sink> {
    let mut sinks = Vec::new();
    let mut in_audio = false;
    let mut in_sinks = false;
    for line in text.lines() {
        if !line.starts_with([' ', '│', '├', '└']) {
            in_audio = line.trim() == "Audio";
            in_sinks = false;
            continue;
        }
        let item = line.trim_start_matches([' ', '│', '├', '└', '─']);
        if item.ends_with(':') {
            in_sinks = in_audio && item == "Sinks:";
            continue;
        }
        if !in_sinks {
            continue;
        }
        let (default, item) = match item.strip_prefix('*') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, item),
        };
        let Some((id, name)) = item.split_once(". ") else {
            continue;
        };
        let Ok(id) = id.trim().parse() else {
            continue;
        };
        let name = name.split(" [vol:").next().unwrap_or(name).trim();
        sinks.push(Sink {
            id,
            name: name.to_owned(),
            default,
        });
    }
    sinks
}

/// Splits a line of nmcli's terse output (`-t`) at unescaped colons.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let (Some(next), Some(field)) = (chars.next(), fields.last_mut()) {
                    field.push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => {
                if let Some(field) = fields.last_mut() {
                    field.push(c);
                }
            }
        }
    }
    fields
}

/// A Wi-Fi network from `nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY device wifi list`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Network {
    in_use: bool,
    ssid: String,
    signal: u8,
    security: String,
}

/// The networks with a name, each SSID once (its strongest), strongest
/// first, at most [`MAX_NETWORKS`] besides the one in use.
fn parse_networks(text: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for fields in text.lines().map(split_terse) {
        let [in_use, ssid, signal, security] = fields.as_slice() else {
            continue;
        };
        if ssid.is_empty() {
            continue;
        }
        let network = Network {
            in_use: in_use.trim() == "*",
            ssid: ssid.clone(),
            signal: signal.parse().unwrap_or(0),
            security: security.clone(),
        };
        match networks.iter_mut().find(|known| known.ssid == network.ssid) {
            Some(known) => {
                known.in_use |= network.in_use;
                known.signal = known.signal.max(network.signal);
            }
            None => networks.push(network),
        }
    }
    networks.sort_by(|a, b| b.in_use.cmp(&a.in_use).then(b.signal.cmp(&a.signal)));
    let keep = MAX_NETWORKS + usize::from(networks.first().is_some_and(|n| n.in_use));
    networks.truncate(keep);
    networks
}

/// `bluetoothctl show`: "Powered: yes"; `None` without a controller.
fn parse_powered(text: &str) -> Option<bool> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("Powered:"))
        .map(|value| value.trim() == "yes")
}

/// `PRETTY_NAME` (else `NAME`) from os-release.
fn os_name(text: &str) -> Option<String> {
    let value = |key: &str| {
        text.lines().find_map(|line| {
            let value = line.strip_prefix(key)?.strip_prefix('=')?;
            let value = value.trim().trim_matches(['"', '\'']);
            (!value.is_empty()).then(|| value.to_owned())
        })
    };
    value("PRETTY_NAME").or_else(|| value("NAME"))
}

/// `MemTotal` from /proc/meminfo, as "1.0 GB" (GiB) or "512 MB" (MiB).
fn memory_total(text: &str) -> Option<String> {
    let kib: u64 = text
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let mib = kib / 1024;
    Some(if mib >= 1024 {
        format!("{:.1} GB", mib as f64 / 1024.0)
    } else {
        format!("{mib} MB")
    })
}

/// Whether a desktop entry with this file name is installed in one of the
/// data directories' `applications`.
pub(super) fn has_desktop_entry(data_dirs: &[std::path::PathBuf], file: &str) -> bool {
    data_dirs
        .iter()
        .any(|dir| Path::new(dir).join("applications").join(file).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wpctl_volumes() {
        assert_eq!(
            parse_volume("Volume: 0.40\n"),
            Some(Volume {
                level: 0.4,
                muted: false
            })
        );
        assert_eq!(
            parse_volume("Volume: 1.00 [MUTED]").map(|v| v.muted),
            Some(true)
        );
        assert_eq!(parse_volume("Error"), None);
    }

    #[test]
    fn reads_the_audio_sinks_of_wpctl_status() {
        let status = "PipeWire 'pipewire-0' [1.4.2, root@tvbox, cookie:1]\n \
                      └─ Clients:\n        31. WirePlumber [1.4.2, root@tvbox, pid:500]\n\n\
                      Audio\n \
                      ├─ Devices:\n \
                      │      42. Built-in Audio                      [alsa]\n \
                      │  \n \
                      ├─ Sinks:\n \
                      │  *   48. Built-in Audio Analog Stereo        [vol: 0.40]\n \
                      │      52. HDMI / DisplayPort                  [vol: 1.00 MUTED]\n \
                      │  \n \
                      ├─ Sources:\n \
                      │      49. Built-in Audio Analog Stereo        [vol: 1.00]\n\n\
                      Video\n \
                      ├─ Sinks:\n \
                      │      70. Not audio\n";
        assert_eq!(
            parse_sinks(status),
            [
                Sink {
                    id: 48,
                    name: "Built-in Audio Analog Stereo".into(),
                    default: true
                },
                Sink {
                    id: 52,
                    name: "HDMI / DisplayPort".into(),
                    default: false
                },
            ]
        );
    }

    #[test]
    fn reads_nmcli_networks_with_escaped_colons() {
        assert_eq!(split_terse(r"a\:b:c\\d:"), ["a:b", r"c\d", ""]);
        let list = " :Neighbour:30:WPA2\n*:Home:80:WPA2\n :Home:60:WPA2\n ::90:\n :Cafe:55:\n";
        let networks = parse_networks(list);
        let names: Vec<&str> = networks.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Home", "Cafe", "Neighbour"]);
        assert!(networks[0].in_use);
        assert_eq!(networks[0].signal, 80);
        assert_eq!(networks[1].security, "");
    }

    #[test]
    fn reads_the_network_interfaces_with_their_signal() {
        let devices = "wlan0:wifi:connected\nwlan1:wifi:disconnected\n\
                       enx00e04c:ethernet:unavailable\neth0:ethernet:connected (externally)\n\
                       lo:loopback:connected (externally)\nusb9:ethernet:unmanaged\n\
                       p2p-dev-wlan0:wifi-p2p:disconnected\n";
        let signals = " :90:wlan0\n*:52:wlan0\n*:99:wlan1\n";
        assert_eq!(
            parse_links(devices, signals),
            [
                Link::Wifi(Some(3)),
                Link::Wifi(None),
                Link::Wired(false),
                Link::Wired(true),
            ]
        );
        // Connected but not in the last scan: full bars.
        assert_eq!(
            parse_links("wlan0:wifi:connected", ""),
            [Link::Wifi(Some(4))]
        );
    }

    #[test]
    fn reads_bluetooth_os_release_and_memory() {
        assert_eq!(
            parse_powered("Controller 00:11 (public)\n\tPowered: yes\n"),
            Some(true)
        );
        assert_eq!(parse_powered("No default controller available\n"), None);
        assert_eq!(
            os_name("NAME=\"Debian GNU/Linux\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n"),
            Some("Debian GNU/Linux 13 (trixie)".into())
        );
        assert_eq!(os_name("NAME=Arch\n"), Some("Arch".into()));
        assert_eq!(
            memory_total("MemTotal:        1003296 kB\nMemFree: 1 kB\n"),
            Some("979 MB".into())
        );
        assert_eq!(
            memory_total("MemTotal:        2097152 kB\n"),
            Some("2.0 GB".into())
        );
    }

    #[test]
    fn a_missing_tool_hides_its_panel_and_fails_to_run() {
        let device = LinuxDevice::default();
        assert!(!device.has(QuickPanel::Audio));
        assert_eq!(device.panel(QuickPanel::Power), None);
        assert_eq!(device.volume(VolumeChange::Up), None);
        assert!(run("rusttv-no-such-tool", &[]).is_err());
        assert_eq!(run("echo", &["hi"]).as_deref(), Ok("hi\n"));
    }
}

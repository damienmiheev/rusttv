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

mod net;

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

/// Control ids of paired Bluetooth devices: this prefix and the address.
const BT: &str = "bt:";

/// Control ids of found, unpaired Bluetooth devices.
const PAIR: &str = "pair:";

/// How long a Bluetooth search runs, in seconds.
const BT_SCAN_SECONDS: &str = "8";

/// Where an image names another system on the same device that the Power
/// panel restarts into (dual boot). It holds a command, so it is used only
/// while root owns it and no one else may write it ([`read_root_only`]).
const OTHER_OS: &str = "/etc/rusttv/other-os.toml";

/// Another system to restart into, from [`OTHER_OS`]:
///
/// ```toml
/// label = "Restart into Android"
/// command = ["systemctl", "start", "--no-block", "hy320f-to-android.service"]
/// ```
///
/// The command runs as the launcher's user, without a shell; it switches
/// the boot target and restarts. How is the image's business: on the
/// HY320F a root service changes the A/B slot priorities, as Android's own
/// boot HAL does, and Android switches back the same way.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct OtherOs {
    label: String,
    command: Vec<String>,
}

/// The text of a file that holds a command, only if root owns it and
/// neither its group nor others may write it: whoever could change it could
/// make the launcher run their command. Checked on the opened file, so it
/// can't be swapped between the check and the read.
fn read_root_only(path: &str) -> Option<String> {
    use std::io::Read as _;
    use std::os::unix::fs::MetadataExt as _;
    let mut file = std::fs::File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || !root_only(meta.uid(), meta.mode()) {
        log::warn!("{path}: ignored, it must be root's and writable only by root");
        return None;
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    Some(text)
}

/// Whether a file with this owner and mode can be changed only by root.
fn root_only(uid: u32, mode: u32) -> bool {
    uid == 0 && mode & 0o022 == 0
}

/// [`OtherOs`] from its file; `None` if it is unreadable or has no command.
fn parse_other_os(text: &str) -> Option<OtherOs> {
    toml::from_str::<OtherOs>(text)
        .map_err(|error| log::warn!("{OTHER_OS}: {error}"))
        .ok()
        .filter(|other| !other.command.is_empty() && !other.label.is_empty())
}

/// [`DeviceSettings`] through the tools found in `PATH` at start.
#[derive(Debug, Clone, Default)]
pub struct LinuxDevice {
    wpctl: bool,
    nmcli: bool,
    bluetoothctl: bool,
    systemctl: bool,
    /// Another system to restart into (dual boot).
    other_os: Option<OtherOs>,
    /// Whether a Bluetooth search ran: Devices then lists what it found.
    scanned: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl LinuxDevice {
    /// Looks for the tools in `PATH`.
    pub fn detect() -> Self {
        Self {
            wpctl: program_exists("wpctl"),
            nmcli: program_exists("nmcli"),
            bluetoothctl: program_exists("bluetoothctl"),
            systemctl: program_exists("systemctl"),
            other_os: read_root_only(OTHER_OS).and_then(|text| parse_other_os(&text)),
            scanned: std::sync::Arc::default(),
        }
    }

    /// Whether this panel can be shown (its tool is installed).
    pub(super) fn has(&self, panel: QuickPanel) -> bool {
        match panel {
            QuickPanel::Audio => self.wpctl,
            QuickPanel::Network => self.nmcli,
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
        if !self.nmcli {
            return None;
        }
        let mut controls = Vec::new();
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
        let wifi =
            run("nmcli", &["-t", "-f", "WIFI", "radio"]).is_ok_and(|text| text.trim() == "enabled");
        controls.push(control(
            "wifi",
            "Wi-Fi",
            "",
            ControlKind::Toggle { on: wifi },
        ));
        if !wifi {
            return Some(tab(QuickPanel::Network, "Network", controls));
        }
        let networks = run(
            "nmcli",
            &[
                "-t",
                "-f",
                "IN-USE,SSID,SIGNAL,SECURITY",
                "device",
                "wifi",
                "list",
                "--rescan",
                "no",
            ],
        )
        .map(|text| parse_networks(&text))
        .unwrap_or_default();
        let saved = run("nmcli", &["-t", "-f", "NAME,TYPE", "connection", "show"])
            .map(|text| parse_saved(&text))
            .unwrap_or_default();
        for network in &networks {
            controls.push(network_control(network, saved.contains(&network.ssid)));
        }
        controls.push(control(
            "rescan",
            "Search for networks",
            "",
            ControlKind::Action,
        ));
        Some(tab(QuickPanel::Network, "Network", controls))
    }

    /// Bluetooth: on or off, the paired devices (OK connects or
    /// disconnects one), and after a search the new ones nearby (OK pairs).
    fn devices(&self) -> Option<Tab> {
        if !self.bluetoothctl {
            return None;
        }
        let on = run("bluetoothctl", &["show"])
            .ok()
            .and_then(|text| parse_powered(&text))?;
        let mut controls = vec![control(
            "bluetooth",
            "Bluetooth",
            "",
            ControlKind::Toggle { on },
        )];
        if on {
            let list = |filter: &str| {
                run("bluetoothctl", &["devices", filter])
                    .map(|text| parse_bt_devices(&text))
                    .unwrap_or_default()
            };
            let paired = list("Paired");
            let connected = list("Connected");
            for (address, name) in &paired {
                let up = connected.iter().any(|(other, _)| other == address);
                controls.push(control(
                    &format!("{BT}{address}"),
                    name,
                    if up { "Connected" } else { "Not connected" },
                    ControlKind::Action,
                ));
            }
            if self.scanned.load(std::sync::atomic::Ordering::Relaxed) {
                let all = run("bluetoothctl", &["devices"])
                    .map(|text| parse_bt_devices(&text))
                    .unwrap_or_default();
                for (address, name) in all
                    .iter()
                    .filter(|(address, _)| !paired.iter().any(|(other, _)| other == address))
                {
                    controls.push(control(
                        &format!("{PAIR}{address}"),
                        name,
                        "New",
                        ControlKind::Action,
                    ));
                }
            }
            controls.push(control(
                "bt-scan",
                "Search for devices",
                "",
                ControlKind::Action,
            ));
        }
        Some(Tab {
            id: "devices".to_owned(),
            title: "Devices".to_owned(),
            controls,
        })
    }

    fn power(&self) -> Option<Tab> {
        self.systemctl.then(|| {
            let mut controls = vec![control("reboot", "Restart", "", ControlKind::Action)];
            if let Some(other) = &self.other_os {
                controls.push(control("other-os", &other.label, "", ControlKind::Action));
            }
            controls.push(control("poweroff", "Power off", "", ControlKind::Action));
            tab(QuickPanel::Power, "Power", controls)
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
        // Settings' Network tab also shows each interface's details (not
        // the quick panel); it is there without nmcli too.
        let interfaces = net::rows();
        let mut network = self.network();
        if !interfaces.is_empty() {
            network
                .get_or_insert_with(|| tab(QuickPanel::Network, "Network", Vec::new()))
                .controls
                .extend(interfaces);
        }
        tabs.extend(network);
        tabs.extend(self.devices());
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
                // A saved network first; else a new, open one.
                run("nmcli", &["connection", "up", "id", ssid])
                    .or_else(|_| run("nmcli", &["device", "wifi", "connect", ssid]))
            }
            (control, Value::Text(password)) if control.starts_with(CONNECT) => {
                let ssid = &control[CONNECT.len()..];
                // `--ask` reads the password from stdin.
                run_with_input(
                    "nmcli",
                    &["--ask", "device", "wifi", "connect", ssid],
                    Some(&password),
                )
            }
            ("rescan", Value::Action) => run(
                "nmcli",
                &[
                    "-t", "-f", "SSID", "device", "wifi", "list", "--rescan", "yes",
                ],
            ),
            ("bt-scan", Value::Action) => {
                let scanned = run(
                    "bluetoothctl",
                    &["--timeout", BT_SCAN_SECONDS, "scan", "on"],
                );
                self.scanned
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                scanned
            }
            (control, Value::Action) if control.starts_with(BT) => {
                let address = bt_address(&control[BT.len()..]).ok_or("not a Bluetooth address")?;
                let connected = run("bluetoothctl", &["info", address])
                    .is_ok_and(|text| text.contains("Connected: yes"));
                run(
                    "bluetoothctl",
                    &[if connected { "disconnect" } else { "connect" }, address],
                )
            }
            (control, Value::Action) if control.starts_with(PAIR) => {
                let address =
                    bt_address(&control[PAIR.len()..]).ok_or("not a Bluetooth address")?;
                run("bluetoothctl", &["pair", address])
                    .and_then(|_| run("bluetoothctl", &["trust", address]))
                    .and_then(|_| run("bluetoothctl", &["connect", address]))
            }
            ("reboot", Value::Action) => run("systemctl", &["reboot"]),
            ("poweroff", Value::Action) => run("systemctl", &["poweroff"]),
            ("other-os", Value::Action) => {
                let other = self.other_os.as_ref().ok_or("no other system")?;
                let (program, args) = other.command.split_first().ok_or("no command")?;
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                run(program, &args)
            }
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

    fn volume_now(&self) -> Option<Volume> {
        if !self.wpctl {
            return None;
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
    run_with_input(program, args, None)
}

/// [`run`] with `input` on the tool's stdin (a password, so it is never
/// on a command line other users can read).
fn run_with_input(program: &str, args: &[&str], input: Option<&str>) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        use std::io::Write as _;
        // Dropped after writing: the tool sees the end of its input.
        let _ = writeln!(stdin, "{input}");
    }
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

/// How a Wi-Fi network shows: saved and open ones connect on OK, a new
/// secured one asks for its password first.
fn network_control(network: &Network, saved: bool) -> Control {
    let id = format!("{CONNECT}{}", network.ssid);
    let open = network.security.is_empty() || network.security == "--";
    let state = if network.in_use {
        "Connected"
    } else if saved {
        "Saved"
    } else if open {
        "Open"
    } else {
        "Secured"
    };
    let detail = format!("{state} · {} %", network.signal);
    let kind = if network.in_use {
        ControlKind::Info
    } else if saved || open {
        ControlKind::Action
    } else {
        ControlKind::Text { secret: true }
    };
    control(&id, &network.ssid, &detail, kind)
}

/// The names of saved Wi-Fi connections, from
/// `nmcli -t -f NAME,TYPE connection show`.
fn parse_saved(text: &str) -> Vec<String> {
    text.lines()
        .map(split_terse)
        .filter_map(|fields| match fields.as_slice() {
            [name, kind] if kind == "802-11-wireless" => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// `bluetoothctl devices`: "Device AA:BB:CC:DD:EE:FF Name" lines, as
/// (address, name); a device without a name shows its address.
fn parse_bt_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("Device ")?;
            let (address, name) = rest.split_once(' ').unwrap_or((rest, rest));
            Some((bt_address(address)?.to_owned(), name.trim().to_owned()))
        })
        .collect()
}

/// `text` if it is a Bluetooth address, `AA:BB:CC:DD:EE:FF` (six pairs of
/// hex digits): only such a word goes to `bluetoothctl`.
fn bt_address(text: &str) -> Option<&str> {
    let pairs: Vec<&str> = text.split(':').collect();
    (pairs.len() == 6
        && pairs
            .iter()
            .all(|pair| pair.len() == 2 && pair.bytes().all(|b| b.is_ascii_hexdigit())))
    .then_some(text)
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
    fn new_secured_networks_ask_for_a_password_and_saved_ones_connect() {
        let saved =
            parse_saved("Home:802-11-wireless\nWired:802-3-ethernet\nLab\\:2:802-11-wireless\n");
        assert_eq!(saved, ["Home", "Lab:2"]);
        let network = |ssid: &str, security: &str, in_use: bool| Network {
            in_use,
            ssid: ssid.to_owned(),
            signal: 70,
            security: security.to_owned(),
        };
        let kind = |control: Control| (control.detail, control.kind);
        assert_eq!(
            kind(network_control(&network("Home", "WPA2", false), true)),
            ("Saved · 70 %".to_owned(), ControlKind::Action)
        );
        assert_eq!(
            kind(network_control(&network("Cafe", "", false), false)),
            ("Open · 70 %".to_owned(), ControlKind::Action)
        );
        assert_eq!(
            kind(network_control(
                &network("Next door", "WPA2 WPA3", false),
                false
            )),
            (
                "Secured · 70 %".to_owned(),
                ControlKind::Text { secret: true }
            )
        );
        assert_eq!(
            kind(network_control(&network("Home", "WPA2", true), true)).1,
            ControlKind::Info
        );
    }

    #[test]
    fn reads_bluetooth_devices() {
        let text =
            "Device AA:BB:CC:DD:EE:FF Bluetooth remote\nDevice 11:22:33:44:55:66\nController x\n";
        assert_eq!(
            parse_bt_devices(text),
            [
                (
                    "AA:BB:CC:DD:EE:FF".to_owned(),
                    "Bluetooth remote".to_owned()
                ),
                (
                    "11:22:33:44:55:66".to_owned(),
                    "11:22:33:44:55:66".to_owned()
                ),
            ]
        );
        // Only addresses: nothing else ever reaches bluetoothctl.
        assert!(parse_bt_devices("Device --help x\nDevice AA:BB:CC:DD:EE x\n").is_empty());
        assert_eq!(bt_address("aa:bb:cc:dd:ee:ff"), Some("aa:bb:cc:dd:ee:ff"));
        for bad in [
            "",
            "AA:BB:CC:DD:EE",
            "AA:BB:CC:DD:EE:FF:00",
            "AA:BB:CC:DD:EE:FG",
            "AAA:B:CC:DD:EE:FF",
            "AA:BB:CC:DD:EE:FF;",
        ] {
            assert_eq!(bt_address(bad), None, "{bad}");
        }
        let device = LinuxDevice {
            bluetoothctl: true,
            ..LinuxDevice::default()
        };
        assert!(
            device
                .set("bluetooth", "pair:--help", Value::Action)
                .is_err()
        );
        assert!(device.set("bluetooth", "bt:AA:BB", Value::Action).is_err());
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
    fn reads_another_system_to_restart_into() {
        let other = parse_other_os(
            "label = \"Restart into Android\"\ncommand = [\"systemctl\", \"start\", \"x.service\"]\n",
        );
        assert_eq!(
            other,
            Some(OtherOs {
                label: "Restart into Android".into(),
                command: vec!["systemctl".into(), "start".into(), "x.service".into()],
            })
        );
        assert_eq!(parse_other_os("label = \"A\"\ncommand = []\n"), None);
        assert_eq!(parse_other_os("label = \"A\"\nshell = \"rm\"\n"), None);
        // Only a file no one but root can change.
        assert!(root_only(0, 0o100_644));
        assert!(root_only(0, 0o100_600));
        assert!(!root_only(1000, 0o100_644), "not root's");
        assert!(!root_only(0, 0o100_664), "group-writable");
        assert!(!root_only(0, 0o100_646), "world-writable");
        // A file of the test's own user is refused unless that user is root.
        let mine = std::env::temp_dir().join(format!("rusttv-other-os-{}", std::process::id()));
        std::fs::write(&mine, "label = \"A\"\ncommand = [\"true\"]\n").unwrap();
        let read = read_root_only(mine.to_str().unwrap());
        assert_eq!(read.is_some(), rustix::process::geteuid().is_root());
        std::fs::remove_file(&mine).unwrap();
        let device = LinuxDevice {
            systemctl: true,
            other_os: other,
            ..LinuxDevice::default()
        };
        let power = device.panel(QuickPanel::Power).expect("a Power panel");
        let ids: Vec<&str> = power.controls.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["reboot", "other-os", "poweroff"]);
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

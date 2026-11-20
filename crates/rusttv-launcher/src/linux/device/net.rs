//! What Settings → Network shows about each network interface, read
//! without a shell: sysfs (`/sys/class/net`) for the kind, state, MAC,
//! MTU, link speed and byte counters; rtnetlink for the IPv4 and IPv6
//! addresses and nl80211 for the Wi-Fi bitrates (`netlink`);
//! `/proc/net/route`, `/proc/net/ipv6_route` and `/proc/net/wireless` for
//! gateways and the Wi-Fi signal; NetworkManager on D-Bus for the Wi-Fi
//! network (SSID, access point, frequency, strength); systemd-resolved on D-Bus for each interface's DNS servers, else
//! `/etc/resolv.conf`.
//!
//! Only physical Wi-Fi and Ethernet interfaces are listed (not `lo`,
//! tunnels or bridges). Every read blocks briefly; called from worker
//! threads only, like the rest of [`super::LinuxDevice`].

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use crate::device::{Control, ControlKind};

mod netlink;

use netlink::{IFA_F_DEPRECATED, IFA_F_TEMPORARY, RT_SCOPE_LINK};

const SYS_NET: &str = "/sys/class/net";

/// `ARPHRD_ETHER`: Ethernet and Wi-Fi interfaces.
const ARPHRD_ETHER: u32 = 1;

/// `IFF_UP` in `/sys/class/net/*/flags`.
const IFF_UP: u32 = 0x1;

/// What an interface is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Wifi,
    Ethernet,
    UsbEthernet,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Wifi => "Wi-Fi",
            Self::Ethernet => "Ethernet",
            Self::UsbEthernet => "USB Ethernet",
        }
    }
}

/// An IP address of an interface, from rtnetlink.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Address {
    ip: IpAddr,
    prefix: u8,
    scope: u8,
    /// `IFA_F_*`.
    flags: u32,
}

/// The Wi-Fi network an interface is on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Wifi {
    ssid: Option<String>,
    bssid: Option<String>,
    /// NetworkManager's 0..100.
    strength: Option<u8>,
    /// From `/proc/net/wireless`, else nl80211.
    level_dbm: Option<i32>,
    frequency_mhz: Option<u32>,
    /// What the interface last received and sent at, from nl80211.
    rx_kbit: Option<u32>,
    tx_kbit: Option<u32>,
}

/// Everything known about one interface.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Interface {
    name: String,
    index: u32,
    kind: Kind,
    driver: Option<String>,
    /// Switched on (`IFF_UP`).
    up: bool,
    /// A cable in, or associated to a Wi-Fi network; `None` when the
    /// kernel doesn't say (interface off).
    carrier: Option<bool>,
    mac: Option<String>,
    mtu: Option<u32>,
    /// Mbit/s, for Ethernet with a link.
    speed: Option<u32>,
    duplex: Option<String>,
    addresses: Vec<Address>,
    gateways: Vec<IpAddr>,
    dns: Vec<IpAddr>,
    wifi: Option<Wifi>,
    rx_bytes: Option<u64>,
    tx_bytes: Option<u64>,
}

/// One Info row per fact of each interface, Wi-Fi first; then the
/// system's DNS servers when no interface named its own.
pub(super) fn rows() -> Vec<Control> {
    let mut interfaces = read_sysfs(Path::new(SYS_NET));
    if interfaces.is_empty() {
        return Vec::new();
    }
    let addresses = netlink::addresses()
        .map_err(|error| log::warn!("reading the IP addresses failed: {error}"))
        .unwrap_or_default();
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let mut gateways = parse_routes(&read("/proc/net/route"));
    gateways.extend(parse_routes6(&read("/proc/net/ipv6_route")));
    let signals = parse_wireless(&read("/proc/net/wireless"));
    let bus = zbus::blocking::Connection::system()
        .map_err(|error| log::debug!("no system bus: {error}"))
        .ok();
    for interface in &mut interfaces {
        interface.addresses = addresses
            .iter()
            .filter(|(index, _)| *index == interface.index)
            .map(|(_, address)| address.clone())
            .collect();
        interface.gateways = gateways
            .iter()
            .filter(|(name, _)| *name == interface.name)
            .map(|(_, ip)| *ip)
            .collect();
        if let Some(bus) = &bus {
            interface.dns = link_dns(bus, interface.index).unwrap_or_default();
        }
        if interface.kind == Kind::Wifi {
            let mut wifi = bus
                .as_ref()
                .and_then(|bus| nm_wifi(bus, &interface.name).ok())
                .unwrap_or_default();
            let station = netlink::station(interface.index, wifi.bssid.as_deref())
                .map_err(|error| log::debug!("no nl80211 station: {error}"))
                .ok()
                .flatten()
                .unwrap_or_default();
            wifi.level_dbm = signals
                .iter()
                .find(|(name, _)| *name == interface.name)
                .map(|(_, level)| *level)
                .or(station.signal_dbm);
            wifi.bssid = wifi.bssid.or(station.bssid);
            wifi.rx_kbit = station.rx_kbit;
            wifi.tx_kbit = station.tx_kbit;
            interface.wifi = Some(wifi);
        }
    }
    let mut controls = interface_rows(&interfaces);
    if interfaces.iter().all(|interface| interface.dns.is_empty()) {
        let system = parse_resolv_conf(&read("/etc/resolv.conf"));
        if !system.is_empty() {
            controls.push(info("net-dns", "DNS servers", &join(&system)));
        }
    }
    controls
}

/// The rows of `interfaces`: a heading row (name; kind and state), then
/// one row per fact, labelled with the interface's kind (its name when
/// two share a kind).
fn interface_rows(interfaces: &[Interface]) -> Vec<Control> {
    let mut controls = Vec::new();
    for interface in interfaces {
        let shared = interfaces
            .iter()
            .filter(|other| other.kind == interface.kind)
            .count()
            > 1;
        let prefix = if shared {
            interface.name.as_str()
        } else {
            interface.kind.name()
        };
        let id = format!("net:{}", interface.name);
        controls.push(info(
            &id,
            &interface.name,
            &format!("{} · {}", interface.kind.name(), state(interface)),
        ));
        for (n, (fact, value)) in facts(interface).into_iter().enumerate() {
            controls.push(info(
                &format!("{id}:{n}"),
                &format!("{prefix} · {fact}"),
                &value,
            ));
        }
    }
    controls
}

/// "Off", "Connected", "No cable", "Not connected".
fn state(interface: &Interface) -> &'static str {
    match (interface.up, interface.carrier, interface.kind) {
        (false, _, _) => "Off",
        (true, Some(true), _) => "Connected",
        (true, _, Kind::Wifi) => "Not connected",
        (true, _, _) => "No cable",
    }
}

/// An interface's facts, as (label, value), in the order they show.
fn facts(interface: &Interface) -> Vec<(&'static str, String)> {
    let mut facts = Vec::new();
    if let Some(wifi) = &interface.wifi {
        if let Some(ssid) = &wifi.ssid {
            facts.push(("Network", ssid.clone()));
        }
        if let Some(bssid) = &wifi.bssid {
            facts.push(("Access point", bssid.clone()));
        }
        let signal: Vec<String> = [
            wifi.level_dbm.map(|level| format!("{level} dBm")),
            wifi.strength.map(|strength| format!("{strength} %")),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !signal.is_empty() {
            facts.push(("Signal", signal.join(" · ")));
        }
        if let Some(frequency) = wifi.frequency_mhz {
            facts.push(("Frequency", frequency_text(frequency)));
        }
        let mbit = |kbit: u32| format!("{} Mbit/s", decimal(kbit.into(), 1000));
        match (wifi.rx_kbit, wifi.tx_kbit) {
            (Some(rx), Some(tx)) => {
                facts.push(("Bitrate", format!("{} down · {} up", mbit(rx), mbit(tx))));
            }
            (Some(rx), None) => facts.push(("Bitrate", format!("{} down", mbit(rx)))),
            (None, Some(tx)) => facts.push(("Bitrate", format!("{} up", mbit(tx)))),
            (None, None) => {}
        }
    }
    for address in &interface.addresses {
        let family = if address.ip.is_ipv4() { "IPv4" } else { "IPv6" };
        let mut text = format!("{}/{}", address.ip, address.prefix);
        if address.scope == RT_SCOPE_LINK {
            text.push_str(" · link-local");
        }
        if address.flags & IFA_F_TEMPORARY != 0 {
            text.push_str(" · temporary");
        }
        if address.flags & IFA_F_DEPRECATED != 0 {
            text.push_str(" · deprecated");
        }
        facts.push((family, text));
    }
    if !interface.gateways.is_empty() {
        facts.push(("Gateway", join(&interface.gateways)));
    }
    if !interface.dns.is_empty() {
        facts.push(("DNS servers", join(&interface.dns)));
    }
    if let Some(speed) = interface.speed {
        let duplex = interface
            .duplex
            .as_deref()
            .map(|duplex| format!(" · {duplex} duplex"))
            .unwrap_or_default();
        facts.push(("Link speed", format!("{speed} Mbit/s{duplex}")));
    }
    if let Some(mac) = &interface.mac {
        facts.push(("MAC address", mac.clone()));
    }
    if let Some(mtu) = interface.mtu {
        facts.push(("MTU", mtu.to_string()));
    }
    if let (Some(rx), Some(tx)) = (interface.rx_bytes, interface.tx_bytes) {
        facts.push((
            "Received / sent",
            format!("{} / {}", bytes_text(rx), bytes_text(tx)),
        ));
    }
    if let Some(driver) = &interface.driver {
        facts.push(("Driver", driver.clone()));
    }
    facts
}

fn info(id: &str, label: &str, detail: &str) -> Control {
    Control {
        id: id.to_owned(),
        label: label.to_owned(),
        detail: detail.to_owned(),
        kind: ControlKind::Info,
    }
}

fn join(ips: &[IpAddr]) -> String {
    ips.iter()
        .map(IpAddr::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `value / unit` with one decimal, the decimal dropped when it is 0.
fn decimal(value: u64, unit: u64) -> String {
    let tenths = (u128::from(value) * 10 + u128::from(unit) / 2) / u128::from(unit.max(1));
    if tenths % 10 == 0 {
        (tenths / 10).to_string()
    } else {
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// Bytes in decimal units: "512 B", "1.2 MB".
fn bytes_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut unit = 1000;
    for (i, name) in UNITS.iter().enumerate() {
        if bytes < unit * 1000 || i == UNITS.len() - 1 {
            return format!("{} {name}", decimal(bytes, unit));
        }
        unit *= 1000;
    }
    format!("{bytes} B")
}

/// "5180 MHz · 5 GHz · channel 36".
fn frequency_text(mhz: u32) -> String {
    let (band, channel) = match mhz {
        2484 => ("2.4 GHz", Some(14)),
        2412..=2472 => ("2.4 GHz", Some((mhz - 2407) / 5)),
        5955..=7115 => ("6 GHz", Some((mhz - 5950) / 5)),
        5000..=5925 => ("5 GHz", Some((mhz - 5000) / 5)),
        _ => return format!("{mhz} MHz"),
    };
    match channel {
        Some(channel) => format!("{mhz} MHz · {band} · channel {channel}"),
        None => format!("{mhz} MHz · {band}"),
    }
}

/// The physical Wi-Fi and Ethernet interfaces under `dir`
/// (`/sys/class/net`), Wi-Fi first, then by name; without the addresses,
/// gateways, DNS and Wi-Fi network, which come from elsewhere.
fn read_sysfs(dir: &Path) -> Vec<Interface> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut interfaces: Vec<Interface> = entries
        .flatten()
        .filter_map(|entry| read_interface(&entry.path()))
        .collect();
    interfaces.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    interfaces
}

/// One interface from its sysfs directory; `None` for virtual ones and
/// those that are not Ethernet or Wi-Fi.
fn read_interface(dir: &Path) -> Option<Interface> {
    let read = |file: &str| {
        std::fs::read_to_string(dir.join(file))
            .ok()
            .map(|text| text.trim().to_owned())
    };
    let number = |file: &str| read(file).and_then(|text| text.parse::<u64>().ok());
    let name = dir.file_name()?.to_str()?.to_owned();
    if number("type") != Some(ARPHRD_ETHER.into()) {
        return None;
    }
    // Virtual interfaces (bridges, veth, tunnels) have no device.
    let device = std::fs::canonicalize(dir.join("device")).ok()?;
    let kind = if dir.join("wireless").exists() || dir.join("phy80211").exists() {
        Kind::Wifi
    } else if device.components().any(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|part| part.starts_with("usb"))
    }) {
        Kind::UsbEthernet
    } else {
        Kind::Ethernet
    };
    let flags = read("flags")
        .and_then(|text| u32::from_str_radix(text.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0);
    let up = flags & IFF_UP != 0;
    let carrier = read("carrier").map(|text| text == "1");
    let speed = read("speed")
        .and_then(|text| text.parse::<i64>().ok())
        .filter(|&speed| speed > 0 && speed < i64::from(u32::MAX))
        .and_then(|speed| u32::try_from(speed).ok())
        .filter(|_| kind != Kind::Wifi && carrier == Some(true));
    Some(Interface {
        index: number("ifindex").and_then(|n| u32::try_from(n).ok())?,
        kind,
        driver: std::fs::read_link(device.join("driver"))
            .ok()
            .and_then(|link| link.file_name()?.to_str().map(str::to_owned)),
        up,
        carrier,
        mac: read("address").filter(|mac| !mac.is_empty() && mac != "00:00:00:00:00:00"),
        mtu: number("mtu").and_then(|n| u32::try_from(n).ok()),
        speed,
        duplex: speed
            .and(read("duplex"))
            .filter(|duplex| duplex == "full" || duplex == "half"),
        addresses: Vec::new(),
        gateways: Vec::new(),
        dns: Vec::new(),
        wifi: None,
        rx_bytes: number("statistics/rx_bytes"),
        tx_bytes: number("statistics/tx_bytes"),
        name,
    })
}

/// Default gateways per interface from `/proc/net/route`: destination 0,
/// `RTF_GATEWAY`; addresses as the kernel's native-endian hex.
fn parse_routes(text: &str) -> Vec<(String, IpAddr)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, destination, gateway, flags, ..] = fields.as_slice() else {
                return None;
            };
            let hex = |text: &str| u32::from_str_radix(text, 16).ok();
            let gateway = hex(gateway)?;
            (hex(destination)? == 0 && hex(flags)? & 0x2 != 0 && gateway != 0).then(|| {
                (
                    (*name).to_owned(),
                    IpAddr::V4(Ipv4Addr::from(gateway.to_ne_bytes())),
                )
            })
        })
        .collect()
}

/// Default gateways per interface from `/proc/net/ipv6_route`: ::/0 with
/// a next hop.
fn parse_routes6(text: &str) -> Vec<(String, IpAddr)> {
    let mut gateways: Vec<(String, IpAddr)> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [destination, length, _, _, next_hop, .., name] = fields.as_slice() else {
            continue;
        };
        let Some(next_hop) = u128::from_str_radix(next_hop, 16).ok().filter(|&n| n != 0) else {
            continue;
        };
        if u128::from_str_radix(destination, 16).ok() != Some(0) || *length != "00" {
            continue;
        }
        let gateway = ((*name).to_owned(), IpAddr::V6(Ipv6Addr::from(next_hop)));
        if !gateways.contains(&gateway) {
            gateways.push(gateway);
        }
    }
    gateways
}

/// The signal level (dBm) per interface from `/proc/net/wireless`.
fn parse_wireless(text: &str) -> Vec<(String, i32)> {
    text.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, rest) = line.split_once(':')?;
            let level = rest.split_whitespace().nth(2)?.trim_end_matches('.');
            let level = level.parse::<i32>().ok()?;
            // Some drivers report an unsigned byte.
            let level = if level > 127 { level - 256 } else { level };
            (level < 0).then(|| (name.trim().to_owned(), level))
        })
        .collect()
}

/// The name servers of a resolv.conf, without systemd-resolved's local
/// stub (127.0.0.53), which says nothing about the network.
fn parse_resolv_conf(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|rest| rest.trim().parse::<IpAddr>().ok())
        .filter(|ip| !ip.is_loopback())
        .collect()
}

/// A D-Bus proxy that reads properties fresh (no cache, no signals).
fn proxy<'a>(
    bus: &zbus::blocking::Connection,
    destination: &'a str,
    path: zbus::zvariant::ObjectPath<'a>,
    interface: &'a str,
) -> zbus::Result<zbus::blocking::Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(bus)
        .destination(destination)?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

/// The DNS servers systemd-resolved uses for interface `index`.
fn link_dns(bus: &zbus::blocking::Connection, index: u32) -> zbus::Result<Vec<IpAddr>> {
    const RESOLVE: &str = "org.freedesktop.resolve1";
    let manager = proxy(
        bus,
        RESOLVE,
        zbus::zvariant::ObjectPath::from_static_str_unchecked("/org/freedesktop/resolve1"),
        "org.freedesktop.resolve1.Manager",
    )?;
    let index = i32::try_from(index).unwrap_or(i32::MAX);
    let link: zbus::zvariant::OwnedObjectPath = manager.call("GetLink", &(index,))?;
    let link = proxy(bus, RESOLVE, link.into(), "org.freedesktop.resolve1.Link")?;
    let servers: Vec<(i32, Vec<u8>)> = link.get_property("DNS")?;
    Ok(servers
        .into_iter()
        .filter_map(|(_, bytes)| match bytes.len() {
            4 => <[u8; 4]>::try_from(bytes).ok().map(IpAddr::from),
            16 => <[u8; 16]>::try_from(bytes).ok().map(IpAddr::from),
            _ => None,
        })
        .collect())
}

/// The Wi-Fi network interface `name` is on, from NetworkManager.
fn nm_wifi(bus: &zbus::blocking::Connection, name: &str) -> zbus::Result<Wifi> {
    const NM: &str = "org.freedesktop.NetworkManager";
    let manager = proxy(
        bus,
        NM,
        zbus::zvariant::ObjectPath::from_static_str_unchecked("/org/freedesktop/NetworkManager"),
        NM,
    )?;
    let device: zbus::zvariant::OwnedObjectPath = manager.call("GetDeviceByIpIface", &(name,))?;
    let wireless = proxy(
        bus,
        NM,
        device.into(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )?;
    let mut wifi = Wifi::default();
    let access_point: zbus::zvariant::OwnedObjectPath =
        wireless.get_property("ActiveAccessPoint")?;
    if access_point.as_str() == "/" {
        return Ok(wifi);
    }
    let access_point = proxy(
        bus,
        NM,
        access_point.into(),
        "org.freedesktop.NetworkManager.AccessPoint",
    )?;
    wifi.ssid = access_point
        .get_property::<Vec<u8>>("Ssid")
        .ok()
        .map(|ssid| String::from_utf8_lossy(&ssid).into_owned())
        .filter(|ssid| !ssid.is_empty());
    wifi.bssid = access_point.get_property("HwAddress").ok();
    wifi.strength = access_point.get_property("Strength").ok();
    wifi.frequency_mhz = access_point.get_property("Frequency").ok();
    Ok(wifi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(name: &str, kind: Kind) -> Interface {
        Interface {
            name: name.to_owned(),
            index: 3,
            kind,
            driver: Some("r8152".to_owned()),
            up: true,
            carrier: Some(true),
            mac: Some("f8:e4:3b:9d:44:63".to_owned()),
            mtu: Some(1500),
            speed: None,
            duplex: None,
            addresses: Vec::new(),
            gateways: Vec::new(),
            dns: Vec::new(),
            wifi: None,
            rx_bytes: Some(1_234_567),
            tx_bytes: Some(999),
        }
    }

    /// Prints what this machine shows: `cargo test -- --ignored --nocapture
    /// this_machine`.
    #[test]
    #[ignore = "reads the machine it runs on"]
    fn this_machine() {
        for control in rows() {
            println!("{:<32} {}", control.label, control.detail);
        }
    }

    fn labels(controls: &[Control]) -> Vec<(&str, &str)> {
        controls
            .iter()
            .map(|control| (control.label.as_str(), control.detail.as_str()))
            .collect()
    }

    #[test]
    fn shows_each_fact_of_a_wifi_and_a_usb_ethernet_interface() {
        let mut wifi = interface("wlan0", Kind::Wifi);
        wifi.driver = None;
        wifi.addresses = vec![
            Address {
                ip: "192.168.1.125".parse().expect("ip"),
                prefix: 24,
                scope: 0,
                flags: 0,
            },
            Address {
                ip: "fe80::acae:e667:8efa:5646".parse().expect("ip"),
                prefix: 64,
                scope: RT_SCOPE_LINK,
                flags: 0x80,
            },
        ];
        wifi.gateways = vec!["192.168.1.1".parse().expect("ip")];
        wifi.dns = vec![
            "192.168.1.1".parse().expect("ip"),
            "fd00::1".parse().expect("ip"),
        ];
        wifi.wifi = Some(Wifi {
            ssid: Some("Home".to_owned()),
            bssid: Some("AA:BB:CC:DD:EE:FF".to_owned()),
            strength: Some(61),
            level_dbm: Some(-49),
            frequency_mhz: Some(5180),
            rx_kbit: Some(286_700),
            tx_kbit: Some(258_000),
        });
        let mut usb = interface("enx1", Kind::UsbEthernet);
        usb.speed = Some(100);
        usb.duplex = Some("full".to_owned());
        usb.carrier = Some(false);
        usb.up = true;
        let rows = interface_rows(&[wifi, usb]);
        assert_eq!(
            labels(&rows),
            [
                ("wlan0", "Wi-Fi · Connected"),
                ("Wi-Fi · Network", "Home"),
                ("Wi-Fi · Access point", "AA:BB:CC:DD:EE:FF"),
                ("Wi-Fi · Signal", "-49 dBm · 61 %"),
                ("Wi-Fi · Frequency", "5180 MHz · 5 GHz · channel 36"),
                ("Wi-Fi · Bitrate", "286.7 Mbit/s down · 258 Mbit/s up"),
                ("Wi-Fi · IPv4", "192.168.1.125/24"),
                ("Wi-Fi · IPv6", "fe80::acae:e667:8efa:5646/64 · link-local"),
                ("Wi-Fi · Gateway", "192.168.1.1"),
                ("Wi-Fi · DNS servers", "192.168.1.1, fd00::1"),
                ("Wi-Fi · MAC address", "f8:e4:3b:9d:44:63"),
                ("Wi-Fi · MTU", "1500"),
                ("Wi-Fi · Received / sent", "1.2 MB / 999 B"),
                ("enx1", "USB Ethernet · No cable"),
                ("USB Ethernet · Link speed", "100 Mbit/s · full duplex"),
                ("USB Ethernet · MAC address", "f8:e4:3b:9d:44:63"),
                ("USB Ethernet · MTU", "1500"),
                ("USB Ethernet · Received / sent", "1.2 MB / 999 B"),
                ("USB Ethernet · Driver", "r8152"),
            ]
        );
        let mut ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), rows.len(), "ids are unique");
        assert!(rows.iter().all(|row| row.kind == ControlKind::Info));
    }

    #[test]
    fn two_interfaces_of_a_kind_are_told_apart_by_name() {
        let mut off = interface("eth1", Kind::Ethernet);
        off.up = false;
        let rows = interface_rows(&[interface("eth0", Kind::Ethernet), off]);
        let labels = labels(&rows);
        assert!(labels.contains(&("eth0 · MTU", "1500")));
        assert!(labels.contains(&("eth1", "Ethernet · Off")));
    }

    #[test]
    fn formats_sizes_and_frequencies() {
        assert_eq!(bytes_text(0), "0 B");
        assert_eq!(bytes_text(1000), "1 kB");
        assert_eq!(bytes_text(1_550_000_000), "1.6 GB");
        assert_eq!(bytes_text(5_000_000_000_000_000), "5000 TB");
        assert_eq!(frequency_text(2437), "2437 MHz · 2.4 GHz · channel 6");
        assert_eq!(frequency_text(2484), "2484 MHz · 2.4 GHz · channel 14");
        assert_eq!(frequency_text(5975), "5975 MHz · 6 GHz · channel 5");
        assert_eq!(frequency_text(60_480), "60480 MHz");
    }

    #[test]
    fn reads_default_gateways_from_proc() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            enx1\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
            wlan0\t0001A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";
        let gateway = Ipv4Addr::from(0x0101_A8C0u32.to_ne_bytes());
        assert_eq!(
            parse_routes(route),
            [("enx1".to_owned(), IpAddr::V4(gateway))]
        );
        let route6 = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003 wlan0\n\
            fd53ca8f47d556ac0000000000000000 40 00000000000000000000000000000000 00 00000000000000000000000000000000 00000100 00000001 00000000 00000001 wlan0\n";
        assert_eq!(
            parse_routes6(route6),
            [("wlan0".to_owned(), "fe80::1".parse().expect("ip"))]
        );
    }

    #[test]
    fn reads_the_wifi_signal_and_resolv_conf() {
        let wireless = "Inter-| sta-|   Quality        |   Discarded packets               | Missed | WE\n \
            face | tus | link level noise |  nwid  crypt   frag  retry   misc | beacon | 22\n \
            wlan0: 0000   61.  -49.  -256        0      0      0      0      0        0\n";
        assert_eq!(parse_wireless(wireless), [("wlan0".to_owned(), -49)]);
        let resolv = "# comment\nnameserver 127.0.0.53\nnameserver 9.9.9.9\nnameserver fd00::1\nsearch lan\n";
        assert_eq!(
            parse_resolv_conf(resolv),
            [
                "9.9.9.9".parse::<IpAddr>().expect("ip"),
                "fd00::1".parse().expect("ip")
            ]
        );
    }
}

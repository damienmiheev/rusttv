//! The kernel's netlink answers Settings → Network needs, parsed by hand
//! (a few fixed layouts): rtnetlink's address dump, and nl80211's station
//! (the access point a Wi-Fi interface is associated with) over generic
//! netlink. Sockets through rustix; no `unsafe`.

use std::io;
use std::net::IpAddr;
use std::os::fd::OwnedFd;
use std::time::Duration;

use rustix::net::{
    AddressFamily, Protocol, RecvFlags, SendFlags, SocketFlags, SocketType, netlink, recv, sendto,
    socket_with, sockopt,
};

use super::Address;

/// How long the kernel may take to answer.
const TIMEOUT: Duration = Duration::from_secs(2);

/// Every request's sequence number: each socket sends one at a time.
const SEQ: u32 = 1;

// linux/netlink.h
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_DUMP: u16 = 0x300;
/// An attribute's type without `NLA_F_NESTED` and `NLA_F_NET_BYTEORDER`.
const NLA_TYPE_MASK: u16 = 0x3fff;

// linux/rtnetlink.h, linux/if_addr.h
const RTM_NEWADDR: u16 = 20;
const RTM_GETADDR: u16 = 22;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_FLAGS: u16 = 8;
pub(super) const IFA_F_TEMPORARY: u32 = 0x01;
pub(super) const IFA_F_DEPRECATED: u32 = 0x20;
/// `RT_SCOPE_LINK`: a link-local address.
pub(super) const RT_SCOPE_LINK: u8 = 253;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;

// linux/genetlink.h
const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;

// linux/nl80211.h
const NL80211_CMD_GET_STATION: u8 = 17;
const NL80211_ATTR_IFINDEX: u16 = 3;
const NL80211_ATTR_MAC: u16 = 6;
const NL80211_ATTR_STA_INFO: u16 = 21;
const NL80211_STA_INFO_SIGNAL: u16 = 7;
const NL80211_STA_INFO_TX_BITRATE: u16 = 8;
const NL80211_STA_INFO_RX_BITRATE: u16 = 14;
const NL80211_RATE_INFO_BITRATE: u16 = 1;
const NL80211_RATE_INFO_BITRATE32: u16 = 5;

/// The access point a Wi-Fi interface is associated with, from nl80211.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Station {
    pub bssid: Option<String>,
    pub signal_dbm: Option<i32>,
    /// What we last received and sent at, in kbit/s.
    pub rx_kbit: Option<u32>,
    pub tx_kbit: Option<u32>,
}

/// All IPv4 and IPv6 addresses, by interface index.
pub(super) fn addresses() -> io::Result<Vec<(u32, Address)>> {
    let socket = open(None)?;
    // ifaddrmsg: any family, the rest 0.
    send(&socket, RTM_GETADDR, NLM_F_DUMP, &[0; 8])?;
    let mut addresses = Vec::new();
    receive(&socket, true, |kind, payload| {
        if kind == RTM_NEWADDR
            && let Some(address) = parse_address(payload)
        {
            addresses.push(address);
        }
    })?;
    Ok(addresses)
}

/// The station of Wi-Fi interface `index`; `None` when not associated or
/// without nl80211. Asks for all stations, else (drivers that only
/// answer for one, like aic8800's) for the access point `bssid`.
pub(super) fn station(index: u32, bssid: Option<&str>) -> io::Result<Option<Station>> {
    let socket = open(Some(netlink::GENERIC))?;
    let mut request = genl_header(CTRL_CMD_GETFAMILY, 1);
    push_attribute(&mut request, CTRL_ATTR_FAMILY_NAME, b"nl80211\0");
    send(&socket, GENL_ID_CTRL, 0, &request)?;
    let mut family = None;
    receive(&socket, false, |kind, payload| {
        if kind == GENL_ID_CTRL {
            family = attributes(payload.get(4..).unwrap_or_default())
                .find(|(kind, _)| *kind == CTRL_ATTR_FAMILY_ID)
                .and_then(|(_, value)| ne_u16(value, 0));
        }
    })?;
    let Some(family) = family else {
        return Ok(None);
    };
    let mut request = genl_header(NL80211_CMD_GET_STATION, 0);
    push_attribute(&mut request, NL80211_ATTR_IFINDEX, &index.to_ne_bytes());
    send(&socket, family, NLM_F_DUMP, &request)?;
    let mut station = None;
    let mut first = |kind: u16, payload: &[u8]| {
        if kind == family && station.is_none() {
            station = Some(parse_station(payload));
        }
    };
    // A driver without a station dump fails it or answers nothing.
    let dumped = receive(&socket, true, &mut first);
    if let Some(mac) = bssid.and_then(parse_mac)
        && (dumped.is_err() || station.is_none())
    {
        push_attribute(&mut request, NL80211_ATTR_MAC, &mac);
        send(&socket, family, 0, &request)?;
        receive(&socket, false, |kind, payload| {
            if kind == family {
                station = Some(parse_station(payload));
            }
        })?;
        return Ok(station);
    }
    dumped.map(|()| station)
}

/// "44:F7:70:20:81:3B" as bytes.
fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let mut mac = [0u8; 6];
    let mut parts = text.split(':');
    for byte in &mut mac {
        *byte = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(mac)
}

fn open(protocol: Option<Protocol>) -> io::Result<OwnedFd> {
    // NETLINK_ROUTE is protocol 0, which rustix spells `None`.
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        protocol,
    )?;
    sockopt::set_socket_timeout(&socket, sockopt::Timeout::Recv, Some(TIMEOUT))?;
    Ok(socket)
}

/// Sends one request of type `kind` to the kernel.
fn send(socket: &OwnedFd, kind: u16, flags: u16, payload: &[u8]) -> io::Result<()> {
    let length = u32::try_from(16 + payload.len()).map_err(io::Error::other)?;
    let mut message = Vec::with_capacity(16 + payload.len());
    message.extend_from_slice(&length.to_ne_bytes());
    message.extend_from_slice(&kind.to_ne_bytes());
    message.extend_from_slice(&(NLM_F_REQUEST | flags).to_ne_bytes());
    message.extend_from_slice(&SEQ.to_ne_bytes());
    message.extend_from_slice(&0u32.to_ne_bytes());
    message.extend_from_slice(payload);
    sendto(
        socket,
        &message,
        SendFlags::empty(),
        &netlink::SocketAddrNetlink::new(0, 0),
    )?;
    Ok(())
}

/// Calls `each` with the type and payload of every answer to the request:
/// until `NLMSG_DONE` for a dump, else the first answer.
fn receive(socket: &OwnedFd, dump: bool, mut each: impl FnMut(u16, &[u8])) -> io::Result<()> {
    let mut buffer = vec![0u8; 32 * 1024];
    loop {
        let (length, _) = recv(socket, &mut buffer[..], RecvFlags::empty())?;
        let data = buffer.get(..length).unwrap_or_default();
        let messages = messages(data).ok_or_else(|| io::Error::other("broken netlink message"))?;
        for (kind, seq, payload) in messages {
            if seq != SEQ {
                continue;
            }
            match kind {
                NLMSG_DONE => return Ok(()),
                NLMSG_ERROR => {
                    let code = ne_u32(payload, 0).map_or(0, |code| code.cast_signed());
                    return if code == 0 {
                        Ok(())
                    } else {
                        Err(io::Error::from_raw_os_error(-code))
                    };
                }
                kind => {
                    each(kind, payload);
                    if !dump {
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// The messages of one datagram: (type, sequence number, payload);
/// `None` if one is cut off.
fn messages(data: &[u8]) -> Option<Vec<(u16, u32, &[u8])>> {
    let mut messages = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let length = ne_u32(data, at)? as usize;
        if length < 16 {
            return None;
        }
        let message = data.get(at..at + length)?;
        messages.push((ne_u16(message, 4)?, ne_u32(message, 8)?, &message[16..]));
        at += length.next_multiple_of(4);
    }
    Some(messages)
}

/// The attributes in `data`: (type, value).
fn attributes(data: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let length = usize::from(ne_u16(data, at)?);
        let kind = ne_u16(data, at + 2)? & NLA_TYPE_MASK;
        let value = data.get(at + 4..at + length.max(4))?;
        at += length.max(4).next_multiple_of(4);
        Some((kind, value))
    })
}

fn genl_header(command: u8, version: u8) -> Vec<u8> {
    vec![command, version, 0, 0]
}

fn push_attribute(data: &mut Vec<u8>, kind: u16, value: &[u8]) {
    let length = 4 + value.len();
    data.extend_from_slice(&u16::try_from(length).unwrap_or(u16::MAX).to_ne_bytes());
    data.extend_from_slice(&kind.to_ne_bytes());
    data.extend_from_slice(value);
    data.resize(data.len().next_multiple_of(4), 0);
}

fn ne_u16(data: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(data.get(at..at + 2)?.try_into().ok()?))
}

fn ne_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// One `RTM_NEWADDR` payload: `ifaddrmsg` and its attributes.
fn parse_address(payload: &[u8]) -> Option<(u32, Address)> {
    let &[family, prefix, flags, scope] = payload.get(..4)? else {
        return None;
    };
    let index = ne_u32(payload, 4)?;
    let mut flags = u32::from(flags);
    let (mut address, mut local) = (None, None);
    for (kind, value) in attributes(payload.get(8..)?) {
        let ip = match (family, value.len()) {
            (AF_INET, 4) => <[u8; 4]>::try_from(value).ok().map(IpAddr::from),
            (AF_INET6, 16) => <[u8; 16]>::try_from(value).ok().map(IpAddr::from),
            _ => None,
        };
        match kind {
            IFA_ADDRESS => address = ip,
            IFA_LOCAL => local = ip,
            IFA_FLAGS => flags = ne_u32(value, 0).unwrap_or(flags),
            _ => {}
        }
    }
    // IFA_ADDRESS is the peer on point-to-point links; IFA_LOCAL is ours.
    let ip = local.or(address)?;
    Some((
        index,
        Address {
            ip,
            prefix,
            scope,
            flags,
        },
    ))
}

/// One `NL80211_CMD_NEW_STATION` payload (after the generic netlink
/// header): the access point's address, signal and bitrates.
fn parse_station(payload: &[u8]) -> Station {
    let mut station = Station::default();
    for (kind, value) in attributes(payload.get(4..).unwrap_or_default()) {
        match kind {
            NL80211_ATTR_MAC if value.len() == 6 => {
                station.bssid = Some(
                    value
                        .iter()
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<Vec<_>>()
                        .join(":"),
                );
            }
            NL80211_ATTR_STA_INFO => {
                for (kind, value) in attributes(value) {
                    match kind {
                        NL80211_STA_INFO_SIGNAL => {
                            station.signal_dbm =
                                value.first().map(|&byte| i32::from(byte.cast_signed()));
                        }
                        NL80211_STA_INFO_TX_BITRATE => station.tx_kbit = bitrate(value),
                        NL80211_STA_INFO_RX_BITRATE => station.rx_kbit = bitrate(value),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    station
}

/// A nested `nl80211_rate_info`'s bitrate (given in 100 kbit/s) in kbit/s.
fn bitrate(rate_info: &[u8]) -> Option<u32> {
    let mut rate = None;
    for (kind, value) in attributes(rate_info) {
        match kind {
            NL80211_RATE_INFO_BITRATE32 => rate = ne_u32(value, 0),
            NL80211_RATE_INFO_BITRATE if rate.is_none() => rate = ne_u16(value, 0).map(u32::from),
            _ => {}
        }
    }
    rate.filter(|&rate| rate > 0).map(|rate| rate * 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A netlink message: header, payload, padded to 4 bytes.
    fn message(kind: u16, seq: u32, payload: &[u8]) -> Vec<u8> {
        let length = 16 + payload.len();
        let mut data = Vec::new();
        data.extend_from_slice(&u32::try_from(length).expect("small").to_ne_bytes());
        data.extend_from_slice(&kind.to_ne_bytes());
        data.extend_from_slice(&2u16.to_ne_bytes());
        data.extend_from_slice(&seq.to_ne_bytes());
        data.extend_from_slice(&0u32.to_ne_bytes());
        data.extend_from_slice(payload);
        data.resize(length.next_multiple_of(4), 0);
        data
    }

    #[test]
    fn reads_addresses_from_an_rtnetlink_dump() {
        let mut v4 = vec![AF_INET, 24, 0, 0];
        v4.extend_from_slice(&5u32.to_ne_bytes());
        push_attribute(&mut v4, IFA_ADDRESS, &[10, 0, 0, 9]);
        push_attribute(&mut v4, IFA_LOCAL, &[192, 168, 1, 211]);
        let mut v6 = vec![AF_INET6, 64, 0, RT_SCOPE_LINK];
        v6.extend_from_slice(&3u32.to_ne_bytes());
        let ip6: std::net::Ipv6Addr = "fe80::1".parse().expect("ip");
        push_attribute(&mut v6, IFA_ADDRESS, &ip6.octets());
        push_attribute(&mut v6, IFA_FLAGS, &IFA_F_TEMPORARY.to_ne_bytes());
        let mut data = message(RTM_NEWADDR, 1, &v4);
        data.extend(message(RTM_NEWADDR, 7, &v4));
        data.extend(message(RTM_NEWADDR, 1, &v6));
        let parsed = messages(&data).expect("whole");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1].1, 7);
        assert_eq!(
            parse_address(parsed[0].2),
            Some((
                5,
                Address {
                    ip: "192.168.1.211".parse().expect("ip"),
                    prefix: 24,
                    scope: 0,
                    flags: 0,
                }
            ))
        );
        assert_eq!(
            parse_address(parsed[2].2),
            Some((
                3,
                Address {
                    ip: IpAddr::V6(ip6),
                    prefix: 64,
                    scope: RT_SCOPE_LINK,
                    flags: IFA_F_TEMPORARY,
                }
            ))
        );
        assert_eq!(messages(&[40, 0, 0, 0]), None, "cut off");
        assert_eq!(messages(&data[..20]), None, "cut off");
    }

    #[test]
    fn reads_a_station_with_its_bitrates() {
        let mut tx = Vec::new();
        push_attribute(&mut tx, NL80211_RATE_INFO_BITRATE, &2580u16.to_ne_bytes());
        push_attribute(&mut tx, NL80211_RATE_INFO_BITRATE32, &2580u32.to_ne_bytes());
        let mut rx = Vec::new();
        push_attribute(&mut rx, NL80211_RATE_INFO_BITRATE, &2867u16.to_ne_bytes());
        let mut info = Vec::new();
        push_attribute(
            &mut info,
            NL80211_STA_INFO_SIGNAL,
            &[(-50i8).cast_unsigned()],
        );
        // Nested attributes may carry NLA_F_NESTED.
        push_attribute(&mut info, NL80211_STA_INFO_TX_BITRATE | 0x8000, &tx);
        push_attribute(&mut info, NL80211_STA_INFO_RX_BITRATE, &rx);
        let mut payload = genl_header(19, 1);
        push_attribute(
            &mut payload,
            NL80211_ATTR_MAC,
            &[0x44, 0xf7, 0x70, 0x20, 0x81, 0x3b],
        );
        push_attribute(&mut payload, NL80211_ATTR_STA_INFO, &info);
        assert_eq!(
            parse_mac("44:F7:70:20:81:3b"),
            Some([0x44, 0xf7, 0x70, 0x20, 0x81, 0x3b])
        );
        assert_eq!(parse_mac("44:F7:70:20:81"), None);
        assert_eq!(parse_mac("44:F7:70:20:81:3B:00"), None);
        assert_eq!(
            parse_station(&payload),
            Station {
                bssid: Some("44:F7:70:20:81:3B".to_owned()),
                signal_dbm: Some(-50),
                rx_kbit: Some(286_700),
                tx_kbit: Some(258_000),
            }
        );
    }
}

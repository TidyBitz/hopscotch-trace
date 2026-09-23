//! Which addresses have no public record, and why.

use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A public, routable address: geolocation and RDAP apply.
    Public,
    /// RFC 1918 (10/8, 172.16/12, 192.168/16): almost always your own router.
    Private,
    /// RFC 6598 shared address space (100.64/10): carrier-grade NAT inside your ISP.
    Cgnat,
    /// Loopback, link-local, or otherwise never on the internet.
    Local,
    /// Multicast, documentation ranges, reserved.
    Reserved,
}

impl Kind {
    pub fn is_public(self) -> bool {
        self == Kind::Public
    }
    /// The RFC or registry note that explains why there's nothing to look up.
    pub fn note(self) -> Option<(&'static str, &'static str)> {
        match self {
            Kind::Private => Some(("Private-use networks", "IANA, in RFC 1918")),
            Kind::Cgnat => Some(("Shared address space", "IANA, in RFC 6598")),
            Kind::Local => Some(("Local-only addresses", "IANA, in RFC 6890")),
            Kind::Reserved => Some(("Reserved", "IANA")),
            Kind::Public => None,
        }
    }
}

pub fn classify(ip: &IpAddr) -> Kind {
    match ip {
        IpAddr::V4(v4) => classify_v4(*v4),
        IpAddr::V6(v6) => classify_v6(*v6),
    }
}

pub fn classify_str(ip: &str) -> Kind {
    ip.parse::<IpAddr>().map(|a| classify(&a)).unwrap_or(Kind::Reserved)
}

fn classify_v4(a: Ipv4Addr) -> Kind {
    let o = a.octets();
    if a.is_private() {
        Kind::Private
    } else if o[0] == 100 && (64..=127).contains(&o[1]) {
        Kind::Cgnat
    } else if a.is_loopback() || a.is_link_local() || a.is_unspecified() || o[0] == 0 {
        Kind::Local
    } else if a.is_multicast()
        || a.is_broadcast()
        || a.is_documentation()
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || o[0] >= 240
    {
        Kind::Reserved
    } else {
        Kind::Public
    }
}

fn classify_v6(a: Ipv6Addr) -> Kind {
    let s = a.segments();
    if a.is_loopback() || a.is_unspecified() {
        Kind::Local
    } else if (s[0] & 0xffc0) == 0xfe80 {
        Kind::Local // link-local
    } else if (s[0] & 0xfe00) == 0xfc00 {
        Kind::Private // unique local
    } else if a.is_multicast() || s[0] == 0x2001 && s[1] == 0x0db8 {
        Kind::Reserved
    } else {
        Kind::Public
    }
}

/// The block an address belongs to when it is not public, for the Registration pane.
pub fn block_of(ip: &str) -> Option<&'static str> {
    let a: Ipv4Addr = ip.parse().ok()?;
    let o = a.octets();
    Some(match o {
        [10, ..] => "10.0.0.0/8",
        [172, b, ..] if (16..=31).contains(&b) => "172.16.0.0/12",
        [192, 168, ..] => "192.168.0.0/16",
        [100, b, ..] if (64..=127).contains(&b) => "100.64.0.0/10",
        [127, ..] => "127.0.0.0/8",
        [169, 254, ..] => "169.254.0.0/16",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kinds() {
        assert_eq!(classify_str("192.168.1.1"), Kind::Private);
        assert_eq!(classify_str("10.20.30.40"), Kind::Private);
        assert_eq!(classify_str("172.31.0.1"), Kind::Private);
        assert_eq!(classify_str("172.32.0.1"), Kind::Public);
        assert_eq!(classify_str("100.64.8.1"), Kind::Cgnat);
        assert_eq!(classify_str("100.128.0.1"), Kind::Public);
        assert_eq!(classify_str("127.0.0.1"), Kind::Local);
        assert_eq!(classify_str("169.254.3.3"), Kind::Local);
        assert_eq!(classify_str("8.8.8.8"), Kind::Public);
        assert_eq!(classify_str("203.0.113.9"), Kind::Reserved);
        assert_eq!(classify_str("2001:db8::1"), Kind::Reserved);
        assert_eq!(classify_str("2606:4700::1111"), Kind::Public);
        assert_eq!(classify_str("fe80::1"), Kind::Local);
        assert_eq!(classify_str("nonsense"), Kind::Reserved);
        assert_eq!(block_of("192.168.4.4"), Some("192.168.0.0/16"));
        assert_eq!(block_of("100.70.1.1"), Some("100.64.0.0/10"));
        assert_eq!(block_of("8.8.8.8"), None);
    }
}

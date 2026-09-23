//! Forward and reverse name lookups through the system resolver.

use std::net::{IpAddr, ToSocketAddrs};

/// Every address a name resolves to, IPv4 first (traceroute tools default to v4).
pub fn resolve(host: &str) -> crate::Result<Vec<String>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![ip.to_string()]);
    }
    let addrs = (host, 0).to_socket_addrs().map_err(|e| crate::Error::Message(format!("{host}: {e}")))?;
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for a in addrs {
        let s = a.ip().to_string();
        let list = if a.is_ipv4() { &mut v4 } else { &mut v6 };
        if !list.contains(&s) {
            list.push(s);
        }
    }
    v4.extend(v6);
    if v4.is_empty() {
        return Err(crate::Error::Message(format!("{host} has no address")));
    }
    Ok(v4)
}

/// The name an address answers to in reverse DNS, if any.
pub fn reverse(ip: &str) -> Option<String> {
    let addr: IpAddr = ip.parse().ok()?;
    let name = dns_lookup::lookup_addr(&addr).ok()?;
    // getnameinfo hands the address back as text when there is no PTR record
    if name == ip || name.parse::<IpAddr>().is_ok() {
        return None;
    }
    Some(name.trim_end_matches('.').to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_addresses_pass_through() {
        assert_eq!(resolve("192.0.2.1").unwrap(), vec!["192.0.2.1"]);
        assert_eq!(resolve("2001:db8::1").unwrap(), vec!["2001:db8::1"]);
    }
    #[test]
    fn loopback_resolves_without_network() {
        let v = resolve("localhost").unwrap();
        assert!(v.iter().any(|a| a == "127.0.0.1" || a == "::1"));
    }
}

//! Which network this computer is on right now: Wi-Fi name, link type,
//! signal, speed. Best effort, read from the system's own tools, and only
//! when the person has left "note the network" switched on.

use crate::process;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct NetInfo {
    /// "wifi", "wired" or "unknown"
    pub kind: String,
    pub ssid: Option<String>,
    pub iface: Option<String>,
    /// e.g. "Wi-Fi 6 (802.11ax) on 5 GHz, channel 44"
    pub link: Option<String>,
    /// e.g. "-52 dBm, strong" or "78%"
    pub signal: Option<String>,
    /// e.g. "866 Mbps"
    pub rate: Option<String>,
}

fn run(tool: &str, args: &[&str]) -> Option<String> {
    let out = process::quiet(tool).args(args).output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `key : value` lines (netsh, ipconfig getsummary) → the value for a key, case-insensitively.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if k.trim().eq_ignore_ascii_case(key) {
            Some(v.trim())
        } else {
            None
        }
    })
}

pub fn parse_netsh(text: &str) -> Option<NetInfo> {
    let ssid = field(text, "SSID")?.to_string();
    if ssid.is_empty() {
        return None;
    }
    let radio = field(text, "Radio type").map(|r| r.to_string());
    let band = field(text, "Band").map(|b| b.to_string());
    let channel = field(text, "Channel").map(|c| c.to_string());
    let link = match (radio, band, channel) {
        (Some(r), Some(b), Some(c)) => Some(format!("{r} on {b}, channel {c}")),
        (Some(r), _, Some(c)) => Some(format!("{r}, channel {c}")),
        (Some(r), _, _) => Some(r),
        _ => None,
    };
    Some(NetInfo {
        kind: "wifi".into(),
        ssid: Some(ssid),
        iface: field(text, "Name").map(String::from),
        link,
        signal: field(text, "Signal").map(String::from),
        rate: field(text, "Receive rate (Mbps)").map(|r| format!("{r} Mbps")),
    })
}

pub fn parse_nmcli(text: &str) -> Option<NetInfo> {
    // nmcli -t -f ACTIVE,SSID,SIGNAL,RATE,CHAN,FREQ dev wifi  →  yes:Maple-5G:78:866 Mbit/s:44:5220 MHz
    for l in text.lines() {
        let parts: Vec<&str> = l.split(':').collect();
        if parts.first() == Some(&"yes") && parts.len() >= 2 && !parts[1].is_empty() {
            let freq = parts.get(5).and_then(|f| f.split_whitespace().next()).and_then(|f| f.parse::<u32>().ok());
            let band = freq.map(|f| if f > 5900 { "6 GHz" } else if f > 4000 { "5 GHz" } else { "2.4 GHz" });
            let link = match (band, parts.get(4)) {
                (Some(b), Some(c)) if !c.is_empty() => Some(format!("Wi-Fi on {b}, channel {c}")),
                (Some(b), _) => Some(format!("Wi-Fi on {b}")),
                _ => None,
            };
            return Some(NetInfo {
                kind: "wifi".into(),
                ssid: Some(parts[1].to_string()),
                iface: None,
                link,
                signal: parts.get(2).filter(|s| !s.is_empty()).map(|s| format!("{s}%")),
                rate: parts.get(3).filter(|s| !s.is_empty()).map(|s| s.replace("Mbit/s", "Mbps")),
            });
        }
    }
    None
}

pub fn parse_mac_summary(text: &str) -> Option<NetInfo> {
    // `ipconfig getsummary en0` prints "  SSID : Maple-5G" among many lines
    let ssid = field(text, "SSID")?.to_string();
    if ssid.is_empty() || ssid == "<redacted>" {
        return None;
    }
    Some(NetInfo { kind: "wifi".into(), ssid: Some(ssid), iface: Some("en0".into()), ..Default::default() })
}

pub fn current() -> NetInfo {
    let found = if cfg!(windows) {
        run("netsh", &["wlan", "show", "interfaces"]).and_then(|t| parse_netsh(&t))
    } else if cfg!(target_os = "macos") {
        run("ipconfig", &["getsummary", "en0"]).and_then(|t| parse_mac_summary(&t)).or_else(|| {
            run("networksetup", &["-getairportnetwork", "en0"]).and_then(|t| {
                let name = t.split_once(':')?.1.trim().to_string();
                if name.is_empty() || t.contains("not associated") {
                    None
                } else {
                    Some(NetInfo { kind: "wifi".into(), ssid: Some(name), iface: Some("en0".into()), ..Default::default() })
                }
            })
        })
    } else {
        run("nmcli", &["-t", "-f", "ACTIVE,SSID,SIGNAL,RATE,CHAN,FREQ", "dev", "wifi"]).and_then(|t| parse_nmcli(&t)).or_else(|| {
            run("iwgetid", &["-r"]).and_then(|t| {
                let name = t.trim().to_string();
                if name.is_empty() {
                    None
                } else {
                    Some(NetInfo { kind: "wifi".into(), ssid: Some(name), ..Default::default() })
                }
            })
        })
    };
    found.unwrap_or(NetInfo { kind: "unknown".into(), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn netsh() {
        let t = "\r\nThere is 1 interface on the system:\r\n\r\n    Name                   : Wi-Fi\r\n    Description            : Intel(R) Wi-Fi 6 AX201\r\n    State                  : connected\r\n    SSID                   : Maple-5G\r\n    BSSID                  : aa:bb:cc:dd:ee:ff\r\n    Network type           : Infrastructure\r\n    Radio type             : 802.11ax\r\n    Band                   : 5 GHz\r\n    Channel                : 44\r\n    Receive rate (Mbps)    : 866\r\n    Transmit rate (Mbps)   : 866\r\n    Signal                 : 91%\r\n";
        let n = parse_netsh(t).unwrap();
        assert_eq!(n.ssid.as_deref(), Some("Maple-5G"));
        assert_eq!(n.link.as_deref(), Some("802.11ax on 5 GHz, channel 44"));
        assert_eq!(n.rate.as_deref(), Some("866 Mbps"));
        assert_eq!(n.signal.as_deref(), Some("91%"));
        assert_eq!(n.iface.as_deref(), Some("Wi-Fi"));
        assert!(parse_netsh("There is no wireless interface on the system.").is_none());
    }
    #[test]
    fn nmcli() {
        let t = "no:Other:40:130 Mbit/s:6:2437 MHz\nyes:Maple-5G:78:866 Mbit/s:44:5220 MHz\n";
        let n = parse_nmcli(t).unwrap();
        assert_eq!(n.ssid.as_deref(), Some("Maple-5G"));
        assert_eq!(n.link.as_deref(), Some("Wi-Fi on 5 GHz, channel 44"));
        assert_eq!(n.rate.as_deref(), Some("866 Mbps"));
        assert!(parse_nmcli("no:Other:40:130 Mbit/s:6:2437 MHz\n").is_none());
    }
    #[test]
    fn mac_summary() {
        let t = "<dictionary> {\n  BSSID : aa:bb\n  SSID : Maple-5G\n  Security : WPA2\n}\n";
        assert_eq!(parse_mac_summary(t).unwrap().ssid.as_deref(), Some("Maple-5G"));
    }
}

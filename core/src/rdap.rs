//! Registration records over RDAP, the successor to WHOIS.
//!
//! rdap.org is a public bootstrap service: it redirects `/ip/{addr}` and
//! `/domain/{name}` to whichever registry holds the record, and we follow
//! the redirect. Records are simplified to the handful of facts the details
//! pane shows.

use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

pub const UA: &str = concat!("hopscotch-trace/", env!("CARGO_PKG_VERSION"), " (+https://github.com/TidyBitz/hopscotch-trace)");

#[derive(Debug, Clone, Serialize, Default)]
pub struct IpRecord {
    pub handle: Option<String>,
    pub name: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub cidr: Option<String>,
    pub kind: Option<String>,
    pub country: Option<String>,
    pub org: Option<String>,
    pub registry: Option<String>,
    pub registered: Option<String>,
    pub changed: Option<String>,
    pub abuse: Option<String>,
    pub remarks: Vec<String>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DomainRecord {
    pub name: Option<String>,
    pub registrar: Option<String>,
    pub registered: Option<String>,
    pub expires: Option<String>,
    pub changed: Option<String>,
    pub status: Vec<String>,
    pub nameservers: Vec<String>,
    pub registrant: Option<String>,
    pub source: Option<String>,
}

fn get(url: &str) -> crate::Result<(Value, String)> {
    let r = ureq::get(url).set("Accept", "application/rdap+json, application/json").set("User-Agent", UA).timeout(Duration::from_secs(12)).call()?;
    let final_url = r.get_url().to_string();
    let v: Value = r.into_json()?;
    Ok((v, final_url))
}

fn registry_of(url: &str) -> Option<String> {
    let host = url.split("//").nth(1)?.split('/').next()?.to_lowercase();
    Some(
        if host.contains("arin.net") {
            "ARIN"
        } else if host.contains("ripe.net") {
            "RIPE NCC"
        } else if host.contains("apnic.net") {
            "APNIC"
        } else if host.contains("lacnic.net") {
            "LACNIC"
        } else if host.contains("afrinic.net") {
            "AFRINIC"
        } else {
            return Some(host);
        }
        .to_string(),
    )
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(|x| x.trim().to_string()).filter(|x| !x.is_empty())
}

fn event(v: &Value, action: &str) -> Option<String> {
    v.get("events")?.as_array()?.iter().find(|e| e.get("eventAction").and_then(Value::as_str) == Some(action)).and_then(|e| s(&e["eventDate"])).map(|d| d.chars().take(10).collect())
}

/// vCard lookups: entities carry `vcardArray: ["vcard", [[name, params, type, value], ...]]`.
fn vcard(entity: &Value, key: &str) -> Option<String> {
    let items = entity.get("vcardArray")?.as_array()?.get(1)?.as_array()?;
    items.iter().find(|i| i.get(0).and_then(Value::as_str) == Some(key)).and_then(|i| match i.get(3) {
        Some(Value::String(x)) => Some(x.clone()),
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ")),
        _ => None,
    }).filter(|x| !x.trim().is_empty())
}

fn has_role(entity: &Value, role: &str) -> bool {
    entity.get("roles").and_then(Value::as_array).map(|r| r.iter().any(|x| x.as_str() == Some(role))).unwrap_or(false)
}

/// Walk entities (and their nested entities) looking for the first that has a role and the vcard key.
fn find_entity(v: &Value, roles: &[&str], key: &str) -> Option<String> {
    let ents = v.get("entities")?.as_array()?;
    for role in roles {
        for e in ents {
            if has_role(e, role) {
                if let Some(x) = vcard(e, key) {
                    return Some(x);
                }
            }
        }
    }
    for e in ents {
        if let Some(x) = find_entity(e, roles, key) {
            return Some(x);
        }
    }
    None
}

pub fn simplify_ip(v: &Value, final_url: &str) -> IpRecord {
    let mut r = IpRecord {
        handle: s(&v["handle"]),
        name: s(&v["name"]),
        start: s(&v["startAddress"]),
        end: s(&v["endAddress"]),
        kind: s(&v["type"]),
        country: s(&v["country"]),
        registry: registry_of(final_url),
        registered: event(v, "registration"),
        changed: event(v, "last changed"),
        org: find_entity(v, &["registrant", "administrative", "technical"], "fn"),
        abuse: find_entity(v, &["abuse"], "email"),
        source: Some(final_url.to_string()),
        ..Default::default()
    };
    if let Some(c) = v.get("cidr0_cidrs").and_then(Value::as_array).and_then(|a| a.first()) {
        if let (Some(p), Some(l)) = (s(&c["v4prefix"]).or_else(|| s(&c["v6prefix"])), c["length"].as_u64()) {
            r.cidr = Some(format!("{p}/{l}"));
        }
    }
    if let Some(rem) = v.get("remarks").and_then(Value::as_array) {
        for m in rem.iter().take(3) {
            if let Some(d) = m.get("description").and_then(Value::as_array) {
                let text: Vec<&str> = d.iter().filter_map(Value::as_str).collect();
                if !text.is_empty() {
                    r.remarks.push(text.join(" "));
                }
            }
        }
    }
    r
}

pub fn simplify_domain(v: &Value, final_url: &str) -> DomainRecord {
    DomainRecord {
        name: s(&v["ldhName"]).or_else(|| s(&v["unicodeName"])).map(|x| x.to_lowercase()),
        registrar: find_entity(v, &["registrar"], "fn"),
        registrant: find_entity(v, &["registrant"], "fn"),
        registered: event(v, "registration"),
        expires: event(v, "expiration"),
        changed: event(v, "last changed"),
        status: v.get("status").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
        nameservers: v.get("nameservers").and_then(Value::as_array).map(|a| a.iter().filter_map(|n| s(&n["ldhName"])).map(|x| x.to_lowercase()).collect()).unwrap_or_default(),
        source: Some(final_url.to_string()),
    }
}

pub fn ip(addr: &str) -> crate::Result<IpRecord> {
    let (v, url) = get(&format!("https://rdap.org/ip/{addr}"))?;
    Ok(simplify_ip(&v, &url))
}

pub fn domain(name: &str) -> crate::Result<DomainRecord> {
    let (v, url) = get(&format!("https://rdap.org/domain/{}", name.trim_end_matches('.')))?;
    Ok(simplify_domain(&v, &url))
}

/// The registrable part of a host name: `www.example.co.uk` → `example.co.uk`.
/// A short public-suffix list covers the common second-level cases.
pub fn registrable(host: &str) -> String {
    let parts: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    if parts.len() <= 2 {
        return host.to_lowercase();
    }
    let tld = parts[parts.len() - 1];
    let sld = parts[parts.len() - 2];
    let deep = tld.len() == 2 && matches!(sld, "co" | "com" | "ac" | "edu" | "gov" | "net" | "org" | "or" | "ne" | "go");
    let take = if deep { 3 } else { 2 };
    parts[parts.len() - take..].join(".").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registrable_names() {
        assert_eq!(registrable("www.example.com"), "example.com");
        assert_eq!(registrable("example.com"), "example.com");
        assert_eq!(registrable("mail.example.co.uk"), "example.co.uk");
        assert_eq!(registrable("chinadaily.com.cn"), "chinadaily.com.cn");
        assert_eq!(registrable("a.b.c.osu.edu"), "osu.edu");
    }
    #[test]
    fn simplifies_ip_records() {
        let v: Value = serde_json::from_str(r#"{
          "handle":"NET-192-0-2-0-1","name":"TEST-NET-1","type":"ASSIGNMENT","startAddress":"192.0.2.0","endAddress":"192.0.2.255",
          "country":"US","cidr0_cidrs":[{"v4prefix":"192.0.2.0","length":24}],
          "events":[{"eventAction":"registration","eventDate":"2009-06-25T00:00:00-04:00"},{"eventAction":"last changed","eventDate":"2019-08-15T12:00:00Z"}],
          "entities":[{"handle":"IANA","roles":["registrant"],"vcardArray":["vcard",[["version",{},"text","4.0"],["fn",{},"text","Internet Assigned Numbers Authority"]]],
             "entities":[{"roles":["abuse"],"vcardArray":["vcard",[["fn",{},"text","Abuse"],["email",{},"text","abuse@iana.org"]]]}]}],
          "remarks":[{"description":["Documentation range","Never routed"]}]
        }"#).unwrap();
        let r = simplify_ip(&v, "https://rdap.arin.net/registry/ip/192.0.2.0");
        assert_eq!(r.cidr.as_deref(), Some("192.0.2.0/24"));
        assert_eq!(r.registry.as_deref(), Some("ARIN"));
        assert_eq!(r.registered.as_deref(), Some("2009-06-25"));
        assert_eq!(r.org.as_deref(), Some("Internet Assigned Numbers Authority"));
        assert_eq!(r.abuse.as_deref(), Some("abuse@iana.org"));
        assert_eq!(r.remarks, vec!["Documentation range Never routed"]);
    }
    #[test]
    fn simplifies_domain_records() {
        let v: Value = serde_json::from_str(r#"{
          "ldhName":"EXAMPLE.COM","status":["client delete prohibited"],
          "events":[{"eventAction":"registration","eventDate":"1995-08-14T04:00:00Z"},{"eventAction":"expiration","eventDate":"2027-08-13T04:00:00Z"}],
          "nameservers":[{"ldhName":"A.IANA-SERVERS.NET"},{"ldhName":"B.IANA-SERVERS.NET"}],
          "entities":[{"roles":["registrar"],"vcardArray":["vcard",[["fn",{},"text","Example Registrar"]]]}]
        }"#).unwrap();
        let d = simplify_domain(&v, "https://rdap.verisign.com/com/v1/domain/EXAMPLE.COM");
        assert_eq!(d.name.as_deref(), Some("example.com"));
        assert_eq!(d.registrar.as_deref(), Some("Example Registrar"));
        assert_eq!(d.expires.as_deref(), Some("2027-08-13"));
        assert_eq!(d.nameservers, vec!["a.iana-servers.net", "b.iana-servers.net"]);
    }
}

//! Where an address is, roughly, and whose network it belongs to.
//!
//! v0.1 asks ip-api.com, which answers batches of up to 100 addresses in one
//! request and needs no key. Its free tier is HTTP-only and for
//! non-commercial use (45 requests a minute), which suits a personal tool;
//! swap `PROVIDER` for a keyed HTTPS service if that changes. Private
//! addresses are never sent anywhere.

use crate::ipkind::{self, Kind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeoInfo {
    pub ip: String,
    pub kind: Option<Kind>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub isp: Option<String>,
    pub org: Option<String>,
    pub asn: Option<u32>,
    pub as_name: Option<String>,
    /// Set when the lookup service had nothing (or refused).
    pub error: Option<String>,
}

const PROVIDER: &str = "http://ip-api.com";
const FIELDS: &str = "status,message,country,countryCode,regionName,city,lat,lon,isp,org,as,asname,query";

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Raw {
    status: String,
    message: Option<String>,
    country: Option<String>,
    countryCode: Option<String>,
    regionName: Option<String>,
    city: Option<String>,
    lat: Option<f64>,
    lon: Option<f64>,
    isp: Option<String>,
    org: Option<String>,
    #[serde(rename = "as")]
    as_field: Option<String>,
    asname: Option<String>,
    query: Option<String>,
}

/// `"AS15169 Google LLC"` → `(Some(15169), Some("Google LLC"))`
pub fn split_as(s: &str) -> (Option<u32>, Option<String>) {
    let s = s.trim();
    let Some(rest) = s.strip_prefix("AS") else { return (None, Some(s.to_string()).filter(|x| !x.is_empty())) };
    let (num, name) = rest.split_once(' ').unwrap_or((rest, ""));
    (num.parse().ok(), Some(name.trim().to_string()).filter(|x| !x.is_empty()))
}

fn from_raw(r: Raw, fallback_ip: &str) -> GeoInfo {
    let ip = r.query.clone().unwrap_or_else(|| fallback_ip.to_string());
    if r.status != "success" {
        return GeoInfo { ip, kind: Some(ipkind::classify_str(fallback_ip)), error: Some(r.message.unwrap_or_else(|| "no answer".into())), ..Default::default() };
    }
    let (asn, as_name) = r.as_field.as_deref().map(split_as).unwrap_or((None, None));
    let clean = |s: Option<String>| s.filter(|x| !x.trim().is_empty());
    GeoInfo {
        kind: Some(ipkind::classify_str(&ip)),
        ip,
        lat: r.lat,
        lon: r.lon,
        city: clean(r.city),
        region: clean(r.regionName),
        country: clean(r.country),
        country_code: clean(r.countryCode),
        isp: clean(r.isp),
        org: clean(r.org),
        asn,
        as_name: clean(r.asname).or(as_name),
        error: None,
    }
}

/// Look up a set of addresses. Non-public ones come back immediately with
/// their kind and no location; the rest go to the provider in one batch.
pub fn lookup(ips: &[String]) -> crate::Result<Vec<GeoInfo>> {
    let mut out: Vec<GeoInfo> = Vec::with_capacity(ips.len());
    let mut ask: Vec<String> = Vec::new();
    for ip in ips {
        let kind = ipkind::classify_str(ip);
        if kind.is_public() {
            if !ask.contains(ip) {
                ask.push(ip.clone());
            }
        } else {
            out.push(GeoInfo { ip: ip.clone(), kind: Some(kind), ..Default::default() });
        }
    }
    for chunk in ask.chunks(100) {
        let body: Vec<serde_json::Value> = chunk.iter().map(|ip| serde_json::json!({ "query": ip, "fields": FIELDS })).collect();
        let resp = ureq::post(&format!("{PROVIDER}/batch")).set("User-Agent", crate::rdap::UA).timeout(std::time::Duration::from_secs(10)).send_json(serde_json::Value::Array(body));
        match resp {
            Ok(r) => {
                let raws: Vec<Raw> = r.into_json()?;
                for (raw, ip) in raws.into_iter().zip(chunk.iter()) {
                    out.push(from_raw(raw, ip));
                }
            }
            Err(e) => {
                for ip in chunk {
                    out.push(GeoInfo { ip: ip.clone(), kind: Some(Kind::Public), error: Some(e.to_string()), ..Default::default() });
                }
            }
        }
    }
    // keep the caller's order
    let mut ordered = Vec::with_capacity(ips.len());
    for ip in ips {
        if let Some(pos) = out.iter().position(|g| &g.ip == ip) {
            ordered.push(out[pos].clone());
        }
    }
    Ok(ordered)
}

/// Where this computer appears to be, from its public address.
pub fn lookup_self() -> crate::Result<GeoInfo> {
    let r: Raw = ureq::get(&format!("{PROVIDER}/json/?fields={FIELDS}")).set("User-Agent", crate::rdap::UA).timeout(std::time::Duration::from_secs(10)).call()?.into_json()?;
    Ok(from_raw(r, ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn as_splitting() {
        assert_eq!(split_as("AS15169 Google LLC"), (Some(15169), Some("Google LLC".into())));
        assert_eq!(split_as("AS64496"), (Some(64496), None));
        assert_eq!(split_as(""), (None, None));
        assert_eq!(split_as("Some ISP"), (None, Some("Some ISP".into())));
    }
    #[test]
    fn raw_conversion() {
        let raw: Raw = serde_json::from_str(r#"{"status":"success","country":"United States","countryCode":"US","regionName":"Virginia","city":"Ashburn","lat":39.04,"lon":-77.49,"isp":"Example ISP","org":"","as":"AS64496 Example Net","asname":"EXAMPLE","query":"192.0.2.1"}"#).unwrap();
        let g = from_raw(raw, "192.0.2.1");
        assert_eq!(g.city.as_deref(), Some("Ashburn"));
        assert_eq!(g.asn, Some(64496));
        assert_eq!(g.as_name.as_deref(), Some("EXAMPLE"));
        assert!(g.org.is_none());
        let raw: Raw = serde_json::from_str(r#"{"status":"fail","message":"private range","query":"10.0.0.1"}"#).unwrap();
        let g = from_raw(raw, "10.0.0.1");
        assert_eq!(g.error.as_deref(), Some("private range"));
    }
    #[test]
    fn private_addresses_never_leave() {
        let v = lookup(&["192.168.1.1".into(), "10.1.1.1".into()]).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].kind, Some(Kind::Private));
        assert!(v[0].lat.is_none());
    }
}

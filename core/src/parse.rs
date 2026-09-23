//! Parsers for the output of the system traceroute tools.
//!
//! Three formats are understood:
//! * Windows `tracert -d`:   `  3    12 ms    11 ms    12 ms  100.64.8.1`
//! * BSD/Linux `traceroute -n`: ` 3  100.64.8.1  12.101 ms  11.9 ms  12.0 ms`
//! * Linux `tracepath -n`:  ` 3:  100.64.8.1    12.3ms asymm 4` (one line per probe)
//!
//! Every parser turns a line into a [`HopLine`], and the caller merges the
//! lines of one hop (tracepath prints several) with [`merge`].

use serde::Serialize;
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct HopLine {
    pub hop: u8,
    /// Addresses that answered, in the order they appeared (a hop can answer
    /// from more than one address when the path is load-balanced).
    pub addrs: Vec<String>,
    /// One entry per probe: the round trip in milliseconds, or `None` for `*`.
    pub rtts: Vec<Option<f64>>,
    /// Any annotation the tool printed, like `!H` or `!X`.
    pub note: Option<String>,
}

impl HopLine {
    pub fn answered(&self) -> bool {
        !self.addrs.is_empty()
    }
    pub fn ip(&self) -> Option<&str> {
        self.addrs.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Tracert,
    Traceroute,
    Tracepath,
}

fn is_ip(s: &str) -> bool {
    s.parse::<IpAddr>().is_ok()
}

/// `12.345` → Some(12.345); `<1` → Some(0.5)
fn ms(s: &str) -> Option<f64> {
    let s = s.trim().trim_end_matches("ms").trim();
    if let Some(rest) = s.strip_prefix('<') {
        return rest.trim().parse::<f64>().ok().map(|v| v / 2.0);
    }
    s.parse::<f64>().ok()
}

pub fn parse_line(line: &str, flavor: Flavor) -> Option<HopLine> {
    match flavor {
        Flavor::Tracert => parse_tracert(line),
        Flavor::Traceroute => parse_traceroute(line),
        Flavor::Tracepath => parse_tracepath(line),
    }
}

/// Windows tracert. Times are printed *before* the address, as `12 ms` or `<1 ms`
/// or `*`. A hop that never answers ends with `Request timed out.`
pub fn parse_tracert(line: &str) -> Option<HopLine> {
    let t = line.trim();
    let mut it = t.split_whitespace();
    let hop: u8 = it.next()?.parse().ok()?;
    let rest: Vec<&str> = it.collect();
    if rest.is_empty() {
        return None;
    }
    let mut out = HopLine { hop, ..Default::default() };
    let mut i = 0;
    while i < rest.len() {
        let tok = rest[i];
        if tok == "*" {
            out.rtts.push(None);
            i += 1;
        } else if let Some(v) = tracert_time(&rest, i) {
            out.rtts.push(Some(v.0));
            i += v.1;
        } else {
            break;
        }
    }
    // whatever is left is the address (with -d) or "name [ip]" (without -d)
    let tail = rest[i..].join(" ");
    if let Some(ip) = tail_ip(&tail) {
        out.addrs.push(ip);
    }
    if out.rtts.is_empty() && out.addrs.is_empty() {
        return None;
    }
    Some(out)
}

/// `12 ms` (two tokens), `<1 ms` (two tokens) or `12ms` (one token)
fn tracert_time(rest: &[&str], i: usize) -> Option<(f64, usize)> {
    let tok = rest[i];
    if let Some(v) = tok.strip_suffix("ms") {
        return ms(v).map(|x| (x, 1));
    }
    if rest.get(i + 1).map(|s| *s == "ms").unwrap_or(false) {
        return ms(tok).map(|x| (x, 2));
    }
    None
}

/// Find the address at the end of a tracert line: `100.64.8.1`, or `host [1.2.3.4]`.
fn tail_ip(tail: &str) -> Option<String> {
    if let Some(start) = tail.rfind('[') {
        let end = tail[start..].find(']')? + start;
        let ip = &tail[start + 1..end];
        if is_ip(ip) {
            return Some(ip.to_string());
        }
    }
    let last = tail.split_whitespace().last()?;
    if is_ip(last) {
        Some(last.to_string())
    } else {
        None
    }
}

/// BSD and Linux traceroute with -n. Addresses come first and may change mid-line.
pub fn parse_traceroute(line: &str) -> Option<HopLine> {
    let t = line.trim();
    let mut it = t.split_whitespace();
    let hop: u8 = it.next()?.parse().ok()?;
    let rest: Vec<&str> = it.collect();
    if rest.is_empty() {
        return None;
    }
    let mut out = HopLine { hop, ..Default::default() };
    let mut i = 0;
    while i < rest.len() {
        let tok = rest[i];
        if tok == "*" {
            out.rtts.push(None);
            i += 1;
        } else if is_ip(tok) {
            if !out.addrs.iter().any(|a| a == tok) {
                out.addrs.push(tok.to_string());
            }
            i += 1;
        } else if tok.starts_with('(') && tok.ends_with(')') && is_ip(&tok[1..tok.len() - 1]) {
            // without -n: "name (ip)"; keep only the ip and drop the name we pushed
            let ip = &tok[1..tok.len() - 1];
            out.addrs.retain(|a| is_ip(a));
            if !out.addrs.iter().any(|a| a == ip) {
                out.addrs.push(ip.to_string());
            }
            i += 1;
        } else if let Some(v) = tok.strip_suffix("ms") {
            if let Some(x) = ms(v) {
                out.rtts.push(Some(x));
            }
            i += 1;
        } else if rest.get(i + 1).map(|s| *s == "ms").unwrap_or(false) {
            if let Some(x) = ms(tok) {
                out.rtts.push(Some(x));
            }
            i += 2;
        } else if tok.starts_with('!') {
            out.note = Some(tok.to_string());
            i += 1;
        } else if tok.contains('.') && !tok.starts_with('[') {
            // a host name printed without -n; remember it only if no ip yet
            i += 1;
        } else {
            i += 1;
        }
    }
    if out.rtts.is_empty() && out.addrs.is_empty() {
        return None;
    }
    Some(out)
}

/// Linux tracepath: one line per probe, `N:  addr   12.3ms [asymm N]`, or `N:  no reply`.
pub fn parse_tracepath(line: &str) -> Option<HopLine> {
    let t = line.trim();
    let (num, rest) = t.split_once(':')?;
    let num = num.trim_end_matches('?').trim();
    let hop: u8 = num.parse().ok()?;
    let rest = rest.trim();
    if rest.starts_with("[LOCALHOST]") {
        return None;
    }
    let mut out = HopLine { hop, ..Default::default() };
    if rest.starts_with("no reply") {
        out.rtts.push(None);
        return Some(out);
    }
    let mut toks = rest.split_whitespace();
    let addr = toks.next()?;
    let addr = addr.trim_matches(|c| c == '(' || c == ')');
    if is_ip(addr) {
        out.addrs.push(addr.to_string());
    } else if let Some(p) = rest.find('(') {
        let e = rest[p..].find(')').map(|x| x + p)?;
        let ip = &rest[p + 1..e];
        if is_ip(ip) {
            out.addrs.push(ip.to_string());
        }
    }
    for tok in toks {
        if let Some(v) = tok.strip_suffix("ms") {
            if let Some(x) = ms(v) {
                out.rtts.push(Some(x));
            }
        } else if tok.starts_with('!') {
            out.note = Some(tok.to_string());
        }
    }
    if out.addrs.is_empty() && out.rtts.is_empty() {
        return None;
    }
    if out.rtts.is_empty() {
        // tracepath prints "pmtu" or "reached" lines with no time
        return if out.addrs.is_empty() { None } else { Some(out) };
    }
    Some(out)
}

/// Fold another line for the same hop into `into` (tracepath prints one line per probe).
pub fn merge(into: &mut HopLine, more: HopLine) {
    for a in more.addrs {
        if !into.addrs.contains(&a) {
            into.addrs.push(a);
        }
    }
    into.rtts.extend(more.rtts);
    if into.note.is_none() {
        into.note = more.note;
    }
}

/// The destination address a tool announced on its first line, e.g.
/// `Tracing route to example.com [93.184.216.34]` or `traceroute to example.com (93.184.216.34), 30 hops max`.
pub fn header_ip(line: &str) -> Option<String> {
    let open = line.find(['[', '(']);
    let start = open? + 1;
    let end = line[start..].find([']', ')'])? + start;
    let ip = &line[start..end];
    if is_ip(ip) {
        Some(ip.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracert_lines() {
        let h = parse_tracert("  1     1 ms     1 ms    <1 ms  192.168.1.1").unwrap();
        assert_eq!(h.hop, 1);
        assert_eq!(h.addrs, vec!["192.168.1.1"]);
        assert_eq!(h.rtts, vec![Some(1.0), Some(1.0), Some(0.5)]);

        let h = parse_tracert("  2     *        *        *     Request timed out.").unwrap();
        assert_eq!(h.hop, 2);
        assert!(h.addrs.is_empty());
        assert_eq!(h.rtts, vec![None, None, None]);

        let h = parse_tracert("  3    12 ms     *       13 ms  100.64.8.1").unwrap();
        assert_eq!(h.rtts, vec![Some(12.0), None, Some(13.0)]);
        assert_eq!(h.ip(), Some("100.64.8.1"));

        // without -d the name and bracketed address are printed
        let h = parse_tracert(" 12    33 ms    34 ms    33 ms  example.com [93.184.216.34]").unwrap();
        assert_eq!(h.addrs, vec!["93.184.216.34"]);

        assert!(parse_tracert("Tracing route to example.com [93.184.216.34]").is_none());
        assert!(parse_tracert("over a maximum of 30 hops:").is_none());
        assert!(parse_tracert("Trace complete.").is_none());
        assert!(parse_tracert("").is_none());
    }

    #[test]
    fn traceroute_lines() {
        let h = parse_traceroute(" 1  192.168.1.1  1.234 ms  1.100 ms  0.998 ms").unwrap();
        assert_eq!(h.hop, 1);
        assert_eq!(h.addrs, vec!["192.168.1.1"]);
        assert_eq!(h.rtts, vec![Some(1.234), Some(1.1), Some(0.998)]);

        let h = parse_traceroute(" 2  * * *").unwrap();
        assert_eq!(h.rtts, vec![None, None, None]);
        assert!(!h.answered());

        // load-balanced: two addresses on one line
        let h = parse_traceroute(" 3  100.64.8.1  12.1 ms 100.64.8.2  11.9 ms  12.0 ms").unwrap();
        assert_eq!(h.addrs, vec!["100.64.8.1", "100.64.8.2"]);
        assert_eq!(h.rtts.len(), 3);

        let h = parse_traceroute(" 4  10.0.0.1  5.0 ms !H  5.1 ms !H  5.2 ms !H").unwrap();
        assert_eq!(h.note.as_deref(), Some("!H"));
        assert_eq!(h.rtts.len(), 3);

        // without -n
        let h = parse_traceroute(" 5  core1.example.net (203.0.113.7)  20.1 ms  20.4 ms  19.8 ms").unwrap();
        assert_eq!(h.addrs, vec!["203.0.113.7"]);

        // macOS puts a * in the middle
        let h = parse_traceroute("13  93.184.216.34  33.552 ms *  33.101 ms").unwrap();
        assert_eq!(h.rtts, vec![Some(33.552), None, Some(33.101)]);

        // IPv6
        let h = parse_traceroute(" 2  2001:4860:4860::8888  9.1 ms  9.0 ms  9.3 ms").unwrap();
        assert_eq!(h.addrs, vec!["2001:4860:4860::8888"]);

        assert!(parse_traceroute("traceroute to example.com (93.184.216.34), 30 hops max, 60 byte packets").is_none());
    }

    #[test]
    fn tracepath_lines() {
        assert!(parse_tracepath(" 1?: [LOCALHOST]                      pmtu 1500").is_none());
        let a = parse_tracepath(" 1:  192.168.1.1                         1.038ms ").unwrap();
        let b = parse_tracepath(" 1:  192.168.1.1                         0.845ms ").unwrap();
        let mut m = a;
        merge(&mut m, b);
        assert_eq!(m.addrs, vec!["192.168.1.1"]);
        assert_eq!(m.rtts, vec![Some(1.038), Some(0.845)]);

        let n = parse_tracepath(" 2:  no reply").unwrap();
        assert_eq!(n.rtts, vec![None]);

        let c = parse_tracepath(" 3:  100.64.8.1                          12.3ms asymm  4").unwrap();
        assert_eq!(c.addrs, vec!["100.64.8.1"]);
        assert_eq!(c.rtts, vec![Some(12.3)]);

        let d = parse_tracepath(" 9:  93.184.216.34                       33.9ms reached").unwrap();
        assert_eq!(d.hop, 9);
        assert_eq!(d.rtts, vec![Some(33.9)]);
        assert!(parse_tracepath("     Resume: pmtu 1500 hops 9 back 9").is_none());
    }

    #[test]
    fn headers() {
        assert_eq!(header_ip("Tracing route to example.com [93.184.216.34]").as_deref(), Some("93.184.216.34"));
        assert_eq!(header_ip("traceroute to example.com (93.184.216.34), 30 hops max, 60 byte packets").as_deref(), Some("93.184.216.34"));
        assert_eq!(header_ip("traceroute to 2606:4700::1111 (2606:4700::1111), 30 hops max").as_deref(), Some("2606:4700::1111"));
        assert!(header_ip("Trace complete.").is_none());
    }
}

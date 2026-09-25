//! Tracing a route and streaming its hops, as events.
//!
//! On Linux the probes are our own (see `native`): UDP datagrams with a rising
//! TTL, and a TCP handshake to find a destination that drops UDP. Elsewhere
//! we run the system tool (`tracert` uses the Windows ICMP API, macOS's
//! `traceroute` is setuid), read its output line by line, and turn each line
//! into an event; `run_auto` picks. BACKEND.md has the plan for the rest.

use crate::parse::{self, Flavor, HopLine};
use crate::process;
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize)]
pub struct TraceOpts {
    pub max_hops: u8,
    pub wait_ms: u32,
    pub queries: u8,
    /// Stop once this many hops in a row have not answered, after at least
    /// one hop has. A destination that drops probes never ends the trace on
    /// its own, so without this the tool runs on to `max_hops`, mostly
    /// printing `* * *`. 0 disables it.
    pub give_up_after: u8,
}

impl Default for TraceOpts {
    fn default() -> Self {
        Self { max_hops: 30, wait_ms: 1500, queries: 3, give_up_after: 5 }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEvent {
    /// The tool started and told us the address it is tracing to.
    Started { tool: String, dest: Option<String> },
    /// One hop finished (all its probes came back or timed out).
    Hop(HopLine),
    /// The tool exited. `reached` is true when the last hop is the destination;
    /// `gave_up` when we stopped it after `give_up_after` silent hops.
    Done { reached: bool, hops: u8, gave_up: bool },
    /// The tool could not be run, or exited with an error before any hop.
    Error { message: String },
}

/// Something the caller keeps so it can stop a trace early.
#[derive(Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    child: Arc<Mutex<Option<u32>>>,
}

impl Cancel {
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.child.lock().unwrap() {
            kill(pid);
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

fn kill(pid: u32) {
    #[cfg(windows)]
    {
        let _ = process::quiet("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).status();
    }
    #[cfg(not(windows))]
    {
        let _ = process::quiet("kill").args(["-TERM", &pid.to_string()]).status();
    }
}

/// Which tool this platform will use, and how it is invoked.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub program: String,
    pub args: Vec<String>,
    pub flavor: String,
}

pub fn pick_tool(target: &str, opts: &TraceOpts) -> Result<(Tool, Flavor), String> {
    let wait_s = ((opts.wait_ms + 999) / 1000).max(1).to_string();
    if cfg!(windows) {
        return Ok((
            Tool {
                program: "tracert".into(),
                args: vec!["-d".into(), "-w".into(), opts.wait_ms.to_string(), "-h".into(), opts.max_hops.to_string(), target.into()],
                flavor: "tracert".into(),
            },
            Flavor::Tracert,
        ));
    }
    if process::available("traceroute") {
        let program = process::locate("traceroute");
        let mut args = vec!["-n".into(), "-q".into(), opts.queries.to_string(), "-w".into(), wait_s, "-m".into(), opts.max_hops.to_string()];
        // GNU traceroute buffers stdout when it is a pipe; stdbuf makes it line-buffered so hops stream in
        let (program, args) = if cfg!(target_os = "linux") && process::available("stdbuf") {
            let mut a = vec!["-oL".into(), program];
            a.extend(args.drain(..));
            ("stdbuf".to_string(), a)
        } else {
            (program, args)
        };
        let mut args = args;
        args.push(target.into());
        return Ok((Tool { program, args, flavor: "traceroute".into() }, Flavor::Traceroute));
    }
    if process::available("tracepath") {
        let mut args = vec!["-n".to_string(), "-m".into(), opts.max_hops.to_string()];
        args.push(target.into());
        return Ok((Tool { program: "tracepath".into(), args, flavor: "tracepath".into() }, Flavor::Tracepath));
    }
    Err("No traceroute tool found. Install the traceroute package (Debian/Ubuntu: sudo apt install traceroute; Fedora: sudo dnf install traceroute) or iputils for tracepath.".into())
}

/// Run a trace, calling `emit` for every event. Blocks until the tool exits or
/// `cancel` fires, so call it from a worker thread.
pub fn run<F: FnMut(TraceEvent)>(target: &str, opts: &TraceOpts, cancel: &Cancel, mut emit: F) {
    let (tool, flavor) = match pick_tool(target, opts) {
        Ok(t) => t,
        Err(message) => {
            emit(TraceEvent::Error { message });
            return;
        }
    };
    let mut cmd = process::quiet(&tool.program);
    cmd.args(&tool.args);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            emit(TraceEvent::Error { message: format!("Couldn't start {}: {}", tool.program, e) });
            return;
        }
    };
    *cancel.child.lock().unwrap() = Some(child.id());
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take();
    let reader = BufReader::new(stdout);
    let mut dest: Option<String> = None;
    let mut started = false;
    let mut pending: Option<HopLine> = None; // tracepath: lines of the hop being assembled
    let mut hops: u8 = 0;
    let mut last: Option<HopLine> = None;
    let mut answered = false;
    let mut silent_run: u8 = 0;
    let mut gave_up = false;
    macro_rules! note_hop {
        ($h:expr) => {{
            let h: HopLine = $h;
            hops = hops.max(h.hop);
            if h.addrs.is_empty() {
                silent_run = silent_run.saturating_add(1);
            } else {
                answered = true;
                silent_run = 0;
            }
            last = Some(h.clone());
            emit(TraceEvent::Hop(h));
        }};
    }

    for line in reader.lines() {
        if cancel.is_cancelled() {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if !started {
            if let Some(ip) = parse::header_ip(&line) {
                dest = Some(ip);
            }
            // tracert prints a blank line and "over a maximum" before hops; traceroute's header is one line
            if parse::parse_line(&line, flavor).is_none() {
                if dest.is_some() || !line.trim().is_empty() {
                    started = true;
                    emit(TraceEvent::Started { tool: tool.flavor.clone(), dest: dest.clone() });
                }
                continue;
            }
            started = true;
            emit(TraceEvent::Started { tool: tool.flavor.clone(), dest: dest.clone() });
        }
        let Some(h) = parse::parse_line(&line, flavor) else { continue };
        match flavor {
            Flavor::Tracepath => {
                match pending.take() {
                    Some(mut p) if p.hop == h.hop => {
                        parse::merge(&mut p, h);
                        pending = Some(p);
                    }
                    Some(p) => {
                        note_hop!(p);
                        pending = Some(h);
                    }
                    None => pending = Some(h),
                }
            }
            _ => note_hop!(h),
        }
        if answered && opts.give_up_after > 0 && silent_run >= opts.give_up_after {
            gave_up = true;
            if let Some(pid) = *cancel.child.lock().unwrap() {
                kill(pid);
            }
            break;
        }
    }
    if let Some(p) = pending.take() {
        note_hop!(p);
    }
    let status = child.wait();
    *cancel.child.lock().unwrap() = None;
    if cancel.is_cancelled() {
        return;
    }
    if !started {
        // nothing parsed at all: report whatever the tool said on stderr
        let mut err = String::new();
        if let Some(mut e) = stderr {
            use std::io::Read;
            let _ = e.read_to_string(&mut err);
        }
        let err = err.trim();
        let message = if err.is_empty() {
            match status {
                Ok(s) if !s.success() => format!("{} exited with {}", tool.program, s),
                _ => format!("{} printed nothing", tool.program),
            }
        } else {
            friendly(err)
        };
        emit(TraceEvent::Error { message });
        return;
    }
    let reached = match (&dest, &last) {
        (Some(d), Some(h)) => h.addrs.iter().any(|a| a == d),
        _ => false,
    };
    emit(TraceEvent::Done { reached, hops, gave_up });
}

/// Which engine `run_auto` will use on this machine.
pub fn engine_name(prefer_native: bool) -> &'static str {
    #[cfg(target_os = "linux")]
    if prefer_native && crate::native::available() {
        return "native";
    }
    let _ = prefer_native;
    "tool"
}

#[cfg(target_os = "linux")]
fn native_dest(target: &str, prefer_native: bool) -> Option<std::net::Ipv4Addr> {
    if !prefer_native || !crate::native::available() {
        return None;
    }
    crate::dns::resolve(target).ok()?.into_iter().find_map(|s| s.parse().ok())
}

/// Trace with our own probes where they work (Linux, an IPv4 target), otherwise with the system tool.
pub fn run_auto<F: FnMut(TraceEvent)>(target: &str, opts: &TraceOpts, cancel: &Cancel, prefer_native: bool, emit: F) {
    #[cfg(target_os = "linux")]
    if let Some(dest) = native_dest(target, prefer_native) {
        return run_native(dest, opts, cancel, emit);
    }
    let _ = prefer_native;
    run(target, opts, cancel, emit)
}

#[cfg(target_os = "linux")]
fn push_addr(line: &mut HopLine, a: std::net::Ipv4Addr) {
    let s = a.to_string();
    if !line.addrs.contains(&s) {
        line.addrs.push(s);
    }
}

/// Trace with native probes: `queries` UDP probes per TTL in parallel, then,
/// when the destination never answers UDP, a TCP handshake on 443 to prove it
/// is up and a search over the TTL to find how far away it is.
#[cfg(target_os = "linux")]
pub fn run_native<F: FnMut(TraceEvent)>(dest: std::net::Ipv4Addr, opts: &TraceOpts, cancel: &Cancel, mut emit: F) {
    use crate::native::{self, Reply};
    use std::time::Duration;
    let timeout = Duration::from_millis(opts.wait_ms.max(100) as u64);
    let queries = opts.queries.clamp(1, 8) as usize;
    let max_hops = opts.max_hops.max(1);
    emit(TraceEvent::Started { tool: "native".into(), dest: Some(dest.to_string()) });
    let (mut hops, mut answered, mut silent_run, mut reached, mut gave_up) = (0u8, false, 0u8, false, false);
    let mut dead_end = false;
    for ttl in 1..=max_hops {
        if cancel.is_cancelled() {
            return;
        }
        let replies: Vec<std::io::Result<Reply>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..queries)
                .map(|q| {
                    // one port per probe, the way traceroute numbers them
                    let port = 33434u16.wrapping_add((ttl as u16).wrapping_sub(1).wrapping_mul(queries as u16)).wrapping_add(q as u16);
                    s.spawn(move || native::probe_udp(dest, ttl, port, timeout))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(std::io::Error::other("probe thread panicked")))).collect()
        });
        if cancel.is_cancelled() {
            return;
        }
        let mut line = HopLine { hop: ttl, ..Default::default() };
        let (mut here, mut errors, mut unreachable) = (false, 0, 0);
        for r in replies {
            match r {
                Ok(Reply::Expired { from, rtt }) => {
                    if let Some(a) = from {
                        push_addr(&mut line, a);
                        // a load balancer or host that lets the TTL run out on the way in still answers from the destination's address
                        if a == dest {
                            here = true;
                        }
                    }
                    line.rtts.push(Some(rtt));
                }
                Ok(Reply::Reached { from, rtt }) => {
                    push_addr(&mut line, from);
                    line.rtts.push(Some(rtt));
                    here = true;
                }
                Ok(Reply::Unreachable { from, rtt, note }) => {
                    if let Some(a) = from {
                        push_addr(&mut line, a);
                        if a == dest {
                            here = true;
                        }
                    }
                    line.rtts.push(Some(rtt));
                    line.note = Some(note);
                    unreachable += 1;
                }
                Ok(Reply::Silent) => line.rtts.push(None),
                Err(e) => {
                    errors += 1;
                    line.rtts.push(None);
                    if ttl == 1 && errors == queries {
                        emit(TraceEvent::Error { message: friendly(&e.to_string()) });
                        return;
                    }
                }
            }
        }
        hops = hops.max(ttl);
        if line.addrs.is_empty() {
            silent_run = silent_run.saturating_add(1);
        } else {
            answered = true;
            silent_run = 0;
        }
        emit(TraceEvent::Hop(line));
        if here {
            reached = true;
            break;
        }
        // "host unreachable" and friends from most probes: this router will never pass us on, as traceroute also concludes
        if unreachable > 0 && unreachable >= queries.saturating_sub(1).max(1) {
            dead_end = true;
            break;
        }
        if answered && opts.give_up_after > 0 && silent_run >= opts.give_up_after {
            gave_up = true;
            break;
        }
    }
    if !reached && !dead_end && answered && !cancel.is_cancelled() {
        // The destination drops UDP. A handshake on 443 shows whether it is up at all, and how far away.
        let known = hops.saturating_sub(silent_run);
        if let Some((dist, rtts)) = native::tcp_distance(dest, 443, known, max_hops, timeout) {
            for t in (hops + 1)..dist {
                emit(TraceEvent::Hop(HopLine { hop: t, rtts: vec![None; queries], ..Default::default() }));
            }
            emit(TraceEvent::Hop(HopLine { hop: dist, addrs: vec![dest.to_string()], rtts, note: Some("tcp/443".into()) }));
            hops = dist;
            reached = true;
            gave_up = false;
        }
    }
    emit(TraceEvent::Done { reached, hops, gave_up });
}

/// Turn the tool's stderr into something a person can act on.
fn friendly(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("network is unreachable") || e.contains("network unreachable") {
        "No route to that address. Is the network up?".into()
    } else if e.contains("unknown host") || e.contains("name or service not known") || e.contains("could not resolve") || e.contains("cannot resolve") || e.contains("temporary failure in name resolution") {
        "That name didn't resolve to an address.".into()
    } else if e.contains("operation not permitted") || e.contains("permission denied") || e.contains("must be root") {
        "The traceroute tool needs more rights than it has. On Linux, install the traceroute package (it works without root) rather than relying on a raw-socket build.".into()
    } else {
        err.lines().next().unwrap_or(err).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friendly_messages() {
        assert!(friendly("traceroute: unknown host nowhere.invalid").contains("didn't resolve"));
        assert!(friendly("traceroute: socket: Operation not permitted").contains("rights"));
        assert_eq!(friendly("something odd\nsecond line"), "something odd");
    }
    #[test]
    fn defaults_give_up_on_silence() {
        let o = TraceOpts::default();
        assert!(o.give_up_after > 0 && o.give_up_after < o.max_hops);
    }
    #[test]
    fn tool_choice_is_sane() {
        let (t, _) = pick_tool("example.org", &TraceOpts::default()).unwrap_or_else(|_| {
            (Tool { program: "none".into(), args: vec![], flavor: "none".into() }, Flavor::Tracepath)
        });
        assert!(t.args.is_empty() || t.args.last().map(|a| a == "example.org").unwrap_or(false));
    }
}

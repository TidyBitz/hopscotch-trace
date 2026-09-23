//! Running the system traceroute tool and streaming its hops.
//!
//! Hopscotch does not send its own probes in v0.1: raw ICMP sockets need
//! administrator rights on most systems, while the traceroute tools that ship
//! with every OS already have the rights they need (`tracert` uses the Windows
//! ICMP API, macOS's `traceroute` is setuid, Linux's works unprivileged over
//! UDP). So we run the tool, read its output line by line, and turn each line
//! into an event. BACKEND.md discusses a native implementation for later.

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
}

impl Default for TraceOpts {
    fn default() -> Self {
        Self { max_hops: 30, wait_ms: 1500, queries: 3 }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraceEvent {
    /// The tool started and told us the address it is tracing to.
    Started { tool: String, dest: Option<String> },
    /// One hop finished (all its probes came back or timed out).
    Hop(HopLine),
    /// The tool exited. `reached` is true when the last hop is the destination.
    Done { reached: bool, hops: u8 },
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
                        hops = hops.max(p.hop);
                        last = Some(p.clone());
                        emit(TraceEvent::Hop(p));
                        pending = Some(h);
                    }
                    None => pending = Some(h),
                }
            }
            _ => {
                hops = hops.max(h.hop);
                last = Some(h.clone());
                emit(TraceEvent::Hop(h));
            }
        }
    }
    if let Some(p) = pending.take() {
        hops = hops.max(p.hop);
        last = Some(p.clone());
        emit(TraceEvent::Hop(p));
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
    emit(TraceEvent::Done { reached, hops });
}

/// Turn the tool's stderr into something a person can act on.
fn friendly(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("unknown host") || e.contains("name or service not known") || e.contains("could not resolve") || e.contains("cannot resolve") || e.contains("temporary failure in name resolution") {
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
    fn tool_choice_is_sane() {
        let (t, _) = pick_tool("example.org", &TraceOpts::default()).unwrap_or_else(|_| {
            (Tool { program: "none".into(), args: vec![], flavor: "none".into() }, Flavor::Tracepath)
        });
        assert!(t.args.is_empty() || t.args.last().map(|a| a == "example.org").unwrap_or(false));
    }
}

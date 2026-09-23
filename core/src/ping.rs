//! Continuous probing of every hop, using the system `ping`.
//!
//! One short-lived `ping` per hop per tick is portable and needs no rights.
//! It costs a process spawn per probe, which is fine at one probe a second for
//! a dozen hops; a native ICMP implementation is the obvious upgrade
//! (see BACKEND.md).

use crate::process;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub ip: String,
    /// Round trip in milliseconds, or `None` when the probe was lost.
    pub ms: Option<f64>,
    /// When the probe was sent, in milliseconds since the Unix epoch.
    pub t: u64,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Send one echo request and wait up to `timeout_ms` for the answer.
pub fn ping_once(ip: &str, timeout_ms: u32) -> Option<f64> {
    let mut cmd = process::quiet("ping");
    if cfg!(windows) {
        cmd.args(["-n", "1", "-w", &timeout_ms.to_string(), ip]);
    } else if cfg!(target_os = "macos") {
        cmd.args(["-n", "-c", "1", "-W", &timeout_ms.to_string(), ip]);
    } else {
        let secs = ((timeout_ms + 999) / 1000).max(1).to_string();
        cmd.args(["-n", "-c", "1", "-W", &secs, ip]);
    }
    if ip.contains(':') && !cfg!(windows) {
        // IPv6: ping6 on older systems, ping -6 on iputils; try both quietly
        if process::available("ping6") {
            cmd = process::quiet("ping6");
            if cfg!(target_os = "macos") {
                cmd.args(["-n", "-c", "1", ip]);
            } else {
                cmd.args(["-n", "-c", "1", "-W", "2", ip]);
            }
        } else {
            cmd.arg("-6");
        }
    }
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_ping(&text)
}

/// Pull `time=12.3 ms` (or `time<1ms`, or `time=12ms`) out of ping's output.
pub fn parse_ping(text: &str) -> Option<f64> {
    for line in text.lines() {
        let l = line.to_ascii_lowercase();
        if let Some(p) = l.find("time=") {
            let v = &l[p + 5..];
            let v: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(x) = v.parse::<f64>() {
                return Some(x);
            }
        } else if let Some(p) = l.find("time<") {
            let v = &l[p + 5..];
            let v: String = v.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if let Ok(x) = v.parse::<f64>() {
                return Some(x / 2.0);
            }
        }
    }
    None
}

/// A running prober: every `interval` it pings each address once, in
/// parallel, and hands each result to `on_sample`.
pub struct Prober {
    stop: Arc<AtomicBool>,
}

impl Prober {
    pub fn start<F>(ips: Vec<String>, interval: Duration, timeout_ms: u32, on_sample: F) -> Prober
    where
        F: Fn(Sample) + Send + Sync + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let cb: Arc<dyn Fn(Sample) + Send + Sync> = Arc::new(on_sample);
        std::thread::Builder::new()
            .name("hopscotch-prober".into())
            .spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    let tick = Instant::now();
                    let handles: Vec<_> = ips
                        .iter()
                        .cloned()
                        .map(|ip| {
                            let cb = cb.clone();
                            let flag = flag.clone();
                            std::thread::spawn(move || {
                                let t = now_ms();
                                let ms = ping_once(&ip, timeout_ms);
                                if !flag.load(Ordering::SeqCst) {
                                    cb(Sample { ip, ms, t });
                                }
                            })
                        })
                        .collect();
                    for h in handles {
                        let _ = h.join();
                    }
                    let spent = tick.elapsed();
                    if spent < interval {
                        // sleep in small steps so stop() is honoured promptly
                        let mut left = interval - spent;
                        while left > Duration::ZERO && !flag.load(Ordering::SeqCst) {
                            let step = left.min(Duration::from_millis(100));
                            std::thread::sleep(step);
                            left = left.saturating_sub(step);
                        }
                    }
                }
            })
            .expect("spawn prober");
        Prober { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

impl Drop for Prober {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ping_output() {
        assert_eq!(parse_ping("64 bytes from 8.8.8.8: icmp_seq=1 ttl=117 time=12.3 ms\n"), Some(12.3));
        assert_eq!(parse_ping("Reply from 8.8.8.8: bytes=32 time=12ms TTL=117\r\n"), Some(12.0));
        assert_eq!(parse_ping("Reply from 192.168.1.1: bytes=32 time<1ms TTL=64\r\n"), Some(0.5));
        assert_eq!(parse_ping("Request timed out.\r\n"), None);
        assert_eq!(parse_ping("--- 8.8.8.8 ping statistics ---\n1 packets transmitted, 0 received, 100% packet loss, time 0ms\n"), None);
        assert_eq!(parse_ping("Reply from 10.0.0.1: Destination host unreachable.\r\n"), None);
    }
}

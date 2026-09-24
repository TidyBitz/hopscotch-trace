//! Native probes on Linux: no root, no tools.
//!
//! Linux hands the ICMP errors that traceroute lives on (time exceeded, port
//! unreachable) to an ordinary UDP socket once `IP_RECVERR` is set, so a route
//! can be traced with plain datagram sockets, one TTL at a time. That is how
//! unprivileged `traceroute` and `tracepath` work. A TCP SYN with a TTL works
//! the same way for a destination that drops UDP, except the kernel only says
//! *that* a router answered, not which one; enough to prove the host is up and
//! find how far away it is. ICMP echo uses a datagram ICMP socket, which most
//! distributions allow every user (`net.ipv4.ping_group_range`); where they
//! don't, the caller falls back to the `ping` tool.
#![cfg(target_os = "linux")]

use std::io;
use std::mem;
use std::net::Ipv4Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use libc::{c_int, c_void};

/// What one probe got back.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    /// A router on the way sent ICMP time exceeded. `from` is unknown for TCP probes.
    Expired { from: Option<Ipv4Addr>, rtt: f64 },
    /// The destination answered: port unreachable, a completed handshake, or a reset.
    Reached { from: Ipv4Addr, rtt: f64 },
    /// ICMP destination unreachable other than "port": `note` is traceroute's mark (!H, !N, !X ...).
    Unreachable { from: Option<Ipv4Addr>, rtt: f64, note: String },
    /// Nothing came back in time.
    Silent,
}

/// Can this process open the sockets the probes need?
pub fn available() -> bool {
    socket(libc::SOCK_DGRAM, libc::IPPROTO_UDP)
        .and_then(|s| setsockopt(s.as_raw_fd(), libc::IPPROTO_IP, libc::IP_RECVERR, 1))
        .is_ok()
}

fn last_err() -> io::Error {
    io::Error::last_os_error()
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 1000.0).round() / 1000.0
}

fn socket(ty: c_int, proto: c_int) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_INET, ty | libc::SOCK_CLOEXEC, proto) };
    if fd < 0 {
        return Err(last_err());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn setsockopt(fd: RawFd, level: c_int, name: c_int, value: c_int) -> io::Result<()> {
    let r = unsafe { libc::setsockopt(fd, level, name, &value as *const c_int as *const c_void, mem::size_of::<c_int>() as libc::socklen_t) };
    if r < 0 {
        Err(last_err())
    } else {
        Ok(())
    }
}

fn so_error(fd: RawFd) -> io::Result<i32> {
    let mut err: c_int = 0;
    let mut len = mem::size_of::<c_int>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut err as *mut c_int as *mut c_void, &mut len) };
    if r < 0 {
        Err(last_err())
    } else {
        Ok(err)
    }
}

fn sockaddr(ip: Ipv4Addr, port: u16) -> libc::sockaddr_in {
    let mut sa: libc::sockaddr_in = unsafe { mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_port = port.to_be();
    sa.sin_addr = libc::in_addr { s_addr: u32::from(ip).to_be() };
    sa
}

fn connect(fd: RawFd, ip: Ipv4Addr, port: u16) -> io::Result<()> {
    let sa = sockaddr(ip, port);
    let r = unsafe { libc::connect(fd, &sa as *const libc::sockaddr_in as *const libc::sockaddr, mem::size_of::<libc::sockaddr_in>() as libc::socklen_t) };
    if r < 0 {
        Err(last_err())
    } else {
        Ok(())
    }
}

/// Wait until `deadline` for `events` (errors are always reported). `None` on timeout.
fn wait(fd: RawFd, events: i16, deadline: Instant) -> io::Result<Option<i16>> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let mut p = libc::pollfd { fd, events, revents: 0 };
        let r = unsafe { libc::poll(&mut p, 1, left.as_millis().clamp(1, i32::MAX as u128) as c_int) };
        if r < 0 {
            let e = last_err();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if r == 0 {
            return Ok(None);
        }
        return Ok(Some(p.revents));
    }
}

struct SockErr {
    errno: i32,
    origin: u8,
    typ: u8,
    code: u8,
    from: Option<Ipv4Addr>,
}

/// One message off the socket's error queue, with the address of whoever sent the ICMP.
fn read_errqueue(fd: RawFd) -> io::Result<Option<SockErr>> {
    let mut data = [0u8; 256];
    let mut ctrl = [0u64; 64]; // u64 keeps the control buffer aligned for cmsg headers
    let mut iov = libc::iovec { iov_base: data.as_mut_ptr() as *mut c_void, iov_len: data.len() };
    let mut msg: libc::msghdr = unsafe { mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = ctrl.as_mut_ptr() as *mut c_void;
    msg.msg_controllen = mem::size_of_val(&ctrl) as _;
    let n = unsafe { libc::recvmsg(fd, &mut msg, libc::MSG_ERRQUEUE | libc::MSG_DONTWAIT) };
    if n < 0 {
        let e = last_err();
        if e.kind() == io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        return Err(e);
    }
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&msg) };
    while !cmsg.is_null() {
        let c = unsafe { &*cmsg };
        if c.cmsg_level == libc::IPPROTO_IP && c.cmsg_type == libc::IP_RECVERR {
            let p = unsafe { libc::CMSG_DATA(cmsg) } as *const libc::sock_extended_err;
            let ee = unsafe { p.read_unaligned() };
            // SO_EE_OFFENDER: the sockaddr that follows the struct
            let need = mem::size_of::<libc::sock_extended_err>() + mem::size_of::<libc::sockaddr_in>();
            let from = if ee.ee_origin == libc::SO_EE_ORIGIN_ICMP && c.cmsg_len as usize >= unsafe { libc::CMSG_LEN(need as u32) } as usize {
                let sa = unsafe { (p.add(1) as *const libc::sockaddr_in).read_unaligned() };
                if sa.sin_family == libc::AF_INET as libc::sa_family_t {
                    Some(Ipv4Addr::from(u32::from_be(sa.sin_addr.s_addr)))
                } else {
                    None
                }
            } else {
                None
            };
            return Ok(Some(SockErr { errno: ee.ee_errno as i32, origin: ee.ee_origin, typ: ee.ee_type, code: ee.ee_code, from }));
        }
        cmsg = unsafe { libc::CMSG_NXTHDR(&msg, cmsg) };
    }
    Ok(None)
}

const ICMP_TIME_EXCEEDED: u8 = 11;
const ICMP_DEST_UNREACH: u8 = 3;

fn unreach_note(code: u8) -> String {
    match code {
        0 => "!N".into(),
        1 => "!H".into(),
        2 => "!P".into(),
        4 => "!F".into(),
        5 => "!S".into(),
        13 => "!X".into(),
        n => format!("!{n}"),
    }
}

fn classify(e: SockErr, rtt: f64) -> io::Result<Reply> {
    if e.origin != libc::SO_EE_ORIGIN_ICMP {
        return Err(io::Error::from_raw_os_error(e.errno));
    }
    Ok(match (e.typ, e.code, e.from) {
        (ICMP_TIME_EXCEEDED, _, from) => Reply::Expired { from, rtt },
        (ICMP_DEST_UNREACH, 3, Some(from)) => Reply::Reached { from, rtt },
        (ICMP_DEST_UNREACH, code, from) => Reply::Unreachable { from, rtt, note: unreach_note(code) },
        (t, c, from) => Reply::Unreachable { from, rtt, note: format!("!{t}/{c}") },
    })
}

/// One UDP probe to `dest:port` with the given TTL, traceroute style.
pub fn probe_udp(dest: Ipv4Addr, ttl: u8, port: u16, timeout: Duration) -> io::Result<Reply> {
    let s = socket(libc::SOCK_DGRAM, libc::IPPROTO_UDP)?;
    let fd = s.as_raw_fd();
    setsockopt(fd, libc::IPPROTO_IP, libc::IP_TTL, ttl.max(1) as c_int)?;
    setsockopt(fd, libc::IPPROTO_IP, libc::IP_RECVERR, 1)?;
    connect(fd, dest, port)?;
    let payload = [0u8; 32];
    let sent = Instant::now();
    if unsafe { libc::send(fd, payload.as_ptr() as *const c_void, payload.len(), 0) } < 0 {
        return Err(last_err());
    }
    let deadline = sent + timeout;
    for _ in 0..16 {
        let Some(re) = wait(fd, libc::POLLIN, deadline)? else { return Ok(Reply::Silent) };
        let rtt = ms(sent.elapsed());
        if re & libc::POLLERR != 0 {
            if let Some(e) = read_errqueue(fd)? {
                return classify(e, rtt);
            }
        }
        if re & libc::POLLIN != 0 {
            let mut buf = [0u8; 64];
            let r = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut c_void, buf.len(), libc::MSG_DONTWAIT) };
            if r >= 0 {
                // something actually listens on that port and answered: we are there
                return Ok(Reply::Reached { from: dest, rtt });
            }
            let e = last_err();
            if e.kind() != io::ErrorKind::WouldBlock {
                return Err(e);
            }
        }
        if re & (libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::other("socket closed"));
        }
    }
    Ok(Reply::Silent)
}

/// One TCP SYN to `dest:port` with the given TTL. A handshake or a reset means
/// the host is there; time exceeded comes back as EHOSTUNREACH (the kernel
/// keeps the router's address to itself for TCP).
pub fn probe_tcp(dest: Ipv4Addr, ttl: u8, port: u16, timeout: Duration) -> io::Result<Reply> {
    let s = socket(libc::SOCK_STREAM | libc::SOCK_NONBLOCK, libc::IPPROTO_TCP)?;
    let fd = s.as_raw_fd();
    setsockopt(fd, libc::IPPROTO_IP, libc::IP_TTL, ttl.max(1) as c_int)?;
    let sent = Instant::now();
    match connect(fd, dest, port) {
        Ok(()) => return Ok(Reply::Reached { from: dest, rtt: ms(sent.elapsed()) }),
        Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) => {}
        Err(e) => return tcp_outcome(e.raw_os_error().unwrap_or(0), dest, ms(sent.elapsed())),
    }
    let Some(_) = wait(fd, libc::POLLOUT, sent + timeout)? else { return Ok(Reply::Silent) };
    let rtt = ms(sent.elapsed());
    tcp_outcome(so_error(fd)?, dest, rtt)
}

fn tcp_outcome(errno: i32, dest: Ipv4Addr, rtt: f64) -> io::Result<Reply> {
    Ok(match errno {
        0 | libc::ECONNREFUSED | libc::ECONNRESET => Reply::Reached { from: dest, rtt },
        libc::EHOSTUNREACH => Reply::Expired { from: None, rtt },
        libc::ENETUNREACH => Reply::Unreachable { from: None, rtt, note: "!N".into() },
        libc::EACCES | libc::EPERM => Reply::Unreachable { from: None, rtt, note: "!X".into() },
        libc::ETIMEDOUT => Reply::Silent,
        e => return Err(io::Error::from_raw_os_error(e)),
    })
}

/// How far away the destination is when UDP probes stop being answered. A SYN
/// to `port` with a full TTL proves it is up; a binary search over the TTL
/// between `known` (the last hop that answered) and `max` finds the distance.
/// Returns the distance and a few handshake round trips measured at it.
pub fn tcp_distance(dest: Ipv4Addr, port: u16, known: u8, max: u8, timeout: Duration) -> Option<(u8, Vec<Option<f64>>)> {
    let first = match probe_tcp(dest, max, port, timeout) {
        Ok(Reply::Reached { rtt, .. }) => rtt,
        _ => return None,
    };
    let (mut lo, mut hi) = (known.min(max), max);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        match probe_tcp(dest, mid, port, timeout) {
            Ok(Reply::Reached { .. }) => hi = mid,
            Ok(_) => lo = mid,
            Err(_) => return None,
        }
    }
    let mut rtts = vec![Some(first)];
    for _ in 0..2 {
        rtts.push(match probe_tcp(dest, hi, port, timeout) {
            Ok(Reply::Reached { rtt, .. }) => Some(rtt),
            _ => None,
        });
    }
    Some((hi, rtts))
}

static SEQ: AtomicU16 = AtomicU16::new(1);

fn checksum(b: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for pair in b.chunks(2) {
        let w = if pair.len() == 2 { u16::from_be_bytes([pair[0], pair[1]]) } else { u16::from_be_bytes([pair[0], 0]) };
        sum += w as u32;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// One ICMP echo request over a datagram ICMP socket. `Err` means the socket
/// could not be opened (the sysctl does not allow it): fall back to `ping`.
/// `Ok(None)` is a lost probe.
pub fn ping_icmp(dest: Ipv4Addr, timeout: Duration) -> io::Result<Option<f64>> {
    let s = socket(libc::SOCK_DGRAM, libc::IPPROTO_ICMP)?;
    let fd = s.as_raw_fd();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut pkt = [0u8; 32];
    pkt[0] = 8; // echo request; the kernel fills in the identifier and recomputes the checksum
    pkt[6..8].copy_from_slice(&seq.to_be_bytes());
    let ck = checksum(&pkt);
    pkt[2..4].copy_from_slice(&ck.to_be_bytes());
    let sa = sockaddr(dest, 0);
    let sent = Instant::now();
    let n = unsafe { libc::sendto(fd, pkt.as_ptr() as *const c_void, pkt.len(), 0, &sa as *const libc::sockaddr_in as *const libc::sockaddr, mem::size_of::<libc::sockaddr_in>() as libc::socklen_t) };
    if n < 0 {
        let e = last_err();
        // no route or the like: not a permission problem, so report it as a lost probe
        return if e.kind() == io::ErrorKind::PermissionDenied { Err(e) } else { Ok(None) };
    }
    let deadline = sent + timeout;
    for _ in 0..16 {
        let Some(re) = wait(fd, libc::POLLIN, deadline)? else { return Ok(None) };
        if re & libc::POLLIN == 0 {
            return Ok(None); // an error (unreachable) rather than an answer
        }
        let mut buf = [0u8; 256];
        let r = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut c_void, buf.len(), libc::MSG_DONTWAIT) };
        if r < 0 {
            let e = last_err();
            if e.kind() == io::ErrorKind::WouldBlock {
                continue;
            }
            return Ok(None);
        }
        let r = r as usize;
        if r >= 8 && buf[0] == 0 && buf[6..8] == seq.to_be_bytes() {
            return Ok(Some(ms(sent.elapsed())));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn icmp_checksum_matches_rfc_example() {
        // an echo request with all-zero fields except type 8 sums to 0x0800, so the checksum is its complement
        let mut p = [0u8; 8];
        p[0] = 8;
        assert_eq!(checksum(&p), !0x0800u16);
    }
    #[test]
    fn unreachable_marks_match_traceroute() {
        assert_eq!(unreach_note(1), "!H");
        assert_eq!(unreach_note(13), "!X");
        assert_eq!(unreach_note(9), "!9");
    }
    #[test]
    fn loopback_udp_probe_reaches() {
        // nothing listens on this port on loopback, so the kernel answers port unreachable at once
        match probe_udp(Ipv4Addr::LOCALHOST, 1, 33434, Duration::from_millis(500)) {
            Ok(Reply::Reached { from, .. }) => assert_eq!(from, Ipv4Addr::LOCALHOST),
            other => panic!("unexpected {other:?}"),
        }
    }
    #[test]
    fn loopback_tcp_probe_reaches_open_and_closed_ports() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(matches!(probe_tcp(Ipv4Addr::LOCALHOST, 1, port, Duration::from_millis(500)), Ok(Reply::Reached { .. })));
        assert!(matches!(probe_tcp(Ipv4Addr::LOCALHOST, 1, 9, Duration::from_millis(500)), Ok(Reply::Reached { .. })));
    }
}

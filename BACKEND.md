# How the backend fits together, and what comes next

## The seam

The UI was built as a simulator first, and the simulator is still there for the browser demo. Inside the desktop app `ui/live.js` defines `window.HopscotchLive`, and the page checks for it once at load (`const LIVE = …`). Everything that touches the network goes through six seams in `ui/index.html`:

| Seam | Demo | Live |
| --- | --- | --- |
| `makeTrace(target)` | builds a fake route from the name | `makeLiveTrace`: an empty trace with a `liveId` |
| `run(tr)` | reveals fake hops on timers | `runLive`: `resolve` → `start_trace` → hop/name events → batched `geo_lookup` → `settleLive` |
| `sample(h)` / cycle | fakes a probe per hop per tick | backend `start_probe`; samples arrive as `probe:sample` events into `record(h, ms, t)` |
| `registration(tr, h)` | made-up record | `liveRegistration`: `rdap_ip` on demand, `rdap_domain` for the destination |
| connection details | the chosen network profile | `net_info` at trace start, saved with the run |
| `fillInstantly(tr)` (Home, reopened tabs) | fills a fake route | `hydrateLive`: rebuilds hops from the last saved run's `route` |

Hops carry `place` (a CITY key from geolocation) and `netKey` (from the AS number); `settleLive` derives the drawn `c` and `net` for every hop from those each time something new arrives, so unknown hops borrow a neighbour's place and the next known network. Cities and networks learned at runtime are added to the same `CITY` / `NETS` tables the simulator uses and persisted (`hopscotch:liveCities`, `hopscotch:liveNets`), so saved runs can be redrawn later.

## Commands and events (src-tauri/src/lib.rs)

| Command | Returns |
| --- | --- |
| `platform()` | os, arch, the traceroute invocation that will be used, or why none is available |
| `resolve(target)` | addresses, IPv4 first |
| `reverse(ip)` | PTR name or null |
| `start_trace(id, target, maxHops?, waitMs?)` | starts a thread; emits `trace:event` `{id, kind: started\|hop\|done\|error, …}` and `trace:name` `{id, ip, name}` |
| `cancel_trace(id)` | kills the tool |
| `start_probe(id, ips, intervalMs?, timeoutMs?)` / `stop_probe(id)` | emits `probe:sample` `{id, ip, ms\|null, t}` |
| `geo_lookup(ips)` | one `GeoInfo` per address, private ones answered locally |
| `geo_self()` | where this machine's public address is |
| `rdap_ip(ip)` / `rdap_domain(name)` | simplified records |
| `net_info()` | Wi-Fi name, link, signal, rate (best effort) |

The core crate (`core/`) is plain Rust with no Tauri dependency, so it can be tested with `cargo test -p hopscotch-core` and reused by a CLI later.

## Why the system tools, for now

| | Windows | macOS | Linux |
| --- | --- | --- | --- |
| Raw ICMP socket | admin | root | root or `CAP_NET_RAW` |
| Unprivileged option | `IcmpSendEcho2` (Icmp.dll) sends echo requests with a TTL and reports the responding router; this is what `tracert` uses | ICMP `SOCK_DGRAM` works for ping; traceroute needs a raw socket, which is why `/usr/sbin/traceroute` is setuid | UDP probes with `IP_RECVERR` deliver the ICMP time-exceeded errors on the same socket (this is how `tracepath` and unprivileged `traceroute` work); ICMP `SOCK_DGRAM` needs `net.ipv4.ping_group_range` to include you |

Shelling out to the tools gives working traces on every platform on day one, with zero rights prompts. The costs are: parsing text (the parsers in `core/src/parse.rs` are tested against real output), one process per probe when probing continuously, and a dependency on `traceroute` being installed on Linux.

## Native probing

**Linux is done** (`core/src/native.rs`, chosen by `trace::run_auto`): a UDP socket per probe with `IP_TTL` and `IP_RECVERR`, the ICMP answer read off the error queue with `MSG_ERRQUEUE` and the router's address taken from `SO_EE_OFFENDER`. Probes for one TTL run in parallel on scoped threads. When the destination never answers UDP, `native::tcp_distance` sends a SYN to port 443 with a full TTL (a handshake or a reset means the host is up; the kernel reports a router's time-exceeded only as `EHOSTUNREACH`, without the address), then binary-searches the TTL to find the distance, and the trace ends at the destination with handshake round trips. Continuous probing sends ICMP echo over a datagram ICMP socket (`ping_icmp`) and falls back to the `ping` tool when the sysctl refuses the socket. `start_trace` takes `engine: "native" | "tool"`, and `platform()` reports which one a build will use.

The plan for the rest:

1. **Windows** — call `IcmpSendEcho2Ex` from `Icmp.dll` via the `windows-sys` crate with `IP_OPTION_INFORMATION.Ttl` set per hop. Unprivileged, no console, and it returns the responding address and the round trip directly. This replaces both `tracert` and `ping` on Windows.
2. **macOS** — keep the setuid `traceroute` for the route (it is reliable and always present), but do continuous probing with an ICMP `SOCK_DGRAM` socket, which is unprivileged on macOS.
3. Keep the tool-based path as the fallback everywhere, so a build that cannot open its sockets still traces.

Other candidates: geolocation via a bundled MaxMind GeoLite2 database (offline, ~60 MB, needs an account and their license text), IPv6 traces (the parsers already handle v6 addresses; the tools need `-6`), MTR-style per-hop statistics from a single stream of probes, and reading the Wi-Fi link speed on macOS (`system_profiler SPAirPortDataType`, slow) only on demand.

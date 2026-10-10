# Changelog

The version shown at the bottom right of the window links here.

## 0.2.0

### Tracing
- **Hopscotch sends its own probes on Linux.** No root, no `traceroute` package. UDP probes with a rising TTL, read straight off the socket the way unprivileged traceroute works.
- **Destinations that ignore traceroute are found anyway.** When UDP stops being answered, a TCP handshake on port 443 shows the host is up and a search over the TTL finds how far away it is, so the trace ends at the destination instead of trailing off into silence.
- **Traces stop after five silent hops in a row** rather than running to 30. When the target never answers, the status bar says so and the silent tail is dropped.
- **Continuous probing uses ICMP echo directly** where the system allows it, falling back to the `ping` tool where it doesn't.
- Settings has a switch back to the system traceroute tool, for comparison. Windows and macOS still use the system tools.

### Map
- **Routers are placed by their names.** A hostname that says `den`, `lhr` or `frankfurt` wins over the address database, which files CDN and transit routers under a head office a continent away.

### Desktop app
- The Linux AppImage works on current Fedora and other Wayland desktops. It carries its own WebKitGTK and GStreamer but leaves Wayland, X11 and Mesa to the host.
- Sound is played by the app itself through the system audio output, not by the webview, whose GStreamer path stalled after a fraction of a second inside the AppImage. Same three packs, same cues.
- The probe log keeps its place while you scroll it during probing.
- The canned "try one of these" targets no longer appear in the desktop app. They stay in the browser demo.
- The version number at the bottom right links to this file.

### Build
- Builds on Ubuntu 24.04 for Linux; CI actions moved off Node 20.

## 0.1.0

First release: trace, map, continuous probing, trends, RDAP registration, privacy for private addresses, three sound packs, skins, demo mode in the browser.

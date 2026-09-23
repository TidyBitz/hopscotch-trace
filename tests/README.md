# Tests

- `node tests/smoke.js` — the page in demo mode (simulated routes), headless Chromium.
- `node tests/live.js` — the page in live mode, with `fake-tauri.js` standing in for the desktop backend. It answers the same commands and emits the same events as `src-tauri/src/lib.rs`, so the UI's live code paths run end to end without a network.
- `cargo test -p hopscotch-core` — Rust unit tests for the traceroute/ping parsers, address classification, RDAP and geolocation shaping, and the Wi-Fi info parsers.

Both page tests need Playwright: `npm install --no-save playwright@1 && npx playwright install chromium`. Set `PLAYWRIGHT_MODULE` to point at a global install instead. Screenshots land in `tests/shots/`.

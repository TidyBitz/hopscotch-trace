/* A stand-in for the desktop backend, injected before the page loads, so the
 * live-mode code paths can be exercised in a plain browser. It answers the
 * same commands and emits the same events as src-tauri, with canned data. */
(() => {
  const listeners = {};
  const emit = (name, payload) => (listeners[name] || []).forEach(cb => cb({ event: name, payload }));
  const probes = {};
  const GEO = {
    '100.64.8.1': { status: 'ok', lat: 39.1, lon: -94.58, city: 'Kansas City', region: 'Missouri', country: 'United States', country_code: 'US', isp: 'Prairie Broadband', org: 'Prairie Broadband LLC', asn: 64496, as_name: 'PRAIRIE-BB' },
    '198.51.100.7': { lat: 41.88, lon: -87.63, city: 'Chicago', country: 'United States', isp: 'Prairie Broadband', org: 'Prairie Broadband LLC', asn: 64496, as_name: 'PRAIRIE-BB' },
    '192.0.2.44': { lat: 40.71, lon: -74.01, city: 'New York', country: 'United States', isp: 'Meridian', org: 'Meridian Backbone', asn: 64500, as_name: 'MERIDIAN' },
    '198.51.100.9': { lat: 51.51, lon: -0.13, city: 'London', country: 'United Kingdom', isp: 'DataCamp', org: 'DataCamp Limited', asn: 64501, as_name: 'CDN77' },
    '93.184.216.34': { lat: 42.36, lon: -71.06, city: 'Boston', country: 'United States', isp: 'Edge Hosting', org: 'Example Edge', asn: 64511, as_name: 'EDGE' }
  };
  const NAMES = { '192.168.1.1': 'router.home', '198.51.100.7': 'be-12.core1.chi.prairie-bb.example', '192.0.2.44': 'ae-4.r3.nyc.meridian-bb.example', '93.184.216.34': 'example.org', '198.51.100.9': 'vl204.den-cs1-core-2.cdn77.example' };
  const ROUTE = [
    { hop: 1, addrs: ['192.168.1.1'], rtts: [1.2, 0.9, 1.1] },
    { hop: 2, addrs: ['100.64.8.1'], rtts: [9.1, 8.7, 9.4] },
    { hop: 3, addrs: [], rtts: [null, null, null] },
    { hop: 4, addrs: ['198.51.100.7'], rtts: [18.2, null, 18.9] },
    { hop: 5, addrs: ['192.0.2.44'], rtts: [31.0, 30.7, 31.4] },
    { hop: 6, addrs: ['93.184.216.34'], rtts: [33.1, 33.0, 33.6] }
  ];
  const cancelled = new Set();
  const commands = {
    platform: () => ({ os: 'linux', arch: 'x86_64', tool: 'traceroute -n -q 3 -w 2 -m 30', tool_error: window.__FAKE_TOOL_ERROR || null, version: '0.1.0-fake' }),
    resolve: ({ target }) => { if (/invalid|nowhere/.test(target)) throw new Error(target + ': no address'); return ['93.184.216.34']; },
    reverse: ({ ip }) => NAMES[ip] || null,
    start_trace: ({ id, target }) => {
      cancelled.delete(id);
      const route = window.__FAKE_ROUTE || ROUTE;
      setTimeout(() => emit('trace:event', { id, kind: 'started', tool: 'traceroute', dest: '93.184.216.34' }), 30);
      route.forEach((h, i) => setTimeout(() => {
        if (cancelled.has(id)) return;
        emit('trace:event', Object.assign({ id, kind: 'hop', note: null }, h));
        if (h.addrs.length) setTimeout(() => emit('trace:name', { id, ip: h.addrs[0], name: NAMES[h.addrs[0]] || null }), 60);
        if (i === route.length - 1) setTimeout(() => { if (!cancelled.has(id)) emit('trace:event', { id, kind: 'done', reached: window.__FAKE_REACHED !== false, hops: route.length, gave_up: window.__FAKE_REACHED === false }); }, 80);
      }, 120 + i * 140));
      if (!route.length) setTimeout(() => emit('trace:event', { id, kind: 'done', reached: false, hops: 0 }), 150);
    },
    cancel_trace: ({ id }) => { cancelled.add(id); },
    geo_lookup: ({ ips }) => ips.map(ip => Object.assign({ ip, kind: 'public' }, GEO[ip] || { error: 'unknown' })),
    geo_self: () => ({ ip: '203.0.113.5', kind: 'public', lat: 39.1, lon: -94.58, city: 'Kansas City', country: 'United States' }),
    rdap_ip: ({ ip }) => ({ handle: 'NET-TEST', name: 'TEST-NET-' + ip.split('.')[0], cidr: ip.replace(/\.\d+$/, '.0/24'), org: 'Example Holder', registry: 'ARIN', registered: '2009-06-25', abuse: 'abuse@example.net', country: 'US' }),
    rdap_domain: ({ name }) => ({ name, registrar: 'Example Registrar', registered: '1995-08-14', expires: '2027-08-13', nameservers: ['a.iana-servers.net', 'b.iana-servers.net'] }),
    net_info: () => ({ kind: 'wifi', ssid: 'Maple-5G', link: '802.11ax on 5 GHz, channel 44', signal: '91%', rate: '866 Mbps' }),
    start_probe: ({ id, ips, intervalMs }) => {
      if (probes[id]) clearInterval(probes[id]);
      probes[id] = setInterval(() => ips.forEach(ip => emit('probe:sample', { id, ip, ms: Math.random() < 0.05 ? null : 10 + Math.random() * 5, t: Date.now() })), intervalMs || 1000);
    },
    stop_probe: ({ id }) => { clearInterval(probes[id]); delete probes[id]; },
    save_text: ({ path, text }) => { window.__FAKE_SAVED = { path, text }; },
    play_cue: ({ name, pack }) => { (window.__FAKE_CUES = window.__FAKE_CUES || []).push(pack + '/' + name); }
  };
  window.__FAKE_CALLS = [];
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => new Promise((res, rej) => { window.__FAKE_CALLS.push(cmd); setTimeout(() => { try { res(commands[cmd] ? commands[cmd](args || {}) : null); } catch (e) { rej(String(e.message || e)); } }, 5); }) },
    dialog: { save: ({ defaultPath }) => Promise.resolve(window.__FAKE_CANCEL_SAVE ? null : '/tmp/' + defaultPath) },
    event: { listen: (name, cb) => { (listeners[name] = listeners[name] || []).push(cb); return Promise.resolve(() => { listeners[name] = listeners[name].filter(x => x !== cb); }); } }
  };
})();

# One-off: fold live-mode support into the demo page. Kept for the record of what changed.
import sys
p = sys.argv[1]; s = open(p).read()
def rep(old, new):
    global s
    assert s.count(old) == 1, ('MISSING/AMBIGUOUS (%d): ' % s.count(old)) + old[:100]
    s = s.replace(old, new, 1)

# 0. load the bridge before the app, favicon, and the badge
rep('<script>\nconst GEO', '<script src="live.js"></script>\n<script>\nconst GEO')

# 1. live registries next to CITY/HOME
rep("""let HOME = store.get('homeCity', 'mci'); if (!HOME_CHOICES.includes(HOME)) HOME = 'mci';""",
"""let HOME = store.get('homeCity', 'mci'); if (!HOME_CHOICES.includes(HOME)) HOME = 'mci';

/* ---------- live mode: inside the desktop app, real hops replace the simulator ---------- */
const LIVE = window.HopscotchLive || null;
let liveHome = LIVE ? store.get('liveHome', null) : null;          // CITY key for where this computer is
// cities and networks learned from lookups are kept, so Home and the reports can still name them next time
const liveCities = store.get('liveCities', {}) || {}, liveNets = store.get('liveNets', {}) || {};
for (const k in liveCities) if (!CITY[k]) CITY[k] = liveCities[k];
const homeKey = () => (liveHome && CITY[liveHome]) ? liveHome : HOME;
let liveSaveTimer = 0;
function saveLiveTables() { clearTimeout(liveSaveTimer); liveSaveTimer = setTimeout(() => { store.set('liveCities', liveCities); store.set('liveNets', liveNets); }, 500); }
function liveCity(g) {
  if (!g || g.lat == null || g.lon == null) return null;
  const key = 'g:' + (+g.lat).toFixed(2) + ',' + (+g.lon).toFixed(2);
  if (!CITY[key]) { CITY[key] = { key, n: g.city || g.region || g.country || 'Somewhere', country: g.country || '', lat: +g.lat, lon: +g.lon, tier: 9, reg: 'live', live: true }; liveCities[key] = CITY[key]; saveLiveTables(); }
  return key;
}
function liveNet(g) {
  if (!g) return null;
  const name = g.as_name && g.org && g.org.length > g.as_name.length ? g.org : (g.org || g.isp || g.as_name);
  if (!g.asn && !name) return null;
  const key = g.asn ? 'as' + g.asn : 'org:' + String(name).toLowerCase().replace(/[^a-z0-9]+/g, '-');
  if (!NETS[key]) { NETS[key] = { n: name || ('AS' + g.asn), asn: g.asn || null, rir: null, dom: null, pool: null, live: true }; liveNets[key] = NETS[key]; saveLiveTables(); }
  return key;
}
function ipKind(ip) {
  if (!ip) return 'none';
  if (ip.includes(':')) return /^fe80/i.test(ip) ? 'local' : /^f[cd]/i.test(ip) ? 'private' : /^::1$/.test(ip) ? 'local' : 'public';
  const m = /^(\\d+)\\.(\\d+)\\./.exec(ip); if (!m) return 'public';
  const a = +m[1], b = +m[2];
  if (a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168)) return 'private';
  if (a === 100 && b >= 64 && b <= 127) return 'cgnat';
  if (a === 127 || (a === 169 && b === 254) || a === 0) return 'local';
  return 'public';
}
function blockOf(ip) { const k = ipKind(ip), m = /^(\\d+)\\.(\\d+)\\./.exec(ip || ''); if (!m) return null; const a = +m[1]; return k === 'private' ? (a === 10 ? '10.0.0.0/8' : a === 172 ? '172.16.0.0/12' : '192.168.0.0/16') : k === 'cgnat' ? '100.64.0.0/10' : k === 'local' ? (a === 127 ? '127.0.0.0/8' : '169.254.0.0/16') : null; }
""")
rep("""const NETWORKS = {""", """for (const k in liveNets) if (!NETS[k]) NETS[k] = liveNets[k];
const NETWORKS = {""")

# 2. makeTrace → live skeleton; record() shared by simulator and live probes
rep("""function makeTrace(target, netKey, steady) {
  const rng = mulberry32(hash(target))""", """function liveHop(tr, i, ip) {
  const kind = ipKind(ip);
  return { i, ip, host: ip, noreply: !ip, samples: [], log: [], sent: 0, lost: 0, loss: 0, revealed: false, resolved: false, live: true, kind, c: null, place: null,
    net: 'pending', netKey: kind === 'private' || kind === 'local' ? 'home' : null, priv: kind === 'private' || kind === 'local', access: kind !== 'public', jit: 1, base: 0, geoDone: false, nameDone: false };
}
function makeLiveTrace(target, netKey) {
  const net = NETWORKS[netKey] ? netKey : (NETWORKS[state.network] ? state.network : 'home');
  const tr = { id: ++traceSeq, target, hops: [], revealed: 0, status: 'idle', fail: null, nets: ['home'], sel: 0, net, keep: false, drift: 0, live: true, liveId: 'tr' + Date.now().toString(36) + '-' + traceSeq };
  if (/\\.(invalid|test|localhost)$/.test(target)) { tr.fail = 'dns'; }
  buildGeometry(tr);
  return tr;
}
// after lookups: hops with no place borrow a neighbour's, networks fill in from the next known hop, legs are re-measured
function settleLive(tr) {
  const hs = tr.hops.filter(h => !h.noreply);
  // places: a hop the lookup placed keeps that; the rest sit with the previous placed hop, or the first one, or home
  const firstKnown = hs.find(h => h.place && CITY[h.place]);
  const fallback = firstKnown ? firstKnown.place : homeKey();
  let prevC = null;
  hs.forEach(h => { if (h.place && CITY[h.place]) { h.c = h.place; prevC = h.c; return; } h.c = prevC || ((h.priv || h.kind === 'cgnat') && liveHome && CITY[liveHome] ? liveHome : fallback); prevC = h.c; });
  // networks: known from the lookup (or "home" for private addresses); unknown hops take the next known one, then the previous
  hs.forEach(h => { h.net = h.netKey || 'pending'; });
  for (let k = hs.length - 1, next = null; k >= 0; k--) { const h = hs[k]; if (h.net === 'pending') h.net = next || 'pending'; else next = h.net; }
  for (let k = 0, prev = 'home'; k < hs.length; k++) { const h = hs[k]; if (h.net === 'pending') h.net = prev; else prev = h.net; }
  tr.nets = ['home']; hs.forEach(h => { if (!tr.nets.includes(h.net)) tr.nets.push(h.net); });
  let prev = null; hs.forEach(h => { if (prev) { const a = CITY[prev.c], b = CITY[h.c]; h.prev = prev.i; h.legKm = prev.c === h.c ? 0 : km([a.lon, a.lat], [b.lon, b.lat]); } else { h.prev = null; h.legKm = 0; } prev = h; });
  buildGeometry(tr);
}
function applyGeo(tr, h, g) {
  h.geo = g; if (!g) return;
  if (g.lat != null) h.place = liveCity(g);
  const nk = liveNet(g); if (nk) h.netKey = nk;
  else if (h.dest) h.netKey = 'dest';
}
function record(h, v, at) {
  h.sent++; if (v == null) h.lost++;
  h.samples.push(v); if (h.samples.length > 48) h.samples.shift();
  h.log.push({ t: at || Date.now(), v }); if (h.log.length > 5000) h.log.shift();
}

function makeTrace(target, netKey, steady) {
  if (LIVE) return makeLiveTrace(target, netKey);
  const rng = mulberry32(hash(target))""")
rep("""function sample(h, forcedLoss, at) {
  h.sent++; let v = null;
  if (forcedLoss || Math.random() < h.loss || Math.random() < (h.linkLoss || 0)) h.lost++;
  else { const j = Math.abs((Math.random() + Math.random() + Math.random() - 1.5) * 2); v = h.base + j * (0.35 + h.base * 0.02) * (h.jit || 1) + (Math.random() < 0.04 ? h.base * 0.06 + 3 : 0); }
  h.samples.push(v); if (h.samples.length > 48) h.samples.shift();
  h.log.push({ t: at || Date.now(), v }); if (h.log.length > 5000) h.log.shift();
}""", """function sample(h, forcedLoss, at) {
  let v = null;
  if (!(forcedLoss || Math.random() < h.loss || Math.random() < (h.linkLoss || 0))) { const j = Math.abs((Math.random() + Math.random() + Math.random() - 1.5) * 2); v = h.base + j * (0.35 + h.base * 0.02) * (h.jit || 1) + (Math.random() < 0.04 ? h.base * 0.06 + 3 : 0); }
  record(h, v, at);
}""")

# 3. no sample history or sample runs in live mode
rep("""  if (!Array.isArray(history)) history = PRESET_ORDER.slice().reverse().map(t => ({ target: t, when: 0, status: 'done' }));""",
    """  if (!Array.isArray(history)) history = LIVE ? [] : PRESET_ORDER.slice().reverse().map(t => ({ target: t, when: 0, status: 'done' }));""")
rep("""  if (!Array.isArray(runs)) runs = seedRuns();""", """  if (!Array.isArray(runs)) runs = LIVE ? [] : seedRuns();""")

# 4. Home rows tolerate a trace with no saved route; ghosts are hydrated from the last run
rep("""  const traceFor = t => state.tabs.find(x => x.target === t) || ghost[t] || (ghost[t] = makeTrace(t, 'home', true));""",
    """  const traceFor = t => state.tabs.find(x => x.target === t) || ghost[t] || (ghost[t] = (tr => { if (tr.live) hydrateLive(tr); return tr; })(makeTrace(t, 'home', true)));""")
rep("""        (tr.fail ? `<td class="fail" colspan="3">No address found</td>` : `<td>${esc(c.n)}""",
    """        (tr.fail || !d ? `<td class="fail" colspan="3">${tr.fail ? 'No address found' : 'No route saved yet'}</td>` : `<td>${esc(c.n)}""")
rep("""        const tr = it.tr; if (tr.fail || !tr.geo) return;""", """        const tr = it.tr; if (tr.fail || !tr.geo || !tr.geo.groups.length) return;""")
rep("""lon0 = CITY[HOME].lon, mid = 15""", """lon0 = CITY[homeKey()].lon, mid = 15""")

# 5. summarise keeps what live hops need to be rebuilt; hydrateLive rebuilds them
rep("""    run.route = tr.hops.map(h => h.noreply ? null : { name: h.noName ? h.ip : h.host, ip: h.ip, city: CITY[h.c].n, net: h.net });""",
    """    run.route = tr.hops.map(h => h.noreply ? null : Object.assign({ name: h.noName ? h.ip : h.host, ip: h.ip, city: CITY[h.c] ? CITY[h.c].n : '', net: h.net },
      h.live ? { live: true, kind: h.kind, priv: !!h.priv, placed: !!(h.geo && h.geo.lat != null), lat: CITY[h.c] ? CITY[h.c].lat : null, lon: CITY[h.c] ? CITY[h.c].lon : null, country: CITY[h.c] ? CITY[h.c].country : '', netName: NETS[h.net] ? NETS[h.net].n : '', asn: NETS[h.net] ? NETS[h.net].asn : null, dest: !!h.dest } : null));
    if (tr.live && tr.conn) run.conn = { ssid: tr.conn.ssid || null, link: tr.conn.link || null, rate: tr.conn.rate || null, signal: tr.conn.signal || null };""")
rep("""  function fillInstantly(tr) { tr.hops.forEach""", """  function hydrateLive(tr) {
    const run = runsOf(tr.target)[0]; tr.hops = []; tr.nets = ['home'];
    if (!run || !run.route || !run.route.length) { tr.revealed = 0; tr.status = 'idle'; buildGeometry(tr); return; }
    run.route.forEach((e, i) => {
      const h = liveHop(tr, i, e ? e.ip : null); h.revealed = true; h.resolved = true;
      if (e) {
        h.host = e.name || e.ip; h.noName = !e.name || e.name === e.ip; h.dest = !!e.dest; h.priv = !!e.priv; h.access = e.kind ? e.kind !== 'public' : h.access; h.kind = e.kind || h.kind;
        if (e.placed && e.lat != null) { h.place = liveCity({ lat: e.lat, lon: e.lon, city: e.city, country: e.country }); h.geo = { lat: e.lat, lon: e.lon }; }
        if (e.net && e.net !== 'pending' && e.net !== 'home') { if (!NETS[e.net]) { NETS[e.net] = { n: e.netName || e.net, asn: e.asn || null, live: true }; liveNets[e.net] = NETS[e.net]; saveLiveTables(); } h.netKey = e.net; }
        if (run.per && run.per[i] != null) record(h, run.per[i], run.when || Date.now());
      }
      tr.hops.push(h);
    });
    if (run.conn) tr.conn = run.conn;
    settleLive(tr); tr.revealed = tr.hops.length; tr.status = 'done'; tr.sel = tr.hops.length - 1;
  }
  function fillInstantly(tr) { if (tr.live) return hydrateLive(tr); tr.hops.forEach""")

# 6. run → runLive; cancel stops the tool; probing goes through the backend
rep("""  function cancel(tr) { if (!tr.tok) return; tr.tok.cancelled = true; tr.tok.waits.forEach""",
    """  function cancel(tr) { if (!tr.tok) return; tr.tok.cancelled = true; if (tr.tok.live) { try { tr.tok.live(); } catch (e) { /* already gone */ } } tr.tok.waits.forEach""")
rep("""  async function run(tr) {
    cancel(tr);""", """  function liveProbe(tr, on) {
    if (!LIVE || !tr.liveId) return;
    if (!on) { LIVE.probe.stop(tr.liveId).catch(() => {}); return; }
    const ips = tr.hops.filter(h => h.revealed && !h.noreply && h.ip).map(h => h.ip);
    if (!ips.length) { tr.keep = false; return; }
    LIVE.probe.start(tr.liveId, ips, state.every * 1000).catch(e => { tr.keep = false; showMsg('Probing couldn\\'t start: ' + e); });
  }
  async function runLive(tr) {
    cancel(tr); const tok = tr.tok = { cancelled: false, waits: [], live: null };
    if (tr.keep) liveProbe(tr, false);
    tr.hops = []; tr.nets = ['home']; tr.revealed = 0; tr.sel = 0; tr.status = 'running'; tr.fail = null; tr.outage = null; tr.destIp = null; tr.liveError = null; tr.conn = null; state.follow = true; buildGeometry(tr);
    MapView.clearAnim(); MapView.refit(true); Sound.play('start'); setStatus(`Looking up ${tr.target}…`); refresh();
    let ips = [];
    try { ips = await LIVE.resolve(tr.target); } catch (e) { ips = []; }
    if (tok.cancelled) return;
    if (!ips.length) { tr.fail = 'dns'; tr.status = 'failed'; tr.tok = null; remember(tr, 'failed'); MapView.clearAnim(); MapView.refit(true); Sound.play('failure'); setStatus(statusFor(tr)); refresh(); return; }
    tr.destIp = ips[0];
    if (state.recordNet) LIVE.netInfo().then(n => { tr.conn = n; }).catch(() => {});
    if (!liveHome) LIVE.geoSelf().then(g => { const k = liveCity(g); if (k) { liveHome = k; store.set('liveHome', k); settleLive(tr); if (isActive(tr)) refresh(); } }).catch(() => {});
    setStatus(`Tracing the route to ${tr.target} (${ips[0]})…`);
    const pendingGeo = []; let geoTimer = 0;
    const markGeo = batch => { batch.forEach(h => { h.geoDone = true; if (h.nameDone) h.resolved = true; }); settleLive(tr); if (isActive(tr)) refresh(); };
    const flushGeo = () => {
      const batch = pendingGeo.splice(0); if (!batch.length) return;
      LIVE.geo(batch.map(h => h.ip)).then(list => { if (tok.cancelled) return; (list || []).forEach(g => batch.filter(h => h.ip === g.ip).forEach(h => applyGeo(tr, h, g))); markGeo(batch); }).catch(() => { if (!tok.cancelled) markGeo(batch); });
    };
    const queueGeo = h => { pendingGeo.push(h); clearTimeout(geoTimer); geoTimer = setTimeout(flushGeo, 250); };
    await new Promise(done => {
      let finished = false; const finish = () => { if (finished) return; finished = true; clearTimeout(geoTimer); flushGeo(); done(); };
      tok.waits.push({ res: done, id: 0 });
      tok.live = LIVE.trace(tr.liveId, tr.target, {
        onStarted: p => { if (p.dest) tr.destIp = p.dest; },
        onHop: p => {
          if (tok.cancelled) return;
          const i = Math.max(0, (p.hop | 0) - 1);
          while (tr.hops.length < i) { const gap = liveHop(tr, tr.hops.length, null); gap.revealed = true; gap.resolved = true; tr.hops.push(gap); }
          const h = liveHop(tr, i, p.addrs && p.addrs.length ? p.addrs[0] : null); h.addrs = p.addrs || []; h.note = p.note || null; h.revealed = true;
          tr.hops[i] = h; tr.revealed = Math.max(tr.revealed, i + 1);
          (p.rtts || []).forEach(v => record(h, v, Date.now()));
          if (h.noreply) { h.resolved = true; if (isActive(tr)) { Sound.play('timeout'); setStatus(`Hop ${i + 1} didn't reply.`); } }
          else {
            h.dest = !!tr.destIp && h.addrs.includes(tr.destIp);
            if (h.kind === 'public') queueGeo(h); else h.geoDone = true;
            const s = stats(h); if (isActive(tr)) { Sound.play('hop', s ? s.last : 40, MapView.panFor(tr, i)); setStatus(hiddenHop(h) ? 'Looking up a hidden hop…' : `Looking up ${h.ip}…`); }
          }
          settleLive(tr);
          if (isActive(tr)) { if (state.follow) tr.sel = i; MapView.arrive(tr, i); refresh(); }
        },
        onName: p => { const h = tr.hops.find(x => x && x.ip === p.ip); if (!h) return; if (p.name) { h.host = p.name; h.noName = false; } else { h.host = h.ip; h.noName = true; } h.nameDone = true; if (h.geoDone) h.resolved = true; if (isActive(tr)) refresh(); },
        onDone: p => { tr.reached = !!p.reached; finish(); },
        onError: msg => { tr.liveError = msg; finish(); }
      }, { maxHops: 30, waitMs: 1500 });
    });
    if (tok.cancelled) return;
    if (!tr.hops.length) { tr.status = 'failed'; tr.fail = 'tool'; tr.failMsg = tr.liveError || 'The traceroute tool printed nothing.'; tr.tok = null; LIVE.release(tr.liveId); showMsg(tr.failMsg); Sound.play('failure'); setStatus(statusFor(tr)); refresh(); return; }
    if (isActive(tr)) { Sound.play('route'); setStatus('Route complete. Finishing the lookups…'); }
    const until = Date.now() + 6000;
    while (tr.hops.some(h => !h.noreply && !h.resolved) && Date.now() < until) { await sleep(90, tok); if (tok.cancelled) return; }
    tr.hops.forEach(h => { if (!h.resolved) { h.resolved = true; if (!h.nameDone) h.noName = true; } });
    settleLive(tr); LIVE.release(tr.liveId);
    tr.status = 'done'; tr.tok = null; remember(tr, 'done'); recordRun(tr); if (state.autoKeep) setKeep(tr, true);
    if (isActive(tr)) { Sound.play('trace'); setStatus(statusFor(tr)); refresh(); } else { renderTabs(); if (state.home) renderHome(true); }
  }
  async function run(tr) {
    if (tr.live) return runLive(tr);
    cancel(tr);""")
rep("""    if (tr.fail) return `Couldn't find an address for ${tr.target}.`;""",
    """    if (tr.fail === 'tool') return tr.failMsg || `The traceroute tool couldn't run.`;
    if (tr.fail) return `Couldn't find an address for ${tr.target}.`;""")
rep("""    setKeep = (tr, on) => { if (!tr || tr.fail) return; tr.keep = !!on;""", """    setKeep = (tr, on) => { if (!tr || tr.fail) return; tr.keep = !!on; if (tr.live) liveProbe(tr, tr.keep);""")
rep("""        if (!tr.keep || tr.fail || tr.status === 'running') return;""", """        if (!tr.keep || tr.fail || tr.status === 'running' || tr.live) return;""")
rep("""    retime = () => { clearInterval(timer); timer = setInterval(cycle, state.every * 1000); };""",
    """    retime = () => { clearInterval(timer); timer = setInterval(cycle, state.every * 1000); state.tabs.forEach(t => { if (t.live && t.keep) liveProbe(t, true); }); };
    if (LIVE) { let dataQueued = false; const queueData = () => { if (dataQueued) return; dataQueued = true; requestAnimationFrame(() => { dataQueued = false; refreshData(); }); };
      LIVE.probe.onSample(p => { const tr = state.tabs.find(t => t.liveId === p.id); if (!tr) return; const h = tr.hops.find(x => x && x.ip === p.ip); if (!h) return; record(h, p.ms == null ? null : +p.ms, p.t || Date.now()); updateRun(tr);
        if (tr === cur() && !state.home) { if (h === tr.hops[tr.sel] && state.pingSound && p.ms == null) Sound.play('timeout'); queueData(); } }); }""")
rep("""  function closeTab(i) {
    const tr = state.tabs[i]; if (!tr) return; cancel(tr); state.tabs.splice(i, 1);""", """  function closeTab(i) {
    const tr = state.tabs[i]; if (!tr) return; cancel(tr); if (tr.live && tr.keep) liveProbe(tr, false); state.tabs.splice(i, 1);""")
rep("""    cancel(state.tabs[i]); state.tabs.splice(i, 1);
    if (!state.tabs.length) { state.active = -1; els.target.value = ''; MapView.clearAnim(); }""", """    cancel(state.tabs[i]); if (state.tabs[i].live && state.tabs[i].keep) liveProbe(state.tabs[i], false); state.tabs.splice(i, 1);
    if (!state.tabs.length) { state.active = -1; els.target.value = ''; MapView.clearAnim(); }""")
rep("""    cancel(tr); tr.status = 'stopped'; tr.keep = false; tr.hops.forEach""", """    cancel(tr); tr.status = 'stopped'; if (tr.live) { liveProbe(tr, false); if (tr.liveId) LIVE.release(tr.liveId); } tr.keep = false; tr.hops.forEach""")

# 7. registration over RDAP
rep("""  function registration(tr, h) {
    if (h.priv) return""", """  function liveRegistration(tr, h) {
    const kind = h.kind || ipKind(h.ip);
    if (kind !== 'public') { const n = { private: ['Private-use networks', 'IANA, in RFC 1918'], cgnat: ['Shared address space', 'IANA, in RFC 6598'], local: ['Local-only addresses', 'IANA, in RFC 6890'] }[kind] || ['Reserved', 'IANA']; return { real: true, rows: [['Block', blockOf(h.ip) || h.ip], ['Listed as', n[0]], ['Set aside by', n[1]]] }; }
    if (!h.reg) {
      if (!h.regPending) {
        h.regPending = true;
        LIVE.rdapIp(h.ip).then(r => { h.reg = r || { error: 'empty answer' }; }).catch(e => { h.reg = { error: String(e) }; }).then(() => { if (cur() === tr) renderInfo(); });
        if (h.dest && !h.noName && !/^\\d+(\\.\\d+){3}$/.test(tr.target) && !tr.target.includes(':')) LIVE.rdapDomain(tr.target).then(d => { h.regDomain = d; if (cur() === tr) renderInfo(); }).catch(() => { h.regDomain = { error: true }; });
      }
      return { live: true, pending: true, rows: [['Status', 'Looking up…']] };
    }
    if (h.reg.error) return { live: true, rows: [['Status', 'No record came back'], ['Detail', String(h.reg.error).slice(0, 120)]] };
    const r = h.reg, rows = [['Block', r.cidr || (r.start && r.end ? `${r.start} \\u2013 ${r.end}` : h.ip)]];
    if (r.name) rows.push(['Net name', r.name]); if (r.org) rows.push(['Held by', r.org]); if (r.registry) rows.push(['Registry', r.registry]); if (r.country) rows.push(['Country', r.country]);
    if (r.registered) rows.push(['Registered', r.registered]); if (r.changed) rows.push(['Updated', r.changed]); if (r.abuse) rows.push(['Abuse contact', r.abuse]);
    const out = { live: true, rows }, d = h.regDomain;
    if (d && !d.error) { out.domain = [['Domain', d.name || tr.target]]; if (d.registrar) out.domain.push(['Registrar', d.registrar]); if (d.registered) out.domain.push(['Created', d.registered]); if (d.expires) out.domain.push(['Renews', d.expires]); if (d.nameservers && d.nameservers.length) out.domain.push(['Name servers', d.nameservers.slice(0, 4).join(', ')]); if (d.registrant) out.domain.push(['Registrant', d.registrant]); }
    return out;
  }
  function registration(tr, h) {
    if (tr.live) return liveRegistration(tr, h);
    if (h.priv) return""")
rep("""      `<p class="fine">${r.real ? 'This record is real. The range is reserved, so every lookup ends here.' : 'Made up, like the route. The real app would ask RDAP, the successor to WHOIS, and only for the hop you pick.'}</p></details>`;""",
    """      `<p class="fine">${r.real ? 'This record is real. The range is reserved, so every lookup ends here.' : r.live ? (r.pending ? 'Asking the registry over RDAP, the successor to WHOIS.' : 'Looked up over RDAP, the successor to WHOIS, for this hop only.') : 'Made up, like the route. The real app would ask RDAP, the successor to WHOIS, and only for the hop you pick.'}</p></details>`;""")

# 8. connection details from the system in live mode
rep("""      if (h.priv) { const nw = NETWORKS[tr.net] || NETWORKS.home;
        html += `<h3>Your connection</h3>""", """      if (h.priv && tr.live) { const nw = NETWORKS[tr.net] || NETWORKS.home, cn = tr.conn;
        html += `<h3>Your connection</h3><dl class="facts" style="border:0;padding:0;margin-bottom:6px"><dt>Network</dt><dd>${esc(nw.n)}</dd>${cn && cn.ssid ? `<dt>Wi-Fi name</dt><dd>${state.recordNet ? esc(cn.ssid) : '<em>Not noted</em>'}</dd>` : ''}${cn && cn.link ? `<dt>Link</dt><dd>${esc(cn.link)}</dd>` : ''}${cn && cn.signal ? `<dt>Signal</dt><dd>${esc(cn.signal)}</dd>` : ''}${cn && cn.rate ? `<dt>Link speed</dt><dd>${esc(cn.rate)}</dd>` : ''}</dl>` +
          `<p class="fine" style="font-size:12.5px;color:var(--ink-3);margin:0 0 4px">${cn && cn.ssid ? 'Read from the system when this trace ran.' : state.recordNet ? 'The system didn\\'t report a Wi-Fi network; a wired link, or one it can\\'t see.' : 'Network details aren\\'t being noted. Change that under Settings.'} The label comes from Settings, Network I'm on.</p>`; }
      else if (h.priv) { const nw = NETWORKS[tr.net] || NETWORKS.home;
        html += `<h3>Your connection</h3>""")

# 9. startup: badge, home-city setting, tool check
rep("""  function start() {
    wire(); wireSettings();""", """  function start() {
    wire(); wireSettings();
    if (LIVE) {
      const sim = $('.sim'); sim.firstChild.textContent = 'Live route'; $('.sim-tip').textContent = 'Real probes, sent from this computer with the system\\'s traceroute tool. Places come from an online lookup and are estimates; addresses set aside for private networks never leave this machine.';
      const hs = $('#setHome').closest('.set-row'); if (hs) hs.hidden = true;
      LIVE.platform().then(p => { if (p && p.tool_error) showMsg(p.tool_error); }).catch(() => {});
      if (!liveHome) LIVE.geoSelf().then(g => { const k = liveCity(g); if (k) { liveHome = k; store.set('liveHome', k); if (state.home) renderHome(); } }).catch(() => {});
    }""")
open(p, 'w').write(s); print('live patch applied')

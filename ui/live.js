/* Hopscotch live bridge.
 *
 * When the page runs inside the desktop app, Tauri injects window.__TAURI__ and
 * this file wraps its invoke/listen calls into a small API the app uses in place
 * of the simulator. In a plain browser nothing is defined and the app stays in
 * demo mode, so the same index.html works for both.
 */
(() => {
  const T = window.__TAURI__;
  if (!T || !T.core || !T.event) return;
  const invoke = T.core.invoke, listen = T.event.listen;

  // one listener per event type, fanned out by id, so a tab can be closed without leaking handlers
  const traceHandlers = new Map(), nameHandlers = new Map(), sampleHandlers = new Set();
  let wired = false;
  async function wire() {
    if (wired) return; wired = true;
    await listen('trace:event', e => { const h = traceHandlers.get(e.payload.id); if (h) h(e.payload); });
    await listen('trace:name', e => { const h = nameHandlers.get(e.payload.id); if (h) h(e.payload); });
    await listen('probe:sample', e => { sampleHandlers.forEach(h => h(e.payload)); });
  }
  const ready = wire();

  window.HopscotchLive = {
    version: 1,
    platform: () => invoke('platform'),
    resolve: target => invoke('resolve', { target }),
    reverse: ip => invoke('reverse', { ip }),
    /* trace(id, target, {onStarted, onHop, onName, onDone, onError}) → cancel() */
    trace(id, target, cb, opts = {}) {
      traceHandlers.set(id, p => {
        if (p.kind === 'started') cb.onStarted && cb.onStarted(p);
        else if (p.kind === 'hop') cb.onHop && cb.onHop(p);
        else if (p.kind === 'done') { cb.onDone && cb.onDone(p); }
        else if (p.kind === 'error') { cb.onError && cb.onError(p.message); }
      });
      nameHandlers.set(id, p => cb.onName && cb.onName(p));
      ready.then(() => invoke('start_trace', { id, target, maxHops: opts.maxHops || 30, waitMs: opts.waitMs || 1500, engine: opts.engine || 'native' })).catch(e => cb.onError && cb.onError(String(e)));
      return () => { traceHandlers.delete(id); nameHandlers.delete(id); invoke('cancel_trace', { id }).catch(() => {}); };
    },
    /* names keep arriving after Done, so a trace releases its handlers itself when it is finished with them */
    release(id) { traceHandlers.delete(id); nameHandlers.delete(id); },
    geo: ips => invoke('geo_lookup', { ips }),
    geoSelf: () => invoke('geo_self'),
    rdapIp: ip => invoke('rdap_ip', { ip }),
    rdapDomain: name => invoke('rdap_domain', { name }),
    netInfo: () => invoke('net_info'),
    /* save(name, text) → the path written, or null when the person cancelled */
    async save(name, text) {
      const ext = (name.split('.').pop() || 'txt').toLowerCase();
      const filters = [{ name: ext === 'csv' ? 'CSV' : ext === 'html' ? 'HTML report' : 'File', extensions: [ext] }];
      const path = T.dialog && T.dialog.save ? await T.dialog.save({ defaultPath: name, filters }) : null;
      if (!path) return null;
      await invoke('save_text', { path, text });
      return path;
    },
    probe: {
      start: (id, ips, intervalMs, timeoutMs) => ready.then(() => invoke('start_probe', { id, ips, intervalMs, timeoutMs: timeoutMs || Math.min(1500, intervalMs) })),
      stop: id => invoke('stop_probe', { id }),
      onSample: fn => { sampleHandlers.add(fn); return () => sampleHandlers.delete(fn); }
    }
  };
})();

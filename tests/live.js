/* Live-mode smoke test: the page with a fake backend standing in for src-tauri.
 * Run: node tests/live.js  (needs playwright; see tests/README) */
const path = require('path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
const fs = require('fs');
(async () => {
  const browser = await chromium.launch(); const errors = []; let failed = 0;
  const check = (name, ok, extra = '') => { console.log((ok ? 'ok   ' : 'FAIL ') + name + (extra ? '  ' + extra : '')); if (!ok) failed++; };
  const ctx = await browser.newContext({ viewport: { width: 1360, height: 860 }, colorScheme: 'dark' });
  await ctx.addInitScript({ path: path.join(__dirname, 'fake-tauri.js') });
  const p = await ctx.newPage();
  p.on('console', m => { if (m.type() === 'error' && !/ERR_|Failed to load resource/.test(m.text())) errors.push(m.text()); });
  p.on('pageerror', e => errors.push('PAGEERROR ' + e.message));
  await p.route(/^https?:/, r => r.abort());
  const url = 'file://' + path.join(__dirname, '..', 'ui', 'index.html');
  await p.goto(url); await p.waitForTimeout(600);
  const txt = async sel => (await p.textContent(sel)).replace(/\s+/g, ' ').trim();
  const count = sel => p.$$eval(sel, a => a.length);
  fs.mkdirSync(path.join(__dirname, 'shots'), { recursive: true });

  check('live bridge present', await p.evaluate(() => !!window.HopscotchLive));
  check('fresh live Home is empty', await count('.hist tbody tr[data-open]') === 0, 'rows=' + await count('.hist tbody tr[data-open]'));
  check('badge says Live route', (await txt('.sim')).startsWith('Live route'));

  // a real-looking trace
  await p.fill('#target', 'example.org'); await p.keyboard.press('Enter');
  await p.waitForTimeout(400);
  check('status while tracing', /example\.org|Looking up/.test(await txt('#status')), await txt('#status'));
  await p.waitForTimeout(2200);
  const rows = await p.$$eval('#dock tbody tr', r => r.map(x => x.textContent.replace(/\s+/g, ' ').trim()));
  check('six hops in the dock', rows.length === 6, rows.length + ' rows');
  check('silent hop shown', /No reply/.test(rows[2] || ''), rows[2]);
  check('names resolved', /router\.home/.test(rows[0]) && /example\.org/.test(rows[5]), rows[0] + ' | ' + rows[5]);
  check('places from geo', /Chicago/.test(rows[3]) && /New York/.test(rows[4]) && /Boston/.test(rows[5]));
  check('networks named', /Prairie Broadband/.test(rows[1]) && /Meridian/.test(rows[4]));
  check('status complete', /Trace complete/.test(await txt('#status')), await txt('#status'));
  await p.screenshot({ path: path.join(__dirname, 'shots', 'live-trace.png') });

  // details pane: registration over RDAP, connection from the system
  const info = await txt('#info');
  check('registration looked up', /Example Holder/.test(info) && /ARIN/.test(info), info.slice(0, 80));
  check('domain record shown', /Example Registrar/.test(info));
  check('RDAP fine print', /Looked up over RDAP/.test(info));
  await p.click('#dock tr[data-hop="0"]'); await p.waitForTimeout(200);
  const info0 = await txt('#info');
  check('private hop: RFC 1918 block', /192\.168\.0\.0\/16/.test(info0) && /RFC 1918/.test(info0));
  check('connection read from system', /Maple-5G/.test(info0) && /866 Mbps/.test(info0), info0.slice(0, 200));

  // probing goes through the backend
  await p.click('#probeBtn'); await p.waitForTimeout(2600);
  const calls = await p.evaluate(() => window.__FAKE_CALLS);
  check('start_probe invoked', calls.includes('start_probe'));
  await p.click('#dock tr[data-hop="5"]'); await p.click('#viewSeg [data-view="ping"]'); await p.waitForTimeout(1200);
  const sent = +(await txt('.ping-stats div:first-child b'));
  check('samples arrive from backend', sent >= 4, 'sent=' + sent);
  await p.click('#probeBtn'); await p.waitForTimeout(200);
  check('stop_probe invoked', (await p.evaluate(() => window.__FAKE_CALLS)).includes('stop_probe'));

  // the run was saved with a live route; Home shows it and a reload hydrates it
  await p.click('#homeBtn'); await p.waitForTimeout(300);
  const home = await p.$$eval('.hist tbody tr[data-open]', r => r.map(x => x.textContent.replace(/\s+/g, ' ').trim()));
  check('Home has the run', home.length === 1 && /Boston/.test(home[0]) && /1 run/.test(home[0]), home[0]);
  await p.reload(); await p.waitForTimeout(600);
  check('no tabs after reload', await count('.tab') === 0);
  const home2 = await p.$$eval('.hist tbody tr[data-open]', r => r.map(x => x.textContent.replace(/\s+/g, ' ').trim()));
  check('Home hydrated from saved run', home2.length === 1 && /Boston/.test(home2[0]), home2[0]);
  await p.click('.hist tr[data-open="example.org"] td.t'); await p.waitForTimeout(500);
  const rows2 = await p.$$eval('#dock tbody tr', r => r.map(x => x.textContent.replace(/\s+/g, ' ').trim()));
  check('reopened trace rebuilt from run', rows2.length === 6 && /Chicago/.test(rows2[3]) && /Prairie Broadband/.test(rows2[1]), rows2.join(' || ').slice(0, 200));
  check('atlas drew without error', errors.length === 0);
  await p.screenshot({ path: path.join(__dirname, 'shots', 'live-reopened.png') });

  // failure paths
  await p.fill('#target', 'nowhere.invalid'); await p.keyboard.press('Enter'); await p.waitForTimeout(500);
  check('dns failure surfaces', /Couldn't find/.test(await txt('#status')), await txt('#status'));
  await p.evaluate(() => { window.__FAKE_ROUTE = []; });
  await p.fill('#target', 'empty.example'); await p.keyboard.press('Enter'); await p.waitForTimeout(600);
  check('empty tool output handled', /printed nothing|couldn't run/i.test(await txt('#status')), await txt('#status'));
  await p.evaluate(() => { window.__FAKE_ROUTE = null; });

  // Trends + report for a live run
  await p.fill('#target', 'example.org'); await p.keyboard.press('Enter'); await p.waitForTimeout(2600);
  await p.click('#viewSeg [data-view="runs"]'); await p.waitForTimeout(300);
  check('trends has two runs', await count('[data-del-run]') === 2, 'runs=' + await count('[data-del-run]'));
  await p.click('[data-export="runs-report"]'); await p.waitForTimeout(300);
  const saved = await p.evaluate(() => window.__FAKE_SAVED);
  check('report saved through the native dialog', !!saved && /\.html$/.test(saved.path) && /be-12\.core1\.chi/.test(saved.text) && /198\.51\.100\.7/.test(saved.text) && !/simulated/.test(saved.text), saved && saved.path);
  check('status confirms the save', /Saved /.test(await txt('#status')), await txt('#status'));
  await p.evaluate(() => { window.__FAKE_CANCEL_SAVE = true; }); await p.click('[data-export="runs-csv"]'); await p.waitForTimeout(200);
  check('cancelled save says so', /Not saved/.test(await txt('#status')), await txt('#status'));

  // a destination that drops probes: the tool trails off into silence, the tail is dropped and the status says so.
  // The router named for Denver sits in Denver even though the address database files it under London.
  await p.evaluate(() => { window.__FAKE_REACHED = false; window.__FAKE_ROUTE = [
    { hop: 1, addrs: ['192.168.1.1'], rtts: [1.1, 1.0, 1.2] }, { hop: 2, addrs: ['198.51.100.9'], rtts: [12, 12.4, 12.1] },
    { hop: 3, addrs: [], rtts: [null, null, null] }, { hop: 4, addrs: [], rtts: [null, null, null] }, { hop: 5, addrs: [], rtts: [null, null, null] },
    { hop: 6, addrs: [], rtts: [null, null, null] }, { hop: 7, addrs: [], rtts: [null, null, null] }]; });
  await p.fill('#target', 'silent.example'); await p.keyboard.press('Enter'); await p.waitForTimeout(2400);
  const rows3 = await p.$$eval('#dock tbody tr', r => r.map(x => x.textContent.replace(/\s+/g, ' ').trim()));
  check('silent tail dropped', rows3.length === 2, rows3.length + ' rows');
  check('status explains the silence', /didn't answer/.test(await txt('#status')) && /5 hops after/.test(await txt('#status')), await txt('#status'));
  check('place taken from the router name', /Denver/.test(rows3[1] || '') && !/London/.test(rows3[1] || ''), rows3[1]);
  await p.evaluate(() => { delete window.__FAKE_REACHED; window.__FAKE_ROUTE = null; });

  check('no console errors', errors.length === 0, errors.join(' | '));
  await browser.close();
  console.log(failed ? `\n${failed} check(s) failed` : '\nall checks passed');
  process.exit(failed ? 1 : 0);
})();

/* Demo-mode smoke test: the page in a plain browser, no backend, simulated routes.
 * Run: node tests/smoke.js */
const path = require('path');
const { chromium } = require(process.env.PLAYWRIGHT_MODULE || 'playwright');
(async () => {
  const browser = await chromium.launch(); const errors = []; let failed = 0;
  const check = (name, ok, extra = '') => { console.log((ok ? 'ok   ' : 'FAIL ') + name + (extra ? '  ' + extra : '')); if (!ok) failed++; };
  const ctx = await browser.newContext({ viewport: { width: 1360, height: 860 }, colorScheme: 'dark' });
  const p = await ctx.newPage();
  p.on('console', m => { if (m.type() === 'error' && !/ERR_|Failed to load resource/.test(m.text())) errors.push(m.text()); });
  p.on('pageerror', e => errors.push('PAGEERROR ' + e.message));
  await p.route(/^https?:/, r => r.abort());
  await p.goto('file://' + path.join(__dirname, '..', 'ui', 'index.html')); await p.waitForTimeout(600);
  const txt = async sel => (await p.textContent(sel)).replace(/\s+/g, ' ').trim();
  const count = sel => p.$$eval(sel, a => a.length);

  check('demo mode (no bridge)', await p.evaluate(() => !window.HopscotchLive));
  check('starts on Home with sample history', await count('.hist tbody tr[data-open]') === 4 && await count('.tab') === 0);
  check('badge says Simulated route', (await txt('.sim')).startsWith('Simulated route'));

  await p.fill('#target', 'example.org'); await p.keyboard.press('Enter'); await p.waitForTimeout(9000);
  check('trace completes', /Trace complete/.test(await txt('#status')), await txt('#status'));
  check('dock has hops', await count('#dock tbody tr') > 3);
  check('probe button offered', !(await p.$eval('#probeBtn', e => e.hidden)));
  await p.click('#probeBtn'); await p.waitForTimeout(2500);
  await p.click('#viewSeg [data-view="ping"]'); await p.waitForTimeout(300);
  check('ping view charts samples', +(await txt('.ping-stats div:first-child b')) >= 4);
  await p.click('#viewSeg [data-view="runs"]'); await p.waitForTimeout(300);
  check('trends view renders', /Trends for example\.org/.test(await txt('#viewRuns h2')));
  await p.click('#mapBtn'); await p.waitForTimeout(200);
  await p.click('#mapPop .chip:has(input[value="cyber"])'); await p.waitForTimeout(300);
  check('cyber skin applies with its map', await p.evaluate(() => document.documentElement.dataset.skin === 'cyber' && document.getElementById('setMapStyle').value === 'cyber'));
  await p.keyboard.press('Escape');
  await p.click('#homeBtn'); await p.waitForTimeout(300);
  await p.click('.hist tr[data-open="example.org"] [data-forget]'); await p.waitForTimeout(100); await p.click('[data-forget-yes]'); await p.waitForTimeout(300);
  check('removing a target closes its tab', await count('.tab') === 0 && await count('.hist tbody tr[data-open]') === 4);
  check('no console errors', errors.length === 0, errors.join(' | '));
  await browser.close();
  console.log(failed ? `\n${failed} check(s) failed` : '\nall checks passed');
  process.exit(failed ? 1 : 0);
})();

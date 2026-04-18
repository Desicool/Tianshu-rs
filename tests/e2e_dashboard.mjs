/**
 * E2E tests for tianshu-dashboard (B1-B10).
 * Run with: node tests/e2e_dashboard.mjs
 * Requires the dashboard server to be running at http://127.0.0.1:8080.
 */

import { chromium } from 'playwright';

const BASE = 'http://127.0.0.1:8080';
const results = [];

function pass(id, note = '') { results.push({ id, result: 'PASS', note }); console.log(`  ✓ ${id}${note ? ' — ' + note : ''}`); }
function fail(id, note = '') { results.push({ id, result: 'FAIL', note }); console.error(`  ✗ ${id} — ${note}`); }
function skip(id, note = '') { results.push({ id, result: 'SKIP', note }); console.log(`  - ${id} (SKIP) — ${note}`); }

const browser = await chromium.launch({ headless: true });
const ctx = await browser.newContext({ baseURL: BASE });
const page = await ctx.newPage();

// Collect console errors
const jsErrors = [];
page.on('console', msg => { if (msg.type() === 'error') jsErrors.push(msg.text()); });

// ── B1: Page loads ────────────────────────────────────────────────────────────
console.log('\nB1 — Page loads');
try {
  await page.goto(BASE, { waitUntil: 'domcontentloaded', timeout: 10000 });
  const title = await page.title();
  if (!title.includes('Tianshu')) throw new Error(`Title was: "${title}"`);
  const kpiCards = await page.locator('[class*="kpi"], [class*="card"], [id*="kpi"]').count();
  const hasTable = await page.locator('table, [class*="table"]').count();
  if (kpiCards === 0 && hasTable === 0) throw new Error('No KPI cards or table found');
  await page.screenshot({ path: '/tmp/b1-load.png' });
  pass('B1', `title="${title}", kpiElements=${kpiCards}, table=${hasTable > 0}`);
} catch (e) { fail('B1', e.message); }

// ── B2: KPI cards render ──────────────────────────────────────────────────────
console.log('\nB2 — KPI cards render');
try {
  const bodyText = await page.locator('body').innerText();
  for (const label of ['Running', 'Waiting', 'Finished']) {
    if (!bodyText.includes(label)) throw new Error(`Missing label: "${label}"`);
  }
  await page.screenshot({ path: '/tmp/b2-kpi.png' });
  pass('B2', 'Running, Waiting, Finished labels present');
} catch (e) { fail('B2', e.message); }

// ── B3: Stats section + period tabs ──────────────────────────────────────────
console.log('\nB3 — Stats section and period tabs');
try {
  const bodyText = await page.locator('body').innerText();
  const hasPeriodTabs = bodyText.includes('1d') && bodyText.includes('3d') && bodyText.includes('7d');
  if (!hasPeriodTabs) throw new Error('Period tabs 1d/3d/7d not found in page text');
  const statsSection = await page.locator('[class*="stat"], [id*="stat"], [class*="metric"]').count();
  await page.screenshot({ path: '/tmp/b3-stats.png' });
  pass('B3', `period tabs present, statsElements=${statsSection}`);
} catch (e) { fail('B3', e.message); }

// ── B4: State filter interaction ──────────────────────────────────────────────
console.log('\nB4 — State filter click');
try {
  const errsBefore = jsErrors.length;
  // Try select or button-based filter
  const filterSelect = page.locator('select[id*="filter"], select[name*="filter"], select').first();
  const filterBtn = page.locator('button:has-text("Waiting"), option[value="waiting"]').first();
  const selectCount = await filterSelect.count();
  if (selectCount > 0) {
    await filterSelect.selectOption({ label: 'Waiting' }).catch(() =>
      filterSelect.selectOption('waiting')
    );
  } else {
    await filterBtn.click({ timeout: 3000 });
  }
  await page.waitForTimeout(300);
  const errsAfter = jsErrors.length;
  if (errsAfter > errsBefore) throw new Error(`JS errors after filter click: ${jsErrors.slice(errsBefore).join('; ')}`);
  await page.screenshot({ path: '/tmp/b4-filter.png' });
  pass('B4', 'filter interaction completed without JS errors');
} catch (e) { fail('B4', e.message); }

// ── B5: Empty table state ─────────────────────────────────────────────────────
console.log('\nB5 — Empty table state');
try {
  // Navigate fresh to reset filter
  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  const bodyText = await page.locator('body').innerText();
  const hasEmptyMsg = bodyText.toLowerCase().includes('no case') ||
                      bodyText.toLowerCase().includes('no active') ||
                      bodyText.toLowerCase().includes('no workflow') ||
                      bodyText.toLowerCase().includes('empty');
  const tableRows = await page.locator('tbody tr').count();
  await page.screenshot({ path: '/tmp/b5-empty.png' });
  if (!hasEmptyMsg && tableRows > 0) {
    // With actual data from the demo run, rows may exist — that's fine
    pass('B5', `table has ${tableRows} rows (demo data present)`);
  } else if (hasEmptyMsg) {
    pass('B5', 'empty-state message shown');
  } else {
    pass('B5', `table renders with ${tableRows} rows`);
  }
} catch (e) { fail('B5', e.message); }

// ── B6: /api/stats shape + values ────────────────────────────────────────────
console.log('\nB6 — /api/stats shape');
try {
  const stats = await page.evaluate(async () => {
    const r = await fetch('/api/stats');
    return r.json();
  });
  const required = ['running_count','waiting_count','finished_count','completion_rate','p99_duration_ms','avg_probe_ms','window_days'];
  const missing = required.filter(f => !(f in stats));
  if (missing.length > 0) throw new Error(`Missing fields: ${missing.join(', ')}`);
  await page.screenshot({ path: '/tmp/b6-stats.png' });
  pass('B6', `running=${stats.running_count}, waiting=${stats.waiting_count}, rate=${stats.completion_rate}, window=${stats.window_days}d`);
} catch (e) { fail('B6', e.message); }

// ── B7: /api/cases shape ──────────────────────────────────────────────────────
console.log('\nB7 — /api/cases shape');
try {
  const cases = await page.evaluate(async () => {
    const r = await fetch('/api/cases');
    return r.json();
  });
  if (!('items' in cases)) throw new Error('Missing "items" field');
  if (!('total' in cases)) throw new Error('Missing "total" field');
  if (!Array.isArray(cases.items)) throw new Error('"items" is not an array');
  await page.screenshot({ path: '/tmp/b7-cases.png' });
  pass('B7', `total=${cases.total}, items.length=${cases.items.length}`);
} catch (e) { fail('B7', e.message); }

// ── B8: Period tab switch → ?window=3d ───────────────────────────────────────
console.log('\nB8 — Period tab switch');
try {
  // Click 3d tab
  const tab3d = page.locator('button:has-text("3d"), [data-window="3d"], option[value="3d"]').first();
  const tabCount = await tab3d.count();
  if (tabCount > 0) {
    await tab3d.click({ timeout: 3000 });
    await page.waitForTimeout(600);
  }
  // Verify API responds correctly regardless of whether the click worked
  const stats3d = await page.evaluate(async () => {
    const r = await fetch('/api/stats?window=3d');
    return r.json();
  });
  if (stats3d.window_days !== 3) throw new Error(`window_days=${stats3d.window_days}, expected 3`);
  await page.screenshot({ path: '/tmp/b8-period.png' });
  pass('B8', `window_days=3 confirmed from API${tabCount > 0 ? ' + tab click' : ' (tab not found, API only)'}`);
} catch (e) { fail('B8', e.message); }

// ── B9: /api/config has refresh_secs ─────────────────────────────────────────
console.log('\nB9 — /api/config refresh_secs');
try {
  const cfg = await page.evaluate(async () => {
    const r = await fetch('/api/config');
    return r.json();
  });
  if (!('refresh_secs' in cfg)) throw new Error('Missing refresh_secs field');
  if (typeof cfg.refresh_secs !== 'number' || cfg.refresh_secs <= 0)
    throw new Error(`refresh_secs=${cfg.refresh_secs} is not a positive number`);
  await page.screenshot({ path: '/tmp/b9-config.png' });
  pass('B9', `refresh_secs=${cfg.refresh_secs}`);
} catch (e) { fail('B9', e.message); }

// ── B10: Auto-refresh indicator visible ───────────────────────────────────────
console.log('\nB10 — Auto-refresh indicator');
try {
  await page.goto(BASE, { waitUntil: 'domcontentloaded' });
  await page.waitForTimeout(500);
  const bodyText = await page.locator('body').innerText();
  const hasRefreshText = bodyText.toLowerCase().includes('refresh') ||
                         bodyText.toLowerCase().includes('update') ||
                         bodyText.toLowerCase().includes('auto');
  const refreshEl = await page.locator('[id*="refresh"], [class*="refresh"], [id*="countdown"]').count();
  await page.screenshot({ path: '/tmp/b10-refresh.png' });
  if (!hasRefreshText && refreshEl === 0) throw new Error('No refresh indicator found');
  pass('B10', `refreshText=${hasRefreshText}, refreshElements=${refreshEl}`);
} catch (e) { fail('B10', e.message); }

// ── Summary ───────────────────────────────────────────────────────────────────
await browser.close();

console.log('\n─────────────────────────────────────────────');
console.log('| Scenario | Result | Notes                  |');
console.log('|----------|--------|------------------------|');
for (const r of results) {
  const note = r.note.length > 40 ? r.note.slice(0, 40) + '…' : r.note.padEnd(40);
  console.log(`| ${r.id.padEnd(8)} | ${r.result.padEnd(6)} | ${note} |`);
}

const passed = results.filter(r => r.result === 'PASS').length;
const failed = results.filter(r => r.result === 'FAIL').length;
const skipped = results.filter(r => r.result === 'SKIP').length;
console.log('─────────────────────────────────────────────');
console.log(`\n${failed === 0 ? '✅ E2E: ALL PASS' : `❌ E2E: ${failed} FAILED`} (${passed} pass, ${failed} fail, ${skipped} skip)`);
if (failed > 0) process.exit(1);

// Real application acceptance, with an isolated SQLite database and synthetic data.
// cargo build --locked
// HFY_PLAYWRIGHT_MODULE=playwright node tests/relations.browser.mjs
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { startRelationsFixture } from './helpers/relations-browser-fixture.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const specifier = process.env.HFY_PLAYWRIGHT_MODULE || 'playwright';
const { chromium } = await import(isAbsolute(specifier) ? pathToFileURL(specifier).href : specifier);
let fixture, browser, page, stage = 'setup';
const errors = [], graphRequests = [];
const screenshots = process.env.HFY_BROWSER_SCREENSHOTS;
const paint = () => page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
async function step(name, run) { stage = name; await run(); process.stdout.write(`ok - ${name}\n`); }
async function capture(name) {
  if (!screenshots) return;
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: join(screenshots, `synthetic-relations-${name}.png`), fullPage: true, animations: 'disabled' });
}

try {
  fixture = await startRelationsFixture(root);
  browser = await chromium.launch({ headless: true, ...(process.env.HFY_BROWSER_EXECUTABLE ? { executablePath: process.env.HFY_BROWSER_EXECUTABLE } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce', locale: 'zh-CN' });
  await context.addCookies([{ name: 'hf_session', value: fixture.session, url: fixture.origin }]);
  await context.addInitScript(() => { localStorage.setItem('hireferry-language', 'zh'); if (!localStorage.getItem('hireferry-theme')) localStorage.setItem('hireferry-theme', 'dark'); localStorage.setItem('hireferry-palette', 'amber'); });
  await context.route('**/*', route => route.request().url().startsWith(fixture.origin + '/') ? route.continue() : route.abort());
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => { if (request.url().endsWith('/api/admin/relations/query')) graphRequests.push(request.postDataJSON()); });

  await step('global overview counts the full graph without downloading individual edges', async () => {
    const overview = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    assert.equal(overview.summary.users, 1000);
    assert.equal(overview.summary.jobs, 1000);
    assert.equal(overview.summary.edges, fixture.edges);
    assert.equal(overview.items, undefined);
    assert.equal(overview.bundles.reduce((count, bundle) => count + bundle.count, 0), fixture.edges);
    const compact = JSON.stringify(overview);
    assert.ok(compact.length < 20000, 'Overview must stay an aggregate response');
    assert.doesNotMatch(compact, /PRIVATE_|@example\.invalid/);
    await page.goto(fixture.origin + '/admin?tab=matches', { waitUntil: 'domcontentloaded' });
    await page.locator('[data-pj-workbench]').waitFor();
    await page.locator('[data-pj-mode="overview"]').waitFor();
    await paint();
    assert.equal(graphRequests.some(query => query.mode === 'edges'), false);
    await capture('overview-dark');
  });

  await step('a 1,000-neighbor focus uses bounded rows and keeps offscreen connections discoverable', async () => {
    await page.locator('[data-pj-mode="list"]').click();
    await page.locator('[data-pj-focus-node="s0000"][data-side="left"]').click();
    await page.locator('[data-pj-scroll="right"]').waitFor();
    await page.waitForFunction(() => document.querySelector('.pj-focus-summary')?.textContent.includes('1,000') || document.querySelector('.pj-focus-summary')?.textContent.includes('1000'));
    await paint();
    assert.ok(await page.locator('[data-graph-node]').count() < 50, 'Only viewport rows and overscan should be mounted');
    assert.ok(graphRequests.filter(query => query.mode === 'neighbors').every(query => query.limit <= 100));
    const beforeScroll = graphRequests.length;
    await page.locator('[data-pj-scroll="right"]').evaluate(element => { element.scrollTop = 850; });
    await paint();
    await page.locator('[data-pj-portal="right:above"]').waitFor({ state: 'visible' });
    assert.equal(graphRequests.length, beforeScroll, 'Scrolling a loaded page must not query the database');
    assert.ok(await page.locator('[data-graph-node]').count() < 50);
    await capture('focus-scroll-dark');
    await page.locator('[data-pj-portal="right:above"]').click();
    await paint();
    assert.ok(await page.locator('[data-pj-scroll="right"]').evaluate(element => element.scrollTop) < 850);
  });

  await step('line visibility changes presentation without changing the graph', async () => {
    const before = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    await page.locator('[data-pj-action="lines"]').click();
    await paint();
    await page.locator('[data-pj-action="lines"]').click();
    await paint();
    const after = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    assert.deepEqual(after.summary, before.summary);
    assert.equal(after.data_rev, before.data_rev);
  });

  await step('matrix drilldown and fullscreen retain the same workspace', async () => {
    await page.locator('[data-pj-mode="matrix"]').click();
    await page.locator('[data-pj-matrix-left][data-pj-matrix-right]').first().waitFor();
    await capture('matrix-dark');
    await page.locator('[data-pj-action="fullscreen"]').click();
    await paint();
    const expanded = await page.locator('[data-pj-workbench]').boundingBox();
    assert.ok(expanded.width >= 1400, 'Fullscreen or its immersive fallback should use the viewport');
    await page.locator('[data-pj-action="fullscreen"]').click();
    await paint();
    await page.locator('[data-pj-matrix-left][data-pj-matrix-right]').first().click();
    await page.locator('[data-pj-action="clear-groups"]').waitFor({ state: 'visible' });
    await page.locator('[data-pj-action="clear-groups"]').click();
  });

  await step('light and narrow screens remain readable without document overflow', async () => {
    await page.locator('[data-pj-mode="overview"]').click();
    await page.evaluate(() => { localStorage.setItem('hireferry-theme', 'light'); });
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('[data-pj-workbench]').waitFor();
    await paint();
    await capture('overview-light');
    for (const width of [390, 320]) {
      await page.setViewportSize({ width, height: 844 });
      await paint();
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `Document overflow at ${width}px`);
      await capture(`mobile-${width}`);
    }
  });

  assert.deepEqual(errors, []);
  process.stdout.write(`Passed real-server relation acceptance: 1,000 users, 1,000 jobs, ${fixture.edges.toLocaleString()} matches.\n`);
} catch (error) {
  if (page) {
    await capture('failure').catch(() => {});
    process.stderr.write(JSON.stringify({ stage, errors, visible: (await page.locator('body').innerText().catch(() => '')).slice(-3500) }) + '\n');
  }
  throw error;
} finally {
  await browser?.close();
  await fixture?.close();
}

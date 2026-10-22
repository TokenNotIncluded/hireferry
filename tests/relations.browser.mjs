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
const errors = [], graphRequests = [], webmcpWarnings = [];
const relationToolNames = ['hf_view_open', 'hf_view_patch', 'hf_graph_query', 'hf_graph_read', 'hf_view_save', 'hf_view_list'].sort();
const screenshots = process.env.HFY_BROWSER_SCREENSHOTS;
const paint = () => page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
async function step(name, run) { stage = name; await run(); process.stdout.write(`ok - ${name}\n`); }
async function until(check, message) {
  const deadline = Date.now() + 15000;
  while (Date.now() < deadline) { if (await check()) return; await new Promise(resolve_ => setTimeout(resolve_, 150)); }
  throw new Error(message);
}
async function capture(name) {
  if (!screenshots) return;
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: join(screenshots, `synthetic-relations-${name}.png`), fullPage: true, animations: 'disabled' });
}
async function nativeToolNames() {
  return page.evaluate(async () => {
    const context = document.modelContext || navigator.modelContext;
    return (await context.getTools()).map(tool => tool.name).sort();
  });
}
async function waitNativeTools(expected) {
  let actual = [];
  await until(async () => {
    actual = await nativeToolNames();
    assert.equal(new Set(actual).size, actual.length, 'Native WebMCP must not contain duplicate names');
    return typeof expected === 'number' ? actual.length === expected : JSON.stringify(actual) === JSON.stringify(expected);
  }, `Native WebMCP did not reach the expected catalog (${typeof expected === 'number' ? expected : expected.length} tools)`);
  return actual;
}
async function navigateAdminTab(tab) {
  const previous = await page.locator('#root').getAttribute('data-navigation-sequence');
  await page.locator(`.sidebar-nav a[href="/admin?tab=${tab}"]`).first().click();
  await page.waitForFunction(({ tab_, previous_ }) =>
    location.pathname === '/admin' && new URLSearchParams(location.search).get('tab') === tab_ &&
    Number(document.querySelector('#root')?.dataset.navigationSequence) > Number(previous_) &&
    document.querySelector('#root')?.dataset.navigationState === 'ready', { tab_: tab, previous_: previous });
  await paint();
}
async function executeNativeTool(name, args) {
  const raw = await page.evaluate(async ({ name_, args_ }) => {
    const context = document.modelContext || navigator.modelContext;
    const tool = (await context.getTools()).find(candidate => candidate.name === name_);
    if (!tool) throw new Error(`Native tool is missing: ${name_}`);
    // Chrome 151 takes JSON text; newer Chrome accepts the structured argument.
    // Both paths execute the real native descriptor, never a retained callback.
    const major = Number(navigator.userAgent.match(/(?:HeadlessChrome|Chrome)\/(\d+)/)?.[1]);
    return context.executeTool(tool, major >= 155 ? args_ : JSON.stringify(args_));
  }, { name_: name, args_: args });
  const result = typeof raw === 'string' ? JSON.parse(raw) : raw;
  assert.notEqual(result?.isError, true, `Native ${name} execution failed`);
  const text = result?.content?.find(item => item.type === 'text')?.text;
  assert.equal(typeof text, 'string', `Native ${name} should return a useful MCP result`);
  return JSON.parse(text);
}

try {
  fixture = await startRelationsFixture(root);
  browser = await chromium.launch({ headless: true, args: ['--enable-blink-features=WebMCP'], ...(process.env.HFY_BROWSER_EXECUTABLE ? { executablePath: process.env.HFY_BROWSER_EXECUTABLE } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce', locale: 'zh-CN' });
  await context.addCookies([{ name: 'hf_session', value: fixture.session, url: fixture.origin }]);
  await context.addInitScript(() => { localStorage.setItem('hireferry-language', 'zh'); if (!localStorage.getItem('hireferry-theme')) localStorage.setItem('hireferry-theme', 'dark'); localStorage.setItem('hireferry-palette', 'amber'); });
  await context.route('**/*', route => route.request().url().startsWith(fixture.origin + '/') ? route.continue() : route.abort());
  page = await context.newPage();
  page.setDefaultTimeout(15000);
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => {
    if (message.type() === 'warning' && /^(?:Relation )?WebMCP /.test(message.text())) webmcpWarnings.push(message.text());
  });
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
    await page.waitForFunction(() => document.querySelector('[data-pj-scroll="right"]')?.scrollTop >= 850 && !document.querySelector('[data-pj-portal="right:above"]')?.disabled);
    assert.equal(graphRequests.length, beforeScroll, 'Scrolling a loaded page must not query the database');
    assert.ok(await page.locator('[data-graph-node]').count() < 50);
    await capture('focus-scroll-dark');
    await page.locator('[data-pj-portal="right:above"]').click();
    await paint();
    assert.ok(await page.locator('[data-pj-scroll="right"]').evaluate(element => element.scrollTop) < 850);
  });

  await step('line visibility changes presentation without changing the graph', async () => {
    const before = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    const scrollBefore = await page.locator('[data-pj-scroll="right"]').evaluate(element => element.scrollTop);
    await page.locator('[data-pj-action="lines"]').click();
    await paint();
    assert.equal(await page.locator('[data-pj-scroll="right"]').evaluate(element => element.scrollTop), scrollBefore, 'Hiding lines keeps the scroll anchor');
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
    await page.locator('[data-pj-matrix-left][data-pj-matrix-right]:not(:disabled)').first().click();
    await page.locator('[data-pj-action="clear-groups"]').waitFor({ state: 'visible' });
    await page.locator('[data-pj-action="clear-groups"]').click();
  });

  await step('MCP controls the visible workspace and acknowledges only rendered revisions', async () => {
    let current;
    await until(async () => {
      current = await fixture.mcp('hf_view_open', { response: 'state' });
      return current.ui.status === 'applied' && current.state.mode === 'focus' && !current.state.filters.left_group_ids?.length;
    }, 'The browser should finish synchronizing its local view');
    const before = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    const changed = await fixture.mcp('hf_view_patch', { view_id: current.view_id, if_view_rev: current.view_rev, operation_id: 'synthetic-mcp-matrix', commands: [{ op: 'set_mode', mode: 'matrix' }, { op: 'set_presentation', presentation: 'immersive' }] });
    assert.notEqual(changed.ui.status, 'applied', 'An accepted command is not yet a rendered view');
    await page.locator('[data-pj-mode="matrix"][aria-pressed="true"]').waitFor();
    await page.locator('[data-pj-workbench].is-immersive').waitFor();
    await until(async () => {
      current = await fixture.mcp('hf_view_open', { mode: 'resume', view_id: changed.view_id, response: 'state' });
      return current.view_rev === changed.view_rev && current.ui.status === 'applied';
    }, 'MCP should see the browser acknowledge the requested revision after rendering');
    const result = await fixture.mcp('hf_graph_query', { query: { mode: 'neighbors', focus: { side: 'left', id: 's0000' } } });
    assert.equal(result.items, undefined, 'Default AI queries return a summary and handle');
    const rows = await fixture.mcp('hf_graph_read', { result_id: result.result_id, limit: 100, fields: ['id', 'label'], max_bytes: 2048 });
    assert.ok(Buffer.byteLength(JSON.stringify(rows)) <= 2048, 'AI output respects its explicit byte budget');
    assert.ok(rows.returned > 0 && rows.returned <= 100 && rows.next_cursor);
    assert.doesNotMatch(JSON.stringify(rows), /PRIVATE_|@example\.invalid/);
    const request = await fixture.mcp('hf_view_patch', { view_id: current.view_id, if_view_rev: current.view_rev, operation_id: 'synthetic-fullscreen-request', commands: [{ op: 'request_fullscreen' }] });
    await until(async () => {
      current = await fixture.mcp('hf_view_open', { mode: 'resume', view_id: request.view_id, response: 'state' });
      return current.ui.status === 'applied' && current.ui.fullscreen === 'requires_user_gesture';
    }, 'Native fullscreen remains an explicit browser gesture');
    assert.equal(await page.evaluate(() => document.fullscreenElement === null), true);
    const after = await fixture.api('/api/admin/relations/query', { mode: 'overview' });
    assert.deepEqual(after.summary, before.summary);
    assert.equal(after.data_rev, before.data_rev);
    await fixture.mcp('hf_view_patch', { view_id: current.view_id, if_view_rev: current.view_rev, operation_id: 'synthetic-back-to-overview', commands: [{ op: 'set_mode', mode: 'overview' }, { op: 'set_presentation', presentation: 'standard' }] });
    await page.locator('[data-pj-mode="overview"][aria-pressed="true"]').waitFor();
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

  // Keep sign-out last: the fixture's administrator session is deliberately
  // revoked by the final stage, after the existing real-server graph checks.
  let baseToolNames, combinedToolNames;
  await step('native Chrome WebMCP combines 38 business tools with exactly six relation tools', async () => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    assert.equal(await page.evaluate(() => typeof (document.modelContext || navigator.modelContext)?.getTools), 'function', 'This gate requires native Chrome WebMCP; the Blink feature must be enabled');
    assert.equal(await page.evaluate(() => typeof (document.modelContext || navigator.modelContext)?.executeTool), 'function');
    await navigateAdminTab('overview');
    baseToolNames = await waitNativeTools(38);
    assert.ok(baseToolNames.every(name => name.startsWith('hireferry_')));
    assert.equal(baseToolNames.some(name => relationToolNames.includes(name)), false);
    combinedToolNames = [...baseToolNames, ...relationToolNames].sort();
    await navigateAdminTab('matches');
    await page.locator('[data-pj-workbench]').waitFor();
    await waitNativeTools(combinedToolNames);
  });

  await step('native executeTool reaches the real relation service and returns a bounded graph summary', async () => {
    const response = page.waitForResponse(value => new URL(value.url()).pathname === '/api/admin/relations/tools/hf_graph_query' && value.request().method() === 'POST');
    const result = await executeNativeTool('hf_graph_query', { query: { mode: 'overview' }, response: 'summary', max_bytes: 2048 });
    const http = await response;
    assert.equal(http.status(), 200);
    assert.equal(http.request().headers()['x-hireferry-ai'], 'webmcp');
    assert.equal(result.summary.users, 1000);
    assert.equal(result.summary.jobs, 1000);
    assert.equal(result.summary.edges, fixture.edges);
    assert.equal(typeof result.result_id, 'string');
    assert.ok(result.result_id.length > 0);
    assert.equal(result.items, undefined);
    assert.ok(Buffer.byteLength(JSON.stringify(result)) <= 2048);
    assert.doesNotMatch(JSON.stringify(result), /PRIVATE_|@example\.invalid/);
    assert.deepEqual(await http.json(), result);
    await waitNativeTools(combinedToolNames);
  });

  await step('native tool ownership survives languages, feedback and repeated SPA route changes', async () => {
    await page.locator('#feedback-launch').click();
    await page.locator('#feedback-panel').waitFor({ state: 'visible' });
    await waitNativeTools(combinedToolNames);
    for (const language of ['en', 'zh-CN', 'en', 'zh-CN']) {
      await page.locator('[data-action="language"]').first().click();
      await page.waitForFunction(value => document.documentElement.lang === value && document.querySelector('#root')?.dataset.navigationState === 'ready', language);
      await paint();
      assert.equal(await page.locator('#feedback-panel').isVisible(), true);
      await waitNativeTools(combinedToolNames);
    }
    await page.locator('[data-feedback-close]').click();
    await waitNativeTools(combinedToolNames);
    for (let cycle = 0; cycle < 2; cycle++) {
      await navigateAdminTab('overview');
      assert.equal(await page.locator('[data-pj-workbench]').count(), 0);
      await waitNativeTools(baseToolNames);
      await navigateAdminTab('matches');
      await page.locator('[data-pj-workbench]').waitFor();
      await waitNativeTools(combinedToolNames);
    }
    assert.deepEqual(webmcpWarnings, [], 'Language and route changes must not cause rejected duplicate registrations');
  });

  await step('signing out removes both native business and relation tool owners', async () => {
    const response = page.waitForResponse(value => new URL(value.url()).pathname === '/api/auth/logout' && value.request().method() === 'POST');
    await page.locator('.sidebar-user [data-action="logout"]').click();
    assert.equal((await response).status(), 200);
    await page.waitForURL(value => value.pathname === '/');
    await waitNativeTools([]);
    await paint();
    assert.deepEqual(await nativeToolNames(), [], 'A late discovery response must not restore tools after sign-out');
  });

  assert.deepEqual(errors, []);
  assert.deepEqual(webmcpWarnings, []);
  process.stdout.write(`Passed real-server relation acceptance: 1,000 users, 1,000 jobs, ${fixture.edges.toLocaleString()} matches.\n`);
} catch (error) {
  if (page) {
    await capture('failure').catch(() => {});
    process.stderr.write(JSON.stringify({ stage, errors, webmcpWarnings, nativeTools: await nativeToolNames().catch(() => []), visible: (await page.locator('body').innerText().catch(() => '')).slice(-3500) }) + '\n');
  }
  throw error;
} finally {
  await browser?.close();
  await fixture?.close();
}

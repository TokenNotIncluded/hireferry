// HF-QA-002: real Chrome WebMCP, real SPA, synthetic localhost sessions only.
// HFY_PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node tests/webmcp-lifecycle.browser.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..'), staticRoot = resolve(root, 'static');
const moduleName = process.env.HFY_PLAYWRIGHT_MODULE || 'playwright';
const { chromium } = await import(isAbsolute(moduleName) ? pathToFileURL(moduleName).href : moduleName);
const config = { name: 'Synthetic WebMCP fixture', default_language: 'zh', google_enabled: false, humangrant_enabled: false, email_enabled: true, development: false, cli_distribution: { release_repository: 'TokenNotIncluded/hireferry', release_tag_prefix: 'cli-v' }, pricing: { currency: 'CNY', currency_decimals: 2 }, limits: { list_page_size: 50 }, providers: [] };
const guest = { user: null, admin: false, super_admin: false, staff: null, staff_role: null, bootstrap: false, csrf: null };
const member = { id: 'synthetic-member', name: 'Synthetic member', email: 'member@example.invalid', public_email: '', resume_md: '', role: 'seeker', employer_paid: true };
let session = { ...guest, admin: true, super_admin: true, csrf: 'synthetic-owner-session' }, eligible = true;
let heldEligibility = null, heldLogout = null, browser, page, origin, stage = 'setup';
const errors = [], warnings = [], unexpected = [], requests = [];
const deferred = () => { let resolve;const promise = new Promise(done => { resolve = done; });return { promise, resolve }; };
const hold = () => ({ started: deferred(), released: deferred() });
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://fixture.local');
    const json = (value, status = 200) => { response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });response.end(JSON.stringify(value)); };
    if (url.pathname.startsWith('/api/')) requests.push({ path: url.pathname, method: request.method });
    if (url.pathname === '/api/config') return json(config);
    if (url.pathname === '/api/session') return json(session);
    if (url.pathname === '/api/profile') return json(session.user);
    if (url.pathname === '/api/ai/eligibility') {
      const gate = heldEligibility;heldEligibility = null;
      if (gate) { gate.started.resolve();await gate.released.promise; }
      return json({ eligible });
    }
    if (url.pathname === '/api/auth/logout' && request.method === 'POST') {
      const gate = heldLogout;heldLogout = null;
      if (gate) { gate.started.resolve();await gate.released.promise; }
      session = { ...guest };return json({ ok: true });
    }
    if (url.pathname === '/api/admin/audit') return json({ items: [] });
    if (url.pathname.startsWith('/api/')) { unexpected.push(url.pathname);return json({ error: 'Unexpected fixture endpoint' }, 404); }
    const file = url.pathname.startsWith('/static/') ? resolve(staticRoot, decodeURIComponent(url.pathname.slice(8))) : resolve(staticRoot, 'index.html');
    if (!file.startsWith(staticRoot + '/')) throw new Error('Invalid fixture path');
    const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml' };
    response.writeHead(200, { 'content-type': types[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' });response.end(await readFile(file));
  } catch (error) { errors.push(error.message);response.writeHead(500);response.end('Synthetic fixture error'); }
});
const toolNames = () => page.evaluate(async () => (await document.modelContext.getTools()).map(tool => tool.name).sort());
const waitTools = async count => {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    const names = await toolNames();
    if (names.filter(name => name.startsWith('hireferry_')).length === count) return names;
    await page.waitForTimeout(25);
  }
  assert.fail(`Expected ${count} workspace tools; found ${JSON.stringify(await toolNames())}`);
};
const frames = () => page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))));
async function navigate(path) {
  await page.evaluate(path_ => { history.pushState({}, '', path_);window.dispatchEvent(new PopStateEvent('popstate')); }, path);
  await page.waitForFunction(() => document.querySelector('#root')?.dataset.navigationState === 'ready');await frames();
}
async function refreshIdentity(next) {
  session = next;
  const response = page.waitForResponse(response => new URL(response.url()).pathname === '/api/session');
  await page.evaluate(() => window.dispatchEvent(new StorageEvent('storage', { key: 'hireferry-session-change', newValue: 'synthetic-change' })));
  await response;
  await page.waitForFunction(() => document.querySelector('#root')?.dataset.configState === 'fresh');await frames();
}

try {
  await new Promise((done, reject) => { server.once('error', reject);server.listen(0, '127.0.0.1', done); });
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true, args: ['--enable-blink-features=WebMCP'], ...(process.env.HFY_BROWSER_EXECUTABLE ? { executablePath: process.env.HFY_BROWSER_EXECUTABLE } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce' });
  await context.addInitScript(() => {
    localStorage.setItem('hireferry-language', 'zh');
    // Observe registrations and retain callbacks; all registration, removal,
    // discovery and execution still go through Chrome's native implementation.
    const nativeContext = document.modelContext || navigator.modelContext;
    if (!nativeContext) return;
    window.fixtureRegistrations = 0;window.fixtureCallbacks = new Map();
    const register = nativeContext.registerTool.bind(nativeContext);
    nativeContext.registerTool = (tool, options) => { window.fixtureRegistrations++;window.fixtureCallbacks.set(tool.name, tool.execute);return register(tool, options); };
  });
  await context.route('**/*', route => route.request().url().startsWith(origin + '/') ? route.continue() : route.abort());
  page = await context.newPage();page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));page.on('console', message => { if (message.type() === 'warning' && message.text().startsWith('WebMCP ')) warnings.push(message.text()); });
  await page.goto(origin + '/');
  assert.equal(await page.evaluate(() => typeof document.modelContext?.getTools), 'function', 'This gate requires real WebMCP support; enable the WebMCP Blink feature in Chrome.');
  await page.waitForFunction(() => document.querySelector('#root')?.dataset.navigationState === 'ready');
  await waitTools(38);await frames();
  const original = await waitTools(38), registrations = await page.evaluate(() => window.fixtureRegistrations);
  stage = 'homepage language and feedback sequence';
  for (let i = 0; i < 8; i++) {
    if (i === 1) await page.locator('#feedback-launch').click();
    if (i === 5) await page.locator('[data-feedback-close]').click();
    await page.locator('[data-action="language"]').first().click();
    await page.waitForFunction(language => document.documentElement.lang === language, i % 2 ? 'zh-CN' : 'en');
    await frames();assert.deepEqual(await toolNames(), original);
  }
  for (const path of ['/cli', '/contact', '/']) { await navigate(path);await waitTools(38);assert.deepEqual(await toolNames(), original); }
  assert.equal(await page.evaluate(() => window.fixtureRegistrations), registrations, 'Presentation-only changes must not churn tool registrations');
  process.stdout.write('ok - real WebMCP: 38 tools survive eight language changes, feedback and SPA routes without re-registration\n');

  stage = 'identity change while an actual native tool is running';
  const gate = hold();heldEligibility = gate;
  await page.evaluate(async () => {
    window.fixtureRetainedAdmin = window.fixtureCallbacks.get('hireferry_admin_audit');
    const tool = (await document.modelContext.getTools()).find(tool => tool.name === 'hireferry_admin_audit');
    const major = Number(navigator.userAgent.match(/(?:HeadlessChrome|Chrome)\/(\d+)/)?.[1]);
    window.fixturePending = document.modelContext.executeTool(tool, major >= 155 ? {} : '{}').then(value => ({ value }), error => ({ error: error.name }));
  });
  await gate.started.promise;
  const manager = { ...guest, user: { ...member }, staff_role: 'manager', csrf: 'synthetic-member-session' };
  await refreshIdentity(manager);await waitTools(0);gate.released.resolve();
  const pending = await page.evaluate(() => window.fixturePending);
  const revokedResult = typeof pending.value === 'string' ? JSON.parse(pending.value) : pending.value;
  assert.ok(pending.error || revokedResult?.isError, `Revoked in-flight execution must not deliver private results: ${JSON.stringify(pending)}`);
  const before = requests.length;
  assert.equal((await page.evaluate(() => window.fixtureRetainedAdmin({}))).isError, true);
  assert.equal(requests.length, before);assert.equal(requests.some(request => request.path === '/api/admin/audit'), false);
  await refreshIdentity({ ...manager, admin: true, staff: { id: 'synthetic-employee' } });await waitTools(38);
  await refreshIdentity(manager);await waitTools(0);
  process.stdout.write('ok - pending native execution is revoked; employee grant removal leaves a manager with no business tools\n');

  stage = 'ordinary role and eligibility changes';
  await refreshIdentity({ ...guest, user: { ...member }, csrf: 'synthetic-member-session' });await waitTools(25);
  assert.equal((await toolNames()).some(name => name.startsWith('hireferry_admin_')), false);
  await refreshIdentity({ ...guest, user: { ...member, role: 'employer' }, csrf: 'synthetic-member-session' });await waitTools(31);
  await page.waitForFunction(async () => (await document.modelContext.getTools()).some(tool => tool.name === 'hireferry_create_job'));
  assert.equal((await toolNames()).includes('hireferry_get_mandate'), false);assert.ok((await toolNames()).includes('hireferry_create_job'));
  eligible = false;await page.evaluate(() => window.dispatchEvent(new Event('online')));await waitTools(0);
  eligible = true;await page.evaluate(() => window.dispatchEvent(new Event('online')));await waitTools(31);
  process.stdout.write('ok - same-account role changes refresh schemas and fresh eligibility removes/restores the user catalog\n');

  stage = 'independent owners and immediate logout';
  await page.evaluate(async () => {
    const { createWebMCPOwner } = await import('/static/webmcp-lifecycle.js');
    // Abort between the native registration call and its Promise settlement.
    const pendingOwner = createWebMCPOwner();
    void pendingOwner.sync({ scope: 'synthetic-revoked', tools: [{ name: 'hf_test_pending_probe', description: 'Pending registration probe', inputSchema: { type: 'object', properties: {} }, execute: async () => 'must not remain' }] });
    await Promise.resolve();await pendingOwner.clear();
    if ((await document.modelContext.getTools()).some(tool => tool.name === 'hf_test_pending_probe')) throw new Error('Revoked pending registration survived');
    window.fixtureOtherOwner = createWebMCPOwner();
    await window.fixtureOtherOwner.sync({ scope: 'synthetic-independent', tools: [{ name: 'hf_test_relation_probe', description: 'Synthetic owner probe', inputSchema: { type: 'object', properties: {} }, execute: async () => 'probe' }] });
  });
  await navigate('/app?tab=profile');await waitTools(31);
  const logout = hold();heldLogout = logout;
  await page.locator('[data-action="logout"]').last().click();await logout.started.promise;await waitTools(0);
  assert.deepEqual(await toolNames(), ['hf_test_relation_probe']);
  logout.released.resolve();await page.waitForURL(origin + '/');await waitTools(0);
  await page.evaluate(() => window.fixtureOtherOwner.clear());assert.deepEqual(await toolNames(), []);
  assert.deepEqual(unexpected, []);assert.deepEqual(errors, []);assert.deepEqual(warnings, []);
  process.stdout.write(`Passed HF-QA-002 with native WebMCP in Chrome ${await browser.version()}; isolated synthetic data only.\n`);
} catch (error) {
  process.stderr.write(JSON.stringify({ stage, errors, warnings, unexpected, tools: await toolNames().catch(() => []) }) + '\n');throw error;
} finally {
  heldEligibility?.released.resolve();heldLogout?.released.resolve();
  await browser?.close();await new Promise(done => { server.closeAllConnections();server.close(done); });
}

// Real-browser UI acceptance with explicitly synthetic data and an isolated local HTTP server.
// This checks browser behavior/layout; server accounting and authorization have separate tests.
//   HFY_PLAYWRIGHT_MODULE=playwright node tests/admin-browser.mjs
// Optional: HFY_BROWSER_EXECUTABLE, HFY_BROWSER_ARGS_MODULE (Chromium class with .args),
// HFY_BROWSER_ARGS (JSON array of process flags),
// HFY_BROWSER_SCREENSHOTS (directory for clearly named synthetic screenshots).
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdir, readFile } from 'node:fs/promises';
import { dirname, extname, isAbsolute, join, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const staticRoot = join(root, 'static');
const DAY = 86400000;
const today = new Date(new Date().toISOString().slice(0, 10) + 'T00:00:00Z').getTime();
const day = offset => new Date(today + offset * DAY).toISOString().slice(0, 10);
const stamp = offset => Math.floor((today + offset * DAY + 12 * 3600000) / 1000);
const token = 's'.repeat(43);
const requests = [], unknown = [], errors = [], external = [];
const screenshotDir = process.env.HFY_BROWSER_SCREENSHOTS;
let origin, browser, page, stage = 'setup';

const config = {
  name: 'HireFerry synthetic browser fixture', default_language: 'zh', development: true,
  humangrant_enabled: false, google_enabled: false, email_enabled: true,
  pricing: { currency: 'CNY', currency_decimals: 2, allowed_currencies: ['CNY', 'USD'], currency_options: [{ currency: 'CNY', currency_decimals: 2 }, { currency: 'USD', currency_decimals: 2 }], onboarding_cents: 10000, publish_mode: 'salary_percent', publish_bps: 1000, edit_mode: 'salary_percent', edit_bps: 100, max_edits: 3 },
  limits: { list_page_size: 50, workbench_page_size: 25 },
  providers: ['stripe'],
};
const people = [
  { id: 'person-a', name: '演示候选人 A', role: 'seeker', email: 'candidate-a@example.test', created_at: stamp(-25), banned: false },
  { id: 'person-b', name: '演示候选人 B', role: 'seeker', email: 'candidate-b@example.test', created_at: stamp(-21), banned: false },
  { id: 'person-c', name: '演示候选人 C', role: 'seeker', email: 'candidate-c@example.test', created_at: stamp(-18), banned: false },
  { id: 'employer-a', name: '演示招聘方', role: 'employer', email: 'employer@example.test', created_at: stamp(-30), banned: false },
];
const jobs = [
  { id: 'job-rust', title: 'Rust 后端工程师 · 示例', owner_id: 'employer-a', salary_cents: 2800000, currency: 'CNY', currency_decimals: 2, headcount: 2, status: 'published', created_at: stamp(-10) },
  { id: 'job-design', title: '产品设计师 · 示例', owner_id: 'employer-a', salary_cents: 2200000, currency: 'CNY', currency_decimals: 2, headcount: 1, status: 'published', created_at: stamp(-8) },
  { id: 'job-qa', title: '质量工程师 · 示例', owner_id: 'employer-a', salary_cents: 2000000, currency: 'CNY', currency_decimals: 2, headcount: 1, status: 'published', created_at: stamp(-6) },
];
const matches = [
  { id: 'match-a', seeker_id: 'person-a', job_id: 'job-rust', status: 'introduced' },
  { id: 'match-b', seeker_id: 'person-b', job_id: 'job-design', status: 'interview' },
  { id: 'match-c', seeker_id: 'person-c', job_id: 'job-rust', status: 'declined' },
].map(match => ({ ...match, seeker_name: people.find(person => person.id === match.seeker_id).name, job_title: jobs.find(job => job.id === match.job_id).title, employer_id: 'employer-a', employer_name: '演示招聘方', created_at: stamp(-4), updated_at: stamp(-1) }));
const team = { smtp_enabled: true, invitation_ttl_hours: 72, ai_token_ttl_days: 30, max_members: 100,
  members: [{ id: 'staff-a', kind: 'human', name: '演示员工', email: 'staff@example.test', active: true, created_at: stamp(-20), last_used_at: stamp(-1) }, { id: 'ai-a', kind: 'ai', name: '演示 AI 助手', email: null, active: true, created_at: stamp(-12), last_used_at: stamp(-1), token_expires_at: stamp(20) }],
  invitations: [{ id: 'staff-invite-a', name: '待加入员工 · 示例', email: 'invited-staff@example.test', status: 'pending', delivery_status: 'sent', expires_at: stamp(2), created_at: stamp(-1) }],
};
const managers = { members: [{ user_id: 'manager-a', name: '演示招聘经理', email: 'manager@example.test', active: true, joined_at: stamp(-15) }], invitations: [{ id: 'manager-invite-a', email: 'invited-manager@example.test', expires_at: stamp(5), sent_at: stamp(-1), accepted_at: null, revoked_at: null }] };
const orders = [
  { id: 'synthetic-live-cny-a', kind: 'job_publish', amount_cents: 60000, currency: 'CNY', status: 'paid', live_mode: true, created_at: stamp(-6), paid_at: stamp(-6) },
  { id: 'synthetic-live-cny-b', kind: 'employer_onboarding', amount_cents: 10000, currency: 'CNY', status: 'paid', live_mode: true, created_at: stamp(-4), paid_at: stamp(-4) },
  { id: 'synthetic-live-cny-c', kind: 'job_publish', amount_cents: 90000, currency: 'CNY', status: 'paid', live_mode: true, created_at: stamp(-2), paid_at: stamp(-2), refunded_cents: 30000, refunded_at: stamp(0) },
  { id: 'synthetic-live-usd-a', kind: 'job_publish', amount_cents: 25000, currency: 'USD', status: 'paid', live_mode: true, created_at: stamp(-3), paid_at: stamp(-3) },
  { id: 'synthetic-test-usd-a', kind: 'job_publish', amount_cents: 75000, currency: 'USD', status: 'paid', live_mode: false, created_at: stamp(-3), paid_at: stamp(-3) },
  { id: 'synthetic-test-cny-a', kind: 'job_publish', amount_cents: 5000000, currency: 'CNY', status: 'paid', live_mode: false, created_at: stamp(-1), paid_at: stamp(-1) },
  { id: 'synthetic-pending-a', kind: 'employer_onboarding', amount_cents: 20000, currency: 'CNY', status: 'pending', live_mode: true, created_at: stamp(0) },
  { id: 'synthetic-expired-a', kind: 'job_publish', amount_cents: 10000, currency: 'CNY', status: 'expired', live_mode: true, created_at: stamp(-5) },
].map(order => ({ currency_decimals: 2, refunded_cents: 0, provider: 'fixture', ...order }));

const initialTeam = structuredClone(team), initialManagers = structuredClone(managers);
const effectiveStatus = order => order.refunded_cents ? 'partially_refunded' : order.status;
const inRange = (date, query) => date && (!query.get('from') || date >= Date.parse(query.get('from') + 'T00:00:00Z') / 1000) && (!query.get('to') || date < Date.parse(query.get('to') + 'T00:00:00Z') / 1000 + 86400);
function filteredOrders(query, withDate = true, withCurrency = true) {
  return orders.filter(order => order.live_mode === (query.get('mode') !== 'test') && (!withCurrency || order.currency === (query.get('currency') || 'CNY')) && (!query.get('status') || effectiveStatus(order) === query.get('status')) && (!withDate || inRange(order.created_at, query)));
}
function report(query) {
  const currency = query.get('currency') || 'CNY';
  const selected = filteredOrders(query, false, false), events = [];
  for (const order of selected) {
    if (inRange(order.paid_at, query)) events.push({ date: new Date(order.paid_at * 1000).toISOString().slice(0, 10), currency: order.currency, gross_cents: order.amount_cents, refund_cents: 0, orders: 1 });
    if (inRange(order.refunded_at, query)) events.push({ date: new Date(order.refunded_at * 1000).toISOString().slice(0, 10), currency: order.currency, gross_cents: 0, refund_cents: order.refunded_cents, orders: 0 });
  }
  const sum = rows => rows.reduce((total, row) => ({ gross_cents: total.gross_cents + row.gross_cents, refund_cents: total.refund_cents + row.refund_cents, revenue_cents: total.revenue_cents + row.gross_cents - row.refund_cents, paid_orders: total.paid_orders + row.orders }), { gross_cents: 0, refund_cents: 0, revenue_cents: 0, paid_orders: 0 });
  const scoped = events.filter(event => event.currency === currency);
  const dates = [...new Set(scoped.map(event => event.date))].sort();
  return { currency, currency_decimals: 2, payment_mode: query.get('mode') || 'live', bucket: 'day', stats: { users: people.length, seekers: 3, employers: 1, active_jobs: 3, pending_orders: 1, review_orders: 0, ...sum(scoped) },
    series: dates.map(date => { const totals = sum(scoped.filter(event => event.date === date)); return { date, ...totals, orders: totals.paid_orders }; }),
    currency_totals: ['CNY', 'USD'].map(code => ({ currency: code, currency_decimals: 2, ...sum(events.filter(event => event.currency === code)) })),
    recent_orders: filteredOrders(query).slice(0, 5), revenue_by_kind: [],
  };
}
const list = items => ({ items, total: items.length, page: 1, page_size: 50, limit: 50, next_cursor: null });
async function fixture(url, request) {
  const query = url.searchParams, path = url.pathname, method = request.method;
  const body = method === 'GET' ? {} : JSON.parse(await Array.fromAsync(request).then(parts => Buffer.concat(parts).toString()) || '{}');
  requests.push({ path, query: new URLSearchParams(query), method, body });
  if (path === '/api/config') return config;
  if (path === '/api/session') return { user: null, admin: true, super_admin: true, bootstrap: false, csrf: 'synthetic-csrf' };
  if (path === '/api/admin/team' && method === 'GET') return team;
  if (path === '/api/admin/staff' && method === 'GET') return managers;
  if (path === '/api/admin/team/ai' && method === 'POST') {
    const member = { id: 'synthetic-created-ai', name: body.name, kind: 'ai', active: true, created_at: stamp(0) }; team.members.push(member);
    return { member, token: 'synthetic-test-credential-not-valid', expires_at: stamp(30) };
  }
  if (path === '/api/admin/team/invitations' && method === 'POST') {
    const invitation = { id: 'synthetic-created-staff', name: body.name, email: body.email, status: 'pending', delivery_status: body.send_email ? 'sent' : 'not_sent', expires_at: stamp(3) }; team.invitations.push(invitation);
    return { invitation, invitation_url: origin + '/team/invite#' + token, delivery_status: invitation.delivery_status };
  }
  if (path === '/api/admin/staff/invitations' && method === 'POST') {
    const invitation = { id: 'synthetic-created-manager', email: body.email, expires_at: stamp(7), sent_at: body.send_email ? stamp(0) : null }; managers.invitations.push(invitation);
    return { ...invitation, invite_url: origin + '/staff/invite/' + token, sent: body.send_email };
  }
  if (path === '/api/admin/overview') return report(query);
  if (path === '/api/admin/orders') return list(filteredOrders(query));
  if (path === '/api/admin/users') return list(people.filter(person => (!query.get('q') || `${person.name} ${person.email}`.toLowerCase().includes(query.get('q').toLowerCase())) && (!query.get('role') || person.role === query.get('role')) && (!query.get('status') || (query.get('status') === 'suspended') === person.banned)));
  if (path === '/api/admin/jobs') return list(jobs.filter(job => (!query.get('q') || job.title.toLowerCase().includes(query.get('q').toLowerCase())) && (!query.get('status') || job.status === query.get('status'))));
  if (path === '/api/admin/workbench/matches') {
    const seekers = (query.get('seeker_ids') || '').split(','), jobIds = (query.get('job_ids') || '').split(',');
    return list(matches.filter(match => seekers.includes(match.seeker_id) && jobIds.includes(match.job_id) && (!query.get('status') || match.status === query.get('status'))));
  }
  unknown.push(`${method} ${path}`);
  return { error: 'Unknown synthetic fixture endpoint' };
}
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2' };
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://127.0.0.1');
    response.setHeader('cache-control', 'no-store');
    if (url.pathname.startsWith('/api/')) {
      response.setHeader('content-type', 'application/json');
      const value = await fixture(url, request);
      response.statusCode = value.error ? 404 : 200;
      response.end(JSON.stringify(value)); return;
    }
    if (url.pathname === '/favicon.ico') { response.statusCode = 204; response.end(); return; }
    const file = url.pathname.startsWith('/static/') ? resolve(staticRoot, decodeURIComponent(url.pathname.slice(8))) : join(staticRoot, 'index.html');
    if (!file.startsWith(staticRoot + sep)) throw new Error('Invalid static path');
    response.setHeader('content-type', types[extname(file)] || 'application/octet-stream');
    response.end(await readFile(file));
  } catch (error) { response.statusCode = 500; response.end('Synthetic fixture server error'); errors.push(error.message); }
});

async function moduleAt(specifier) { return import(isAbsolute(specifier) ? pathToFileURL(specifier).href : specifier); }
function safeBrowserArguments(args) {
  return args.filter(argument => !['--disable-web-security', '--disable-site-isolation-trials', '--allow-running-insecure-content'].includes(argument.split('=')[0])).map(argument => argument.startsWith('--disable-features=') ? '--disable-features=' + argument.slice(19).split(',').filter(feature => !['IsolateOrigins', 'site-per-process', 'SitePerProcess'].includes(feature)).join(',') : argument).filter(argument => argument !== '--disable-features=');
}
async function step(name, action) { stage = name; await action(); process.stdout.write(`ok - ${name}\n`); }
async function navigate(tab, extra = '') {
  await page.goto(`${origin}/admin?tab=${tab}${extra}`, { waitUntil: 'domcontentloaded' });
  await page.locator(tab === 'team' || tab === 'staff' ? '.team-workspace' : tab === 'matches' || tab === 'users' || tab === 'jobs' ? '.people-jobs-workspace' : '.finance-page').waitFor();
  await page.locator('#root.page-pending').waitFor({ state: 'hidden' });
  await page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
}
async function screenshot(name, clip = null) {
  if (!screenshotDir) return;
  await mkdir(screenshotDir, { recursive: true });
  await page.screenshot({ path: join(screenshotDir, `synthetic-admin-${name}.png`), fullPage: !clip, ...(clip ? { clip } : {}), animations: 'disabled' });
}
async function controlScreenshot(name, popup) {
  if (!screenshotDir) return;
  const toolbar = await page.locator('.finance-toolbar').boundingBox();
  const menu = await page.locator(popup).boundingBox(), viewport = page.viewportSize();
  const x = Math.max(0, Math.floor(Math.min(toolbar.x, menu.x) - 18));
  const y = Math.max(0, Math.floor(Math.min(toolbar.y, menu.y) - 18));
  const right = Math.min(viewport.width, Math.ceil(Math.max(toolbar.x + toolbar.width, menu.x + menu.width) + 18));
  const bottom = Math.min(viewport.height, Math.ceil(Math.max(toolbar.y + toolbar.height, menu.y + menu.height) + 18));
  await screenshot(name, { x, y, width: right - x, height: bottom - y });
}
async function noHorizontalOverflow() {
  const layout = await page.evaluate(() => ({ viewport: innerWidth, document: document.documentElement.scrollWidth, body: document.body.scrollWidth, content: document.querySelector('.app-content')?.getBoundingClientRect().right }));
  assert.ok(layout.document <= layout.viewport + 1 && layout.body <= layout.viewport + 1 && layout.content <= layout.viewport + 1, `Unexpected page overflow: ${JSON.stringify(layout)}`);
}
const currentReport = () => page.locator('[data-finance-report]').getAttribute('data-finance-report');
async function waitReport(previous) { await page.waitForFunction(key => document.querySelector('[data-finance-report]')?.dataset.financeReport !== key && !document.querySelector('#root.page-pending'), previous); }
async function choose(selector, value) {
  const option = await page.locator(selector).evaluate((select, selected) => [...select.options].find(item => item.value === selected)?.label, value);
  assert.ok(option, `Missing native select option ${value}`);
  const trigger = page.locator(selector).locator('..').locator('.choice-trigger');
  await trigger.scrollIntoViewIfNeeded();
  await page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
  await trigger.click();
  await page.locator('.choice-menu').getByRole('option', { name: option, exact: true }).click();
}
async function financeChoice(selector, value) { const previous = await currentReport(); await choose(selector, value); await waitReport(previous); }
async function financeDate(selector, value) { const previous = await currentReport(); await page.locator(selector).fill(value); await page.locator(selector).blur().catch(() => {}); await waitReport(previous); }
async function total(amount, currency = 'CNY') {
  const expected = new Intl.NumberFormat('zh-CN', { style: 'currency', currency, currencyDisplay: 'code', minimumFractionDigits: 2 }).format(amount / 100).replace(/\s/g, '');
  const value = (await page.locator('[data-finance-main-total]').textContent()).replace(/\s/g, '');
  assert.equal(value, expected);
}

try {
  await new Promise((resolve_, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve_); });
  origin = `http://127.0.0.1:${server.address().port}`;
  const { chromium } = await moduleAt(process.env.HFY_PLAYWRIGHT_MODULE || 'playwright');
  const launch = { headless: true };
  if (process.env.HFY_BROWSER_EXECUTABLE) launch.executablePath = process.env.HFY_BROWSER_EXECUTABLE;
  if (process.env.HFY_BROWSER_ARGS) launch.args = safeBrowserArguments(JSON.parse(process.env.HFY_BROWSER_ARGS));
  if (process.env.HFY_BROWSER_ARGS_MODULE) { const loaded = await moduleAt(process.env.HFY_BROWSER_ARGS_MODULE); launch.args = safeBrowserArguments((loaded.default || loaded).args); }
  browser = await chromium.launch(launch);
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce', locale: 'zh-CN', timezoneId: 'UTC' });
  await context.addInitScript(() => { localStorage.setItem('hireferry-language', 'zh'); localStorage.setItem('hireferry-theme', 'dark'); localStorage.setItem('hireferry-palette', 'amber'); });
  await context.route('**/*', route => { if (route.request().url().startsWith(origin + '/')) return route.continue(); external.push(new URL(route.request().url()).origin); return route.abort(); });
  page = await context.newPage(); page.setDefaultTimeout(10000);
  page.on('pageerror', error => errors.push(error.message));

  await step('one team entry and all three creation flows', async () => {
    await navigate('team');
    assert.equal(await page.locator('.sidebar-nav a[href="/admin?tab=team"]:visible').count(), 1);
    assert.equal(await page.locator('.sidebar-nav a[href="/admin?tab=staff"]').count(), 0);
    for (const kind of ['human', 'manager', 'ai']) assert.ok(await page.locator(`[data-team-section="members"] [data-team-kind="${kind}"]`).count());
    const before = requests.length;
    await page.locator('[data-action="team-filter"][data-kind="manager"]').click();
    assert.equal(await page.locator('[data-team-section="members"] [data-team-kind]:visible').count(), 1);
    assert.equal(await page.locator('[data-team-section="invitations"] [data-team-kind]:visible').count(), 1);
    assert.equal(requests.length, before, 'Local role selection must not refetch the roster');
    await page.locator('[data-action="team-filter"][data-kind="all"]').click();
    await screenshot('team-dark-1440');
    for (const kind of ['human', 'manager', 'ai']) {
      await page.locator('[data-action="team-add-member"]').click();
      await page.locator(`[data-action="team-add-kind"][data-kind="${kind}"]`).click();
      assert.equal(await page.locator('[data-team-add-kind]:visible').count(), 1);
      const form = page.locator(`[data-team-add-kind="${kind}"] form`);
      if (kind === 'ai') await form.locator('[name="name"]').fill('浏览器测试 AI · 合成数据');
      else await form.locator('[name="email"]').fill(`synthetic-${kind}@example.test`);
      if (kind === 'human') { await form.locator('[name="name"]').fill('浏览器测试员工 · 合成数据'); await screenshot('team-add-member-dark-1440'); }
      await form.locator('button[type="submit"]').click();
      await page.locator(kind === 'ai' ? '#team-ai-secret' : '#team-invitation-secret').waitFor();
      await page.locator('[data-action="close-dialog"]').click();
    }
    for (const path of ['/api/admin/team/invitations', '/api/admin/staff/invitations', '/api/admin/team/ai']) assert.equal(requests.filter(request => request.path === path && request.method === 'POST').length, 1);
    assert.equal(requests.find(request => request.path === '/api/admin/staff/invitations' && request.method === 'POST').body.role, 'manager');
    await navigate('staff');
    assert.equal(await page.locator('.team-workspace').count(), 1);
  });

  await step('finance dates, status, currency and test mode update the entire report', async () => {
    await navigate('orders', '&mode=live'); await total(130000);
    assert.equal(await page.locator('.finance-chart-svg').count(), 1);
    await financeDate('#finance-from', day(-4)); await total(70000);
    await financeDate('#finance-to', day(-2)); await total(100000);
    for (const path of ['/api/admin/overview', '/api/admin/orders']) {
      const last = requests.filter(request => request.path === path).at(-1);
      assert.equal(last.query.get('from'), day(-4)); assert.equal(last.query.get('to'), day(-2));
    }
    const before = requests.length;
    await page.locator('[data-finance-metric="orders"]').click();
    assert.equal((await page.locator('[data-finance-main-total]').textContent()).trim(), '2');
    await page.locator('[data-finance-metric="revenue_cents"]').click();
    assert.equal(requests.length, before, 'Changing a chart metric must use the already loaded report');
    await financeChoice('#finance-status', 'paid'); await total(10000);
    assert.equal(await page.locator('.finance-orders tbody tr').count(), 1);
    await financeChoice('#finance-currency', 'USD'); await total(25000, 'USD');
    await financeChoice('#finance-mode', 'test'); await total(75000, 'USD');
    for (const path of ['/api/admin/overview', '/api/admin/orders']) {
      const last = requests.filter(request => request.path === path).at(-1);
      assert.equal(last.query.get('status'), 'paid'); assert.equal(last.query.get('currency'), 'USD'); assert.equal(last.query.get('mode'), 'test');
    }
  });

  await step('themed select supports keyboard, click, dismissal and date calendar', async () => {
    let trigger = page.locator('#finance-currency').locator('..').locator('.choice-trigger');
    await trigger.focus(); await page.keyboard.press('Enter'); await page.keyboard.press('Home'); await page.keyboard.press('Escape');
    assert.equal(await page.locator('.choice-menu').count(), 0);
    assert.equal(await page.locator('#finance-currency').inputValue(), 'USD');
    assert.equal(await trigger.evaluate(element => element === document.activeElement), true);
    const previous = await currentReport();
    await page.keyboard.press('Enter'); await page.keyboard.press('Home'); await page.keyboard.press('Enter'); await waitReport(previous);
    assert.equal(await page.locator('#finance-currency').inputValue(), 'CNY'); await total(0);
    trigger = page.locator('#finance-status').locator('..').locator('.choice-trigger');
    await trigger.click();
    await page.locator('.app-content').evaluate(element => element.scrollBy(0, 24));
    await page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
    assert.equal(await page.locator('.choice-menu').count(), 1, 'A visible dropdown should reposition when its workspace scrolls');
    await page.locator('.finance-page-heading h1').click();
    assert.equal(await page.locator('.choice-menu').count(), 0);
    assert.equal(await page.locator('#finance-status').inputValue(), 'paid');
    const dateTrigger = page.locator('#finance-from').locator('..').locator('.date-trigger');
    await dateTrigger.click();
    const activeDate = await page.locator('.calendar-day[tabindex="0"]').getAttribute('data-date');
    await page.keyboard.press('ArrowRight');
    assert.notEqual(await page.locator('.calendar-day[tabindex="0"]').getAttribute('data-date'), activeDate);
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('.calendar-menu').count(), 0);
    assert.equal(await page.locator('#finance-from').inputValue(), day(-4));
    await dateTrigger.click();
    const reportKey = await currentReport();
    await page.locator(`.calendar-day[data-date="${day(-3)}"]`).click();
    await waitReport(reportKey);
    assert.equal(await page.locator('#finance-from').inputValue(), day(-3));
    assert.equal(new URL(page.url()).searchParams.get('from'), day(-3));
  });

  await step('two-column matching only connects real records and highlights selected relationships', async () => {
    await navigate('matches');
    await page.waitForFunction(() => document.querySelectorAll('.pj-connection').length === 3);
    assert.equal(await page.locator('.sidebar-nav a[href="/admin?tab=users"],.sidebar-nav a[href="/admin?tab=jobs"]').count(), 0);
    const left = await page.locator('.pj-people-column').boundingBox(), right = await page.locator('.pj-jobs-column').boundingBox();
    assert.ok(left.x + left.width < right.x);
    assert.deepEqual(await page.locator('.pj-connection').evaluateAll(nodes => nodes.map(node => node.dataset.id).sort()), ['match-a', 'match-b', 'match-c']);
    await page.locator('[data-graph-select="person"][data-id="person-a"]').click();
    await page.waitForFunction(() => document.querySelectorAll('.pj-connection.is-highlighted').length === 1);
    assert.equal(await page.locator('.pj-connection.is-highlighted').getAttribute('data-id'), 'match-a');
    assert.equal(await page.locator('[data-graph-node="job:job-rust"].is-related').count(), 1);
    await page.locator('[data-graph-select="job"][data-id="job-rust"]').click();
    assert.equal(await page.locator('[data-graph-followup]').isVisible(), true);
    assert.equal(await page.locator('[data-graph-introduce]').isVisible(), false);
    await screenshot('matching-selected-dark-1440');
    const beforePreview = requests.length;
    await page.locator('[data-graph-select="job"][data-id="job-qa"]').click();
    assert.equal(await page.locator('[data-graph-introduce]').isVisible(), true);
    await page.locator('[data-graph-preview]').waitFor();
    assert.equal(await page.locator('.pj-connection[data-id]').count(), 3, 'A pending preview must not increase existing introductions');
    assert.equal(await page.locator('[data-graph-preview] .pj-connection[data-id]').count(), 0, 'Pending previews must not claim a saved match identifier');
    assert.equal(await page.locator('[data-graph-relation]').count(), 3);
    assert.equal(await page.locator('[data-graph-preview-legend]').isVisible(), true);
    await screenshot('matching-preview-dark-1440');
    await page.locator('[data-graph-select="job"][data-id="job-rust"]').click();
    await page.locator('[data-graph-preview]').waitFor({ state: 'detached' });
    assert.equal(await page.locator('[data-graph-followup]').isVisible(), true);
    await page.locator('[data-graph-select="person"][data-id="employer-a"]').click();
    assert.equal(await page.locator('[data-graph-preview]').count(), 0, 'Employers must not become seeker-side previews');
    assert.equal(await page.locator('[data-graph-introduce]').isVisible(), false);
    assert.equal(requests.length, beforePreview, 'Selection and previews must stay local until explicit confirmation');
    await page.locator('[data-graph-clear]').click();
    await choose('select[name="graph_status"]', 'interview');
    await page.waitForFunction(() => document.querySelectorAll('.pj-connection').length === 1 && document.querySelector('.pj-connection')?.dataset.id === 'match-b');
    await page.locator('input[name="people_q"]').fill('候选人 A');
    await page.locator('[data-list="people"] button[type="submit"]').click();
    await page.waitForFunction(() => document.querySelectorAll('.pj-person').length === 1 && document.querySelectorAll('.pj-connection').length === 0);
    assert.equal(await page.locator('[data-graph-node="person:person-a"]').count(), 1);
  });

  await step('1440 light/dark and 390/320 mobile layouts stay inside the viewport', async () => {
    Object.assign(team, structuredClone(initialTeam)); Object.assign(managers, structuredClone(initialManagers));
    for (const [width, theme] of [[1440, 'light'], [1440, 'dark'], [390, 'dark'], [320, 'light']]) {
      await page.setViewportSize({ width, height: width < 600 ? 844 : 1000 });
      for (const tab of ['team', 'orders', 'matches']) {
        await navigate(tab, tab === 'orders' ? `&mode=live&from=${day(-6)}&to=${day(0)}` : '');
        await page.evaluate(value => { const root_ = document.documentElement; root_.dataset.theme = value; root_.dataset.palette = value === 'light' ? 'neutral' : 'amber'; root_.classList.toggle('dark', value === 'dark'); root_.classList.toggle('light', value === 'light'); }, theme);
        await noHorizontalOverflow();
        if (tab === 'orders') {
          const amountsFit = await page.locator('.finance-secondary-totals dd').evaluateAll(nodes => nodes.every(node => node.getBoundingClientRect().height <= parseFloat(getComputedStyle(node).lineHeight) + 2));
          assert.ok(amountsFit, 'Each summary amount must remain on one line, including cents');
        }
        if (tab === 'matches') {
          const columns = await page.locator('.pj-column').evaluateAll(nodes => nodes.map(node => node.getBoundingClientRect().x));
          assert.ok(columns[0] < columns[1], 'Mobile must preserve left/right columns inside its own scroll area');
          assert.equal(await page.locator('.pj-scroll').evaluate(element => ['auto', 'scroll'].includes(getComputedStyle(element).overflowX)), true);
          if (width < 600) {
            const right = await page.locator('.pj-jobs-column').boundingBox();
            assert.ok(right.x + right.width <= width + 1, `${width}px should show both columns together`);
            const person = await page.locator('.pj-person').first().boundingBox(), job = await page.locator('.pj-job').first().boundingBox();
            assert.ok(Math.abs(person.y - job.y) <= 1, `${width}px must align both columns after the enhanced filters`);
            const titlesReadable = await page.locator('.pj-node-select strong').evaluateAll(nodes => nodes.every(node => parseFloat(getComputedStyle(node).fontSize) >= 12));
            assert.ok(titlesReadable, `${width}px card titles must remain at least 12px`);
          }
        }
        await screenshot(`${tab}-${theme}-${width}`);
        if (tab === 'orders' && width === 1440 && theme === 'dark') {
          await page.locator('#finance-currency').locator('..').locator('.choice-trigger').click();
          await screenshot('orders-select-dark-1440'); await controlScreenshot('orders-select-detail-dark-1440', '.choice-menu'); await page.keyboard.press('Escape');
          await page.locator('#finance-from').locator('..').locator('.date-trigger').click();
          await screenshot('orders-calendar-dark-1440'); await controlScreenshot('orders-calendar-detail-dark-1440', '.calendar-menu'); await page.keyboard.press('Escape');
        }
        if (tab === 'orders' && width === 320) {
          await page.locator('#finance-currency').locator('..').locator('.choice-trigger').click();
          const bounds = await page.locator('.choice-menu').boundingBox();
          assert.ok(bounds.x >= 0 && bounds.x + bounds.width <= width + 1); await noHorizontalOverflow();
          await page.keyboard.press('Escape');
          await page.locator('#finance-from').locator('..').locator('.date-trigger').click();
          const calendarBox = await page.locator('.calendar-menu').boundingBox();
          assert.ok(calendarBox.x >= 0 && calendarBox.x + calendarBox.width <= width + 1); await noHorizontalOverflow();
          await page.keyboard.press('Escape');
        }
      }
    }
  });
  assert.deepEqual(unknown, [], 'Every API request must have an explicit fixture');
  assert.deepEqual(external, [], 'UI tests must never contact an external service');
  assert.deepEqual(errors, [], 'No browser or fixture server errors are allowed');
  process.stdout.write('Passed admin browser acceptance with synthetic fixtures only.\n');
} catch (error) {
  if (page && screenshotDir) await screenshot('failure-' + stage.replace(/[^a-z0-9]+/gi, '-').slice(0, 70)).catch(() => {});
  const visible = await page?.locator('#root').innerText().catch(() => '') || '';
  process.stderr.write(`Fixture diagnostics: ${JSON.stringify({ visible: visible.slice(0, 600), errors, unknown })}\n`);
  process.stderr.write(`FAILED (${stage}): ${error.stack || error.message}\n`);
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  await new Promise(resolve_ => { server.closeAllConnections(); server.close(resolve_); });
}

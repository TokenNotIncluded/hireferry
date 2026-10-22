// Browser regression for the real chat UI. All API responses are synthetic;
// request interception serves repository assets and rejects external requests.
// PLAYWRIGHT_MODULE, BROWSER_EXECUTABLE, FONTCONFIG_FILE, CHAT_QA_REPO and
// CHAT_QA_OUT_DIR allow existing local browser/runtime installations to be reused.
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { extname, resolve, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath, pathToFileURL } from 'node:url';

const runtime = process.env.PLAYWRIGHT_MODULE || (process.env.CODEX_PRIMARY_RUNTIME_NODE_MODULES
  ? pathToFileURL(resolve(process.env.CODEX_PRIMARY_RUNTIME_NODE_MODULES, 'playwright/index.mjs')).href : 'playwright');
const { chromium } = await import(runtime.startsWith('/') ? pathToFileURL(runtime).href : runtime);
const repo = resolve(process.env.CHAT_QA_REPO || fileURLToPath(new URL('..', import.meta.url)));
const assets = resolve(repo, 'static');
const output = resolve(process.env.CHAT_QA_OUT_DIR || resolve(tmpdir(), 'hireferry-steady-chat-qa'));
await mkdir(output, { recursive: true });
const results = { fixture: 'Real static frontend and synthetic APIs served by browser request interception; no listening socket, production backend, or external requests', checks: [], metrics: [], screenshots: [], failures: [], pageErrors: [], unexpectedRequests: [] };
const now = 1792630800;
const config = {
  name: 'HireFerry', default_language: 'zh', humangrant_enabled: false,
  google_enabled: false, email_enabled: true, development: false,
  support_email: 'support@example.test', providers: [],
  pricing: { currency: 'CNY', currency_decimals: 2, allowed_currencies: ['CNY'], onboarding_cents: 10000, publish_bps: 1000, edit_bps: 100, max_edits: 3 },
  limits: { list_page_size: 50, workbench_page_size: 25, recent_list_limit: 200 },
};
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const origin = 'http://hireferry.test';
let browser;

function message(id, body, own = false, admin = false) {
  const senderKind = admin ? (own ? 'platform' : 'user') : (own ? 'user' : 'platform');
  return {
    id, body, created_at: now + id * 30, client_id: `fixture-message-${id}`,
    sender_kind: senderKind,
    sender_id: senderKind === 'platform' ? null : 'qa-member',
    sender_name: own ? '我' : '沟通对象', is_own: own, staff_name: own && admin ? '招聘顾问' : undefined,
  };
}

async function fixture(name, { width = 390, height = 844, lang = 'zh', dark = false, role = 'seeker', admin = false, matched = false, count = 0, title, archived = false, mockVisualViewport = false } = {}) {
  const context = await browser.newContext({ viewport: { width, height }, colorScheme: dark ? 'dark' : 'light', reducedMotion: 'reduce', locale: lang === 'en' ? 'en-US' : 'zh-CN' });
  await context.addInitScript(({ lang, dark, mockVisualViewport }) => {
    localStorage.setItem('hireferry-language', lang);
    localStorage.setItem('hireferry-theme', dark ? 'dark' : 'light');
    if (mockVisualViewport) {
      // Simulate the visual viewport independently of the layout viewport.
      // This covers the keyboard fallback branch, not a real mobile keyboard.
      const viewport = new EventTarget();
      Object.assign(viewport, { height: innerHeight, width: innerWidth, scale: 1 });
      Object.defineProperty(window, 'visualViewport', { configurable: true, value: viewport });
      window.__chatQaVisualViewport = (height, scale = 1) => {
        Object.assign(viewport, { height, scale });
        viewport.dispatchEvent(new Event('resize'));
      };
    }
  }, { lang, dark, mockVisualViewport });
  const session = { user: admin ? null : { id: 'qa-member', name: '测试用户', email: 'member@example.test', role, employer_paid: false }, admin, super_admin: admin, staff: null, bootstrap: false, csrf: 'synthetic-csrf' };
  const room = { id: matched ? 'match_fixture' : 'platform_qa-member', kind: matched ? 'matched' : 'platform', title: title || '产品设计师 · 远程协作', user_id: 'qa-member', job_id: matched ? 'fixture-role' : null, match_id: matched ? 'fixture-match' : null, writable: !archived, unread_count: 0, participants: [{ id: 'qa-member', name: '求职者', role: 'seeker' }, { id: 'qa-counterparty', name: '招聘团队', role: 'employer' }] };
  const history = Array.from({ length: count }, (_, i) => message(i + 1,
    lang === 'en' ? (i % 2 ? 'Thank you. I can share my experience and confirm the next step here.' : 'Let’s check the role, working arrangements, and the questions that matter to you.')
      : (i % 2 ? '好的，我会补充相关经历，也想了解一下岗位的工作安排。' : '我们可以先确认工作内容、地点和你最关心的条件，再讨论下一步。'), i % 2 === 1, admin));
  const calls = [];
  let failNextSend = false;
  await context.route('**/*', async route => {
    const request = route.request(), url = new URL(request.url());
    if (url.origin !== origin) { results.unexpectedRequests.push({ name, url: url.href }); await route.abort(); return; }
    if (!url.pathname.startsWith('/api/')) {
      const file = url.pathname.startsWith('/static/') ? url.pathname.slice(8) : 'index.html';
      const path = resolve(assets, file);
      if (!path.startsWith(assets + sep)) { await route.fulfill({ status: 403, body: '' }); return; }
      try { await route.fulfill({ status: 200, contentType: mime[extname(path)] || 'application/octet-stream', body: await readFile(path) }); }
      catch { await route.fulfill({ status: 404, body: '' }); }
      return;
    }
    calls.push({ path: url.pathname, search: url.search, method: request.method() });
    let data, status = 200;
    if (url.pathname === '/api/config') data = config;
    else if (url.pathname === '/api/session') data = session;
    else if (url.pathname === '/api/ai/eligibility') data = { eligible: true };
    else if (url.pathname === '/api/admin/auth/status') data = { initialized: true };
    else if (url.pathname === '/api/chat/conversations') data = { items: [{ ...room, last_message_preview: history.at(-1)?.body || '', last_message_at: history.at(-1)?.created_at || 0 }] };
    else if (url.pathname === `/api/chat/conversations/${room.id}`) data = room;
    else if (url.pathname === `/api/chat/conversations/${room.id}/read`) data = { ok: true };
    else if (url.pathname === `/api/chat/conversations/${room.id}/messages`) {
      if (request.method() === 'POST') {
        if (failNextSend) { failNextSend = false; status = 503; data = { error: 'Synthetic send unavailable' }; }
        else {
          const payload = request.postDataJSON();
          data = history.find(item => item.client_id === payload.client_id) || { ...message((history.at(-1)?.id || 0) + 1, payload.body, true, admin), client_id: payload.client_id };
          if (!history.includes(data)) history.push(data);
        }
      } else {
        const after = Number(url.searchParams.get('after') || 0), before = Number(url.searchParams.get('before') || Infinity);
        const items = history.filter(item => item.id > after && item.id < before).slice(-100);
        data = { items, next_cursor: items.at(-1)?.id || after, first_cursor: items[0]?.id || 0, has_more: false, has_older: false };
      }
    } else { status = 404; data = { error: 'Unhandled synthetic endpoint' }; results.unexpectedRequests.push({ name, path: url.pathname }); }
    await route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(data) });
  });
  const page = await context.newPage();
  page.setDefaultTimeout(8000);
  page.on('pageerror', error => results.pageErrors.push({ name, error: error.message }));
  await page.goto(`${origin}/${admin ? 'admin' : 'app'}?tab=messages&conversation=${room.id}`);
  await page.waitForFunction(() => document.querySelector('#root')?.dataset.navigationState === 'ready');
  await page.locator('#chat-scroll[aria-busy="false"]').waitFor();
  await settle(page);
  return {
    context, page, room, history, calls,
    addReply(body) { const item = message((history.at(-1)?.id || 0) + 1, body, false, admin); history.push(item); return item; },
    failSend() { failNextSend = true; },
  };
}

async function settle(page) { await page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done)))); }
async function geometry(page, name) {
  const metrics = await page.evaluate(() => {
    const size = selector => { const element = document.querySelector(selector); return element ? Math.round(element.getBoundingClientRect().height * 100) / 100 : 0; };
    const scroll = document.querySelector('#chat-scroll');
    return {
      viewportWidth: innerWidth, viewportHeight: innerHeight,
      documentWidth: document.documentElement.scrollWidth, bodyWidth: document.body.scrollWidth,
      siteHeader: size('.workspace-header'), chatHeader: size('.chat-thread-header'), composer: size('.chat-composer'),
      messageViewport: size('#chat-scroll'), navigation: size('.workspace-sidebar'),
      messageWidth: scroll.clientWidth, messageScrollWidth: scroll.scrollWidth,
      atBottomDistance: Math.round((scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight) * 100) / 100,
    };
  });
  results.metrics.push({ name, ...metrics });
  assert.ok(metrics.documentWidth <= metrics.viewportWidth + 1, `${name}: document overflow ${JSON.stringify(metrics)}`);
  assert.ok(metrics.bodyWidth <= metrics.viewportWidth + 1, `${name}: body overflow`);
  assert.ok(metrics.messageScrollWidth <= metrics.messageWidth + 1, `${name}: chat overflow`);
  return metrics;
}
async function screenshot(page, name) { await page.screenshot({ path: resolve(output, name), fullPage: false }); results.screenshots.push(name); }
async function run(name, options, check) {
  if (process.env.CHAT_QA_CASES && !process.env.CHAT_QA_CASES.split(',').includes(name)) return;
  let scene;
  try {
    scene = await fixture(name, options);
    await check(scene);
    results.checks.push(name);
    console.log(`PASS ${name}`);
  } catch (error) {
    results.failures.push({ name, error: error.message });
    console.error(`FAIL ${name}: ${error.message}`);
    if (scene) await screenshot(scene.page, `failure-${name.replace(/[^a-z0-9]+/gi, '-').toLowerCase()}.png`).catch(() => {});
  } finally { await scene?.context.close(); }
}
const bottomDistance = page => page.locator('#chat-scroll').evaluate(element => element.scrollHeight - element.scrollTop - element.clientHeight);
async function readingAnchor(page) {
  return page.locator('#chat-scroll').evaluate(scroll => {
    const top = scroll.getBoundingClientRect().top;
    const item = [...scroll.querySelectorAll('[data-message-id]')].find(node => node.getBoundingClientRect().bottom > top);
    return { id: item?.dataset.messageId, offset: item ? item.getBoundingClientRect().top - top : 0 };
  });
}

try {
  browser = await chromium.launch({ executablePath: process.env.BROWSER_EXECUTABLE || undefined, headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu'] });
  await run('mobile-empty-light', {}, async ({ page }) => {
    assert.match(await page.locator('#chat-empty').innerText(), /先说说你的情况/);
    assert.equal(await page.locator('#feedback-launch').isVisible(), false);
    const metrics = await geometry(page, 'mobile-empty-light');
    assert.equal(metrics.siteHeader, 0); assert.equal(metrics.navigation, 0);
    assert.ok(metrics.chatHeader <= 60); assert.ok(metrics.composer <= 70);
    assert.ok(metrics.messageViewport / metrics.viewportHeight >= 0.8);
    await screenshot(page, 'mobile-empty-light.png');
    await page.locator('.chat-tools > summary').click();
    await page.locator('.chat-tools [data-chat-feedback]').click();
    await page.locator('#feedback-panel').waitFor({ state: 'visible' });
    assert.equal(await page.locator('#feedback-message').evaluate(node => node === document.activeElement), true);
    await page.locator('[data-feedback-close]').click();
    assert.equal(await page.locator('.chat-tools > summary').evaluate(node => node === document.activeElement), true);
    await page.locator('[data-chat-back]').click();
    await page.locator('.conversation-sidebar').waitFor({ state: 'visible' });
    assert.equal(await page.locator('.workspace-header').isVisible(), true);
    assert.equal(await page.locator('.workspace-sidebar').isVisible(), true);
    await page.locator('.conversation-item').click();
    await page.locator('#chat-body').waitFor({ state: 'visible' });
    assert.equal(await page.locator('.workspace-header').isVisible(), false);
    assert.equal(await page.locator('.workspace-sidebar').isVisible(), false);
  });

  await run('mobile-thread-dark', { count: 24, dark: true }, async ({ page, addReply }) => {
    assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark');
    assert.ok(await bottomDistance(page) <= 8);
    await geometry(page, 'mobile-thread-dark');
    await screenshot(page, 'mobile-thread-dark.png');
    await page.locator('#chat-body').fill('第一行\n第二行\n第三行\n第四行');
    await settle(page);
    assert.ok(await bottomDistance(page) <= 8, 'multiline input keeps the latest message visible');
    await page.locator('#chat-scroll').evaluate(scroll => { scroll.scrollTop -= 240; });
    await settle(page);
    const before = await readingAnchor(page);
    await page.locator('#chat-body').fill('第一行\n第二行\n第三行\n第四行\n第五行\n第六行');
    await settle(page);
    const after = await readingAnchor(page);
    assert.equal(after.id, before.id); assert.ok(Math.abs(after.offset - before.offset) <= 2, 'growing input retains history anchor');
    await page.locator('[data-chat-back]').click();
    await page.locator('.conversation-sidebar').waitFor({ state: 'visible' });
    await settle(page);
    const hiddenReply = addReply('你查看会话列表时收到的回复，也会保留原来的阅读位置。');
    await page.locator(`[data-message-id="${hiddenReply.id}"]`).waitFor({ state: 'attached', timeout: 7000 });
    await page.locator('.conversation-item').click();
    await page.locator('#chat-scroll').waitFor({ state: 'visible' });
    await settle(page);
    const returned = await readingAnchor(page);
    assert.equal(returned.id, before.id, 'Back and reopening the same room retains the historical message');
    assert.ok(Math.abs(returned.offset - before.offset) <= 2, 'hidden list mode cannot replace the reading anchor with zero geometry');
    const reply = addReply('这是一条新的回复，阅读历史时只提示，不移动阅读位置。');
    await page.locator(`[data-message-id="${reply.id}"]`).waitFor({ timeout: 7000 });
    await settle(page);
    const arrived = await readingAnchor(page);
    assert.equal(arrived.id, before.id); assert.ok(Math.abs(arrived.offset - before.offset) <= 2, 'new reply retains history anchor');
    assert.equal(await page.locator('[data-action="chat-latest"]').isVisible(), true);
    await page.locator('#chat-scroll').evaluate(scroll => { scroll.scrollTop = scroll.scrollHeight; });
    await page.locator('[data-action="chat-latest"]').waitFor({ state: 'hidden' });
    await page.locator('#chat-body').fill(''); await settle(page);
    await page.setViewportSize({ width: 390, height: 500 }); await settle(page);
    assert.ok(await bottomDistance(page) <= 8, 'shorter viewport retains the latest message');
    const small = await geometry(page, 'mobile-short-viewport');
    assert.ok(small.messageViewport > 300);
    await page.setViewportSize({ width: 390, height: 844 }); await settle(page);
  });

  await run('mobile-thread-light', { count: 12 }, async ({ page }) => {
    await geometry(page, 'mobile-thread-light');
    await screenshot(page, 'mobile-thread-light.png');
  });

  await run('mobile-visual-viewport-only', { count: 24, mockVisualViewport: true }, async ({ page }) => {
    const viewportState = () => page.evaluate(() => ({
      layoutHeight: innerHeight,
      visualHeight: visualViewport.height,
      visualScale: visualViewport.scale,
      heightVariable: document.querySelector('#root').style.getPropertyValue('--chat-viewport-height'),
      rootHeight: document.querySelector('#root').getBoundingClientRect().height,
      composerBottom: document.querySelector('.chat-composer').getBoundingClientRect().bottom,
    }));
    const resizeVisual = async (height, scale = 1) => {
      await page.evaluate(({ height, scale }) => window.__chatQaVisualViewport(height, scale), { height, scale });
      await settle(page);
    };
    await resizeVisual(500);
    const shortened = await viewportState();
    results.metrics.push({ name: 'mobile-visual-viewport-only', ...shortened });
    assert.equal(shortened.layoutHeight, 844, 'layout viewport remains unchanged');
    assert.equal(shortened.heightVariable, '500px');
    assert.ok(Math.abs(shortened.rootHeight - 500) <= 1, 'root follows the available visual height');
    assert.ok(Math.abs(shortened.composerBottom - 500) <= 1, 'composer remains above the synthetic keyboard');
    assert.ok(await bottomDistance(page) <= 8, 'visual viewport shrink keeps the latest message visible');
    await geometry(page, 'mobile-visual-viewport-short');
    await resizeVisual(844);
    await page.locator('#chat-scroll').evaluate(scroll => { scroll.scrollTop -= 240; });
    await settle(page);
    const before = await readingAnchor(page);
    await resizeVisual(500);
    const after = await readingAnchor(page);
    assert.equal(after.id, before.id);
    assert.ok(Math.abs(after.offset - before.offset) <= 2, 'visual viewport shrink retains the history anchor');
    await resizeVisual(420, 2);
    const zoomed = await viewportState();
    results.metrics.push({ name: 'mobile-visual-viewport-zoom', ...zoomed });
    assert.equal(zoomed.heightVariable, '', 'pinch zoom removes the keyboard height override');
    assert.equal(zoomed.layoutHeight, 844);
    assert.ok(Math.abs(zoomed.rootHeight - 844) <= 1, 'zoom branch uses the unmodified layout viewport');
  });

  for (const width of [360, 320]) {
    await run(`narrow-english-${width}`, { width, height: 740, lang: 'en', matched: true, title: 'Senior Product Designer, Customer Experience and Distributed Collaboration Platform', count: 10 }, async ({ page }) => {
      await geometry(page, `narrow-english-${width}`);
      assert.equal(await page.locator('[data-chat-back]').isVisible(), true);
      assert.equal(await page.locator('.chat-tools > summary').isVisible(), true);
      await page.locator('.chat-tools > summary').click();
      const menu = await page.locator('.chat-tools-menu').boundingBox();
      assert.ok(menu.x >= 0 && menu.x + menu.width <= width + 1);
      await page.keyboard.press('Escape');
      if (width === 320) await screenshot(page, 'mobile-english-320.png');
    });
  }

  await run('employer-activation', { role: 'employer' }, async ({ page, calls }) => {
    assert.equal(await page.locator('.chat-account-note').count(), 1);
    assert.equal(await page.locator('.chat-account-note').getAttribute('open'), null);
    assert.equal(await page.locator('.workspace-activation,.promo-activation-banner').count(), 0);
    assert.equal(await page.locator('#chat-scroll .chat-account-note').count(), 1);
    await page.locator('.chat-account-note summary').click();
    assert.equal(await page.locator('.chat-account-note a[href="/app?tab=orders"]').isVisible(), true);
    assert.equal(await page.locator('.chat-account-note a[href="/app?tab=promos"]').isVisible(), true);
    await page.locator('#chat-body').fill('我们正在招聘产品设计师，先确认一下招聘需求。');
    await page.locator('.chat-composer [type="submit"]').click();
    await page.locator('.message-delivery').waitFor();
    assert.equal(await page.locator('#chat-body').inputValue(), '');
    assert.equal(calls.filter(call => call.method === 'POST' && call.path.endsWith('/messages')).length, 1);
    await geometry(page, 'employer-activation');
  });

  await run('desktop-admin', { width: 1440, height: 960, admin: true, matched: true, count: 8 }, async ({ page }) => {
    assert.equal(await page.locator('#chat-body').isDisabled(), false);
    assert.equal(await page.locator('.chat-context-link').isVisible(), true);
    await geometry(page, 'desktop-admin');
    await screenshot(page, 'desktop-admin.png');
  });
  await run('admin-archived', { width: 390, height: 844, admin: true, matched: true, archived: true, count: 5 }, async ({ page }) => {
    assert.match(await page.locator('[data-chat-subtitle]').innerText(), /只读/);
    assert.equal(await page.locator('#chat-body').isDisabled(), true);
    assert.equal(await page.locator('.chat-composer [type="submit"]').isDisabled(), true);
    await geometry(page, 'admin-archived');
  });

  await run('send-failure-keeps-draft', { count: 3 }, async ({ page, failSend }) => {
    failSend();
    await page.locator('#chat-body').fill('这条消息需要收到真实回执。');
    await page.locator('.chat-composer [type="submit"]').click();
    await page.locator('[data-chat-resend]').waitFor({ state: 'visible' });
    const failure = await page.locator('#chat-connection').innerText();
    await page.waitForResponse(response => new URL(response.url()).searchParams.has('after'), { timeout: 7000 });
    await settle(page);
    assert.equal(await page.locator('#chat-connection').innerText(), failure);
    assert.equal(await page.locator('#chat-body').inputValue(), '这条消息需要收到真实回执。');
    await page.locator('[data-chat-resend]').click();
    await page.locator('[data-chat-status]').waitFor({ state: 'hidden' });
    assert.equal(await page.locator('#chat-body').inputValue(), '');
  });
} finally {
  await browser?.close();
  await writeFile(resolve(output, process.env.CHAT_QA_REPORT || 'result.json'), JSON.stringify(results, null, 2));
}
console.log(JSON.stringify(results, null, 2));
assert.deepEqual(results.failures, [], 'browser cases failed');
assert.deepEqual(results.pageErrors, [], 'uncaught browser errors');
assert.deepEqual(results.unexpectedRequests, [], 'unexpected requests escaped the fixture');

// Real browser + production static UI/renderer + live incremental HTTP SSE.
// Account/conversation API data is synthetic and local; no real accounts are used.
// Reuse Playwright with HFY_PLAYWRIGHT_MODULE (or PLAYWRIGHT_MODULE) and optional
// HFY_BROWSER_EXECUTABLE. Screenshots, when requested, go to CHAT_STREAM_QA_OUT_DIR.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
import { resolve, extname, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as pause } from 'node:timers/promises';

const repo = resolve(fileURLToPath(new URL('..', import.meta.url)));
const assets = resolve(repo, 'static');
const runtime = process.env.HFY_PLAYWRIGHT_MODULE || process.env.PLAYWRIGHT_MODULE || (process.env.CODEX_PRIMARY_RUNTIME_NODE_MODULES ? pathToFileURL(resolve(process.env.CODEX_PRIMARY_RUNTIME_NODE_MODULES, 'playwright/index.mjs')).href : 'playwright');
const { chromium } = await import(runtime.startsWith('/') ? pathToFileURL(runtime).href : runtime);
const rooms = [
  { id: 'platform_qa-member', kind: 'platform', title: '平台团队', user_id: 'qa-member', writable: true, unread_count: 0, participants: [] },
  { id: 'match_fixture', kind: 'matched', title: 'Rust 工程师 · 测试团队', user_id: 'qa-member', job_id: 'fixture-job', match_id: 'fixture-match', writable: true, unread_count: 0, participants: [] },
];
const histories = new Map(rooms.map((room) => [room.id, []]));
const subscriptions = new Set();
const events = [];
const apiCalls = [];
const pageErrors = [];
const externalRequests = [];
let nextMessage = 0;
let nextEvent = 0;
let origin;
let browser;
const config = {
  name: 'HireFerry', default_language: 'zh', humangrant_enabled: false, google_enabled: false, email_enabled: true,
  development: false, support_email: 'fixture@example.test', providers: [],
  pricing: { currency: 'CNY', currency_decimals: 2, allowed_currencies: ['CNY'], onboarding_cents: 10000, publish_bps: 1000, edit_bps: 100, max_edits: 3 },
  limits: { list_page_size: 50, workbench_page_size: 25, recent_list_limit: 200 },
  chat: { welcome: {
    platform_seeker: { title: '为你的下一步搭桥', body_html: '<p>欢迎使用 <strong>可配置欢迎语</strong>。</p>', prompts: [{ label: '讨论我的方向', body: '我想聊聊适合我的岗位。' }] },
    matched: { title: '开始岗位沟通', body_html: '<p>确认下一步安排。</p>', prompts: [] },
  } },
};
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.woff2': 'font/woff2' };
const csp = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
const json = (response, value, status = 200) => { response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' }); response.end(JSON.stringify(value)); };
const openSubscriptions = (room) => [...subscriptions].filter((entry) => entry.room === room && !entry.response.destroyed && !entry.response.writableEnded);
const encodeEvent = (event) => `id: ${event.id}\nevent: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`;
function publish(event) {
  const value = { ...event, id: ++nextEvent };
  events.push(value);
  for (const entry of openSubscriptions(value.conversation_id)) entry.response.write(encodeEvent(value));
  return value;
}
function newMessage(room, body = '', format = 'markdown', status = 'complete', own = false) {
  const message = { id: ++nextMessage, conversation_id: room, body, body_html: '', format, status, sequence: 0, created_at: Math.floor(Date.now() / 1000), updated_at: Math.floor(Date.now() / 1000), sender_kind: own ? 'user' : 'platform', sender_id: own ? 'qa-member' : null, sender_name: own ? '测试用户' : '平台团队', is_own: own, client_id: `fixture-${nextMessage}`, via_ai: !own };
  histories.get(room).push(message);
  return message;
}
function begin(room, format = 'markdown', body = '') {
  const message = newMessage(room, body, format, 'streaming');
  publish({ type: 'message.started', conversation_id: room, message_id: message.id, sequence: message.sequence, message: { ...message } });
  return message;
}
function append(message, delta) {
  message.body += delta; message.sequence++;
  return publish({ type: 'message.delta', conversation_id: message.conversation_id, message_id: message.id, sequence: message.sequence, delta });
}
function finish(message, status = 'complete') {
  message.status = status; message.sequence++;
  publish({ type: status === 'cancelled' ? 'message.cancelled' : 'message.completed', conversation_id: message.conversation_id, message_id: message.id, sequence: message.sequence, message: { ...message } });
}
async function eventually(condition, label) {
  const deadline = Date.now() + 8000;
  while (Date.now() < deadline) { if (await condition()) return; await pause(25); }
  assert.fail(label);
}
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, origin || 'http://localhost');
    if (!url.pathname.startsWith('/api/')) {
      const relative = url.pathname.startsWith('/static/') ? decodeURIComponent(url.pathname.slice(8)) : 'index.html';
      const path = resolve(assets, relative);
      if (!path.startsWith(assets + sep)) return json(response, { error: 'Not found' }, 404);
      const body = await readFile(path).catch(() => null);
      if (!body) return json(response, { error: 'Not found' }, 404);
      response.writeHead(200, { 'content-type': mime[extname(path)] || 'application/octet-stream', 'content-security-policy': csp, 'cache-control': 'no-store' }); response.end(body); return;
    }
    apiCalls.push({ path: url.pathname, method: request.method });
    if (url.pathname === '/api/config') return json(response, config);
    if (url.pathname === '/api/session') return json(response, { user: { id: 'qa-member', name: '测试用户', email: 'fixture@example.test', role: 'seeker', employer_paid: false }, admin: false, super_admin: false, staff: null, bootstrap: false, csrf: 'synthetic-csrf' });
    if (url.pathname === '/api/ai/eligibility') return json(response, { eligible: true });
    if (url.pathname === '/api/admin/auth/status') return json(response, { initialized: true });
    if (url.pathname === '/api/chat/events/stream') {
      const room = url.searchParams.get('conversation_id');
      if (!histories.has(room)) return json(response, { error: 'Not found' }, 404);
      const after = Math.max(Number(url.searchParams.get('after') || 0), Number(request.headers['last-event-id'] || 0));
      response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache, no-transform', connection: 'keep-alive', 'x-accel-buffering': 'no' });
      response.write('retry: 150\n: connected\n\n');
      for (const event of events.filter((event) => event.conversation_id === room && event.id > after)) response.write(encodeEvent(event));
      const entry = { room, response };
      subscriptions.add(entry);
      response.once('close', () => subscriptions.delete(entry));
      return;
    }
    if (url.pathname === '/api/chat/conversations') return json(response, { items: rooms.map((room) => ({ ...room, last_message_preview: histories.get(room.id).at(-1)?.body || '', last_message_at: histories.get(room.id).at(-1)?.created_at || 0 })) });
    const route = /^\/api\/chat\/conversations\/([^/]+)(?:\/(read|messages))?$/u.exec(url.pathname);
    if (route && histories.has(route[1])) {
      const room = rooms.find((item) => item.id === route[1]), history = histories.get(room.id);
      if (!route[2]) return json(response, room);
      if (route[2] === 'read') { request.resume(); return json(response, { ok: true }); }
      if (request.method === 'POST') {
        let body = ''; for await (const chunk of request) body += chunk;
        const payload = JSON.parse(body);
        const message = newMessage(room.id, payload.body, payload.format || 'markdown', 'complete', true);
        message.client_id = payload.client_id;
        publish({ type: 'message.created', conversation_id: room.id, message_id: message.id, sequence: 0, message: { ...message } });
        return json(response, message);
      }
      const after = Number(url.searchParams.get('after') || 0), before = Number(url.searchParams.get('before') || Infinity);
      const items = history.filter((item) => item.id > after && item.id < before).slice(-100);
      return json(response, { items, next_cursor: items.at(-1)?.id || after, first_cursor: items[0]?.id || 0, has_more: false, has_older: false, event_cursor: nextEvent });
    }
    return json(response, { error: 'Unhandled fixture endpoint' }, 404);
  } catch (error) { if (!response.headersSent) json(response, { error: error.message }, 500); else response.destroy(); }
});

try {
  await new Promise((done, fail) => { server.once('error', fail); server.listen(0, '127.0.0.1', done); });
  origin = `http://127.0.0.1:${server.address().port}`;
  const browserArgs = ['--no-sandbox', '--disable-dev-shm-usage'];
  if (process.env.HFY_BROWSER_ARGS_MODULE) {
    const source = process.env.HFY_BROWSER_ARGS_MODULE;
    const module = await import(source.startsWith('/') ? pathToFileURL(source).href : source);
    browserArgs.push(...((module.default || module).args || []));
  }
  browser = await chromium.launch({ headless: true, executablePath: process.env.HFY_BROWSER_EXECUTABLE || process.env.BROWSER_EXECUTABLE || undefined, args: browserArgs });
  const context = await browser.newContext({ viewport: { width: 1180, height: 900 }, reducedMotion: 'reduce', locale: 'zh-CN' });
  await context.addInitScript(() => { localStorage.setItem('hireferry-language', 'zh'); localStorage.setItem('hireferry-theme', 'light'); });
  await context.route('**/*', (route) => {
    if (new URL(route.request().url()).origin === origin) return route.continue();
    externalRequests.push(route.request().url()); return route.abort();
  });
  const page = await context.newPage(); page.setDefaultTimeout(10000);
  page.on('pageerror', (error) => pageErrors.push(error.message));
  const room = rooms[0].id;
  await page.goto(`${origin}/app?tab=messages&conversation=${room}`);
  await page.locator('#chat-scroll[aria-busy="false"]').waitFor();
  assert.equal(await page.locator('#chat-empty h3').textContent(), '为你的下一步搭桥');
  assert.equal(await page.locator('#chat-empty strong').textContent(), '可配置欢迎语');
  await page.locator('[data-chat-prompt]').click();
  assert.equal(await page.locator('#chat-body').inputValue(), '我想聊聊适合我的岗位。');
  await page.locator('#chat-body').fill('');
  await eventually(() => openSubscriptions(room).length === 1, 'chat did not open its native SSE connection');
  console.log('PASS configurable welcome and editable prompt');

  const markdown = begin(room);
  append(markdown, '# 实时消息\n\n正在');
  await page.locator(`[data-message-id="${markdown.id}"] h1`).waitFor();
  await page.evaluate((id) => { window.__streamMessageNode = document.querySelector(`[data-message-id="${id}"]`); window.__streamBodyNode = window.__streamMessageNode.querySelector('.hfy-rich-message'); }, markdown.id);
  append(markdown, '分段生成。\n\n| 项目 | 状态 |\n| --- | --- |\n| 岗位 | 开放 |\n\n$E=mc^2$\n\n```rust\nlet answer = 42;\n```');
  await page.locator(`[data-message-id="${markdown.id}"] .katex`).waitFor();
  assert.equal(await page.locator(`[data-message-id="${markdown.id}"] table tbody tr`).count(), 1);
  finish(markdown);
  await page.waitForFunction((id) => document.querySelector(`[data-message-id="${id}"]`)?.dataset.messageStatus === 'complete', markdown.id);
  assert.equal(await page.locator(`[data-message-id="${markdown.id}"]`).count(), 1);
  assert.ok(await page.evaluate((id) => document.querySelector(`[data-message-id="${id}"]`) === window.__streamMessageNode && window.__streamMessageNode.querySelector('.hfy-rich-message') === window.__streamBodyNode, markdown.id));
  assert.equal(await page.evaluate(async () => { await document.fonts.ready; return document.fonts.check('16px KaTeX_Main'); }), true);
  console.log('PASS native SSE deltas update one message with GFM/code/math');

  const openui = begin(room, 'openui');
  append(openui, '$name = ""\nroot = Col([title, table, form])\ntitle = Title("面试确认")\n');
  await page.locator(`[data-message-id="${openui.id}"] .hfy-ui-title`).waitFor();
  append(openui, 'table = Table(["日期", "方式"], [["周三", "线上"]])\nform = Form([field, button])\nfield = Input($name, "姓名")\nbutton = Button("确认时间", @ToAssistant("我想选择周三面试"))');
  const action = page.locator(`[data-message-id="${openui.id}"] button`);
  await action.waitFor(); assert.equal(await action.isDisabled(), true);
  finish(openui);
  await page.waitForFunction((id) => !document.querySelector(`[data-message-id="${id}"] button`)?.disabled, openui.id);
  await page.locator(`[data-message-id="${openui.id}"] input`).fill('测试候选人');
  await action.click();
  const actionDraft = await page.locator('#chat-body').inputValue();
  assert.match(actionDraft, /我想选择周三面试/u);
  assert.match(actionDraft, /\$name/u); // OpenUI identifies bound fields by their state variable.
  assert.match(actionDraft, /测试候选人/u);
  assert.equal(apiCalls.filter((entry) => entry.method === 'POST' && entry.path.endsWith('/messages')).length, 0);
  console.log('PASS OpenUI streaming and draft-only action');

  // A sequence gap must fetch a snapshot, not concatenate out-of-order text.
  const gap = begin(room, 'markdown', 'Before gap');
  await page.locator(`[data-message-id="${gap.id}"]`).waitFor();
  const snapshots = apiCalls.filter((entry) => entry.method === 'GET' && entry.path.endsWith('/messages')).length;
  gap.body = '**Recovered snapshot**'; gap.sequence = 4;
  publish({ type: 'message.delta', conversation_id: room, message_id: gap.id, sequence: 4, delta: 'must not concatenate' });
  await page.locator(`[data-message-id="${gap.id}"] .hfy-rich-message strong`).waitFor();
  assert.equal(await page.locator(`[data-message-id="${gap.id}"] .hfy-rich-message strong`).textContent(), 'Recovered snapshot');
  assert.ok(apiCalls.filter((entry) => entry.method === 'GET' && entry.path.endsWith('/messages')).length > snapshots);
  await eventually(() => openSubscriptions(room).length === 1, 'stream did not resume after snapshot');
  finish(gap);
  console.log('PASS sequence-gap resync');

  // Exercise native EventSource reconnection and Last-Event-ID replay.
  const reconnect = begin(room, 'markdown', 'Before reconnect');
  await page.locator(`[data-message-id="${reconnect.id}"]`).waitFor();
  for (const entry of openSubscriptions(room)) entry.response.end();
  append(reconnect, ' **after reconnect**'); finish(reconnect);
  await page.locator(`[data-message-id="${reconnect.id}"] .hfy-rich-message strong`).waitFor();
  assert.equal(await page.locator(`[data-message-id="${reconnect.id}"] .hfy-rich-message strong`).textContent(), 'after reconnect');
  assert.equal(await page.locator(`[data-message-id="${reconnect.id}"]`).count(), 1);
  console.log('PASS EventSource reconnect/replay');

  await page.locator(`.conversation-item[href*="${rooms[1].id}"]`).click();
  await page.locator('#chat-empty h3').waitFor();
  assert.equal(await page.locator('#chat-empty h3').textContent(), '开始岗位沟通');
  await eventually(() => openSubscriptions(room).length === 0 && openSubscriptions(rooms[1].id).length === 1, 'switching rooms leaked the old event stream');
  assert.equal(await page.evaluate(() => window.__streamMessageNode.isConnected), false);
  const hidden = begin(room, 'markdown', 'Only the old room'); finish(hidden);
  await pause(100);
  assert.equal(await page.locator(`[data-message-id="${hidden.id}"]`).count(), 0);
  await page.locator(`.conversation-item[href*="${room}"]`).click();
  await page.locator(`[data-message-id="${hidden.id}"]`).waitFor();
  assert.equal(await page.locator('#chat-body').inputValue(), actionDraft);
  console.log('PASS room-switch cleanup and draft preservation');

  if (process.env.CHAT_STREAM_QA_OUT_DIR) {
    await mkdir(resolve(process.env.CHAT_STREAM_QA_OUT_DIR), { recursive: true });
    await page.screenshot({ path: resolve(process.env.CHAT_STREAM_QA_OUT_DIR, 'chat-streaming-desktop.png'), fullPage: false });
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => document.fonts.ready);
  const geometry = await page.evaluate(() => ({ viewport: innerWidth, document: document.documentElement.scrollWidth, chat: document.querySelector('#chat-scroll').clientWidth, content: document.querySelector('#chat-scroll').scrollWidth }));
  assert.ok(geometry.document <= geometry.viewport + 1 && geometry.content <= geometry.chat + 1, `chat overflow: ${JSON.stringify(geometry)}`);
  if (process.env.CHAT_STREAM_QA_OUT_DIR) { await mkdir(resolve(process.env.CHAT_STREAM_QA_OUT_DIR), { recursive: true }); await page.screenshot({ path: resolve(process.env.CHAT_STREAM_QA_OUT_DIR, 'chat-streaming-mobile.png'), fullPage: false }); }
  assert.deepEqual(externalRequests, [], 'renderer tried an external network request');
  assert.deepEqual(pageErrors, [], 'browser had unhandled errors');
  console.log('PASS mobile layout, self-hosted assets and no unhandled errors');
  await page.goto('about:blank');
  await eventually(() => subscriptions.size === 0, 'navigation left event streams open');
  console.log('PASS navigation disposes live streams');
} finally {
  await browser?.close();
  for (const entry of subscriptions) entry.response.end();
  server.closeAllConnections();
  await new Promise((done) => server.close(done));
}

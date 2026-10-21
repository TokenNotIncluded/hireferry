import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';

const serviceSource = await readFile(new URL('../static/service-ui.js', import.meta.url), 'utf8');
const { createServiceUI, createMessageId } = await import('data:text/javascript;base64,' + Buffer.from(serviceSource).toString('base64'));
const appSource = (await readFile(new URL('../static/app.js', import.meta.url), 'utf8')).replace(/^import .*;\n/gm, '').replace(/render\(\);\s*$/, '');
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

test('message IDs use secure random bytes when randomUUID is unavailable', () => {
  const nonSecureContextCrypto = { getRandomValues: webcrypto.getRandomValues.bind(webcrypto) };
  assert.equal(nonSecureContextCrypto.randomUUID, undefined);
  const first = createMessageId(nonSecureContextCrypto);
  const second = createMessageId(nonSecureContextCrypto);
  assert.match(first, /^[0-9a-f]{32}$/);
  assert.match(second, /^[0-9a-f]{32}$/);
  assert.notEqual(first, second);
});

function ui(api, { admin = false, search = '', state = {} } = {}) {
  globalThis.location = { search, origin: 'https://hireferry.test' };
  return createServiceUI({
    state: { lang: 'en', user: { id: 'seeker-one', role: 'seeker' }, admin, ...state }, api,
    t: (_, en) => en, esc: escape, icon: name => `<i data-icon="${name}"></i>`, avatar: () => '',
    money: cents => `CNY ${Number(cents) / 100}`, date: value => String(value || ''),
    heading: (title, subtitle, action) => `<h1>${title}</h1><p>${subtitle}</p>${action || ''}`,
    shell: body => body, empty: (title, text) => `<h3>${title}</h3><p>${text}</p>`, notice: text => `<p>${text}</p>`,
    badge: value => value, matchText: value => value,
    openDialog() {}, closeDialog() {}, notify() {}, go() {}, render: async () => {},
    centsInput: value => Math.round(Number(value) * 100),
  });
}

test('default workspace reads only the private platform thread and escapes message metadata', async () => {
  const requests = [];
  const service = ui(async (path, options) => {
    requests.push([path, options?.method || 'GET']);
    if (path === '/api/chat/platform') return {};
    if (path === '/api/chat/conversations') return { items: [
      { id: 'platform_seeker-one', kind: 'platform', user_id: 'seeker-one', title: 'Private', writable: true, participants: [] },
      { id: 'match_one', kind: 'matched', title: 'Introduced role', writable: true, participants: [] },
    ] };
    assert.equal(path, '/api/chat/conversations/platform_seeker-one/messages?limit=100');
    return { items: [{ id: 7, sender_kind: 'user', sender_id: 'seeker-one', sender_name: '<script>bad</script>', body: 'private-only message', via_ai: true, created_at: 1 }], next_cursor: 7, first_cursor: 7 };
  });
  const html = await service.messagesPage();
  assert.match(html, /private-only message/);
  assert.match(html, /&lt;script&gt;bad&lt;\/script&gt;/);
  assert.match(html, /via AI/);
  assert.match(html, /Only you and HireFerry/);
  assert.equal(requests.some(([path]) => path.includes('/match_one/messages')), false);
});

test('archived administrator rooms display history but cannot be composed into', async () => {
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'match_archived', kind: 'matched', title: 'Concluded search', writable: false, participants: [] }] }
    : { items: [], next_cursor: 0, has_older: false }, { admin: true });
  const html = await service.messagesPage(true);
  assert.match(html, /Archived conversation · read-only/);
  assert.match(html, /<textarea[^>]*disabled/);
  assert.match(html, /<button class="button small" type="submit" disabled/);
  assert.match(html, /Private platform history stays separate/);
});

test('a new search brief never preselects external outreach consent', async () => {
  const service = ui(async path => path === '/api/mandate' ? { mandate: null } : { items: [] });
  const html = await service.mandatePage();
  const checkbox = html.match(/<input[^>]*name="allow_external"[^>]*>/)?.[0];
  assert.ok(checkbox);
  assert.doesNotMatch(checkbox, /checked/);
  assert.match(html, /Only HireFerry can view this brief/);
  assert.match(html, /Not submitted/);
});

test('administrator form updates preserve existing consent rather than trust submitted checkbox data', async () => {
  let captured;
  const service = ui(async (path, options) => { captured = { path, ...options }; return {}; }, { admin: true });
  const data = new FormData();
  for (const [key, value] of Object.entries({ target_industry: 'Software', target_roles: 'Engineer', salary_min: '3000', salary_max: '6000', location: 'Remote', preferences: 'Part-time', status: 'paused', allow_external: 'on' })) data.set(key, value);
  await service.saveMandate({ dataset: { id: 'seeker-one', allowExternal: 'false' } }, data, true);
  assert.equal(captured.path, '/api/admin/mandates/seeker-one');
  assert.equal(captured.method, 'PATCH');
  assert.equal(captured.body.allow_external, false);
  assert.equal(captured.body.salary_min_cents, 300000);
  assert.equal(captured.body.salary_max_cents, 600000);
});

test('matching board renders real stages and keeps private and shared conversation targets distinct', async () => {
  const service = ui(async () => ({ items: [
    { id: 'active', seeker_id: 's1', seeker_name: 'Person A', employer_id: 'e1', employer_name: 'Team A', job_id: 'j1', job_title: 'Engineer', status: 'interview', note: 'Shared introduction' },
    { id: 'closed', seeker_id: 's2', seeker_name: 'Person B', employer_id: 'e2', employer_name: 'Team B', job_id: 'j2', job_title: 'Designer', status: 'declined', note: '' },
  ] }), { admin: true });
  const html = await service.matchesPage(true);
  for (const status of ['introduced', 'interview', 'hired', 'declined']) assert.match(html, new RegExp(`id="match-column-${status}"`));
  for (const target of ['match_active', 'platform_s1', 'platform_e1', 'match_closed']) assert.match(html, new RegExp(`conversation=${target}`));
  assert.match(html, /2 real introductions/);
  assert.doesNotMatch(html, /compatibility|match score|98%/i);
});

function webmcp({ eligible = true, role = 'seeker', admin = false } = {}) {
  const registered = new Map();
  const requests = [];
  const modelContext = { registerTool: tool => registered.set(tool.name, tool), unregisterTool: name => registered.delete(name) };
  const node = { innerHTML: '', replaceChildren() {} };
  const document = { querySelector: () => node, addEventListener() {}, removeEventListener() {}, body: { style: {} }, documentElement: {}, modelContext };
  const fetch = async (path, options) => {
    requests.push({ path, options });
    return { ok: true, json: async () => path === '/api/ai/eligibility' ? { eligible } : { ok: true } };
  };
  const app = new Function('document', 'navigator', 'window', 'localStorage', 'location', 'history', 'fetch', 'createServiceUI', 'createMessageId', appSource + '\nreturn {state,registerWebMCP};')(
    document, { modelContext }, { addEventListener() {} }, { getItem: () => null }, { pathname: '/app', search: '', origin: 'https://hireferry.test' }, {}, fetch, () => ({}), createMessageId,
  );
  Object.assign(app.state, { user: admin ? null : { id: 'one', role, employer_paid: true }, admin, csrf: 'synthetic-csrf', eligibility: { eligible } });
  app.registerWebMCP();
  return { app, registered, requests, setEligible(value) { eligible = value; } };
}

test('an ineligible employer receives no WebMCP tools', () => {
  assert.equal(webmcp({ eligible: false, role: 'employer' }).registered.size, 0);
});

test('registered user tools recheck current eligibility before any business request', async () => {
  const harness = webmcp({ role: 'employer' });
  const tool = harness.registered.get('hireferry_list_my_jobs');
  assert.ok(tool);
  harness.setEligible(false);
  const result = await tool.execute({});
  assert.equal(result.isError, true);
  assert.deepEqual(harness.requests.map(r => r.path), ['/api/ai/eligibility']);
});

test('WebMCP messages carry the AI attribution header and preserve explicit conversation IDs', async () => {
  const harness = webmcp();
  await harness.registered.get('hireferry_send_message').execute({ conversation_id: 'platform_one', body: 'Hello', client_id: 'synthetic-message-id' });
  const sent = harness.requests.at(-1);
  assert.equal(sent.path, '/api/chat/conversations/platform_one/messages');
  assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
  assert.equal(sent.options.headers['X-CSRF-Token'], 'synthetic-csrf');
  assert.deepEqual(JSON.parse(sent.options.body), { body: 'Hello', client_id: 'synthetic-message-id' });
  assert.equal(harness.registered.has('hireferry_admin_list_users'), false);
  assert.equal(harness.registered.has('hireferry_list_people'), false);
});

test('administrator WebMCP exposes business operations without token or passkey management', () => {
  const { registered } = webmcp({ admin: true });
  for (const name of ['hireferry_admin_list_users', 'hireferry_admin_create_introduction', 'hireferry_admin_record_outreach', 'hireferry_admin_send_message']) assert.ok(registered.has(name));
  assert.equal([...registered.keys()].some(name => /token|passkey|config/.test(name)), false);
  assert.equal(registered.has('hireferry_update_profile'), false);
});

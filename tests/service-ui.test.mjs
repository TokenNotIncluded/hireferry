import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { createNavigationHarness } from './helpers/navigation-harness.mjs';

const moneySource = await readFile(new URL('../static/money.js', import.meta.url), 'utf8');
const moneyHelpers = await import('data:text/javascript;base64,' + Buffer.from(moneySource).toString('base64'));
const requestSource = await readFile(new URL('../static/requests.js', import.meta.url), 'utf8');
const requestHelpers = await import('data:text/javascript;base64,' + Buffer.from(requestSource).toString('base64'));
const teamSource = await readFile(new URL('../static/team-ui.js', import.meta.url), 'utf8');
const teamHelpers = await import('data:text/javascript;base64,' + Buffer.from(teamSource).toString('base64'));
const cliGuideSource = await readFile(new URL('../static/cli-guide.js', import.meta.url), 'utf8');
const cliGuideHelpers = await import('data:text/javascript;base64,' + Buffer.from(cliGuideSource).toString('base64'));
const billingSource = await readFile(new URL('../static/billing-ui.js', import.meta.url), 'utf8');
const billingHelpers = await import('data:text/javascript;base64,' + Buffer.from(billingSource).toString('base64'));
const browserCacheSource = await readFile(new URL('../static/browser-cache.js', import.meta.url), 'utf8');
const browserCacheHelpers = await import('data:text/javascript;base64,' + Buffer.from(browserCacheSource).toString('base64'));
const cliMotionSource = await readFile(new URL('../static/cli-motion.js', import.meta.url), 'utf8');
const cliMotionHelpers = await import('data:text/javascript;base64,' + Buffer.from(cliMotionSource).toString('base64'));
const contentRefreshSource = await readFile(new URL('../static/content-refresh.js', import.meta.url), 'utf8');
const contentRefreshHelpers = await import('data:text/javascript;base64,' + Buffer.from(contentRefreshSource).toString('base64'));
const workspaceHelpers = Object.assign({}, ...await Promise.all(['controls.js','finance-ui.js','people-jobs-ui.js','staff-ui.js','relation-webmcp.js'].map(name => import(new URL('../static/' + name, import.meta.url).href))));
// Page markup tests use a DOM sink; disclosure events have their own real controller tests.
const createDisclosureUI = () => ({ sync() {}, reset() {}, dispose() {} });
const defaultConfig = { pricing: { currency:'CNY', currency_decimals:2, allowed_currencies:['CNY','USD','JPY'], currency_options:[{currency:'CNY',currency_decimals:2,publish_fixed_minor:1000,edit_fixed_minor:100,employer_ai_min_salary_minor:300000},{currency:'USD',currency_decimals:2,publish_fixed_minor:500,edit_fixed_minor:50,employer_ai_min_salary_minor:100000},{currency:'JPY',currency_decimals:0,publish_fixed_minor:1000,edit_fixed_minor:100,employer_ai_min_salary_minor:200000}], onboarding_cents:10000,publish_mode:'salary_percent',publish_bps:1000,publish_fixed_minor:1000,edit_mode:'salary_percent',edit_bps:100,edit_fixed_minor:100,max_edits:3 }, limits:{max_job_headcount:1000000,list_page_size:50,list_max_page:10000,workbench_page_size:25,recent_list_limit:200}, providers:['stripe'] };

const serviceSource = await readFile(new URL('../static/service-ui.js', import.meta.url), 'utf8');
const { createServiceUI, createMessageId, filterConversations, chatSendShortcut, messageDateKey, createChatDateFormatter } = await import('data:text/javascript;base64,' + Buffer.from(serviceSource).toString('base64'));
const promoSource = await readFile(new URL('../static/promo-ui.js', import.meta.url), 'utf8');
const {createPromoUI} = await import('data:text/javascript;base64,' + Buffer.from(promoSource).toString('base64'));
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

function ui(api, { admin = false, search = '', state = {}, hooks = {} } = {}) {
  globalThis.location = { search, origin: 'https://hireferry.test' };
  const uiState = { config: structuredClone(defaultConfig), lang: 'en', user: { id: 'seeker-one', role: 'seeker' }, admin, ...state };
  const currency = record => moneyHelpers.currencyMeta(uiState.config,record);
  return createServiceUI({
    state: uiState, api,
    t: (zh, en) => uiState.lang === 'zh' ? zh : en, esc: escape, icon: name => `<i data-icon="${name}"></i>`, avatar: () => '',
    money: (amount,record) => moneyHelpers.formatMinor(amount,currency(record),'en-US'), currency, currencies:()=>moneyHelpers.currencyOptions(uiState.config), amountValue:(amount,record)=>moneyHelpers.minorToDecimal(amount,currency(record).currency_decimals), amountStep:record=>moneyHelpers.minorStep(currency(record).currency_decimals), currencySelect:(selected,name,id)=>`<select id="${id}" name="${name}">${moneyHelpers.currencyOptions(uiState.config).map(option=>`<option value="${option.currency}" ${option.currency===selected?'selected':''}>${option.currency}</option>`).join('')}</select>`, date: value => String(value || ''),
    heading: (title, subtitle, action) => `<h1>${title}</h1><p>${subtitle}</p>${action || ''}`,
    shell: body => body, empty: (title, text) => `<h3>${title}</h3><p>${text}</p>`, notice: text => `<p>${text}</p>`,
    badge: value => value, matchText: value => value,
    openDialog() {}, closeDialog() {}, notify() {}, go() {}, render: async () => {},
    centsInput: (value,record,options={}) => moneyHelpers.parseMinor(value,currency(record).currency_decimals,{min:1,max:1000000000,...options}),
    ...hooks,
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
  assert.match(html,/class="chat-thread" data-page-key="conversation:platform_seeker-one"/);
  assert.match(html, /&lt;script&gt;bad&lt;\/script&gt;/);
  assert.match(html, /via AI/);
  assert.match(html, /Only you and HireFerry/);
  assert.equal(requests.some(([path]) => path.includes('/match_one/messages')), false);
});

test('seekers with a generated placeholder handle see a profile claim nudge in the inbox', async () => {
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'platform_seeker-one', kind: 'platform', title: 'Private', writable: true, participants: [] }] }
    : { items: [], next_cursor: 0, first_cursor: 0 },
  { state: { user: { id: 'seeker-one', role: 'seeker', handle: 'user_0123456789abcdef' } } });
  const html = await service.messagesPage();
  assert.match(html, /profile-claim-nudge/);
  assert.match(html, /Claim your unique name first/);
  assert.match(html, /href="\/app\?tab=profile"[^>]*data-nav/);
});

test('claimed seekers do not see the profile claim nudge in the inbox', async () => {
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'platform_seeker-one', kind: 'platform', title: 'Private', writable: true, participants: [] }] }
    : { items: [], next_cursor: 0, first_cursor: 0 },
  { state: { user: { id: 'seeker-one', role: 'seeker', handle: 'lightdot' } } });
  const html = await service.messagesPage();
  assert.doesNotMatch(html, /profile-claim-nudge/);
});

test('archived administrator rooms display history but cannot be composed into', async () => {
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'match_archived', kind: 'matched', title: 'Concluded search', writable: false, participants: [] }] }
    : { items: [], next_cursor: 0, has_older: false }, { admin: true });
  const html = await service.messagesPage(true);
  assert.match(html, /read.only/i);
  assert.match(html, /<textarea[^>]*disabled/);
  assert.match(html, /<button\b(?=[^>]*class="button small")(?=[^>]*type="submit")(?=[^>]*\bdisabled)[^>]*>/);
  assert.match(html, /Private platform history stays separate/);
});

test('internal conversations distinguish coworkers and AI from the current staff member', async () => {
  const messages = [
    { id: 1, sender_kind: 'platform', is_own: true, staff_name: 'My staff identity', body: 'Mine', created_at: 1 },
    { id: 2, sender_kind: 'platform', is_own: false, staff_name: '<Colleague>', body: 'Colleague reply', created_at: 2 },
    { id: 3, sender_kind: 'platform', is_own: false, staff_name: '平台AI', body: 'AI reply', via_ai: true, created_at: 3 },
    { id: 4, sender_kind: 'platform', is_own: false, staff_name: '超级管理员', body: 'Owner reply', created_at: 4 },
    { id: 5, sender_kind: 'platform', body: 'Legacy message', created_at: 5 },
  ];
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'platform_member', kind: 'platform', title: 'Private', writable: true, participants: [] }] }
    : { items: messages, next_cursor: 5, first_cursor: 1 }, { admin: true, search: '?tab=messages&conversation=platform_member' });
  const html = await service.messagesPage(true);
  const message = id => html.match(new RegExp(`<article[^>]*data-message-id="${id}"[\\s\\S]*?</article>`))?.[0] || '';
  assert.match(message(1), /class="chat-message own from-platform"/);
  assert.match(message(1), /<strong>My staff identity<\/strong>/);
  for (const id of [2, 3, 4]) assert.doesNotMatch(message(id), /chat-message own/);
  assert.match(message(2), /&lt;Colleague&gt;/);
  assert.doesNotMatch(message(2), /<Colleague>/);
  assert.match(message(3), /<strong>Platform AI<\/strong>/);
  assert.match(message(3), /via AI/);
  assert.match(message(4), /<strong>Super administrator<\/strong>/);
  assert.match(message(5), /chat-message own/);
  assert.match(message(5), /<strong>HireFerry team<\/strong>/);
});

test('customer conversations never expose internal staff names or adopt administrator ownership metadata', async () => {
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [{ id: 'platform_seeker-one', kind: 'platform', title: 'Private', writable: true, participants: [] }] }
    : { items: [{ id: 1, sender_kind: 'platform', sender_id: 'internal-actor', sender_name: 'internal-actor', staff_name: 'Private staff name', is_own: true, body: 'Hello', via_ai: true, created_at: 1 }], next_cursor: 1, first_cursor: 1 });
  const html = await service.messagesPage();
  assert.match(html, /<strong>HireFerry team<\/strong>/);
  assert.match(html, /via AI/);
  assert.doesNotMatch(html, /Private staff name|internal-actor|chat-message own/);
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

const workbenchRole = { id: 'role-one', owner_id: 'employer-one', employer_id: 'employer-one', employer_name: 'Team A', title: 'Rust Engineer', salary_cents: 600000, headcount: 3, status: 'published', jd_excerpt: 'Build a service.', active_matches: 2 };
const workbenchPerson = { id: 'person-one', name: 'Person A', target_industry: 'Software', target_roles: 'Engineer', location: 'Remote', salary_min_cents: 500000, salary_max_cents: 900000, history_match: null };
const workbenchDetail = (person = workbenchPerson, match = null) => ({ user: { ...person, role: 'seeker', banned: false, public_email: 'contact@example.test', resume_md: 'Résumé' }, mandate: { ...person, status: 'active', allow_external: false, preferences: 'Remote first.' }, resume_html: '<h2>Sanitized résumé</h2><p>Rust experience.</p>', match });
const workbenchResult = (items, next_cursor = null) => ({ items, total: items.length, limit: 25, next_cursor });
const browserNavigate = path => { globalThis.location.search = new URL(path, 'https://hireferry.test').search; };

test('matching workspace initially fetches only one server page of hiring roles', async () => {
  const requests = [];
  const service = ui(async path => { requests.push(path); return { ...workbenchResult([workbenchRole]), total: 1000 }; }, { admin: true });
  const html = await service.matchesPage(true);
  assert.deepEqual(requests, ['/api/admin/workbench/jobs?limit=25']);
  assert.match(html, /Select a role first/);
  assert.match(html, /1 on this page · 1000 total/);
  assert.match(html, /Target: 3/);
  assert.doesNotMatch(html, /match-column|<select[^>]*name="seeker_id"|match score|98%/i);
});

test('selected role queries candidates with server filters without fetching profiles', async () => {
  const requests = [];
  const service = ui(async path => {
    requests.push(path);
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([workbenchRole]);
    const url = new URL(path, 'https://hireferry.test');
    assert.equal(url.pathname, '/api/admin/workbench/candidates');
    assert.deepEqual(Object.fromEntries(url.searchParams), { limit: '25', job_id: 'role-one', q: 'Rust', industry: 'Software', roles: 'Engineer', location: 'Remote', salary_min_cents: '600000', salary_max_cents: '600000', salary_currency: 'CNY' });
    return { ...workbenchResult([workbenchPerson]), total: 100000 };
  }, { admin: true, search: '?tab=matches&job=role-one&candidate_q=Rust&industry=Software&roles=Engineer&location=Remote&salary_fit=1' });
  const html = await service.matchesPage(true);
  assert.equal(requests.length, 2);
  assert.match(html, /Person A/);
  assert.match(html,/data-dom-key="candidate:person-one"/);
  assert.match(html,/data-page-key="candidate-page:\d+"/);
  assert.match(html, /1 on this page · 100000 total/);
  assert.match(html, /Not introduced/);
  assert.doesNotMatch(html, /Sanitized résumé/);
  assert.match(html, /<details[^>]*data-disclosure="candidate-filters:[^"]*" open>/);
  assert.match(html, /Advanced filters[\s\S]*Industry: Software · Role: Engineer · Location: Remote/);
});

test('candidate details are loaded on demand and reopened introductions reuse the existing match', async () => {
  const writes = [];
  const oldMatch = { id: 'old-introduction', status: 'declined', note: 'Earlier shared note' };
  const person = { ...workbenchPerson, name: '<script>candidate</script>', history_match: oldMatch };
  const service = ui(async (path, options) => {
    if (options?.method) { writes.push({ path, ...options }); return {}; }
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([workbenchRole]);
    if (path.startsWith('/api/admin/workbench/candidates?')) return workbenchResult([person]);
    assert.equal(path, '/api/admin/workbench/candidates/person-one?job_id=role-one');
    return workbenchDetail(person, oldMatch);
  }, { admin: true, search: '?tab=matches&job=role-one&candidate=person-one&candidates_cursor=opaque-second-page', hooks: { go: browserNavigate } });
  const html = await service.matchesPage(true);
  assert.match(html, /&lt;script&gt;candidate&lt;\/script&gt;/);
  assert.doesNotMatch(html, /<script>candidate/);
  assert.match(html, /<h2>Sanitized résumé<\/h2>/);
  assert.match(html, /<details[^>]*data-disclosure="candidate-resume:person-one" open>/);
  assert.match(html, /<\/details><section class="wb-introduction">/, 'introducing remains available when the résumé is folded');
  assert.match(html, /Reintroducing restores profile access and the existing shared room/);
  assert.match(html, /conversation=platform_person-one/);
  const data = new FormData(); data.set('note', 'Ready to reconnect.'); data.set('job_id', 'tampered-role');
  await service.introduceCandidate({ dataset: { user: 'person-one', job: 'role-one' } }, data);
  assert.deepEqual(writes, [{ path: '/api/admin/matches/old-introduction', method: 'PATCH', body: { status: 'introduced', note: 'Ready to reconnect.' } }]);
  assert.equal(new URLSearchParams(globalThis.location.search).has('candidate'), false);
  assert.equal(new URLSearchParams(globalThis.location.search).has('candidates_cursor'), false);
  assert.equal(new URLSearchParams(globalThis.location.search).get('job'), 'role-one');
});

test('opaque cursor navigation supports previous pages and search resets pagination', async () => {
  const requests = [];
  const service = ui(async path => {
    requests.push(path);
    const cursor = new URL(path, 'https://hireferry.test').searchParams.get('cursor');
    return { ...workbenchResult([workbenchRole], cursor ? null : 'opaque+second/page'), total: 30 };
  }, { admin: true, search: '?tab=matches&job_q=Engineer', hooks: { go: browserNavigate } });
  await service.matchesPage(true);
  await service.workbenchPage('jobs', 'next');
  const second = await service.matchesPage(true);
  assert.equal(new URL(requests.at(-1), 'https://hireferry.test').searchParams.get('cursor'), 'opaque+second/page');
  assert.match(second, /data-direction="previous"[^>]*aria-label="Previous page" >|data-direction="previous"[^>]*aria-label="Previous page"\s*>/);
  await service.workbenchPage('jobs', 'previous');
  await service.matchesPage(true);
  assert.equal(new URL(requests.at(-1), 'https://hireferry.test').searchParams.has('cursor'), false);
  await service.workbenchPage('jobs', 'next');
  const form = new FormData(); form.set('q', 'Designer');
  await service.searchWorkbench({ dataset: { form: 'workbench-jobs' } }, form);
  const params = new URLSearchParams(globalThis.location.search);
  assert.equal(params.get('job_q'), 'Designer');
  assert.equal(params.has('jobs_cursor'), false);
  assert.equal(params.has('candidate'), false);
});

test('out-of-page selected roles load a single summary rather than scanning the role directory', async () => {
  const paths = [];
  const service = ui(async path => {
    paths.push(path);
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([]);
    if (path === '/api/admin/workbench/jobs/role-one') return workbenchRole;
    if (path.startsWith('/api/admin/workbench/candidates?')) return workbenchResult([]);
    throw new Error('Unexpected broad request: ' + path);
  }, { admin: true, search: '?tab=matches&job=role-one' });
  const html = await service.matchesPage(true);
  assert.match(html, /Rust Engineer/);
  assert.equal(paths.length, 3);
  assert.equal(paths.filter(path => path.startsWith('/api/admin/workbench/jobs?')).length, 1);
});

test('follow-ups use server stage filters and a single detail endpoint for private versus shared chat links', async () => {
  const paths = [];
  let dialog = '';
  const record = { id: 'active', seeker_id: 's1', seeker_name: 'Person A', employer_id: 'e1', employer_name: 'Team A', job_id: 'j1', job_title: 'Engineer', status: 'interview', note: 'Shared note', updated_at: 1700000000, conversation_id: 'match_active', seeker_conversation_id: 'platform_s1', employer_conversation_id: 'platform_e1' };
  const service = ui(async path => {
    paths.push(path);
    if (path === '/api/admin/workbench/matches/active') return record;
    const url = new URL(path, 'https://hireferry.test');
    assert.equal(url.pathname, '/api/admin/workbench/matches');
    assert.equal(url.searchParams.get('status'), 'interview');
    assert.equal(url.searchParams.get('q'), 'Engineer');
    assert.equal(url.searchParams.get('limit'), '25');
    return { ...workbenchResult([record]), stage_counts: { introduced: 12, interview: 3, hired: 4, declined: 1 } };
  }, { admin: true, search: '?tab=matches&view=followups&status=interview&follow_q=Engineer', hooks: { openDialog: html => { dialog = html; }, go: browserNavigate } });
  const html = await service.matchesPage(true);
  assert.match(html, /Last update/);
  assert.match(html, /All<span>20<\/span>/);
  assert.match(html, /conversation=match_active/);
  await service.matchDetail('active');
  for (const target of ['match_active', 'platform_s1', 'platform_e1']) assert.match(dialog, new RegExp(`conversation=${target}`));
  assert.equal(paths.length, 2);
  assert.doesNotMatch(paths.join('\n'), /\/api\/admin\/matches(?:\?|$)|\/api\/admin\/users|\/api\/admin\/jobs/);
  globalThis.location.search += '&matches_cursor=stale-page';
  await service.savedFollowup();
  assert.equal(new URLSearchParams(globalThis.location.search).has('matches_cursor'), false);
});

test('late candidate responses cannot replace the selection or introduce someone to a different role', async () => {
  let releaseOld;
  let startedOld;
  const waiting = new Promise(resolve => { startedOld = resolve; });
  const writes = [];
  const jobB = { ...workbenchRole, id: 'role-two', title: 'Designer' };
  const personB = { ...workbenchPerson, id: 'person-two', name: 'Person B' };
  const service = ui(async (path, options) => {
    if (options?.method) { writes.push({ path, ...options }); return {}; }
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([workbenchRole, jobB]);
    if (path.startsWith('/api/admin/workbench/candidates?')) {
      const job = new URL(path, 'https://hireferry.test').searchParams.get('job_id');
      if (job === 'role-one') return new Promise(resolve => { releaseOld = resolve; startedOld(); });
      return workbenchResult([personB]);
    }
    assert.equal(path, '/api/admin/workbench/candidates/person-two?job_id=role-two');
    return workbenchDetail(personB);
  }, { admin: true, search: '?tab=matches&job=role-one&candidate=person-one', hooks: { go: browserNavigate } });
  const oldRender = service.matchesPage(true);
  await waiting;
  globalThis.location.search = '?tab=matches&job=role-two&candidate=person-two';
  const newHtml = await service.matchesPage(true);
  assert.match(newHtml, /Person B/);
  releaseOld(workbenchResult([workbenchPerson]));
  await oldRender;
  const form = { dataset: { user: 'person-two', job: 'role-two' } };
  const data = new FormData(); data.set('note', 'Reviewed together.');
  globalThis.location.search = '?tab=matches&job=role-one&candidate=person-one';
  await assert.rejects(() => service.introduceCandidate(form, data), /selection changed/i);
  assert.equal(writes.length, 0);
  globalThis.location.search = '?tab=matches&job=role-two&candidate=person-two';
  await service.introduceCandidate(form, data);
  assert.deepEqual(writes, [{ path: '/api/admin/matches', method: 'POST', body: { seeker_id: 'person-two', job_id: 'role-two', note: 'Reviewed together.' } }]);
});

test('workbench labels and candidate confirmation are available in Chinese without changing private user introductions', async () => {
  const service = ui(async path => path.startsWith('/api/admin/workbench/jobs?') ? workbenchResult([workbenchRole]) : path.startsWith('/api/admin/workbench/candidates?') ? workbenchResult([workbenchPerson]) : workbenchDetail(), { admin: true, search: '?tab=matches&job=role-one&candidate=person-one', state: { lang: 'zh' } });
  const html = await service.matchesPage(true);
  assert.match(html,/data-page-key="candidate:role-one:person-one"/);
  for (const label of ['匹配工作台', '筛选候选人', '匹配跟进', '招聘中的岗位', '介绍说明 · 双方可见', '平台私聊历史不会共享', '月薪最低（CNY）']) { assert.match(html, new RegExp(label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')));
  }
  assert.doesNotMatch(html, /Find candidates|Introduction note|Role \/ employer/);
  const userPaths = [];
  const user = ui(async path => { userPaths.push(path); return { items: [{ id: 'm1', seeker_id: 's1', seeker_name: 'A', employer_id: 'e1', employer_name: 'E', job_id: 'j1', job_title: 'Role', status: 'introduced' }] }; });
  const userHtml = await user.matchesPage();
  assert.deepEqual(userPaths, ['/api/matches']);
  assert.match(userHtml, /conversation=match_m1/);
  assert.doesNotMatch(userHtml, /platform_s1|platform_e1|wb-workspace/);
});

function webmcp({ eligible = true, role = 'seeker', admin = false, search = '', order = null, responses = {}, serviceUIFactory = () => ({}), legalRenderer = (content,lang) => `<main data-legal-language="${lang}">${content.html}</main>` } = {}) {
  const registered = new Map();
  const requests = [];
  const listeners = new Map();
  const modelContext = { registerTool: tool => registered.set(tool.name, tool), unregisterTool: name => registered.delete(name) };
  // These tests inspect business schemas and handlers synchronously. The actual
  // async registration lifecycle has its own unit suite and real Chrome gate.
  const createWebMCPOwner = () => {
    let currentScope = null;
    return {
      sync({ scope, tools }) { registered.clear();currentScope = scope;for (const tool of tools) registered.set(tool.name, tool); },
      clear() { currentScope = null;registered.clear(); },
      isCurrent: scope => scope === currentScope,
    };
  };
  const node = { innerHTML: '', querySelector() { return null; }, replaceChildren() { this.innerHTML = ''; } };
  const document = { querySelector: () => node, addEventListener(type, handler) { if (!listeners.has(type)) listeners.set(type, new Set()); listeners.get(type).add(handler); }, removeEventListener(type, handler) { listeners.get(type)?.delete(handler); }, body: { style: {} }, documentElement: {}, modelContext };
  const fetch = async (path, options) => {
    requests.push({ path, options });
    const result = Object.hasOwn(responses, path) ? responses[path] : path === '/api/ai/eligibility' ? { eligible } : path.startsWith('/api/orders/') ? order : { ok: true };
    const status = result?.httpStatus || 200;
    return { ok: status >= 200 && status < 300, status, json: async () => result?.httpStatus ? result.body : result };
  };
  const storage = new Map();
  const sessionStorage = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  const localValues = new Map();
  const localStorage = { getItem: key => localValues.get(key) || null, setItem: (key, value) => localValues.set(key, value), removeItem: key => localValues.delete(key) };
  const publicPageHelpers = { ...browserCacheHelpers, ...cliMotionHelpers, ...contentRefreshHelpers, createPublicConfigCache: load => browserCacheHelpers.createPublicConfigCache(load, { storage: localStorage }) };
  const browserLocation = { pathname: '/app', search, origin: 'https://hireferry.test' };
  const app = new Function('document', 'navigator', 'window', 'localStorage', 'location', 'history', 'fetch', 'createServiceUI', 'createMessageId', 'sessionStorage', 'renderLegal', 'createDisclosureUI', 'createPromoUI', 'createWebMCPOwner', ...Object.keys(moneyHelpers), ...Object.keys(requestHelpers), ...Object.keys(billingHelpers), ...Object.keys(teamHelpers), ...Object.keys(cliGuideHelpers), ...Object.keys(publicPageHelpers), ...Object.keys(workspaceHelpers), appSource + '\nreturn {state,shell,registerWebMCP,orderRefundDetails,statusText,requestErrorText,paymentReturn,refundsPage,contactPage,legal,footer,acquisitionSource,marketingHref,isGeneratedHandle,defaultRoleReturn,shouldAutoActivateEmployer,safeBillingReturn,safeJobReturn,rememberBillingReturn,afterSignIn,profilePage,headcountInput,newJobInput,saveJobHeadcount,headcountControl,jobBadge,myJobs,jobEditor,jobDeletionPrompt,changeJobLifecycle,payOrder,header,home,pricingSection,feeSettings,feeRule,currency,money,centsInput,adminOverview,orderTable,ordersPage,agentsPage,employerActivation,refreshJobPrice,paymentProviders,gatewayInputs,syncCurrencyFields,activationOptions,refreshActivationPayment,jobToolsLandingPage,openDialog,closeDialog};')(
    document, { modelContext }, { addEventListener() {}, getComputedStyle: node => ({ visibility: node.visibility || 'visible' }) }, localStorage, browserLocation, {}, fetch, serviceUIFactory, createMessageId, sessionStorage, legalRenderer, createDisclosureUI, createPromoUI, createWebMCPOwner, ...Object.values(moneyHelpers), ...Object.values(requestHelpers), ...Object.values(billingHelpers), ...Object.values(teamHelpers), ...Object.values(cliGuideHelpers), ...Object.values(publicPageHelpers), ...Object.values(workspaceHelpers),
  );
  Object.assign(app.state, { config:structuredClone(defaultConfig), user: admin ? null : { id: 'one', role, employer_paid: true }, admin, csrf: 'synthetic-csrf', eligibility: { eligible } });
  app.registerWebMCP();
  return { app, registered, requests, document, dialogNode: node, listeners, location: browserLocation, setEligible(value) { eligible = value; } };
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

test('WebMCP chat capabilities are shared by eligible identities and recheck eligibility on execution', async () => {
  const capabilities = {
    formats: ['plain', 'markdown', 'openui'],
    openui: { library: 'hireferry@1', spec_url: '/static/chat-openui.json' },
    welcome: { platform: { en: { body: 'Welcome **aboard**', prompts: ['Ask a question'] } } },
  };
  for (const identity of [{ role: 'seeker' }, { role: 'employer' }, { admin: true }]) {
    const harness = webmcp({ ...identity, responses: { '/api/chat/capabilities': capabilities } });
    const tool = harness.registered.get('hireferry_get_chat_capabilities');
    assert.ok(tool);
    assert.equal(tool.annotations.readOnlyHint, true);
    assert.deepEqual(tool.inputSchema, { type: 'object', properties: {}, required: [], additionalProperties: false });
    const result = await tool.execute({});
    assert.equal(result.isError, undefined);
    assert.deepEqual(JSON.parse(result.content[0].text), capabilities);
    assert.deepEqual(harness.requests.map(request => request.path), ['/api/ai/eligibility', '/api/chat/capabilities']);
    assert.ok(harness.requests.every(request => request.options.method === 'GET'));
    assert.equal(harness.registered.has('hireferry_admin_get_chat_capabilities'), false);

    harness.setEligible(false);
    const requestCount = harness.requests.length;
    assert.equal((await tool.execute({})).isError, true);
    assert.deepEqual(harness.requests.slice(requestCount).map(request => request.path), ['/api/ai/eligibility']);
  }
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

test('WebMCP send supports each explicit format and retains the legacy omitted format and generated client ID', async () => {
  for (const admin of [false, true]) {
    const harness = webmcp({ admin });
    const tool = harness.registered.get(`${admin ? 'hireferry_admin_' : 'hireferry_'}send_message`);
    assert.deepEqual(tool.inputSchema.properties.format.enum, ['plain', 'markdown', 'openui']);
    assert.equal(tool.inputSchema.required.includes('format'), false);
    assert.equal(tool.inputSchema.required.includes('client_id'), false);
    for (const format of ['plain', 'markdown', 'openui', undefined]) {
      const input = { conversation_id: 'room/one?after=999#end', body: 'Hello **world**' };
      if (format !== undefined) Object.assign(input, { format, client_id: `synthetic-${format}` });
      const result = await tool.execute(input);
      assert.equal(result.isError, undefined);
      const sent = harness.requests.at(-1);
      assert.equal(sent.path, '/api/chat/conversations/room%2Fone%3Fafter%3D999%23end/messages');
      assert.equal(sent.options.method, 'POST');
      assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
      assert.equal(sent.options.headers['X-CSRF-Token'], 'synthetic-csrf');
      const body = JSON.parse(sent.options.body);
      if (format === undefined) {
        assert.match(body.client_id, /^[0-9a-f]{32}$/);
        assert.deepEqual(body, { body: input.body, client_id: body.client_id });
      } else {
        assert.deepEqual(body, { body: input.body, client_id: input.client_id, format });
      }
    }
  }
});

test('WebMCP stream tools encode the room route and preserve sequences, receipts and AI attribution', async () => {
  const conversation_id = 'room/one?after=999#end';
  const base = '/api/chat/conversations/room%2Fone%3Fafter%3D999%23end/streams';
  const operations = [
    { name: 'begin_message', path: base, input: { conversation_id, client_id: 'synthetic-stream', body: '**First', format: 'markdown' }, body: { client_id: 'synthetic-stream', body: '**First', format: 'markdown' }, receipt: { id: 42, conversation_id, sequence: 0, status: 'streaming', body: '**First', event_cursor: 100 } },
    { name: 'append_message', path: `${base}/42/append`, input: { conversation_id, message_id: 42, sequence: 1, delta: ' chunk**' }, body: { sequence: 1, delta: ' chunk**' }, receipt: { id: 42, conversation_id, sequence: 1, status: 'streaming', event_cursor: 101 } },
    { name: 'finish_message', path: `${base}/42/finish`, input: { conversation_id, message_id: 42, sequence: 2 }, body: { sequence: 2 }, receipt: { id: 42, conversation_id, sequence: 2, status: 'complete', event_cursor: 102 } },
    { name: 'cancel_message', path: `${base}/43/cancel`, input: { conversation_id, message_id: 43, sequence: 3 }, body: { sequence: 3 }, receipt: { id: 43, conversation_id, sequence: 3, status: 'cancelled', event_cursor: 103 } },
  ];
  for (const admin of [false, true]) {
    const harness = webmcp({ admin, responses: Object.fromEntries(operations.map(operation => [operation.path, operation.receipt])) });
    for (const operation of operations) {
      const tool = harness.registered.get(`${admin ? 'hireferry_admin_' : 'hireferry_'}${operation.name}`);
      assert.ok(tool);
      assert.equal(tool.annotations.readOnlyHint, false);
      assert.equal(tool.inputSchema.additionalProperties, false);
      const before = harness.requests.length;
      const result = await tool.execute(operation.input);
      assert.equal(result.isError, undefined);
      assert.deepEqual(JSON.parse(result.content[0].text), operation.receipt);
      assert.deepEqual(harness.requests.slice(before).map(request => request.path), ['/api/ai/eligibility', operation.path]);
      const sent = harness.requests.at(-1);
      assert.equal(sent.options.method, 'POST');
      assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
      assert.equal(sent.options.headers['X-CSRF-Token'], 'synthetic-csrf');
      assert.deepEqual(JSON.parse(sent.options.body), operation.body);
    }
  }
});

test('WebMCP conversation and event reads preserve room scoping and distinct zero or saved event cursors', async () => {
  const conversation_id = 'room/one?after=999#end';
  for (const admin of [false, true]) {
    const harness = webmcp({ admin });
    const prefix = admin ? 'hireferry_admin_' : 'hireferry_';
    const conversation = harness.registered.get(prefix + 'get_conversation');
    assert.ok(conversation);
    assert.equal(conversation.annotations.readOnlyHint, true);
    assert.equal((await conversation.execute({ conversation_id })).isError, undefined);
    const read = harness.requests.at(-1);
    assert.equal(read.path, '/api/chat/conversations/room%2Fone%3Fafter%3D999%23end');
    assert.equal(read.options.method, 'GET');
    assert.equal(read.options.headers['X-HireFerry-AI'], 'webmcp');

    const events = harness.registered.get(prefix + 'get_chat_events');
    assert.ok(events);
    assert.equal(events.annotations.readOnlyHint, true);
    assert.deepEqual(events.inputSchema.required, ['after']);
    assert.equal(events.inputSchema.properties.after.minimum, 0);
    assert.equal(events.inputSchema.properties.limit.maximum, 100);
    for (const input of [{ conversation_id, after: 12345, limit: 7 }, { conversation_id, after: 0 }, { after: 0 }]) {
      const before = harness.requests.length;
      const result = await events.execute(input);
      assert.equal(result.isError, undefined);
      const sent = harness.requests.at(-1);
      assert.equal(harness.requests[before].path, '/api/ai/eligibility');
      const url = new URL(sent.path, 'https://hireferry.test');
      assert.equal(url.pathname, '/api/chat/events');
      assert.equal(url.hash, '');
      assert.deepEqual([...url.searchParams], Object.entries(input).map(([key, value]) => [key, String(value)]));
      assert.equal(sent.options.method, 'GET');
      assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
      assert.equal(sent.options.body, undefined);
    }
  }
});

test('administrator WebMCP exposes business operations without token or passkey management', () => {
  const { registered } = webmcp({ admin: true });
  for (const name of ['hireferry_admin_list_users', 'hireferry_admin_create_introduction', 'hireferry_admin_record_outreach', 'hireferry_admin_send_message']) assert.ok(registered.has(name));
  assert.equal([...registered.keys()].some(name => /token|passkey|admin_config|webhook/.test(name)), false);
  assert.equal(registered.has('hireferry_update_profile'), false);
});

const endpoint = { id: 'endpoint-one', name: 'My workflow', url: 'https://receiver.example/hooks/hireferry', events: ['message.created', 'match.updated'], enabled: true, created_at: 1 };

test('webhook pages load only the user’s endpoints and latest delivery without automatic registration', async () => {
  const requests = [];
  const service = ui(async (path, options) => {
    requests.push({ path, options });
    if (path === '/api/webhooks') return { enabled: true, limits: { max_endpoints: 5, max_attempts: 5 }, items: [{ ...endpoint, name: '<script>receiver</script>' }] };
    assert.equal(path, '/api/webhooks/endpoint-one/deliveries?limit=1');
    return { items: [{ id: 'delivery-one', event_type: 'message.created', status: 'delivered', created_at: 1 }] };
  });
  const html = await service.webhooksPage();
  assert.match(html, /&lt;script&gt;receiver&lt;\/script&gt;/);
  assert.match(html, /Delivered/);
  assert.match(html, /do not contain résumés, contact emails, or message bodies/);
  assert.match(html, /deduplicate using your receiver ID plus body\.id from the verified payload/);
  const guide = html.slice(html.indexOf('<section class="panel webhook-guide">'));
  assert.match(guide, /<\/p><\/div><\/div><details[^>]*data-disclosure="webhook-guide:configured"\s*>/, 'privacy scope remains visible while the setup guide starts folded');
  assert.match(guide, /class="disclosure-content"><div class="webhook-principles">/, 'the explanation grid stays outside the flex header');
  assert.ok(requests.every(request => request.options === undefined));
  assert.equal(requests.length, 2);
});

test('webhook subscriptions show role-appropriate events and work in Chinese', async () => {
  for (const role of ['seeker', 'employer']) {
    let dialog = '';
    const service = ui(async () => ({ enabled: true, items: [] }), { state: { lang: 'zh', user: { id: 'one', role } }, hooks: { openDialog: html => { dialog = html; } } });
    const html = await service.webhooksPage();
    service.webhookDialog();
    assert.match(html, /我的接收端点/);
    assert.match(html, /<details[^>]*data-disclosure="webhook-guide:new" open>/, 'the first receiver setup keeps its instructions available');
    assert.match(dialog, /订阅事件/);
    assert.match(dialog, /只通知你本人有权访问的对象/);
    for (const event of ['message.created', 'match.updated', 'job.updated', 'order.updated']) assert.ok(dialog.includes(`value="${event}"`));
    assert.equal(dialog.includes('value="mandate.updated"'), role === 'seeker');
    assert.equal(dialog.includes('value="outreach.updated"'), role === 'seeker');
  }
});

test('webhook creation returns its secret only in a removable dialog and rejects insecure destinations', async () => {
  let dialog = '', page = '', created = false;
  const requests = [];
  const secret = 'synthetic-webhook-secret-for-test';
  let service;
  service = ui(async (path, options) => {
    requests.push({ path, options });
    if (options?.method === 'POST') { created = true; return { endpoint, signing_secret: secret }; }
    if (path === '/api/webhooks') return { enabled: true, items: created ? [endpoint] : [] };
    return { items: [] };
  }, { hooks: { openDialog: html => { dialog = html; }, closeDialog: () => { dialog = ''; }, render: async () => { page = await service.webhooksPage(); } } });
  const data = new FormData();
  data.set('name', 'My workflow'); data.set('url', 'http://receiver.example/hooks'); data.append('events', 'message.created');
  await assert.rejects(service.saveWebhook({ dataset: {} }, data), /HTTPS on port 443/);
  assert.equal(requests.length, 0);
  data.set('url', endpoint.url);
  await service.saveWebhook({ dataset: {} }, data);
  const write = requests.find(request => request.options?.method === 'POST');
  assert.deepEqual(write.options.body, { name: 'My workflow', events: ['message.created'], url: endpoint.url });
  assert.equal(write.options.headers, undefined, 'manual endpoint management must not carry the WebMCP AI header');
  assert.equal(dialog.split(secret).length - 1, 1);
  assert.match(dialog, /id="webhook-signing-secret"/);
  assert.doesNotMatch(dialog, /data-copy|value="synthetic/);
  assert.equal(page.includes(secret), false);
  // Reopening ordinary endpoint editing cannot recover the one-time secret.
  service.webhookDialog(endpoint.id);
  assert.equal(dialog.includes(secret), false);
  assert.doesNotMatch(appSource.slice(appSource.indexOf('function registerWebMCP')), /\/api\/webhooks/);
});

test('delivery history localizes failures and only failed deliveries have retry controls', async () => {
  let dialog = '';
  const deliveries = ['pending', 'processing', 'delivered', 'failed', 'cancelled'].map((status, i) => ({ id: 'delivery-' + i, event_id: 'event-' + i, event_type: 'message.created', status, attempts: 2, max_attempts: 5, created_at: 1, last_error: status === 'failed' ? 'timeout' : null }));
  const service = ui(async path => path === '/api/webhooks' ? { enabled: true, items: [endpoint] } : { items: deliveries }, { state: { lang: 'zh' }, hooks: { openDialog: html => { dialog = html; } } });
  await service.webhooksPage();
  await service.webhookDeliveries(endpoint.id);
  assert.match(dialog, /接收服务响应超时/);
  assert.doesNotMatch(dialog, />timeout</);
  assert.equal((dialog.match(/data-action="webhook-retry"/g) || []).length, 1);
  assert.match(dialog, /data-delivery="delivery-3"/);
  assert.match(dialog, /<details[^>]*data-disclosure="webhook-delivery:endpoint-one:delivery-3">/);
  assert.match(dialog, /<\/details><p class="webhook-warning">接收服务响应超时/, 'the failure is visible without opening technical metadata');
  assert.match(dialog, /已送达/);
  assert.match(dialog, /已取消/);
});

test('disabled delivery retains history and deletion while disabling create, tests, and failed retries', async () => {
  let dialog = '';
  const service = ui(async path => path === '/api/webhooks' ? { enabled: false, items: [endpoint] } : { items: [{ id: 'failed-one', event_id: 'event-one', event_type: 'match.updated', status: 'failed', attempts: 5 }] }, { hooks: { openDialog: html => { dialog = html; } } });
  const html = await service.webhooksPage();
  assert.match(html, /data-action="webhook-new" disabled/);
  assert.match(html, /data-action="webhook-test"[^>]*disabled/);
  assert.doesNotMatch(html.match(/<button[^>]*data-action="webhook-delete"[^>]*>/)[0], /disabled/);
  assert.doesNotMatch(html.match(/<button[^>]*data-action="webhook-toggle"[^>]*>/)[0], /disabled/);
  await service.webhookDeliveries(endpoint.id);
  assert.match(dialog, /data-action="webhook-retry"[^>]*disabled/);
});

test('dialog Tab order reaches delivery summaries with disabled actions and excludes unfocusable elements', () => {
  const { app, document, dialogNode, listeners } = webmcp();
  const control = (tag, extra = {}) => ({
    tag, tabIndex: 0, disabled: false, offsetParent: {},
    matches(selector) { return selector === ':disabled' && (this.disabled || this.disabledByFieldset); },
    closest() { return this.hiddenAncestor || this.inertAncestor ? {} : null; },
    getClientRects() { return this.rendered === false ? [] : [{}]; },
    focus() { document.activeElement = this; },
    ...extra,
  });
  const close = control('button'), refresh = control('button'), summary = control('summary');
  const controls = [close, refresh, summary, control('button', { disabled: true })];
  dialogNode.querySelector = () => dialogNode.innerHTML ? {} : null;
  dialogNode.querySelectorAll = selector => controls.filter(node => selector.split(',').some(part => part === node.tag || (part === '[tabindex]' && node.explicitTabIndex)));
  app.openDialog('<h2 id="dialog-title">Delivery history</h2>');
  assert.equal(document.activeElement, close);
  const tab = (active, shiftKey = false) => {
    document.activeElement = active;
    let prevented = false;
    for (const handler of listeners.get('keydown')) handler({ key: 'Tab', shiftKey, preventDefault() { prevented = true; } });
    return prevented;
  };
  assert.equal(tab(refresh), false, 'Tab from Refresh must proceed to the native summary, not wrap to Close');
  assert.equal(tab(summary), true);
  assert.equal(document.activeElement, close);
  assert.equal(tab(close, true), true);
  assert.equal(document.activeElement, summary);

  const custom = control('div', { explicitTabIndex: true, offsetParent: null });
  controls.push(custom,
    control('button', { tabIndex: -1, explicitTabIndex: true }),
    control('button', { hiddenAncestor: true }),
    control('button', { inertAncestor: true }),
    control('button', { visibility: 'hidden' }),
    control('button', { rendered: false }),
    control('button', { disabledByFieldset: true }));
  assert.equal(tab(summary), false, 'a visible custom tabindex=0 control remains reachable');
  assert.equal(tab(custom), true, 'hidden, inert, disabled and negative-tabindex controls are outside the cycle');
  assert.equal(document.activeElement, close);
  assert.equal(tab(close, true), true);
  assert.equal(document.activeElement, custom);
  app.closeDialog();
  assert.equal(listeners.get('keydown').size, 0);
});

const expiredActivation = { id: 'activation-old', kind: 'onboarding', status: 'expired', provider: 'waffo', amount_cents: 10000, refunded_cents: 0, currency: 'CNY', created_at: 1, checkout_expires_at: 2, checkout_url: 'https://checkout.example/expired' };

test('inactive employers can restart an expired activation from Orders without expanding history', async () => {
  const { app, requests } = webmcp({ eligible: false, role: 'employer', responses: { '/api/orders': { items: [expiredActivation] } } });
  app.state.lang = 'en'; app.state.user.employer_paid = false;
  const html = await app.ordersPage();
  const overview = html.match(/<section class="activation-overview">[\s\S]*?<\/section>/)?.[0];
  assert.ok(overview);
  assert.match(overview, /Order expired/);
  assert.match(overview, /data-action="role-employer"/);
  assert.match(overview, /Restart activation/);
  assert.doesNotMatch(overview, /<details/);
  assert.doesNotMatch(html, /data-action="pay-order"/);
  assert.match(html, /data-disclosure="order-activation-old"/);
  assert.deepEqual(requests.map(request => [request.path, request.options?.method || 'GET']), [['/api/orders', 'GET']]);
});

test('activation stays reachable when there are no orders and when posting is blocked by an inactive account', async () => {
  const { app } = webmcp({ eligible: false, role: 'employer', responses: { '/api/orders': { items: [] } } });
  app.state.lang = 'en'; app.state.user.employer_paid = false;
  const orders = await app.ordersPage();
  assert.match(orders, /Activate employer account/);
  assert.match(orders, /No orders yet/);
  assert.match(orders, /<button[^>]*data-action="role-employer"/);
  const editor = await app.jobEditor();
  assert.match(editor, /Your employer account is not active/);
  assert.match(editor, /<button[^>]*data-action="role-employer"/);
});

test('an expired payment return offers recovery using only the account-authorized order read', async () => {
  const { app, requests } = webmcp({ eligible: false, role: 'employer', search: '?order_id=activation-old&status=paid', order: { ...expiredActivation, status: 'pending' } });
  app.state.lang = 'en'; app.state.user.employer_paid = false;
  const html = await app.paymentReturn();
  assert.match(html, /This order has expired/);
  assert.match(html, /data-action="role-employer"/);
  assert.match(html, /Restart activation/);
  assert.doesNotMatch(html, /data-action="pay-order"|Payment confirmed\./);
  assert.deepEqual(requests.map(request => [request.path, request.options?.method || 'GET']), [['/api/orders/activation-old', 'GET']]);
});

test('active employers are not offered payment or reactivation for leftover activation orders', async () => {
  const leftover = { ...expiredActivation, id: 'activation-pending', status: 'pending', checkout_expires_at: Math.floor(Date.now() / 1000) + 3600 };
  const { app } = webmcp({ role: 'employer', search: '?order_id=activation-old', order: expiredActivation, responses: { '/api/orders': { items: [leftover, expiredActivation] } } });
  app.state.lang = 'en';
  const orders = await app.ordersPage();
  assert.match(orders, /Your employer account is active/);
  assert.match(orders, /Manage roles/);
  assert.doesNotMatch(orders, /data-action="pay-order"|data-action="role-employer"/);
  assert.doesNotMatch(await app.paymentReturn(), /data-action="pay-order"|data-action="role-employer"/);
});

// Signed chart geometry and single-day refunds are covered in finance-ui.test.mjs.

test('refund amounts and webhook management errors are localized without losing amounts', () => {
  const { app } = webmcp();
  assert.equal(app.statusText('partially_refunded'), '部分退款');
  assert.match(app.orderRefundDetails({ amount_cents: 60000, refunded_cents: 15000 }), /当前净额.*CNY\s*450\.00/);
  assert.match(app.requestErrorText('Wait one minute between webhook tests'), /等待一分钟/);
  app.state.lang = 'en';
  assert.equal(app.statusText('partially_refunded'), 'Partially refunded');
  assert.match(app.orderRefundDetails({ amount_cents: 60000, refunded_cents: 60000 }), /Net amount.*CNY\s*0\.00/);
  assert.match(app.requestErrorText('Webhook endpoint limit reached'), /endpoint limit/);
});

test('checkout return ignores claimed success and renders only the signed-in account order read', async () => {
  const order = { id: 'order_one', kind: 'publish', status: 'pending', amount_cents: 60000, refunded_cents: 0, provider: 'waffo', live_mode: false, created_at: 1 };
  const { app, requests } = webmcp({ search: '?order_id=order_one&status=paid&success=1&canceled=1&provider=stripe', order });
  app.state.lang = 'en';
  const html = await app.paymentReturn();
  assert.match(html, /<h1>Waiting for payment confirmation\.<\/h1>/);
  assert.doesNotMatch(html, /<h1>Payment confirmed\.<\/h1>/);
  assert.match(html, /Waffo Pancake/);
  assert.match(html, /Refresh order status/);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].path, '/api/orders/order_one');
  assert.equal(requests[0].options.method, 'GET');
  assert.equal(requests[0].options.body, undefined);
});

test('checkout return requires sign-in before reading any order and uses an allowlisted return target', async () => {
  const { app, requests } = webmcp({ search: '?order_id=order_one&status=paid' });
  app.state.user = null;
  const html = await app.paymentReturn();
  assert.match(html, /先登录，再查看订单/);
  assert.match(html, /return_to=%2Fbilling%2Freturn%3Forder_id%3Dorder_one/);
  assert.equal(requests.length, 0);
  assert.equal(app.safeBillingReturn('https://outside.example/billing/return?order_id=order_one'), '');
  assert.equal(app.safeBillingReturn('/admin'), '');
  assert.equal(app.safeBillingReturn('/billing/return?order_id=order_one&status=paid&success=1'), '/billing/return?order_id=order_one');
});

test('Google sign-in can retain only a safe checkout return and consumes it once', () => {
  const { app, location } = webmcp({ search: '?return_to=%2Fbilling%2Freturn%3Forder_id%3Dorder_one%26status%3Dpaid' });
  app.rememberBillingReturn();
  location.search = '';
  assert.equal(app.afterSignIn(), '/billing/return?order_id=order_one');
  assert.equal(app.afterSignIn(), '/app');
  assert.equal(app.safeBillingReturn('/billing/return?order_id=%3Cscript%3E'), '');
});

test('employer and seeker return targets survive role selection without allowing open redirects', () => {
  const employer = webmcp({ search: '?role=employer&return_to=%2Fapp%2Fjobs%2Fnew' });
  assert.equal(employer.app.safeJobReturn('/app/jobs/new'), '/app/jobs/new');
  assert.equal(employer.app.safeJobReturn('/app/jobs/new?template=ai_uiux'), '/app/jobs/new?template=ai_uiux');
  assert.equal(employer.app.safeJobReturn('/app/jobs/new?template=opensource'), '/app/jobs/new?template=opensource');
  assert.equal(employer.app.safeJobReturn('/app/jobs/new?template=evil'), '');
  assert.equal(employer.app.safeJobReturn('https://evil.example/app/jobs/new'), '');
  employer.app.rememberBillingReturn();
  employer.location.search = '';
  assert.equal(employer.app.afterSignIn(), '/app/jobs/new');
  assert.equal(employer.app.afterSignIn(), '/app');

  const seeker = webmcp({ search: '?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile' });
  seeker.app.rememberBillingReturn();
  seeker.location.search = '';
  assert.equal(seeker.app.afterSignIn(), '/app?tab=profile');
  assert.equal(seeker.app.afterSignIn(), '/app');
});

test('signup attribution keeps medium and content for growth funnel analysis', () => {
  const tagged = webmcp({ search: '?utm_source=x&utm_medium=web&utm_campaign=cn_jobseeker_oct24&utm_content=reply_veracat' });
  assert.equal(tagged.app.acquisitionSource(), 'x:web:cn_jobseeker_oct24:reply_veracat');
});

test('landing-page CTAs preserve inbound attribution through the login hop', () => {
  const tagged = webmcp({ search: '?utm_source=x&utm_medium=reply&utm_campaign=cn_job_recruiter_oct24&utm_content=gmwallet&ref=hf_abc123' });
  const href = new URL(tagged.app.marketingHref('/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew', {
    utm_source: 'employer_landing', utm_medium: 'cta', utm_campaign: 'employer_cta', utm_content: 'hero',
  }), 'https://hireferry.test');
  assert.equal(href.searchParams.get('role'), 'employer');
  assert.equal(href.searchParams.get('return_to'), '/app/jobs/new');
  assert.equal(href.searchParams.get('utm_source'), 'x');
  assert.equal(href.searchParams.get('utm_medium'), 'reply');
  assert.equal(href.searchParams.get('utm_campaign'), 'cn_job_recruiter_oct24');
  assert.equal(href.searchParams.get('utm_content'), 'gmwallet');
  assert.equal(href.searchParams.get('ref'), 'hf_abc123');
  const direct = new URL(webmcp().app.marketingHref('/login?role=employer', { utm_source: 'employer_landing', utm_medium: 'cta' }), 'https://hireferry.test');
  assert.equal(direct.searchParams.get('utm_source'), 'employer_landing');
  assert.equal(direct.searchParams.get('utm_medium'), 'cta');
});

test('local-first job tools landing keeps private applications local and exposes public discovery', () => {
  const { app } = webmcp();
  const html = app.jobToolsLandingPage();
  assert.match(html, /继续保存草稿、投递记录和私密简历/);
  assert.match(html, /公共 MCP/);
  assert.match(html, /for-employers/);
  assert.match(html, /utm_campaign=local_first_integration/);
  assert.match(html, /docs\/mcp-public\.md/);
});

test('generated sign-up handles still show the custom-name onboarding path', () => {
  const { app } = webmcp();
  assert.equal(app.isGeneratedHandle('user_' + 'a'.repeat(16)), true);
  assert.equal(app.isGeneratedHandle('user_' + 'a'.repeat(27)), true);
  assert.equal(app.isGeneratedHandle('lightdot'), false);
  assert.equal(app.isGeneratedHandle('user_not-a-hex-handle'), false);
  assert.match(app.requestErrorText('Claim a custom username before publishing your profile'), /抢注自定义用户名/);
});

test('new role selection lands on the work surface that completes onboarding', () => {
  const { app } = webmcp();
  assert.equal(app.defaultRoleReturn('seeker'), '/app?tab=profile');
  assert.equal(app.defaultRoleReturn('employer'), '/app?tab=jobs');
});

test('an existing seeker with an unclaimed generated handle returns to profile setup', () => {
  const { app } = webmcp({ role: 'seeker' });
  app.state.user.handle = 'user_' + 'a'.repeat(16);
  assert.equal(app.afterSignIn(), '/app?tab=profile');
  app.state.user.handle = 'lightdot';
  assert.equal(app.afterSignIn(), '/app');
});

test('unclaimed seekers see one workspace profile claim banner outside profile and inbox', () => {
  const { app } = webmcp({ role: 'seeker' });
  app.state.user.handle = 'user_' + 'a'.repeat(16);
  const jobs = app.shell('<p>jobs</p>', 'jobs');
  assert.match(jobs, /workspace-profile-claim/);
  assert.match(jobs, /href="\/app\?tab=profile"[^>]*data-nav/);
  assert.match(jobs, /先抢注你的独一无二名字/);
  assert.doesNotMatch(app.shell('<p>profile</p>', 'profile'), /workspace-profile-claim/);
  assert.doesNotMatch(app.shell('<p>messages</p>', 'messages'), /workspace-profile-claim/);
});

test('unclaimed profile setup offers an immediate save-name action beside the username field', async () => {
  const profile = { email: 'seeker@example.test', name: 'A seeker', role: 'seeker', handle: 'user_' + 'a'.repeat(16), public_profile: false, public_email: '', resume_md: '', ai_profile_links: {}, email_notifications: { enabled: false, preferences: { messages: false, matches: false, orders: false, platform: false } } };
  const { app } = webmcp({ responses: { '/api/profile': profile, '/api/auth/passkeys': { passkeys: [] } } });
  const html = await app.profilePage();
  assert.match(html, /<form id="profile-form" data-form="profile">/);
  assert.match(html, /name="handle" form="profile-form" autofocus/);
  assert.match(html, /value="a-seeker"/);
  assert.match(html, /点击保存后才会真正抢注/);
  assert.match(html, /profile-claim-actions/);
  assert.match(html, /保存名字并继续/);
});

test('published profiles offer a copyable README badge', async () => {
  const profile = { email: 'ada@example.test', name: 'Ada', role: 'seeker', handle: 'ada', public_profile: true, public_email: '', resume_md: '## Rust', ai_profile_links: {}, email_notifications: { enabled: false, preferences: { messages: false, matches: false, orders: false, platform: false } } };
  const { app } = webmcp({ responses: { '/api/profile': profile, '/api/auth/passkeys': { passkeys: [] } } });
  const html = await app.profilePage();
  assert.match(html, /README 徽章/);
  assert.match(html, /badge\.svg\?label=ada/);
  assert.match(html, /utm_source=readme_badge/);
  assert.match(html, /utm_campaign=profile_badge/);
  assert.match(html, /copy-profile-badge/);
  assert.match(html, /\[!\[ada AI profile\]/);
});

test('free early access remains available to an existing inactive employer', () => {
  const { app } = webmcp({ role: 'employer' });
  app.state.user.employer_paid = false;
  app.state.config.pricing.onboarding_minor = 0;
  assert.equal(app.shouldAutoActivateEmployer(app.state.user), true);
  app.state.config.pricing.onboarding_minor = 10000;
  assert.equal(app.shouldAutoActivateEmployer(app.state.user), false);
});

test('contact, refund policy, and related notices are bilingual and use configured support details', () => {
  const { app, requests } = webmcp();
  app.state.config = { ...app.state.config, support_email: 'support@lmm.best', public_origin: 'https://hireferry.lmm.best' };
  assert.match(app.contactPage(), /支持邮箱/);
  assert.match(app.refundsPage(), /退款问题，我们认真核对/);
  app.state.lang = 'en';
  for (const html of [app.contactPage(), app.refundsPage()]) assert.match(html, /support@lmm\.best/);
  assert.match(app.contactPage(), /https:\/\/hireferry\.lmm\.best/);
  assert.match(app.refundsPage(), /Partially refunded or Fully refunded/);
  assert.match(app.footer(), /href="\/refunds"[^>]*>Refunds/);
  assert.match(app.footer(), /href="\/contact"[^>]*>Contact/);
  assert.equal(requests.length, 0, 'public information pages must not send messages or initiate payments');
});

const roleFixture = { id: 'job-one', owner_id: 'one', title: 'Engineer', jd_md: 'Build useful things.', salary_cents: 300000, headcount: 1, status: 'published', edits_used: 2, delete_count: 0, version: 3, created_at: 1 };

test('new roles default to one planned hire and validate zero and the exact integer range', async () => {
  const { app } = webmcp({ role: 'employer' });
  app.state.lang = 'en';
  app.state.config = { ...app.state.config, providers: ['stripe'] };
  const data = new FormData();
  for (const [key, value] of Object.entries({ title: 'Engineer', jd_md: 'Build useful things.', salary: '3000', provider: 'stripe' })) data.set(key, value);
  assert.equal(app.newJobInput(data).headcount, 1);
  data.set('headcount', '0');
  assert.deepEqual(app.newJobInput(data), { title: 'Engineer', jd_md: 'Build useful things.', salary_cents: 300000, currency:'CNY', provider: 'stripe', headcount: 0 });
  assert.equal(app.headcountInput('1000000'), 1000000);
  for (const value of ['', null, '-1', '1.5', '1000001', '1e3', 'Infinity']) assert.throws(() => app.headcountInput(value), /integer from 0 to 1,000,000/);
  const html = await app.jobEditor();
  assert.match(html, /name="headcount"[^>]*min="0"[^>]*max="1000000"[^>]*step="1"[^>]*value="1"/);
  assert.match(html, /does not change the publication fee/);
});

test('open-source employer template reduces the blank-role start', async () => {
  const { app } = webmcp({ role: 'employer', search: '?template=opensource' });
  app.state.lang = 'en';
  const html = await app.jobEditor();
  assert.match(html, /Open-source maintainer \/ contributor/);
  assert.match(html, /An open-source role template is loaded/);
  assert.match(html, /public code, issues, documentation, or release history/i);
});

test('free headcount changes send only the target, with no order or paid description request', async () => {
  const { app, requests } = webmcp({ role: 'employer' });
  app.state.lang = 'en';
  await app.saveJobHeadcount(roleFixture.id, '0');
  await app.saveJobHeadcount(roleFixture.id, '12');
  await app.saveJobHeadcount(roleFixture.id, '8', true);
  assert.deepEqual(requests.map(request => [request.path, request.options.method, JSON.parse(request.options.body)]), [
    ['/api/jobs/job-one/headcount', 'PATCH', { headcount: 0 }],
    ['/api/jobs/job-one/headcount', 'PATCH', { headcount: 12 }],
    ['/api/admin/jobs/job-one', 'PATCH', { headcount: 8 }],
  ]);
  assert.ok(requests.every(request => request.options.headers['X-CSRF-Token'] === 'synthetic-csrf'));
  await assert.rejects(app.saveJobHeadcount(roleFixture.id, '0.5'));
  assert.equal(requests.length, 3, 'invalid counts must be rejected before any network mutation');
  assert.equal(app.headcountControl({ ...roleFixture, status: 'closed', headcount: 0 }), '');
  assert.equal(app.headcountControl({ ...roleFixture, status: 'deleted', headcount: 0 }, true), '');
  assert.equal(app.headcountControl({ ...roleFixture, status: 'draft' }, true), '', 'admin draft editing is not supported by its paid-role endpoint');
  assert.match(app.headcountControl({ ...roleFixture, status: 'draft' }), /confirmed publication payment/);
});

test('role management shows zero as paused and keeps free controls separate from paid description edits', async () => {
  const { app, requests } = webmcp({ role: 'employer', search: '?tab=jobs', responses: { '/api/jobs/mine': { items: [
    { ...roleFixture, headcount: 0 },
    { ...roleFixture, id: 'restored-role', delete_count: 1, headcount: 5 },
    { ...roleFixture, id: 'closed-role', status: 'closed', headcount: 0 },
  ], total: 3 } } });
  app.state.lang = 'en';
  const html = await app.myJobs();
  assert.match(html, /Hiring paused/);
  assert.match(html, /Hiring target: 0/);
  assert.match(html, /Existing introductions and chats remain/);
  assert.equal((html.match(/data-form="job-headcount"/g) || []).length, 2);
  assert.match(html, /data-action="delete-job" data-id="job-one">Delete role/);
  assert.match(html, /data-action="delete-job" data-id="restored-role">Delete permanently/);
  assert.doesNotMatch(html, /data-action="close-job"/);
  assert.match(html, /href="\/app\?tab=jobs&view=trash"/);
  assert.equal(requests.length, 1);
  app.state.lang = 'zh';
  assert.match(app.jobBadge({ ...roleFixture, headcount: 0 }), /暂不招聘/);
});

test('first deletion and deletion after restoration give different bilingual confirmations', () => {
  const { app } = webmcp({ role: 'employer' });
  const first = app.jobDeletionPrompt(roleFixture);
  assert.equal(first.permanent, false);
  assert.match(first.title, /回收站/);
  assert.match(first.description, /恢复一次/);
  assert.match(first.description, /不占描述修改次数/);
  const second = app.jobDeletionPrompt({ ...roleFixture, delete_count: 1 });
  assert.equal(second.permanent, true);
  assert.match(second.description, /已用过一次回收机会/);
  assert.match(second.description, /不能恢复/);
  app.state.lang = 'en';
  assert.match(app.jobDeletionPrompt(roleFixture).description, /first deletion is free/);
  assert.match(app.jobDeletionPrompt({ ...roleFixture, delete_count: 1 }).description, /already used its one recovery opportunity/);
  assert.match(app.jobDeletionPrompt(roleFixture, false, true).description, /Administrator deletion is permanent/);
  assert.match(app.jobDeletionPrompt(roleFixture, true).description, /does not trigger a refund/);
});

test('trash reads only the recoverable list and lifecycle mutations carry the current version without payment', async () => {
  const { app, requests } = webmcp({ role: 'employer', search: '?tab=jobs&view=trash', responses: { '/api/my/jobs/trash': { items: [{ ...roleFixture, status: 'deleted', headcount: 0, delete_count: 1, version: 4 }] } } });
  app.state.lang = 'en';
  const html = await app.myJobs();
  assert.match(html, /Restore for free/);
  assert.match(html, /data-action="permanently-delete-job"/);
  assert.doesNotMatch(html, /data-form="job-headcount"/);
  await app.changeJobLifecycle(roleFixture, 'delete');
  await app.changeJobLifecycle({ ...roleFixture, version: 4 }, 'restore');
  await app.changeJobLifecycle({ ...roleFixture, version: 6 }, 'permanent');
  await app.changeJobLifecycle({ ...roleFixture, version: 8 }, 'permanent', true);
  await app.changeJobLifecycle({ ...roleFixture, version: 9 }, 'restore', true);
  assert.deepEqual(requests.map(request => [request.path, request.options.method, request.options.body ? JSON.parse(request.options.body) : null]), [
    ['/api/my/jobs/trash', 'GET', null],
    ['/api/jobs/job-one', 'DELETE', { expected_version: 3 }],
    ['/api/jobs/job-one/restore', 'POST', { expected_version: 4 }],
    ['/api/jobs/job-one/permanent', 'DELETE', { expected_version: 6 }],
    ['/api/admin/jobs/job-one', 'DELETE', { expected_version: 8 }],
    ['/api/admin/jobs/job-one/restore', 'POST', { expected_version: 9 }],
  ]);
});

test('stale deletion is localized and never automatically retried with a newer version', async () => {
  const { app, requests } = webmcp({ role: 'employer', responses: { '/api/jobs/job-one': { httpStatus: 409, body: { error: 'The job changed; refresh before deleting or restoring it' } } } });
  await assert.rejects(app.changeJobLifecycle(roleFixture, 'delete'), /刷新后重新确认删除或恢复/);
  assert.equal(requests.length, 1);
  assert.deepEqual(JSON.parse(requests[0].options.body), { expected_version: 3 });
  assert.match(app.requestErrorText('This job cannot be restored'), /永久删除不能恢复/);
  assert.match(app.requestErrorText('Closed jobs cannot resume hiring'), /不能重新开始招聘/);
  app.state.lang = 'en';
  assert.match(app.requestErrorText('This job cannot be restored'), /restored once/);
  assert.match(app.requestErrorText('The job changed; refresh before deleting or restoring it'), /confirm deletion or restoration again/);
});

test('cancelled orders after permanent deletion stay cancelled on an untrusted success return', async () => {
  const order = { id: 'order_deleted_role', kind: 'publish', status: 'cancelled', amount_cents: 30000, refunded_cents: 0, provider: 'stripe', created_at: 1 };
  const { app, requests } = webmcp({ role: 'employer', search: '?order_id=order_deleted_role&success=1&status=paid', order });
  assert.equal(app.statusText('cancelled'), '已取消');
  let html = await app.paymentReturn();
  assert.match(html, /这笔订单已取消/);
  assert.doesNotMatch(html, /data-action="pay-order"/);
  app.state.lang = 'en';
  html = await app.paymentReturn();
  assert.match(html, /This order was cancelled/);
  assert.match(html, /no longer accepts payment/);
  assert.ok(requests.every(request => request.options.method === 'GET'));
});

test('a cached checkout URL cannot bypass the current deleted-role payment guard', async () => {
  const { app, requests, location } = webmcp({ role: 'employer', responses: { '/api/orders/order_old/checkout': { httpStatus: 409, body: { error: 'A deleted job cannot be checked out' } } } });
  let navigated = false;
  location.assign = () => { navigated = true; };
  await assert.rejects(app.payOrder({ id: 'order_old', status: 'pending', checkout_url: 'https://checkout.example/old-session' }), /已删除岗位的订单暂不能付款/);
  assert.equal(navigated, false);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].path, '/api/orders/order_old/checkout');
  assert.equal(requests[0].options.method, 'POST');
});

test('WebMCP exposes free target changes and versioned lifecycle tools with destructive annotations', async () => {
  const { registered, requests } = webmcp({ role: 'employer' });
  const target = registered.get('hireferry_set_job_headcount');
  assert.ok(target);
  await target.execute({ id: roleFixture.id, headcount: 0 });
  const sent = requests.at(-1);
  assert.equal(sent.path, '/api/jobs/job-one/headcount');
  assert.equal(sent.options.method, 'PATCH');
  assert.deepEqual(JSON.parse(sent.options.body), { headcount: 0 });
  assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
  const create = registered.get('hireferry_create_job');
  assert.equal(create.inputSchema.properties.headcount.default, 1);
  assert.ok(!create.inputSchema.required.includes('headcount'));
  await create.execute({ title: 'Engineer', jd_md: 'Build.', salary_cents: 300000, provider: 'stripe' });
  assert.equal(JSON.parse(requests.at(-1).options.body).headcount, 1);
  for (const name of ['hireferry_delete_job', 'hireferry_permanently_delete_job']) {
    const tool = registered.get(name);
    assert.equal(tool.annotations.destructiveHint, true);
    assert.ok(tool.inputSchema.required.includes('expected_version'));
  }
  await registered.get('hireferry_restore_job').execute({ id: roleFixture.id, expected_version: 4 });
  assert.equal(requests.at(-1).path, '/api/jobs/job-one/restore');
  assert.deepEqual(JSON.parse(requests.at(-1).options.body), { expected_version: 4 });
  await registered.get('hireferry_list_deleted_jobs').execute({});
  assert.equal(requests.at(-1).path, '/api/my/jobs/trash');
  assert.equal(webmcp({ role: 'seeker' }).registered.has('hireferry_set_job_headcount'), false);
  const admin = webmcp({ admin: true }).registered;
  assert.ok(admin.get('hireferry_admin_update_job').inputSchema.properties.headcount);
  assert.equal(admin.get('hireferry_admin_delete_job').annotations.destructiveHint, true);
  assert.ok(admin.get('hireferry_admin_restore_job').inputSchema.required.includes('expected_version'));
});

test('administrator WebMCP search tools use bounded server queries and retain the AI attribution header', async () => {
  const { registered, requests } = webmcp({ admin: true });
  const candidates = registered.get('hireferry_admin_search_candidates');
  assert.ok(candidates);
  assert.equal(candidates.annotations.readOnlyHint, true);
  assert.deepEqual(candidates.inputSchema.required, ['job_id']);
  assert.equal(candidates.inputSchema.properties.q.maxLength, 120);
  assert.equal(candidates.inputSchema.properties.location.maxLength, 80);
  assert.equal(candidates.inputSchema.properties.limit.maximum, 100);
  await candidates.execute({ job_id: 'role-one', q: 'Rust', cursor: 'opaque+cursor', limit: 25, salary_min_cents: 600000 });
  const request = requests.at(-1);
  const url = new URL(request.path, 'https://hireferry.test');
  assert.equal(url.pathname, '/api/admin/workbench/candidates');
  assert.equal(url.searchParams.get('cursor'), 'opaque+cursor');
  assert.equal(url.searchParams.get('job_id'), 'role-one');
  assert.equal(request.options.headers['X-HireFerry-AI'], 'webmcp');
  assert.ok(registered.has('hireferry_admin_search_jobs'));
  assert.ok(registered.has('hireferry_admin_search_matches'));
  assert.equal(webmcp({ role: 'seeker' }).registered.has('hireferry_admin_search_candidates'), false);
});

test('appearance control and workbench failures are bilingual and do not trap invalid selections', async () => {
  const { app } = webmcp({ admin: true });
  app.state.lang = 'zh';
  const zh = app.header();
  assert.match(zh, /class="mode-options" role="group" aria-label="外观"/);
  assert.match(zh, /class="palette-options" role="group" aria-label="配色"/);
  assert.equal((zh.match(/data-palette-choice=/g)||[]).length,6);
  assert.match(zh, /data-palette-choice="neutral" aria-label="纸墨"/);
  for (const label of ['跟随系统', '浅色', '深色']) assert.ok(zh.includes(label));
  assert.match(app.requestErrorText('Invalid cursor or changed search filters; restart from the first page'), /第一页/);
  assert.match(app.requestErrorText('This job is no longer available for new matches; choose another job'), /新匹配/);
  app.state.lang = 'en';
  const en = app.header();
  assert.match(en, /class="mode-options" role="group" aria-label="Appearance"/);
  for (const label of ['System', 'Light', 'Dark']) assert.ok(en.includes(`aria-label="${label}"`));
  assert.doesNotMatch(en, /data-theme-select|data-palette-select/);
  const service = ui(async path => {
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([]);
    throw Object.assign(new Error('This role is unavailable.'), { status: 404 });
  }, { admin: true, search: '?tab=matches&job=stale-job' });
  const html = await service.matchesPage(true);
  assert.match(html, /This role is unavailable/);
  assert.match(html, /href="\/admin\?tab=matches"[^>]*data-nav/);
  assert.match(html, /Clear selection and filters/);
  assert.doesNotMatch(html, /data-form="workbench-introduce"/);
});

const deferred = () => { let resolve; const promise = new Promise(done => { resolve = done; }); return { promise, resolve }; };
const introductionRecord = id => ({ id, seeker_id: 'seeker-' + id, seeker_name: 'Private ' + id, employer_id: 'employer-' + id, employer_name: 'Team ' + id, job_id: 'job-' + id, job_title: 'Role ' + id, status: 'introduced', note: 'Shared note ' + id, updated_at: 1700000000 });

test('late match details never open after logout or replace a more recently requested record', async () => {
  const pending = new Map();
  const dialogs = [];
  const viewState = { admin: true, user: null, lang: 'en' };
  const service = ui(async path => {
    const id = path.split('/').at(-1);
    const request = deferred(); pending.set(id, request); return request.promise;
  }, { admin: true, search: '?tab=matches&view=followups', hooks: { state: viewState, openDialog: html => dialogs.push(html) } });
  const stale = service.matchDetail('signed-out');
  service.stopChat();
  viewState.admin = false;
  globalThis.location.search = '';
  pending.get('signed-out').resolve(introductionRecord('signed-out'));
  await stale;
  assert.equal(dialogs.length, 0);
  viewState.admin = true;
  globalThis.location.search = '?tab=matches&view=followups';
  const old = service.matchDetail('older-request');
  const latest = service.matchDetail('latest-request');
  pending.get('latest-request').resolve(introductionRecord('latest-request'));
  await latest;
  pending.get('older-request').resolve(introductionRecord('older-request'));
  await old;
  assert.equal(dialogs.length, 1);
  assert.match(dialogs[0], /Private latest-request/);
  assert.doesNotMatch(dialogs[0], /Private older-request|Private signed-out/);
});

test('an introduction completing after navigation preserves the new role and candidate selection', async () => {
  const firstWrite = deferred();
  const writes = [];
  const navigations = [];
  const secondRole = { ...workbenchRole, id: 'role-two', title: 'Designer' };
  const secondPerson = { ...workbenchPerson, id: 'person-two', name: 'Person B' };
  const service = ui(async (path, options) => {
    if (options?.method) { writes.push(options.body); return writes.length === 1 ? firstWrite.promise : {}; }
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([workbenchRole, secondRole]);
    if (path.startsWith('/api/admin/workbench/candidates?')) return workbenchResult([workbenchPerson, secondPerson]);
    return workbenchDetail(path.includes('/person-two?') ? secondPerson : workbenchPerson);
  }, { admin: true, search: '?tab=matches&job=role-one&candidate=person-one', hooks: { go: path => { navigations.push(path); browserNavigate(path); } } });
  await service.matchesPage(true);
  const data = new FormData(); data.set('note', 'Confirmed with both parties.');
  const savingFirst = service.introduceCandidate({ dataset: { user: 'person-one', job: 'role-one' } }, data);
  service.stopChat();
  globalThis.location.search = '?tab=matches&job=role-two&candidate=person-two';
  await service.matchesPage(true);
  firstWrite.resolve({ id: 'first-created' });
  await savingFirst;
  assert.deepEqual(navigations, []);
  assert.equal(new URLSearchParams(globalThis.location.search).get('candidate'), 'person-two');
  await service.introduceCandidate({ dataset: { user: 'person-two', job: 'role-two' } }, data);
  assert.equal(writes[0].job_id, 'role-one');
  assert.equal(writes[1].job_id, 'role-two');
  assert.equal(writes[1].seeker_id, 'person-two');
  assert.equal(navigations.length, 1);
});

test('a late follow-up save cannot close a newer detail dialog or navigate away from it', async () => {
  const pendingSave = deferred();
  const dialogs = [];
  const navigations = [];
  const writes = [];
  let closed = 0;
  const service = ui(async (path, options) => {
    if (options?.method) { writes.push({ path, ...options }); return writes.length === 1 ? pendingSave.promise : {}; }
    return introductionRecord(path.split('/').at(-1));
  }, { admin: true, search: '?tab=matches&view=followups&matches_cursor=second-page', hooks: { openDialog: html => dialogs.push(html), closeDialog: () => { closed++; }, go: path => { navigations.push(path); browserNavigate(path); } } });
  await service.matchDetail('first');
  const firstEpoch = dialogs[0].match(/data-detail-epoch="(\d+)"/)[1];
  const data = new FormData(); data.set('status', 'interview'); data.set('note', 'Interview arranged.');
  const save = service.saveFollowup({ dataset: { id: 'first', detailEpoch: firstEpoch }, isConnected: true }, data);
  await service.matchDetail('second');
  pendingSave.resolve({ ok: true });
  await save;
  assert.equal(closed, 0);
  assert.deepEqual(navigations, []);
  const secondEpoch = dialogs[1].match(/data-detail-epoch="(\d+)"/)[1];
  await service.saveFollowup({ dataset: { id: 'second', detailEpoch: secondEpoch }, isConnected: true }, data);
  assert.equal(closed, 1);
  assert.equal(navigations.length, 1);
  assert.equal(new URLSearchParams(globalThis.location.search).has('matches_cursor'), false);
  assert.deepEqual(writes.map(x => x.path), ['/api/admin/matches/first', '/api/admin/matches/second']);
});

test('a conversation outside the latest 200 opens by authorized metadata and stays visible while polling rechecks writability', async () => {
  const paths = [];
  let archived = false;
  const recent = Array.from({ length: 200 }, (_, i) => ({ id: 'match_recent-' + i, kind: 'matched', title: 'Recent ' + i, writable: true, participants: [] }));
  const service = ui(async path => {
    paths.push(path);
    if (path === '/api/chat/conversations') return { items: recent };
    if (path === '/api/chat/conversations/match_old') return { id: 'match_old', kind: 'matched', title: 'Older authorized introduction', writable: !archived, participants: [], unread_count: 0 };
    assert.match(path, /^\/api\/chat\/conversations\/match_old\/messages\?/);
    return { items: [], next_cursor: 0, first_cursor: 0, has_older: false };
  }, { admin: true, search: '?tab=messages&conversation=match_old' });
  const html = await service.messagesPage(true);
  assert.match(html, /Older authorized introduction/);
  assert.equal(paths.filter(path => path === '/api/chat/conversations').length, 1);
  assert.equal(paths.filter(path => path === '/api/chat/conversations/match_old').length, 1);
  const originalDocument = globalThis.document, originalTimeout = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  const nodes = { '#chat-scroll': { scrollTop: 0, scrollHeight: 500, clientHeight: 500 }, '#chat-messages': {}, '#conversation-list': { innerHTML: '' }, '#chat-connection': { textContent: '' }, '[data-chat-subtitle]': { textContent: '' }, '[data-chat-status]': { hidden: true }, '[data-action="chat-latest"]': { hidden: false } };
  const controls = [{ disabled: false, getAttribute: () => null }, { disabled: false, getAttribute: () => null }];
  let poll;
  try {
    globalThis.document = { hidden: false, querySelector: selector => nodes[selector] || null, querySelectorAll: () => controls };
    globalThis.setTimeout = callback => { poll = callback; return 1; };
    globalThis.clearTimeout = () => {};
    service.mountChat();
    archived = true;
    await poll();
    assert.equal(paths.filter(path => path === '/api/chat/conversations/match_old').length, 2);
    assert.match(nodes['#conversation-list'].innerHTML, /conversation=match_old/);
    assert.match(nodes['#conversation-list'].innerHTML, /Older authorized introduction/);
    assert.match(nodes['[data-chat-subtitle]'].textContent, /archived/i);
    assert.ok(controls.every(control => control.disabled));
    service.stopChat();
  } finally {
    globalThis.document = originalDocument;
    globalThis.setTimeout = originalTimeout;
    globalThis.clearTimeout = originalClear;
  }
});

test('denied conversation metadata never mounts a room even when its independently authorized history request is in flight', async () => {
  for (const status of [401, 403, 404]) {
    const paths = [];
    const service = ui(async path => {
      paths.push(path);
      if (path === '/api/chat/conversations') return { items: [] };
      if (path.endsWith('/messages?limit=100')) throw Object.assign(new Error('Access denied'), { status });
      assert.equal(path, '/api/chat/conversations/match_denied');
      throw Object.assign(new Error('Access denied'), { status });
    }, { admin: true, search: '?tab=messages&conversation=match_denied' });
    await assert.rejects(() => service.messagesPage(true), error => error.status === status);
    assert.deepEqual(paths, ['/api/chat/conversations/match_denied/messages?limit=100', '/api/chat/conversations', '/api/chat/conversations/match_denied']);
  }
});

const bulkForm = html => ({ dataset: { bulkReview: html.match(/data-bulk-review="(\d+)"/)[1] }, isConnected: true });
const batchFixtures = [workbenchPerson, { ...workbenchPerson, id: 'person-two', name: 'Person <B>' }, { ...workbenchPerson, id: 'person-ended', name: 'Ended person', history_match: { id: 'ended', status: 'declined' } }];

function bulkUi(write, hooks = {}, extra = {}) {
  return ui(async (path, options) => {
    if (options?.method) return write(path, options);
    if (path.startsWith('/api/admin/workbench/jobs?')) return workbenchResult([workbenchRole]);
    if (path.startsWith('/api/admin/workbench/candidates?')) return workbenchResult(batchFixtures, 'next-candidates');
    throw new Error('Unexpected batch read: ' + path);
  }, { admin: true, search: '?tab=matches&job=role-one&candidates_cursor=page-two&matches_cursor=old-followups', hooks, ...extra });
}

test('bulk selection is page-bound and excludes ended introductions from its escaped review', async () => {
  let dialog = '';
  const service = bulkUi(() => { throw new Error('Review must not write'); }, { openDialog: html => { dialog = html; } });
  const html = await service.matchesPage(true);
  assert.match(html, /data-workbench-select-all/);
  assert.match(html, /0 selected/);
  assert.match(html, /data-workbench-candidate="person-ended"[^>]*disabled/);
  assert.match(html, /Select Person &lt;B&gt;/);
  assert.match(html, /Selection is limited to this page/);
  assert.throws(() => service.selectWorkbenchCandidate('other-page', true), /on this page/i);
  assert.throws(() => service.selectWorkbenchCandidate('person-ended', true), /previous introduction/i);
  service.selectWorkbenchPage(true);
  service.reviewCandidateBatch();
  assert.match(dialog, /Review 2 introductions/);
  assert.match(dialog, /Person A/);
  assert.match(dialog, /Person &lt;B&gt;/);
  assert.doesNotMatch(dialog, /Person <B>|Ended person/);
  assert.match(dialog, /Rust Engineer/);
  assert.match(dialog, /Team A/);
  assert.match(dialog, /Each person gets a separate introduction/);
  service.selectWorkbenchPage(false);
  assert.throws(() => service.reviewCandidateBatch(), /Select at least one/);
});

test('bulk selection uses keyboard checkbox state and announces select-all and mixed selections', async () => {
  const service = bulkUi(() => ({}));
  await service.matchesPage(true);
  const inputs = batchFixtures.map(c => ({ dataset: { workbenchCandidate: c.id }, checked: false, disabled: !!c.history_match }));
  const counter = {}, all = {}, button = {};
  const nodes = { '#wb-bulk-count': counter, '[data-workbench-select-all]': all, '[data-action="workbench-bulk-review"]': button };
  const previous = globalThis.document;
  try {
    globalThis.document = { querySelector: selector => nodes[selector] || null, querySelectorAll: () => inputs };
    service.selectWorkbenchCandidate('person-one', true);
    assert.equal(counter.textContent, '1 selected');
    assert.equal(all.indeterminate, true);
    assert.equal(all.checked, false);
    assert.equal(button.disabled, false);
    service.selectWorkbenchPage(true);
    assert.equal(counter.textContent, '2 selected');
    assert.equal(all.indeterminate, false);
    assert.equal(all.checked, true);
    assert.deepEqual(inputs.map(input => input.checked), [true, true, false]);
    assert.equal(inputs[2].disabled, true);
    service.selectWorkbenchPage(false);
    assert.equal(button.disabled, true);
    assert.equal(all.checked, false);
    assert.equal(counter.textContent, '0 selected');
  } finally { globalThis.document = previous; }
});

test('bulk submission uses the reviewed snapshot and resets cursors after reporting created and reused counts', async () => {
  let dialog = '', closed = 0;
  const writes = [], notices = [];
  const service = bulkUi((path, options) => { writes.push({ path, ...options }); return { created: 1, reused: 1 }; }, { openDialog: html => { dialog = html; }, closeDialog: () => { closed += 1; }, notify: text => notices.push(text), go: browserNavigate });
  await service.matchesPage(true);
  service.selectWorkbenchPage(true);
  service.reviewCandidateBatch();
  const data = new FormData(); data.set('note', '  Reviewed together.  '); data.set('job_id', 'other-role'); data.append('seeker_ids', 'outside-this-page');
  await service.introduceCandidateBatch(bulkForm(dialog), data);
  assert.deepEqual(writes, [{ path: '/api/admin/workbench/introductions', method: 'POST', body: { job_id: 'role-one', seeker_ids: ['person-one', 'person-two'], note: 'Reviewed together.' } }]);
  assert.equal(closed, 1);
  assert.deepEqual(notices, ['1 introductions created; 1 existing introductions reused.']);
  const search = new URLSearchParams(globalThis.location.search);
  assert.equal(search.get('job'), 'role-one');
  assert.equal(search.has('candidates_cursor'), false);
  assert.equal(search.has('matches_cursor'), false);
  assert.throws(() => service.reviewCandidateBatch(), /page changed/i);
});

test('failed bulk submissions retain the exact selection and note for a safe retry', async () => {
  let dialog = '', attempts = 0, closed = 0;
  const writes = [];
  const service = bulkUi((path, options) => {
    writes.push({ path, ...options });
    if (++attempts === 1) throw new Error('Connection interrupted');
    return { created: 0, reused: 2 };
  }, { openDialog: html => { dialog = html; }, closeDialog: () => { closed += 1; }, go: browserNavigate });
  await service.matchesPage(true);
  service.selectWorkbenchPage(true);
  service.reviewCandidateBatch();
  const errorBox = { hidden: true, textContent: '' };
  const form = { ...bulkForm(dialog), querySelector: () => errorBox };
  const data = new FormData(); data.set('note', 'Keep this note.');
  await assert.rejects(() => service.introduceCandidateBatch(form, data), /Connection interrupted/);
  assert.equal(closed, 0);
  assert.equal(errorBox.hidden, false);
  assert.match(errorBox.textContent, /selection and note are kept/);
  assert.equal(data.get('note'), 'Keep this note.');
  await service.introduceCandidateBatch(form, data);
  assert.deepEqual(writes[1], writes[0]);
  assert.equal(closed, 1);
});

test('stale bulk reviews reject changed selections, routes, detached forms, and invalidated views', async () => {
  let dialog = '';
  const writes = [];
  const service = bulkUi((path, options) => { writes.push({ path, ...options }); return {}; }, { openDialog: html => { dialog = html; } });
  await service.matchesPage(true);
  service.selectWorkbenchCandidate('person-one', true);
  service.reviewCandidateBatch();
  const first = bulkForm(dialog), data = new FormData();
  service.selectWorkbenchCandidate('person-two', true);
  await assert.rejects(() => service.introduceCandidateBatch(first, data), /selection or review changed/i);
  service.reviewCandidateBatch();
  const current = bulkForm(dialog);
  await assert.rejects(() => service.introduceCandidateBatch({ ...current, isConnected: false }, data), /selection or review changed/i);
  const saved = globalThis.location.search;
  globalThis.location.search += '&candidate_q=changed';
  await assert.rejects(() => service.introduceCandidateBatch(current, data), /page changed/i);
  globalThis.location.search = saved;
  service.stopChat();
  await assert.rejects(() => service.introduceCandidateBatch(current, data), /page changed/i);
  assert.equal(writes.length, 0);
});

test('bulk requests prevent duplicate submits and never navigate a newer route after completion', async () => {
  let dialog = '';
  const pending = deferred(), writes = [], navigations = [];
  const service = bulkUi((path, options) => { writes.push({ path, ...options }); return pending.promise; }, { openDialog: html => { dialog = html; }, go: path => navigations.push(path) });
  await service.matchesPage(true);
  service.selectWorkbenchPage(true);
  service.reviewCandidateBatch();
  const form = bulkForm(dialog), data = new FormData();
  const operation = service.introduceCandidateBatch(form, data);
  await assert.rejects(() => service.introduceCandidateBatch(form, data), /in progress/i);
  assert.throws(() => service.selectWorkbenchPage(false), /in progress/i);
  globalThis.location.search = '?tab=matches&job=another-role';
  pending.resolve({ created: 2, reused: 0 });
  await operation;
  assert.equal(writes.length, 1);
  assert.deepEqual(navigations, []);
  assert.equal(new URLSearchParams(globalThis.location.search).get('job'), 'another-role');
});

test('bulk review is bilingual and applies the server note byte limit before writing', async () => {
  let dialog = '';
  const writes = [];
  const service = bulkUi((path, options) => { writes.push({ path, ...options }); return {}; }, { openDialog: html => { dialog = html; } }, { state: { lang: 'zh' } });
  const html = await service.matchesPage(true);
  assert.match(html, /全选本页可介绍的人/);
  service.selectWorkbenchCandidate('person-one', true);
  service.reviewCandidateBatch();
  assert.match(dialog, /核对 1 人的介绍/);
  assert.match(dialog, /确认介绍 1 人/);
  const data = new FormData(); data.set('note', '字'.repeat(667));
  await assert.rejects(() => service.introduceCandidateBatch(bulkForm(dialog), data), /介绍说明过长/);
  assert.equal(writes.length, 0);
});

test('WebMCP bulk introductions are administrative bounded atomic writes with AI attribution', async () => {
  const { registered, requests } = webmcp({ admin: true });
  const tool = registered.get('hireferry_admin_introduce_candidates');
  assert.ok(tool);
  assert.equal(tool.annotations.readOnlyHint, false);
  assert.deepEqual(tool.inputSchema.required, ['job_id', 'seeker_ids']);
  assert.equal(tool.inputSchema.properties.seeker_ids.minItems, 1);
  assert.equal(tool.inputSchema.properties.seeker_ids.maxItems, 100);
  assert.equal(tool.inputSchema.properties.seeker_ids.uniqueItems, true);
  await tool.execute({ job_id: 'role-one', seeker_ids: ['person-one', 'person-two'], note: 'Reviewed people only.' });
  const sent = requests.at(-1);
  assert.equal(sent.path, '/api/admin/workbench/introductions');
  assert.equal(sent.options.method, 'POST');
  assert.equal(sent.options.headers['X-HireFerry-AI'], 'webmcp');
  assert.deepEqual(JSON.parse(sent.options.body), { job_id: 'role-one', seeker_ids: ['person-one', 'person-two'], note: 'Reviewed people only.' });
  assert.equal(webmcp({ role: 'seeker' }).registered.has('hireferry_admin_introduce_candidates'), false);
});

const usdConfig = () => {
  const config = structuredClone(defaultConfig);
  Object.assign(config.pricing, { currency:'USD', currency_decimals:2, allowed_currencies:['USD','JPY'], publish_mode:'fixed', publish_fixed_minor:500, edit_mode:'free', max_edits:7 });
  config.pricing.currency_options = config.pricing.currency_options.filter(option => option.currency !== 'CNY');
  config.limits.max_job_headcount = 12;
  return config;
};

test('new role prices, salary precision, currency choices, and hiring caps come from configuration', async () => {
  const { app } = webmcp({ role:'employer' });
  app.state.config = usdConfig(); app.state.lang = 'en';
  const html = await app.jobEditor();
  assert.match(html, /id="job-currency" name="currency"/);
  assert.match(html, /value="USD" selected/);
  assert.match(html, /value="JPY"/);
  assert.match(html, /Fixed monthly base salary · USD/);
  assert.match(html, /USD\s*5\.00 per operation/);
  assert.match(html, /name="headcount"[^>]*max="12"/);
  assert.doesNotMatch(app.pricingSection(), /CNY|人民币|10%/);
  assert.match(app.pricingSection(), /Up to 7 updates/);
  const data = new FormData();
  for (const [key,value] of Object.entries({title:'Engineer',jd_md:'Build useful things.',salary:'3000',currency:'JPY',provider:'stripe',headcount:'12'})) data.set(key,value);
  assert.equal(app.newJobInput(data).salary_cents,3000);
  assert.equal(app.newJobInput(data).currency,'JPY');
  data.set('salary','3000.01');
  assert.throws(()=>app.newJobInput(data),/JPY.*0 decimal places/);
  assert.throws(()=>app.headcountInput('13'),/0 to 12/);
  assert.equal(app.centsInput('0.29','USD'),29);
});

test('historical job editor uses its recorded currency and frozen fixed fee despite new free pricing', async () => {
  const job = {...roleFixture,currency:'JPY',currency_decimals:0,salary_cents:300000};
  const terms = {currency:'JPY',currency_decimals:0,edit_mode:'fixed',edit_fixed_minor:375,edit_bps:9999,max_edits:5};
  const { app } = webmcp({role:'employer',responses:{'/api/jobs/job-one':{job,terms}}});
  app.state.config=usdConfig();app.state.lang='en';
  const html=await app.jobEditor('job-one');
  assert.match(html,/value="JPY" readonly/);
  assert.match(html,/name="salary"[^>]*step="1"[^>]*value="300000"[^>]*readonly/);
  assert.match(html,/JPY\s*375 per operation/);
  assert.match(html,/Updates remaining: 3 \/ 5/);
  assert.doesNotMatch(html,/USD\s*3\.75|99\.99%/);
});

test('explicit free publishing and updates remain usable with no payment providers', async () => {
  const job={...roleFixture,currency:'USD',currency_decimals:2};
  const {app}=webmcp({role:'employer',responses:{'/api/jobs/job-one':{job,terms:{edit_mode:'free',edit_bps:100,edit_fixed_minor:500,max_edits:7}}}});
  app.state.config=usdConfig();app.state.config.providers=[];app.state.config.pricing.publish_mode='free';app.state.lang='en';
  for(const id of [undefined,'job-one']) {
    const html=await app.jobEditor(id);
    assert.match(html,/This operation is free and takes effect on confirmation/);
    assert.match(html,/id="job-cost">USD\s*0\.00/);
    assert.match(html,/<button[^>]*data-job-submit >Confirm/);
    assert.doesNotMatch(html,/Online payments are not available|Confirm and continue to payment/);
  }
});

test('mandates retain zero and parse each currency in its own minor units', async () => {
  const written=[];
  const mandate={target_roles:'Engineer',salary_currency:'JPY',currency_decimals:0,salary_min_cents:0,salary_max_cents:300000,status:'active'};
  const service=ui(async (path,options)=>{if(options?.method){written.push(options.body);return {};}return path==='/api/mandate'?{mandate}:{items:[]};},{state:{config:usdConfig()}});
  const html=await service.mandatePage();
  assert.match(html,/Expected monthly base · minimum（JPY）/);
  assert.match(html,/name="salary_min"[^>]*step="1"[^>]*value="0"/);
  assert.match(html,/name="salary_max"[^>]*value="300000"/);
  assert.doesNotMatch(html,/minimum CNY|人民币/);
  const data=new FormData();
  for(const [key,value] of Object.entries({target_roles:'Engineer',salary_currency:'JPY',salary_min:'0',salary_max:'300000',status:'active'}))data.set(key,value);
  await service.saveMandate({dataset:{}},data);
  assert.deepEqual([written[0].salary_currency,written[0].salary_min_cents,written[0].salary_max_cents],['JPY',0,300000]);
  data.set('salary_min','3000.01');
  await assert.rejects(service.saveMandate({dataset:{}},data),/decimal places/);
  assert.equal(written.length,1);
});

test('an existing mandate can retain a currency no longer offered for new briefs', async () => {
  let body;
  const service=ui(async (_path,options)=>{body=options.body;return {};},{state:{config:usdConfig()}});
  const data=new FormData();
  for(const [key,value]of Object.entries({target_roles:'Engineer',salary_currency:'CNY',original_salary_currency:'CNY',salary_min:'3000',salary_max:'6000',status:'active'}))data.set(key,value);
  await service.saveMandate({dataset:{}},data);
  assert.equal(Object.hasOwn(body,'salary_currency'),false,'omission asks the server to preserve the saved currency');
  assert.equal(body.salary_min_cents,300000);
  data.set('original_salary_currency','');
  await assert.rejects(service.saveMandate({dataset:{}},data),/available salary currency/);
});

test('salary filters send the selected precision and role base-pay filtering overrides another currency', async () => {
  const job={...workbenchRole,currency:'JPY',currency_decimals:0,salary_cents:300000};
  const observed=[];
  const api=async path=>{if(path.includes('/jobs?'))return workbenchResult([job]);const params=new URL(path,'https://hireferry.test').searchParams;observed.push(Object.fromEntries(params));return workbenchResult([]);};
  await ui(api,{admin:true,state:{config:usdConfig()},search:'?tab=matches&job=role-one&salary_currency=USD&salary_min=0.29&salary_max=3000.01'}).matchesPage(true);
  assert.deepEqual([observed[0].salary_currency,observed[0].salary_min_cents,observed[0].salary_max_cents],['USD','29','300001']);
  await ui(api,{admin:true,state:{config:usdConfig()},search:'?tab=matches&job=role-one&salary_currency=USD&salary_min=0.29&salary_fit=1'}).matchesPage(true);
  assert.deepEqual([observed[1].salary_currency,observed[1].salary_min_cents,observed[1].salary_max_cents],['JPY','300000','300000']);
  await ui(api,{admin:true,state:{config:usdConfig()},search:'?tab=matches&job=role-one'}).matchesPage(true);
  assert.equal(Object.hasOwn(observed[2],'salary_currency'),false,'no currency or salary filter keeps candidates across currencies');
});

test('historical refunds and free return records never change to the site default currency', async () => {
  const order={id:'order_one',kind:'publish',provider:'free',status:'paid',amount_cents:0,currency:'JPY',currency_decimals:0,live_mode:false};
  const {app,requests}=webmcp({role:'employer',search:'?order_id=order_one',order});
  app.state.config=usdConfig();app.state.lang='en';
  const refund=app.orderRefundDetails({amount_cents:300000,refunded_cents:12345,currency:'JPY',currency_decimals:0});
  assert.match(refund,/Refunded JPY\s*12,345/);
  assert.match(refund,/Net amount.*JPY\s*287,655/);
  assert.doesNotMatch(refund,/USD|2876\.55/);
  const html=await app.paymentReturn();
  assert.match(html,/Completed free of charge/);
  assert.match(html,/JPY\s*0/);
  assert.doesNotMatch(html,/Payment confirmed|verified payment confirmation|Test order/);
  assert.deepEqual(requests.map(request=>request.path),['/api/orders/order_one']);
});

test('admin reporting queries one currency and renders separate totals without a mixed sum', async () => {
  const report={currency:'JPY',currency_decimals:0,payment_mode:'live',stats:{users:3,revenue_cents:75000,gross_cents:100000,refund_cents:25000,paid_orders:1},series:[{date:'2026-10-21',revenue_cents:75000}],currency_totals:[{currency:'JPY',currency_decimals:0,gross_cents:100000,refund_cents:25000,revenue_cents:75000,paid_orders:1},{currency:'USD',currency_decimals:2,gross_cents:12345,refund_cents:0,revenue_cents:12345,paid_orders:1}],recent_orders:[]};
  const {app,requests}=webmcp({admin:true,search:'?tab=overview&currency=JPY',responses:{'/api/admin/overview?currency=JPY&mode=live':report}});
  app.state.config=usdConfig();app.state.lang='en';
  const html=await app.adminOverview();
  assert.equal(requests[0].path,'/api/admin/overview?currency=JPY&mode=live');
  assert.match(html,/name="currency"/);
  assert.match(html,/JPY\s*75,000/);
  assert.match(html,/USD\s*123\.45/);
  assert.match(html,/without conversion|not combined|never added|separately/);
  assert.doesNotMatch(html,/CNY|87,345|750\.00/);
});

test('WebMCP exposes public currency configuration and optional precision-aware monetary fields', async () => {
  const {app,registered,requests}=webmcp({role:'employer'});
  app.state.config=usdConfig();app.state.lang='en';app.registerWebMCP();
  const create=registered.get('hireferry_create_job');
  assert.deepEqual(create.inputSchema.properties.currency.enum,['USD','JPY']);
  assert.equal(create.inputSchema.properties.salary_cents.minimum,1);
  assert.equal(create.inputSchema.properties.headcount.maximum,12);
  assert.equal(create.inputSchema.required.includes('provider'),false);
  assert.match(create.description,/USD: USD\s*5\.00 per operation/);
  assert.doesNotMatch(create.description,/CNY cents|10%/);
  assert.ok(registered.has('hireferry_get_site_config'));
  await registered.get('hireferry_get_site_config').execute({});
  assert.equal(requests.at(-1).path,'/api/config');
  const seeker=webmcp({role:'seeker'}).registered.get('hireferry_update_mandate');
  assert.equal(seeker.inputSchema.properties.salary_min_cents.minimum,0);
  assert.ok(seeker.inputSchema.properties.salary_currency);
});

test('payment pickers honor each provider currency list while older config remains compatible', async () => {
  const {app}=webmcp({role:'employer'});app.state.lang='en';app.state.config=usdConfig();
  app.state.config.providers=['stripe','waffo'];
  assert.deepEqual(app.paymentProviders('JPY'),['stripe','waffo']);
  app.state.config.payment_currencies={stripe:['USD'],waffo:['JPY']};
  assert.deepEqual(app.paymentProviders(),['stripe'],'activation uses the site default currency');
  assert.deepEqual(app.paymentProviders({currency:'JPY',currency_decimals:0}),['waffo']);
  assert.match(app.gatewayInputs('JPY'),/value="waffo"/);
  assert.doesNotMatch(app.gatewayInputs('JPY'),/value="stripe"/);
  assert.match(app.gatewayInputs('GBP'),/No online payment method is available for GBP/);
  const html=await app.jobEditor();
  assert.match(html,/data-payment-options data-currency="USD"/);
  assert.doesNotMatch(html,/value="waffo"/);
  delete app.state.config.payment_currencies.stripe;
  assert.deepEqual(app.paymentProviders('USD'),[],'a present mapping does not silently enable an absent provider');
});

test('changing job currency refreshes gateways and exact units without resetting a provider on salary edits', () => {
  const {app}=webmcp({role:'employer'});app.state.lang='en';app.state.config=usdConfig();
  app.state.config.providers=['stripe','waffo'];app.state.config.payment_currencies={stripe:['USD'],waffo:['USD','JPY']};
  const nodes={
    '[data-payment-options]':{dataset:{currency:'USD'},innerHTML:'original provider selection'},
    '#job-salary-label':{},'#job-fee-rule':{},'#job-cost':{},'#job-price-error':{},'[data-job-submit]':{},
  };
  const form={dataset:{id:'',exhausted:'false'},elements:{currency:{value:'USD'},salary:{value:'3000.29'},provider:{value:'waffo'}},querySelector:selector=>nodes[selector]};
  app.refreshJobPrice(form);
  assert.equal(nodes['[data-payment-options]'].innerHTML,'original provider selection');
  assert.equal(form.elements.provider.value,'waffo');
  form.elements.currency.value='JPY';form.elements.salary.value='300000';app.refreshJobPrice(form);
  assert.match(nodes['[data-payment-options]'].innerHTML,/value="waffo"/);
  assert.doesNotMatch(nodes['[data-payment-options]'].innerHTML,/value="stripe"/);
  assert.equal(form.elements.salary.step,'1');
  assert.equal(nodes['#job-salary-label'].textContent,'Fixed monthly base salary · JPY');
  assert.equal(nodes['[data-job-submit]'].disabled,false);
  form.elements.currency.value='GBP';app.refreshJobPrice(form);
  assert.equal(nodes['[data-job-submit]'].disabled,true);
  assert.match(nodes['[data-payment-options]'].innerHTML,/No online payment method is available for GBP/);
});

test('legal pages load the requested backend document and language instead of embedded policy copy', async () => {
  const documents={
    '/api/legal/terms?lang=zh':{kind:'terms',lang:'zh',version:'2026-10-reviewed',effective_date:'2026-10-21',html:'<h1>服务条款</h1><h2 id="pricing">配置费用</h2><p>JPY 375</p>'},
    '/api/legal/privacy?lang=en':{kind:'privacy',lang:'en',version:'2026-10-reviewed',effective_date:'2026-10-21',html:'<h1>Privacy Policy</h1><h2 id="contact">Support</h2><p>custom-support@example.test</p>'},
  };
  const rendered=[];
  const {app,requests}=webmcp({responses:documents,legalRenderer:(content,language)=>{rendered.push({content,language});return content.html;}});
  assert.equal(await app.legal('terms'),documents['/api/legal/terms?lang=zh'].html);
  app.state.lang='en';
  assert.equal(await app.legal('privacy'),documents['/api/legal/privacy?lang=en'].html);
  assert.deepEqual(rendered,[{content:documents['/api/legal/terms?lang=zh'],language:'zh'},{content:documents['/api/legal/privacy?lang=en'],language:'en'}]);
  assert.deepEqual(requests.map(request=>[request.path,request.options.method]),[['/api/legal/terms?lang=zh','GET'],['/api/legal/privacy?lang=en','GET']]);
  assert.match(appSource,/body=await legal\(path\.slice\(1\)\)/);
  assert.match(app.footer(),/href="\/refunds"[^>]*>Refunds/);
  assert.match(app.footer(),/href="\/contact"[^>]*>Contact/);
});

test('legal document loading retains its request language and never silently falls back after an API failure', async () => {
  const waiting=deferred(),rendered=[];
  const {app}=webmcp({responses:{'/api/legal/terms?lang=zh':waiting.promise},legalRenderer:(content,language)=>{rendered.push(language);return content.html;}});
  const result=app.legal('terms');
  app.state.lang='en';waiting.resolve({html:'<h1>服务条款</h1>'});
  assert.equal(await result,'<h1>服务条款</h1>');
  assert.deepEqual(rendered,['zh']);
  const failed=webmcp({responses:{'/api/legal/privacy?lang=zh':{httpStatus:503,body:{error:'Policy service unavailable'}}},legalRenderer:()=>{throw new Error('A failed API response must not reach the renderer');}});
  await assert.rejects(failed.app.legal('privacy'),/Policy service unavailable/);
  assert.equal(failed.requests.length,1);
});

test('configured workbench page size applies to queries and bulk selection beyond the former 25-person limit', async () => {
  const config=structuredClone(defaultConfig);config.limits.workbench_page_size=40;
  const people=Array.from({length:40},(_,i)=>({...workbenchPerson,id:`configured-${i}`,name:`Configured person ${i}`,history_match:i===1?{id:'ended',status:'declined'}:null}));
  let dialog='',written;
  const requests=[];
  const service=ui(async (path,options)=>{
    if(options?.method){written=options.body;return {created:39,reused:0};}
    requests.push(path);assert.equal(new URL(path,'https://hireferry.test').searchParams.get('limit'),'40');
    return path.includes('/jobs?')?{...workbenchResult([workbenchRole]),limit:40}:{...workbenchResult(people),limit:40};
  },{admin:true,search:'?tab=matches&job=role-one',state:{config},hooks:{openDialog:html=>{dialog=html;}}});
  const html=await service.matchesPage(true);
  assert.match(html,/40 on this page/);
  service.selectWorkbenchCandidate('configured-39',true);
  service.selectWorkbenchPage(true);service.reviewCandidateBatch();
  assert.match(dialog,/Review 39 introductions/);
  assert.match(dialog,/Configured person 39/);
  assert.doesNotMatch(dialog,/Configured person 1<\/li>/);
  await service.introduceCandidateBatch(bulkForm(dialog),new FormData());
  assert.equal(written.seeker_ids.length,39);
  assert.equal(written.seeker_ids.includes('configured-39'),true);
  assert.equal(written.seeker_ids.includes('configured-1'),false);
  assert.equal(requests.length,2);
});

test('workbench response limits define the selectable page and the client preserves the 100-person ceiling', async () => {
  for(const [configured,responseLimit,expectedRequest,expectedRows]of [[40,10,40,10],[500,500,100,100],[undefined,undefined,25,25]]) {
    const config=structuredClone(defaultConfig);config.limits.workbench_page_size=configured;
    const people=Array.from({length:120},(_,i)=>({...workbenchPerson,id:`bounded-${i}`,name:`Bounded ${i}`}));
    let dialog='';
    const service=ui(async path=>{
      assert.equal(new URL(path,'https://hireferry.test').searchParams.get('limit'),String(expectedRequest));
      return path.includes('/jobs?')?{items:[workbenchRole],total:1,limit:responseLimit}:{items:people,total:120,limit:responseLimit};
    },{admin:true,search:'?tab=matches&job=role-one',state:{config},hooks:{openDialog:html=>{dialog=html;}}});
    const html=await service.matchesPage(true);
    assert.equal((html.match(/data-workbench-candidate=/g)||[]).length,expectedRows);
    service.selectWorkbenchPage(true);service.reviewCandidateBatch();
    assert.match(dialog,new RegExp(`Review ${expectedRows} introductions`));
    assert.throws(()=>service.selectWorkbenchCandidate(`bounded-${expectedRows}`,true),/on this page/);
  }
});

test('post-mount currency synchronization uses retained live selections without changing drafts or requesting data', () => {
  const {app,requests}=webmcp({role:'employer',serviceUIFactory:createServiceUI});
  app.state.lang='en';app.state.config=usdConfig();
  app.state.config.providers=['stripe','waffo'];app.state.config.payment_currencies={stripe:['USD'],waffo:['USD','JPY']};
  const jobNodes={
    '[data-payment-options]':{dataset:{currency:'USD'},innerHTML:'Fresh default-currency gateways'},
    '#job-salary-label':{textContent:'Fixed monthly base salary · USD'},
    '#job-fee-rule':{textContent:'USD 5.00 per operation'},
    '#job-cost':{textContent:'USD 5.00'},'#job-price-error':{},'[data-job-submit]':{},
  };
  const jobForm={
    dataset:{id:'',exhausted:'false'},
    elements:{currency:{value:'JPY'},salary:{value:'300000',step:'0.01'},title:{value:'My unsaved role'},jd_md:{value:'My unsaved description'},headcount:{value:'4'},provider:{value:'waffo'}},
    querySelector:selector=>jobNodes[selector],
  };
  const salarySelects=['mandate','workbench-candidates'].map(kind=>{
    const labels={min:{textContent:'Minimum USD'},max:{textContent:'Maximum USD'}};
    const form={dataset:{form:kind,defaultCurrency:'USD'},elements:{salary_min:{value:'200000',step:'0.01'},salary_max:{value:'350000',step:'0.01'}},querySelector:selector=>labels[selector.includes('"min"')?'min':'max']};
    return {value:'JPY',closest:()=>form,form,labels};
  });
  const container={querySelector:selector=>{if(['#salary-example','#salary-example-result'].includes(selector))return null;assert.equal(selector,'form[data-form="job"]');return jobForm;},querySelectorAll:selector=>{assert.equal(selector,'select[name="salary_currency"]');return salarySelects;}};
  app.syncCurrencyFields(container);
  assert.equal(jobForm.elements.currency.value,'JPY');
  assert.equal(jobForm.elements.salary.value,'300000');
  assert.equal(jobForm.elements.salary.step,'1');
  assert.equal(jobNodes['#job-salary-label'].textContent,'Fixed monthly base salary · JPY');
  assert.match(jobNodes['#job-cost'].textContent,/JPY\s*1,000$/);
  assert.match(jobNodes['[data-payment-options]'].innerHTML,/value="waffo"/);
  assert.doesNotMatch(jobNodes['[data-payment-options]'].innerHTML,/value="stripe"/);
  assert.equal(jobForm.elements.title.value,'My unsaved role');
  assert.equal(jobForm.elements.jd_md.value,'My unsaved description');
  assert.equal(jobForm.elements.headcount.value,'4');
  for(const select of salarySelects){
    assert.equal(select.value,'JPY');
    assert.equal(select.form.elements.salary_min.value,'200000');
    assert.equal(select.form.elements.salary_max.value,'350000');
    assert.equal(select.form.elements.salary_min.step,'1');
    assert.match(select.labels.min.textContent,/（JPY）$/);
    assert.match(select.labels.max.textContent,/（JPY）$/);
  }
  assert.equal(requests.length,0);
  const mounted=appSource.indexOf('mountPage(root,header()+body+footer(),pageKey);');
  const synchronized=appSource.indexOf('syncCurrencyFields()',mounted);
  const ready=appSource.indexOf("recordPaint('ready')",mounted);
  assert.ok(mounted>=0&&synchronized>mounted&&ready>synchronized,'currency fields synchronize after mounting and before reporting the page ready');
});

function chatDom() {
  const originals = { document: globalThis.document, setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, matchMedia: globalThis.matchMedia };
  const listeners = new Map();
  const scrollListeners = new Map();
  const controls = [{ disabled: false, getAttribute: () => null }, { disabled: false, getAttribute: () => null }];
  const nodes = {
    '#chat-scroll': { scrollHeight: 600, clientHeight: 300, setAttribute() {}, addEventListener: (event, listener) => scrollListeners.set(event, listener), removeEventListener: event => scrollListeners.delete(event) },
    '#chat-messages': { innerHTML: '', insertAdjacentHTML(where, html) { this.innerHTML = where === 'afterbegin' ? html + this.innerHTML : this.innerHTML + html; }, querySelector: () => null },
    '#chat-body': { id: 'chat-body', value: '', disabled: false, maxLength: 10000, scrollHeight: 24, style: {}, focus() {} },
    '#conversation-list': { innerHTML: '' }, '#chat-connection': { textContent: '', closest: () => nodes['[data-chat-status]'] },
    '[data-chat-status]': { hidden: true },
    '[data-chat-subtitle]': { textContent: '' }, '[data-chat-resend]': { hidden: true, disabled: false },
    '[data-action="chat-latest"]': { hidden: true }, '[data-action="chat-older"]': { hidden: true },
    '.chat-workspace': { addEventListener: (event, listener) => listeners.set(event, listener), removeEventListener: event => listeners.delete(event), classList: { contains: () => true, toggle() {} }, querySelectorAll: () => [] },
  };
  let scrollTop = 0;
  Object.defineProperty(nodes['#chat-scroll'], 'scrollTop', {
    get: () => scrollTop,
    set: value => { scrollTop = Math.max(0, Math.min(value, nodes['#chat-scroll'].scrollHeight - nodes['#chat-scroll'].clientHeight)); },
  });
  let threadHtml = '';
  nodes['.chat-thread'] = { attributes: new Map(), setAttribute(name, value) { this.attributes.set(name, value); }, getAttribute(name) { return this.attributes.get(name) || null; }, get innerHTML() { return threadHtml; }, set innerHTML(html) {
    threadHtml = html;
    nodes['#chat-messages'].innerHTML = html.match(/id="chat-messages"[^>]*>([\s\S]*?)<\/div><\/div>\s*<form\b[^>]*>\s*<button[^>]*data-action="chat-latest"/)?.[1] || '';
    nodes['#chat-body'].value = html.match(/<textarea[^>]*>([\s\S]*?)<\/textarea>/)?.[1] || '';
    nodes['#chat-body'].placeholder = html.match(/<textarea[^>]*placeholder="([^"]*)"/)?.[1] || '';
    nodes['[data-chat-subtitle]'].textContent = html.match(/<p[^>]*data-chat-subtitle[^>]*>([^<]*)<\/p>/)?.[1] || '';
    nodes['#chat-body'].disabled = /<textarea[^>]*disabled/.test(html);
    controls.forEach(control => { control.disabled = nodes['#chat-body'].disabled; });
  } };
  let scheduled, scheduledDelay;
  globalThis.document = { hidden: false, querySelector: selector => nodes[selector] || null, querySelectorAll: selector => selector.includes('.chat-composer') ? controls : [] };
  globalThis.setTimeout = (callback, delay) => { if (delay === 0) { queueMicrotask(callback); return 2; } scheduled = callback; scheduledDelay = delay; return 1; };
  globalThis.clearTimeout = () => {};
  globalThis.matchMedia = () => ({ matches: false });
  return { nodes, controls, listeners, scrollListeners, poll: () => scheduled?.(), get delay() { return scheduledDelay; }, restore() { Object.assign(globalThis, originals); } };
}

const chatRoom = (id = 'platform_seeker-one', extra = {}) => ({ id, kind: id.startsWith('platform_') ? 'platform' : 'matched', title: id, writable: true, participants: [], ...extra });
const historyPage = items => ({ items, next_cursor: items.at(-1)?.id || 0, first_cursor: items[0]?.id || 0, has_older: false });
const chatMessage = (id, body) => ({ id, sender_kind: 'platform', sender_name: 'HireFerry', body, created_at: 1790000000 + id });

test('reopening an archived matched conversation restores its subtitle, composer hint and ability to send', async () => {
  let writable = false;
  const writes = [];
  const service = ui(async (path, options) => {
    if (path === '/api/chat/conversations') return { items: [chatRoom('match_reopened', { writable })] };
    if (options?.method === 'POST' && path.endsWith('/messages')) {
      writes.push({ path, body: options.body });
      return { ...chatMessage(2, options.body.body), is_own: true, client_id: options.body.client_id };
    }
    if (path.includes('/messages?')) return historyPage(path.includes('after=') ? [] : [chatMessage(1, 'Earlier conversation')]);
    return {};
  }, { admin: true, search: '?tab=messages&conversation=match_reopened' });
  const html = await service.messagesPage(true);
  const dom = chatDom();
  try {
    const field = dom.nodes['#chat-body'];
    field.getAttribute = () => null;
    dom.controls[0] = field;
    dom.nodes['.chat-thread'].innerHTML = html;
    await service.mountChat();
    assert.equal(dom.nodes['[data-chat-subtitle]'].textContent, 'Archived conversation · read-only');
    assert.equal(field.placeholder, 'This conversation is archived');
    assert.ok(dom.controls.every(control => control.disabled));
    const form = { querySelector: () => field }, data = new FormData();
    data.set('body', 'Confirming the next step after reopening');
    await assert.rejects(service.sendMessage(form, data), /cannot be sent/);
    assert.equal(writes.length, 0, 'an archived room cannot start a send');

    writable = true;
    await dom.poll();
    assert.equal(dom.nodes['[data-chat-subtitle]'].textContent, 'You, your match & HireFerry');
    assert.equal(field.placeholder, 'Write a message…');
    assert.ok(dom.controls.every(control => !control.disabled), 'polling restores both the textarea and send control');
    field.value = data.get('body');
    dom.listeners.get('input')({ target: field });
    await service.sendMessage(form, data);
    assert.equal(writes.length, 1);
    assert.equal(writes[0].path, '/api/chat/conversations/match_reopened/messages');
    assert.equal(writes[0].body.body, data.get('body'));
    assert.equal(field.value, '');
    assert.match(dom.nodes['#chat-messages'].innerHTML, /Confirming the next step after reopening/);
  } finally { service.stopChat(); dom.restore(); }
});

test('new messages preserve a reader 100px above the bottom and manual scrolling dismisses the latest prompt', async () => {
  let updates = [];
  const service = ui(async path => {
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (path.includes('/messages?')) return historyPage(path.includes('after=') ? updates : [chatMessage(1, 'Earlier discussion')]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    const scroll = dom.nodes['#chat-scroll'], messages = dom.nodes['#chat-messages'];
    const insert = messages.insertAdjacentHTML;
    messages.insertAdjacentHTML = function (where, html) { insert.call(this, where, html); scroll.scrollHeight += 120; };
    await service.mountChat();
    scroll.scrollTop = scroll.scrollHeight - scroll.clientHeight - 100;
    const readingPosition = scroll.scrollTop;
    updates = [chatMessage(2, 'A new reply')];
    await dom.poll();
    assert.equal(scroll.scrollTop, readingPosition, 'an incoming reply must not pull a reader away from history');
    assert.equal(dom.nodes['[data-action="chat-latest"]'].hidden, false);
    assert.equal(typeof dom.scrollListeners.get('scroll'), 'function');
    scroll.scrollTop = scroll.scrollHeight;
    dom.scrollListeners.get('scroll')({ target: scroll });
    assert.equal(dom.nodes['[data-action="chat-latest"]'].hidden, true, 'reaching the bottom manually acknowledges the prompt');
    scroll.scrollTop = scroll.scrollHeight - scroll.clientHeight - 8;
    updates = [chatMessage(3, 'Continue at the bottom')];
    await dom.poll();
    assert.equal(scroll.scrollTop, scroll.scrollHeight - scroll.clientHeight, 'a reader within 8px of the bottom continues following replies');
    assert.equal(dom.nodes['[data-action="chat-latest"]'].hidden, true);
  } finally { service.stopChat(); assert.equal(dom.scrollListeners.size, 0); dom.restore(); }
});

test('growing the composer keeps bottom readers at the latest message and preserves historical reading positions', async () => {
  const service = ui(async path => path === '/api/chat/conversations' ? { items: [chatRoom()] } : historyPage([chatMessage(1, 'Keep this visible')]));
  await service.messagesPage();
  const dom = chatDom();
  try {
    const scroll = dom.nodes['#chat-scroll'], field = dom.nodes['#chat-body'];
    Object.defineProperty(scroll, 'clientHeight', { get: () => 324 - (Number.parseFloat(field.style.height) || 24) });
    await service.mountChat();
    scroll.scrollTop = scroll.scrollHeight;
    field.value = 'A longer\nmessage\nwith several lines'; field.scrollHeight = 96;
    dom.listeners.get('input')({ target: field });
    assert.ok(Number.parseFloat(field.style.height) > 24, 'the composer actually grows');
    assert.equal(scroll.scrollTop, scroll.scrollHeight - scroll.clientHeight, 'the last message remains visible while composing');
    scroll.scrollTop = 90;
    field.value += '\nAnother line'; field.scrollHeight = 140;
    dom.listeners.get('input')({ target: field });
    assert.equal(scroll.scrollTop, 90, 'writing does not pull someone out of older history');
  } finally { service.stopChat(); dom.restore(); }
});

test('successful polling cannot erase an unconfirmed send and retry reuses its receipt key', async () => {
  const attempts = [];
  let fail = true;
  const service = ui(async (path, options) => {
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (options?.method === 'POST' && path.endsWith('/messages')) {
      attempts.push(options.body);
      if (fail) throw new Error('Network uncertain');
      return { ...chatMessage(2, options.body.body), sender_kind: 'user', sender_id: 'seeker-one', client_id: options.body.client_id };
    }
    if (path.includes('/messages?')) return historyPage(path.includes('after=') ? [] : [chatMessage(1, 'Previous message')]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    await service.mountChat();
    const field = dom.nodes['#chat-body'], form = { querySelector: () => field };
    field.value = 'Please keep this message';
    dom.listeners.get('input')({ target: field });
    const data = new FormData(); data.set('body', field.value);
    await assert.rejects(service.sendMessage(form, data), /Network uncertain/);
    const failure = dom.nodes['#chat-connection'].textContent;
    assert.ok(failure, 'an uncertain send has a visible explanation');
    assert.equal(dom.nodes['[data-chat-status]'].hidden, false);
    await dom.poll();
    assert.equal(dom.nodes['#chat-connection'].textContent, failure, 'a successful history read is not a delivery receipt');
    assert.equal(field.value, 'Please keep this message');
    assert.equal(dom.nodes['[data-chat-status]'].hidden, false);
    fail = false;
    await service.sendMessage(form, data);
    assert.equal(attempts.length, 2);
    assert.equal(attempts[1].client_id, attempts[0].client_id, 'a retry must remain idempotent');
    assert.equal(field.value, '');
    assert.equal(dom.nodes['#chat-connection'].textContent, '');
    assert.equal(dom.nodes['[data-chat-status]'].hidden, true);
    assert.match(dom.nodes['#chat-messages'].innerHTML, /class="message-delivery"[^>]*>Sent</);
    assert.equal((dom.nodes['#chat-messages'].innerHTML.match(/data-message-id="2"/g) || []).length, 1);
  } finally { service.stopChat(); dom.restore(); }
});

test('only the current sender’s matching receipt can resolve a send whose response was lost', async () => {
  let attempt, updates = [];
  const service = ui(async (path, options) => {
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (options?.method === 'POST' && path.endsWith('/messages')) { attempt = options.body; throw new Error('Response lost'); }
    if (path.includes('/messages?')) return historyPage(path.includes('after=') ? updates : [chatMessage(1, 'Previous message')]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    await service.mountChat();
    const field = dom.nodes['#chat-body']; field.value = 'Awaiting a receipt';
    dom.listeners.get('input')({ target: field });
    const data = new FormData(); data.set('body', field.value);
    await assert.rejects(service.sendMessage({ querySelector: () => field }, data), /Response lost/);
    const failure = dom.nodes['#chat-connection'].textContent;
    updates = [{ ...chatMessage(2, attempt.body), client_id: attempt.client_id }];
    await dom.poll();
    assert.equal(dom.nodes['#chat-connection'].textContent, failure, 'another sender’s client ID is not our receipt');
    assert.equal(field.value, 'Awaiting a receipt');
    updates = [{ ...chatMessage(3, attempt.body), sender_kind: 'user', sender_id: 'seeker-one', client_id: attempt.client_id }];
    await dom.poll();
    assert.equal(dom.nodes['#chat-connection'].textContent, '');
    assert.equal(dom.nodes['[data-chat-status]'].hidden, true);
    assert.equal(field.value, '', 'a confirmed unchanged draft is cleared');
    assert.equal((dom.nodes['#chat-messages'].innerHTML.match(/data-message-id="3"/g) || []).length, 1);
  } finally { service.stopChat(); dom.restore(); }
});

test('conversation starters append to an existing draft without sending it', async () => {
  const writes = [];
  const service = ui(async (path, options) => {
    if (options?.method === 'POST' && path.endsWith('/messages')) writes.push(options.body);
    return path === '/api/chat/conversations' ? { items: [chatRoom()] } : historyPage([]);
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    await service.mountChat();
    const field = dom.nodes['#chat-body']; field.value = 'My original context';
    const click = text => dom.listeners.get('click')({ target: { closest: selector => selector === '[data-chat-prompt]' ? { dataset: { chatPrompt: text } } : null } });
    click('A useful starting question');
    assert.ok(field.value.startsWith('My original context'));
    assert.ok(field.value.includes('A useful starting question'));
    assert.deepEqual(writes, [], 'choosing a prompt only edits the local draft');
    field.value = 'x'.repeat(field.maxLength - 2);
    click('A much longer suggestion');
    assert.ok(field.value.startsWith('x'.repeat(field.maxLength - 2)), 'a nearly full draft is not replaced');
    assert.ok(field.value.length <= field.maxLength, 'inserting a suggestion respects the message limit');
  } finally { service.stopChat(); dom.restore(); }
});

test('empty conversations give seekers, employers and administrators relevant starting points', async () => {
  const cases = [
    { role: 'seeker', title: 'Start with what matters to you', prompt: /résumé|experience|search/i },
    { role: 'employer', title: 'Let’s clarify your hiring needs', prompt: /hiring|recruit|role/i },
    { role: 'seeker', admin: true, title: 'Confirm the next step', prompt: /brief|update|progress/i },
    { role: 'seeker', matched: true, title: 'Discuss the role', prompt: /role|work/i },
    { role: 'employer', matched: true, title: 'Discuss the role', prompt: /team|experience|background|interview/i },
  ];
  for (const item of cases) {
    const room = chatRoom(item.matched ? 'match_role' : 'platform_seeker-one');
    const service = ui(async path => path === '/api/chat/conversations' ? { items: [room] } : historyPage([]), {
      admin: !!item.admin, search: `?tab=messages&conversation=${room.id}`,
      state: { user: { id: 'seeker-one', role: item.role, employer_paid: true } },
    });
    try {
      const html = await service.messagesPage(!!item.admin);
      assert.ok(html.includes(item.title), `${item.admin ? 'administrator' : item.role} receives the right introduction`);
      const prompts = [...html.matchAll(/data-chat-prompt="([^"]+)"/g)].map(match => match[1]);
      assert.ok(prompts.length > 0);
      assert.match(prompts.join(' '), item.prompt);
      if (item.role === 'employer' && item.matched) assert.doesNotMatch(prompts.join(' '), /Could you tell me more about the day-to-day work/);
      assert.match(html, /<textarea\b[^>]*rows="1"/);
      assert.match(html, /<details\b[^>]*class="chat-tools"/);
    } finally { service.stopChat(); }
  }
});

test('an inactive employer has one optional activation note inside the conversation rather than stacked fixed banners', async () => {
  const { app } = webmcp({ role: 'employer' });
  app.state.lang = 'en'; app.state.user.employer_paid = false;
  const room = chatRoom('platform_one');
  const service = ui(async path => path === '/api/chat/conversations' ? { items: [room] } : historyPage([]), {
    search: '?tab=messages&conversation=platform_one', state: app.state, hooks: { shell: app.shell },
  });
  try {
    const html = await service.messagesPage();
    const note = html.match(/<details\b[^>]*class="chat-account-note"[^>]*>[\s\S]*?<\/details>/)?.[0];
    assert.ok(note, 'activation remains available without blocking a conversation');
    assert.doesNotMatch(note.slice(0, note.indexOf('>')), /\bopen\b/, 'the optional details start collapsed');
    assert.equal((html.match(/class="chat-account-note"/g) || []).length, 1);
    assert.ok(html.indexOf(note) > html.indexOf('id="chat-scroll"'));
    assert.ok(html.indexOf(note) < html.indexOf('<form data-form="chat"'));
    assert.match(note, /href="\/app\?tab=orders"/);
    assert.match(note, /href="\/app\?tab=promos"/);
    assert.doesNotMatch(html, /class="(?:workspace-activation|promo-activation-banner)"/);
    assert.doesNotMatch(html.match(/<textarea[^>]*>/)?.[0] || '', /\bdisabled\b/, 'inactive employers can still discuss hiring needs');
  } finally { service.stopChat(); }
});

test('conversation index and history start together; a slow history does not block the inbox or its thread shell', async () => {
  const index = deferred(), history = deferred(), requests = [];
  const service = ui((path, options) => {
    requests.push([path, options?.method || 'GET']);
    if (path === '/api/chat/conversations') return index.promise;
    if (path.endsWith('/messages?limit=100')) return history.promise;
    return Promise.resolve({});
  });
  const page = service.messagesPage();
  assert.deepEqual(requests.map(([path]) => path), ['/api/chat/conversations/platform_seeker-one/messages?limit=100', '/api/chat/conversations']);
  index.resolve({ items: [chatRoom()] });
  const html = await page;
  assert.match(html, /Search conversations/);
  assert.match(html, /chat-message-placeholder/);
  assert.match(html, /<textarea[^>]*disabled/);
  assert.equal(requests.some(([path]) => path === '/api/chat/platform'), false);
  const dom = chatDom();
  try {
    const mounted = service.mountChat();
    history.resolve(historyPage([chatMessage(3, 'History arrives independently')]));
    await mounted;
    assert.match(dom.nodes['#chat-messages'].innerHTML, /History arrives independently/);
    assert.ok(dom.controls.every(control => !control.disabled));
    service.stopChat();
  } finally { dom.restore(); }
});

test('late history cannot overwrite a newly selected conversation', async () => {
  const oldHistory = deferred();
  const service = ui(async path => {
    if (path === '/api/chat/conversations') return { items: [chatRoom(), chatRoom('match_next')] };
    if (path.includes('/platform_seeker-one/messages')) return oldHistory.promise;
    if (path.includes('/match_next/messages')) return historyPage([chatMessage(8, 'Current conversation')]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    const oldMount = service.mountChat();
    service.stopChat();
    globalThis.location.search = '?tab=messages&conversation=match_next';
    const html = await service.messagesPage();
    assert.match(html, /Current conversation/);
    dom.nodes['#chat-messages'].innerHTML = 'Current conversation';
    await service.mountChat();
    oldHistory.resolve(historyPage([chatMessage(2, 'Stale private history')]));
    await oldMount;
    assert.equal(dom.nodes['#chat-messages'].innerHTML, 'Current conversation');
    service.stopChat();
  } finally { dom.restore(); }
});

test('drafts and scroll positions belong to a conversation and are discarded on account change', async () => {
  const user = { id: 'seeker-one', role: 'seeker' };
  const service = ui(async path => path === '/api/chat/conversations'
    ? { items: [chatRoom('platform_' + user.id), chatRoom('match_second')] } : historyPage([]), { state: { user } });
  await service.messagesPage();
  const dom = chatDom();
  try {
    dom.nodes['#chat-body'].value = 'Draft <only A>';
    dom.nodes['#chat-scroll'].scrollTop = 71;
    service.stopChat();
    globalThis.location.search = '?tab=messages&conversation=match_second';
    let html = await service.messagesPage();
    assert.doesNotMatch(html, /Draft &lt;only A&gt;/);
    dom.nodes['#chat-body'].value = 'Draft B'; service.stopChat();
    globalThis.location.search = '?tab=messages';
    html = await service.messagesPage();
    assert.match(html, /Draft &lt;only A&gt;/);
    dom.nodes['#chat-body'].value = 'Draft <only A>';
    dom.nodes['#chat-scroll'].scrollTop = 0;
    await service.mountChat();
    assert.equal(dom.nodes['#chat-scroll'].scrollTop, 71);
    user.id = 'another-account'; service.stopChat();
    html = await service.messagesPage();
    assert.doesNotMatch(html, /Draft/);
    service.stopChat();
  } finally { dom.restore(); }
});

test('revoked history erases messages and drafts, disables sending, and stops polling', async () => {
  let denied = false;
  const service = ui(async path => {
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (path.includes('/messages?')) { if (denied) throw Object.assign(new Error('Access denied'), { status: 403 }); return historyPage([chatMessage(1, 'Private message')]); }
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    dom.nodes['#chat-body'].value = 'Unsent private draft';
    dom.nodes['#chat-messages'].innerHTML = 'Private message';
    await service.mountChat();
    denied = true;
    await dom.poll();
    assert.doesNotMatch(dom.nodes['#chat-messages'].innerHTML, /Private message/);
    assert.match(dom.nodes['#chat-messages'].innerHTML, /no longer available/);
    assert.equal(dom.nodes['#chat-body'].value, '');
    assert.ok(dom.controls.every(control => control.disabled));
    service.stopChat();
    denied = false;
    assert.doesNotMatch(await service.messagesPage(), /Unsent private draft/);
    service.stopChat();
  } finally { dom.restore(); }
});

test('mobile inbox does not mark an unopened thread as read', async () => {
  const writes = [];
  const service = ui(async (path, options) => {
    if (options?.method) { writes.push(path); return {}; }
    if (path === '/api/chat/conversations') return { items: [chatRoom(undefined, { unread_count: 4 })] };
    return historyPage([chatMessage(4, 'Unread message')]);
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    globalThis.matchMedia = () => ({ matches: true });
    dom.nodes['.chat-workspace'].classList.contains = () => false;
    await service.mountChat(); await dom.poll();
    assert.deepEqual(writes, []);
    assert.match(dom.nodes['#conversation-list'].innerHTML, /4 unread messages/);
    service.stopChat();
  } finally { dom.restore(); }
});

test('conversation filtering uses names, previews and unread state without mutating source data', () => {
  const rooms = [chatRoom('one', { title: 'Engineering', participants: [{ name: 'Lina' }], unread_count: 3 }), chatRoom('two', { title: 'Design', last_message_preview: 'Tuesday interview', unread_count: 0 })];
  assert.deepEqual(filterConversations(rooms, 'LINA').map(room => room.id), ['one']);
  assert.deepEqual(filterConversations(rooms, 'interview').map(room => room.id), ['two']);
  assert.deepEqual(filterConversations(rooms, '', 'unread').map(room => room.id), ['one']);
  assert.deepEqual(filterConversations(rooms, 'interview', 'unread'), []);
  assert.equal(rooms[0].unread_count, 3);
});

test('keyboard sending respects multiline composition and IME input', () => {
  assert.equal(chatSendShortcut({ key: 'Enter' }), false);
  assert.equal(chatSendShortcut({ key: 'Enter', ctrlKey: true }), true);
  assert.equal(chatSendShortcut({ key: 'Enter', metaKey: true }), true);
  assert.equal(chatSendShortcut({ key: 'Enter', ctrlKey: true, isComposing: true }), false);
  assert.equal(chatSendShortcut({ key: 'Enter', ctrlKey: true, keyCode: 229 }), false);
  assert.equal(chatSendShortcut({ key: 'Enter', ctrlKey: true, shiftKey: true }), false);
  assert.equal(messageDateKey(1790000000), messageDateKey(1790000000000));
  assert.equal(messageDateKey(undefined), '');
});

test('a send completing after the same conversation remounts clears only its matching draft and shows the receipt once', async () => {
  const sent = deferred();
  let postBody;
  const service = ui(async (path, options) => {
    if (options?.method === 'POST' && path.endsWith('/messages')) { postBody = options.body; return sent.promise; }
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (path.includes('/messages?')) return historyPage([]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    dom.nodes['#chat-body'].value = 'One send, one receipt';
    const form = { querySelector: () => dom.nodes['#chat-body'] }, data = new FormData(); data.set('body', dom.nodes['#chat-body'].value);
    const sending = service.sendMessage(form, data);
    service.stopChat();
    const remounted = await service.messagesPage();
    assert.match(remounted, /One send, one receipt/);
    await service.mountChat();
    sent.resolve({ ...chatMessage(40, postBody.body), sender_kind: 'user', sender_id: 'seeker-one' });
    await sending;
    assert.equal(dom.nodes['#chat-body'].value, '');
    assert.equal((dom.nodes['#chat-messages'].innerHTML.match(/data-message-id="40"/g) || []).length, 1);
    service.stopChat();
    assert.doesNotMatch(await service.messagesPage(), />One send, one receipt<\/textarea>/);
    service.stopChat();
  } finally { dom.restore(); }
});

test('mobile back switches to the retained list immediately even when the selected platform room is unchanged', () => {
  for (const admin of [false, true]) {
    const service = ui(async () => ({}), { admin, search: '?tab=messages&view=list' });
    const dom = chatDom();
    try {
      let selected = true;
      dom.nodes['.chat-thread'] = { getAttribute: () => 'conversation:platform_seeker-one' };
      dom.nodes['.chat-workspace'].classList.toggle = (_name, value) => { selected = value; };
      service.showPendingChat(null);
      assert.equal(selected, false);
    } finally { dom.restore(); }
  }
});

test('a denied send erases the same conversation after it remounts', async () => {
  const sent = deferred();
  const service = ui(async (path, options) => {
    if (options?.method === 'POST' && path.endsWith('/messages')) throw await sent.promise;
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (path.includes('/messages?')) return historyPage([chatMessage(1, 'Private history')]);
    return {};
  });
  await service.messagesPage();
  const dom = chatDom();
  try {
    dom.nodes['#chat-body'].value = 'Private unsent draft';
    const data = new FormData(); data.set('body', dom.nodes['#chat-body'].value);
    const sending = service.sendMessage({}, data);
    service.stopChat();
    await service.messagesPage();
    await service.mountChat();
    dom.nodes['#chat-messages'].innerHTML = 'Private history';
    sent.resolve(Object.assign(new Error('Access denied'), { status: 403 }));
    await assert.rejects(sending, /Access denied/);
    assert.doesNotMatch(dom.nodes['#chat-messages'].innerHTML, /Private history/);
    assert.match(dom.nodes['#chat-messages'].innerHTML, /no longer available/);
    assert.equal(dom.nodes['#chat-body'].value, '');
    assert.ok(dom.controls.every(control => control.disabled));
    service.stopChat();
    assert.doesNotMatch(await service.messagesPage(), /Private unsent draft/);
    service.stopChat();
  } finally { dom.restore(); }
});


test('activation currency and region controls read configured prices, filter choices and show test mode',()=>{
  const {app}=webmcp({role:'employer'});app.state.lang='en';
  app.state.config.providers=['waffo'];app.state.config.payment_currencies={waffo:['CNY','USD','JPY']};
  app.state.config.payment_modes={waffo:'test'};
  app.state.config.waffo_regions={global:{currencies:['USD','JPY'],payment_methods:['card']},china:{currencies:['CNY','USD'],payment_methods:['wechat','alipay']}};
  app.state.config.pricing.activation_currency_options=[{currency:'CNY',currency_decimals:2,onboarding_minor:10000},{currency:'USD',currency_decimals:2,onboarding_minor:1498},{currency:'JPY',currency_decimals:0,onboarding_minor:2368}];
  assert.deepEqual(app.activationOptions('china').map(x=>x.currency),['CNY','USD']);
  const nodes={'[data-activation-total]':{},'[data-activation-gateway]':{},'[data-activation-rule]':{},'[type="submit"]':{},'[data-activation-submit]':{}};
  const form={elements:{currency:{value:'CNY'},payment_region:{value:'global'}},querySelector:selector=>nodes[selector]};
  app.refreshActivationPayment(form);
  assert.match(form.elements.currency.innerHTML,/value="USD" selected/);
  assert.doesNotMatch(form.elements.currency.innerHTML,/CNY/);
  assert.match(nodes['[data-activation-total]'].textContent,/USD.*14.98/);
  assert.match(nodes['[data-activation-gateway]'].innerHTML,/No real charges/);
  form.elements.currency.value='JPY';app.refreshActivationPayment(form);
  assert.match(nodes['[data-activation-total]'].textContent,/JPY.*2,368/);
  assert.doesNotMatch(nodes['[data-activation-total]'].textContent,/2,368.00/);
  app.state.config.pricing.activation_currency_options[2].onboarding_minor=999;
  app.refreshActivationPayment(form);assert.match(nodes['[data-activation-total]'].textContent,/999/);
  app.state.config.payment_modes.waffo='prod';assert.doesNotMatch(app.gatewayInputs('JPY'),/No real charges/);
});

test('cached chat formatters preserve both languages, local calendar dates and missing/invalid values', () => {
  const Original = Intl.DateTimeFormat;
  let language = 'en-GB', constructions = 0;
  const today = new Date(2026, 9, 22, 12, 10).getTime();
  const formatter = createChatDateFormatter(() => language, () => today);
  try {
    Intl.DateTimeFormat = new Proxy(Original, { construct(target, args) { constructions++; return Reflect.construct(target, args); } });
    formatter.sync();
    for (const locale of ['en-GB', 'zh-CN']) {
      language = locale;
      for (const value of [today, Math.floor(today / 1000), new Date(2024, 1, 28, 23, 59).getTime(), 'invalid']) {
        const date = new Date(Number(value) < 1e12 ? Number(value) * 1000 : value);
        assert.equal(formatter.detail(value), date.toLocaleString(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }));
        const options = messageDateKey(value) === messageDateKey(today) ? { hour: '2-digit', minute: '2-digit' } : { month: 'short', day: 'numeric' };
        assert.equal(formatter.compact(value), date.toLocaleString(locale, options));
        assert.equal(formatter.day(value), date.toLocaleDateString(locale, { year: 'numeric', month: 'long', day: 'numeric' }));
      }
    }
    const warmed = constructions;
    for (let i = 0; i < 300; i++) { formatter.detail(today); formatter.compact(today); }
    assert.equal(constructions, warmed, 'formatting existing kinds must reuse their Intl formatter');
    assert.equal(formatter.detail(0), ''); assert.equal(formatter.compact(null), '');
  } finally { Intl.DateTimeFormat = Original; }
});

test('unchanged polling compares generated list output without serializing browser HTML', async () => {
  const service = ui(async path => path === '/api/chat/conversations' ? { items: [chatRoom()] } : historyPage([]));
  await service.messagesPage();
  const dom = chatDom();
  try {
    let writes = 0;
    Object.defineProperty(dom.nodes['#conversation-list'], 'innerHTML', { get() { throw new Error('Browser serialization must not be read'); }, set() { writes++; } });
    await service.mountChat(); await dom.poll();
    const first = writes;
    await dom.poll(); await dom.poll();
    assert.equal(writes, first);
  } finally { service.stopChat(); dom.restore(); }
});

test('list updates retain stable row nodes and focus while moving, changing, adding and removing conversations', async () => {
  // This small DOM fixture exercises node identity and mutation APIs, not HTML parsing or paint.
  class Element {
    constructor(tag = 'span', attributes = {}, children = []) {
      this.tag = tag; this.attrs = new Map(Object.entries(attributes)); this.childNodes = []; this.parentNode = null;
      this.replaceChildren(...children);
    }
    get dataset() { return Object.fromEntries([...this.attrs].filter(([key]) => key.startsWith('data-')).map(([key, value]) => [key.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase()), value])); }
    get attributes() { return [...this.attrs].map(([name, value]) => ({ name, value })); }
    get children() { return this.childNodes.filter(node => node.tag !== '#text'); }
    get firstChild() { return this.childNodes[0] || null; }
    get firstElementChild() { return this.children[0] || null; }
    get lastElementChild() { return this.children.at(-1) || null; }
    get nextSibling() { return this.parentNode?.childNodes[this.parentNode.childNodes.indexOf(this) + 1] || null; }
    get textContent() { return this.tag === '#text' ? this.nodeValue : this.childNodes.map(node => node.textContent).join(''); }
    set textContent(value) { this.replaceChildren(text(value)); }
    set className(value) { this.setAttribute('class', value); }
    getAttribute(name) { return this.attrs.get(name) ?? null; }
    hasAttribute(name) { return this.attrs.has(name); }
    setAttribute(name, value) { this.attrs.set(name, String(value)); }
    removeAttribute(name) { this.attrs.delete(name); }
    remove() { if (this.parentNode) this.parentNode.childNodes.splice(this.parentNode.childNodes.indexOf(this), 1); this.parentNode = null; }
    insertBefore(node, before) { node.remove(); const index = before ? this.childNodes.indexOf(before) : this.childNodes.length; assert.ok(index >= 0); this.childNodes.splice(index, 0, node); node.parentNode = this; }
    replaceChildren(...nodes) { for (const child of [...this.childNodes]) child.remove(); for (const child of nodes) this.insertBefore(child, null); }
    querySelectorAll(selector) { return this.childNodes.flatMap(child => [...(child.hasAttribute(selector.slice(1, -1)) ? [child] : []), ...child.querySelectorAll(selector)]); }
    set innerHTML(html) {
      if (this.tag === 'template') { this.content = new Element('fragment', {}, [row(html)]); return; }
      assert.equal(this.tag, 'section');
      const [, label, count] = html.match(/<h3>(.*?)<span>(\d+)<\/span><\/h3>/);
      this.replaceChildren(new Element('h3', {}, [text(label), new Element('span', {}, [text(count)])]));
    }
    get innerHTML() { throw new Error('Existing DOM must not be serialized'); }
  }
  const text = value => { const node = new Element('#text'); node.nodeValue = value; return node; };
  const row = html => {
    const [, attributes, body] = html.match(/<a\b([^>]*)>([\s\S]*?)<\/a>/);
    return new Element('a', Object.fromEntries([...attributes.matchAll(/([\w-]+)(?:="([^"]*)")?/g)].map(([, key, value]) => [key, value || ''])), [text(body)]);
  };
  const a = chatRoom('match_a'), b = chatRoom('match_b'), c = chatRoom('match_c'), d = chatRoom('match_d');
  let rooms = [chatRoom(), a, b, c];
  const service = ui(async path => path === '/api/chat/conversations' ? { items: structuredClone(rooms) } : historyPage([]));
  const html = await service.messagesPage();
  const list = new Element('div'); list.scrollTop = 117;
  for (const [, kind, contents] of html.matchAll(/<section class="conversation-group" data-conversation-group="([^"]+)">([\s\S]*?)<\/section>/g)) {
    const section = new Element('section', { 'data-conversation-group': kind }); section.innerHTML = contents;
    for (const [markup] of contents.matchAll(/<a\b[^>]*>[\s\S]*?<\/a>/g)) section.insertBefore(row(markup), null);
    list.insertBefore(section, null);
  }
  const dom = chatDom();
  try {
    document.createElement = tag => new Element(tag);
    dom.nodes['#conversation-list'] = list;
    await service.mountChat(); await dom.poll();
    const rows = new Map(list.querySelectorAll('[data-conversation-id]').map(node => [node.dataset.conversationId, node]));
    const unchangedChild = rows.get(a.id).firstChild, changedChild = rows.get(c.id).firstChild;
    document.activeElement = rows.get(c.id);
    rooms = [chatRoom(), { ...c, last_message_preview: 'Updated preview', unread_count: 2 }, a, d];
    await dom.poll();
    const next = list.querySelectorAll('[data-conversation-id]');
    assert.deepEqual(next.map(node => node.dataset.conversationId), rooms.map(room => room.id));
    assert.equal(next[1], rows.get(c.id)); assert.equal(document.activeElement, rows.get(c.id));
    assert.notEqual(next[1].firstChild, changedChild); assert.match(next[1].textContent, /Updated preview/);
    assert.equal(next[2], rows.get(a.id)); assert.equal(next[2].firstChild, unchangedChild);
    assert.equal(rows.get(b.id).parentNode, null); assert.equal(list.scrollTop, 117);
    const stableChildren = next.map(node => node.firstChild);
    await dom.poll();
    assert.deepEqual(list.querySelectorAll('[data-conversation-id]'), next);
    assert.deepEqual(next.map(node => node.firstChild), stableChildren);
  } finally { service.stopChat(); dom.restore(); }
});

test('revocation prevents late older history and successful send receipts from restoring private content', async () => {
  for (const operation of ['older', 'send']) {
    const late = deferred(), calls = [];
    let revoked = false;
    const service = ui(async (path, options) => {
      calls.push([path, options]);
      if (path === '/api/chat/conversations') return { items: [chatRoom()] };
      if (path.includes('before=') || (options?.method === 'POST' && path.endsWith('/messages'))) return late.promise;
      if (path.includes('after=') && revoked) throw Object.assign(new Error('Revoked'), { status: 403 });
      if (path.includes('/messages?')) return { ...historyPage([chatMessage(10, 'Private recent history')]), has_older: true };
      return {};
    });
    await service.messagesPage(); const dom = chatDom();
    try {
      await service.mountChat();
      const data = new FormData(); data.set('body', 'Private sent body'); dom.nodes['#chat-body'].value = 'Private sent body';
      const pending = operation === 'older' ? service.olderMessages() : service.sendMessage({}, data);
      revoked = true; await dom.poll();
      assert.match(dom.nodes['#chat-messages'].innerHTML, /no longer available/);
      const writeCount = calls.filter(([, options]) => options?.method === 'POST').length;
      const message = chatMessage(operation === 'older' ? 1 : 11, 'Late private content');
      late.resolve(operation === 'older' ? historyPage([message]) : message); await pending;
      assert.doesNotMatch(dom.nodes['#chat-messages'].innerHTML, /Late private content/);
      assert.equal(dom.nodes['#chat-body'].value, ''); assert.ok(dom.controls.every(control => control.disabled));
      assert.equal(calls.filter(([, options]) => options?.method === 'POST').length, writeCount, 'late receipts must not write a read mark or resend');
      if (operation === 'older') assert.equal(calls.find(([path]) => path.includes('before='))[1].signal.aborted, true);
      else assert.equal(calls.find(([path, options]) => options?.method === 'POST' && path.endsWith('/messages'))[1].signal, undefined, 'submitted writes are not aborted by a view change');
    } finally { service.stopChat(); dom.restore(); }
  }
});

test('leaving a room while earlier history loads silently cancels that read', async () => {
  let signal;
  const service = ui(async (path, options) => {
    if (path === '/api/chat/conversations') return { items: [chatRoom()] };
    if (path.includes('before=')) return new Promise((resolve, reject) => {
      signal = options.signal;
      signal.addEventListener('abort', () => reject(new DOMException('Canceled', 'AbortError')), { once: true });
    });
    return { ...historyPage([chatMessage(10, 'Recent history')]), has_older: true };
  });
  await service.messagesPage(); const dom = chatDom();
  try {
    await service.mountChat();
    const pending = service.olderMessages();
    service.stopChat();
    assert.equal(signal.aborted, true);
    await assert.doesNotReject(pending, 'intentional cancellation must not reach the global error toast');
  } finally { service.stopChat(); dom.restore(); }
});

test('same-room list/detail navigation keeps its nodes, draft and scroll with zero requests', async () => {
  for (const admin of [false, true]) {
    const calls = [];
    const room = chatRoom(admin ? 'match_admin' : undefined);
    const path = admin ? '/admin' : '/app';
    const service = ui(async (url, options) => { calls.push([url, options?.method || 'GET']); return url === '/api/chat/conversations' ? { items: [room] } : historyPage([]); }, { admin, search: `?tab=messages&conversation=${room.id}` });
    await service.messagesPage(admin);
    const dom = chatDom();
    try {
      let selected = true;
      dom.nodes['.chat-workspace'].classList.toggle = (_name, value) => { selected = value; };
      await service.mountChat();
      dom.nodes['#chat-body'].value = 'Unsaved local draft'; dom.nodes['#chat-scroll'].scrollTop = 81;
      const body = dom.nodes['#chat-body'], thread = dom.nodes['.chat-thread'], before = calls.length;
      const list = new URL(path + '?tab=messages&view=list', location.origin);
      globalThis.location.search = list.search;
      await service.navigateChat(list);
      assert.equal(selected, false);
      const detail = new URL(path + '?tab=messages&conversation=' + room.id, location.origin);
      globalThis.location.search = detail.search;
      await service.navigateChat(detail);
      assert.equal(selected, true); assert.equal(calls.length, before);
      assert.equal(dom.nodes['.chat-thread'], thread); assert.equal(dom.nodes['#chat-body'], body);
      assert.equal(body.value, 'Unsaved local draft'); assert.equal(dom.nodes['#chat-scroll'].scrollTop, 81);
      assert.equal(service.navigateChat(detail, { onlyIfChanged: true }), null, 'explicit same-page refresh must retain the full refresh path');
      for (const destination of ['/app?tab=orders', '/app?order=untrusted', '/admin', path + '?tab=messages&conversation=unknown']) {
        assert.equal(service.navigateChat(new URL(destination, location.origin)), null);
      }
    } finally { service.stopChat(); dom.restore(); }
  }
});

test('switching rooms replaces only the thread, waits for fresh authorized history, and does not wait for its index', async () => {
  const history = deferred(), background = deferred(), calls = [];
  let later = false;
  const rooms = [chatRoom(), chatRoom('match_second')];
  const service = ui(async (path, options) => {
    calls.push([path, options]);
    if (path === '/api/chat/conversations') return later ? background.promise : { items: rooms };
    if (path.includes('/match_second/messages?')) return history.promise;
    return historyPage([]);
  }, { search: '?tab=messages&conversation=platform_seeker-one' });
  await service.messagesPage();
  const dom = chatDom();
  try {
    await service.mountChat(); later = true;
    dom.nodes['#chat-messages'].innerHTML = 'Previous private history'; dom.nodes['#chat-body'].value = 'Draft for old room';
    const list = dom.nodes['#conversation-list'], workspace = dom.nodes['.chat-workspace'];
    const next = new URL('/app?tab=messages&conversation=match_second', location.origin); globalThis.location.search = next.search;
    const transition = service.navigateChat(next);
    assert.ok(transition instanceof Promise);
    assert.doesNotMatch(dom.nodes['#chat-messages'].innerHTML, /Previous private history/);
    assert.equal(dom.nodes['#chat-body'].value, ''); assert.ok(dom.controls.every(control => control.disabled));
    assert.equal(service.navigateChat(new URL('/app?tab=messages&view=list', location.origin)), null, 'loading rooms fall back to the guarded full pipeline');
    history.resolve(historyPage([chatMessage(17, 'New authorized history')]));
    await transition;
    assert.match(dom.nodes['#chat-messages'].innerHTML, /New authorized history/);
    assert.equal(dom.nodes['#conversation-list'], list); assert.equal(dom.nodes['.chat-workspace'], workspace);
    const request = calls.find(([path]) => path.includes('/match_second/messages?'));
    assert.equal(request[1].fresh, true); assert.ok(request[1].signal);
    background.resolve({ items: rooms });
    await Promise.resolve();
  } finally { service.stopChat(); background.resolve({ items: rooms }); dom.restore(); }
});

test('denied background room metadata cannot be followed by late history restoring private content', async () => {
  const history = deferred(), background = deferred();
  let later = false;
  const service = ui(async path => {
    if (path === '/api/chat/conversations') return later ? background.promise : { items: [chatRoom(), chatRoom('match_second')] };
    if (path === '/api/chat/conversations/match_second') throw Object.assign(new Error('Revoked'), { status: 403 });
    if (path.includes('/match_second/messages?')) return history.promise;
    return historyPage([]);
  });
  await service.messagesPage(); const dom = chatDom();
  try {
    await service.mountChat(); later = true;
    const next = new URL('/app?tab=messages&conversation=match_second', location.origin); location.search = next.search;
    const transition = service.navigateChat(next);
    background.resolve({ items: [chatRoom()] });
    await new Promise(resolve => setImmediate(resolve));
    assert.match(dom.nodes['#chat-messages'].innerHTML, /no longer available/);
    history.resolve(historyPage([chatMessage(20, 'Must stay hidden')])); await transition;
    assert.doesNotMatch(dom.nodes['#chat-messages'].innerHTML, /Must stay hidden/);
    assert.ok(dom.controls.every(control => control.disabled));
  } finally { service.stopChat(); dom.restore(); }
});

test('a late room transition cannot replace a newer room or a different owner', async () => {
  const delayed = deferred();
  const user = { id: 'seeker-one', role: 'seeker' };
  const service = ui(async path => {
    if (path === '/api/chat/conversations') return { items: [chatRoom(), chatRoom('match_b'), chatRoom('match_c')] };
    if (path.includes('/match_b/messages?')) return delayed.promise;
    return historyPage([]);
  }, { state: { user } });
  await service.messagesPage(); const dom = chatDom();
  try {
    await service.mountChat();
    const second = new URL('/app?tab=messages&conversation=match_b', location.origin); location.search = second.search;
    const old = service.navigateChat(second);
    service.stopChat(); location.search = '?tab=messages&conversation=match_c';
    await service.messagesPage(); await service.mountChat();
    dom.nodes['#chat-messages'].innerHTML = 'Current C';
    delayed.resolve(historyPage([chatMessage(30, 'Stale B')])); await old;
    assert.equal(dom.nodes['#chat-messages'].innerHTML, 'Current C');
    user.id = 'other-account';
    assert.equal(service.navigateChat(new URL('/app?tab=messages&view=list', location.origin)), null);
  } finally { service.stopChat(); dom.restore(); }
});

test('incremental polling drains pages with task yields and a bounded continuation instead of five-second gaps', async () => {
  for (const pages of [3, 7]) {
    const calls = [];
    let next = 0;
    const service = ui(async path => {
      calls.push(path);
      if (path === '/api/chat/conversations') return { items: [chatRoom()] };
      if (path.includes('after=')) {
        const cursor = Number(new URL(path, location.origin).searchParams.get('after'));
        next++;
        return { ...historyPage([chatMessage(cursor + 1, 'Page ' + next)]), has_more: next < pages };
      }
      return historyPage([]);
    });
    await service.messagesPage(); const dom = chatDom();
    try {
      await service.mountChat(); await dom.poll();
      assert.equal(calls.filter(path => path.includes('after=')).length, Math.min(4, pages));
      assert.equal(dom.delay, pages > 4 ? 250 : 5000);
      assert.match(dom.nodes['#chat-messages'].innerHTML, /Page 1/);
      assert.match(dom.nodes['#chat-messages'].innerHTML, new RegExp('Page ' + Math.min(4, pages)));
      if (pages > 4) { await dom.poll(); assert.equal(next, pages); assert.equal(dom.delay, 5000); }
    } finally { service.stopChat(); dom.restore(); }
  }
});

test('real go and browser-history render use the chat hook, while expired sessions force validation', async () => {
  const dom = chatDom();
  const harness = await createNavigationHarness(fileURLToPath(new URL('../static/', import.meta.url)), { mountChat: true });
  try {
    const originalQuery = harness.document.querySelector;
    harness.document.querySelector = selector => dom.nodes[selector] || originalQuery(selector);
    harness.document.querySelectorAll = selector => selector.includes('.chat-composer') ? dom.controls : [];
    await harness.prime(); await harness.app.go('/app?tab=messages&conversation=platform_fixture-user');
    await Promise.resolve();
    const requests = harness.network.length, mounts = harness.mounts.length;
    await harness.app.go('/app?tab=messages&view=list');
    assert.equal(harness.network.length, requests); assert.equal(harness.mounts.length, mounts);
    harness.location.assign('/app?tab=messages&conversation=platform_fixture-user');
    await harness.app.render(); // popstate uses this same render entry point.
    assert.equal(harness.network.length, requests); assert.equal(harness.mounts.length, mounts);
    await harness.app.api('/api/profile', { method: 'PUT', body: { name: 'Updated' } });
    const afterMutation = harness.network.length;
    await harness.app.go('/app?tab=messages&view=list');
    assert.ok(harness.network.slice(afterMutation).some(request => request.path === '/api/session'));
    assert.ok(harness.mounts.length > mounts);
  } finally { harness.dispose(); dom.restore(); }
});

test('fast room changes and incremental messages preserve staff attribution and customer privacy', async () => {
  for (const admin of [true, false]) {
    const calls = [], rooms = [chatRoom(), chatRoom('match_second')];
    const first = { ...chatMessage(1, 'My reply'), is_own: true, staff_name: 'My staff identity' };
    const other = { ...chatMessage(2, 'Colleague reply'), is_own: false, staff_name: '<Colleague>' };
    const added = { ...chatMessage(3, 'AI follow-up'), is_own: false, staff_name: '平台AI', via_ai: true };
    const service = ui(async path => {
      calls.push(path);
      if (path === '/api/chat/conversations') return { items: rooms };
      if (path.includes('/match_second/messages?')) return historyPage(path.includes('after=') ? [added] : [first, other]);
      return historyPage([]);
    }, { admin, search: '?tab=messages&conversation=platform_seeker-one', state: admin ? { user: { id: 'staff-user', role: 'unassigned' }, super_admin: false, staff: { id: 'staff-one' }, csrf: 'staff-csrf' } : {} });
    await service.messagesPage(admin);
    const dom = chatDom();
    try {
      await service.mountChat();
      const base = admin ? '/admin' : '/app';
      const detail = new URL(base + '?tab=messages&conversation=match_second', location.origin);
      location.search = detail.search;
      await service.navigateChat(detail); await dom.poll();
      const html = dom.nodes['#chat-messages'].innerHTML;
      const message = id => html.match(new RegExp(`<article[^>]*data-message-id="${id}"[^>]*>[\\s\\S]*?<\\/article>`))?.[0] || '';
      if (admin) {
        assert.match(message(1), /chat-message own/); assert.match(message(1), /My staff identity/);
        assert.doesNotMatch(message(2), /chat-message own/); assert.match(message(2), /&lt;Colleague&gt;/);
        assert.doesNotMatch(message(3), /chat-message own/); assert.match(message(3), /Platform AI/);
      } else {
        for (const id of [1, 2, 3]) { assert.match(message(id), /HireFerry team/); assert.doesNotMatch(message(id), /chat-message own/); }
        assert.doesNotMatch(html, /My staff identity|Colleague&gt;|Platform AI/);
      }
      const count = calls.length;
      const list = new URL(base + '?tab=messages&view=list', location.origin);
      location.search = list.search; await service.navigateChat(list);
      location.search = detail.search; await service.navigateChat(detail);
      assert.equal(calls.length, count); assert.equal(dom.nodes['#chat-messages'].innerHTML, html);
    } finally { service.stopChat(); dom.restore(); }
  }
});

test('staff chat navigation cannot bypass owner page guards or retain access after session revocation', async () => {
  const dom = chatDom();
  const harness = await createNavigationHarness(fileURLToPath(new URL('../static/', import.meta.url)), { mountChat: true });
  try {
    const originalQuery = harness.document.querySelector;
    harness.document.querySelector = selector => dom.nodes[selector] || originalQuery(selector);
    harness.document.querySelectorAll = selector => selector.includes('.chat-composer') ? dom.controls : [];
    const user = { id: 'staff-user', role: 'unassigned', email: 'staff@example.test' };
    harness.overrides.set('/api/session', { user, admin: true, super_admin: false, staff: { id: 'staff-one', name: 'QA colleague', kind: 'human', active: true }, csrf: 'staff-csrf' });
    await harness.prime(); await harness.app.go('/admin?tab=messages&conversation=platform_fixture-user');
    await Promise.resolve();
    const requests = harness.network.length, mounts = harness.mounts.length;
    await harness.app.go('/admin?tab=messages&view=list');
    assert.equal(harness.network.length, requests); assert.equal(harness.mounts.length, mounts);
    await harness.app.go('/admin?tab=team');
    assert.match(harness.root.innerHTML, /This page is managed by the super administrator/);
    assert.equal(harness.network.some(request => ['/api/admin/team', '/api/admin/passkeys', '/api/admin/tokens'].includes(request.path)), false);
    assert.doesNotMatch(harness.root.innerHTML, /href="\/admin\?tab=(team|keys|agents)"|href="\/oauth\/connections"/);
    await harness.app.go('/admin?tab=messages&conversation=platform_fixture-user');
    await Promise.resolve();
    harness.overrides.set('/api/session', { user, admin: false, super_admin: false, staff: null, csrf: 'staff-csrf' });
    await harness.app.refreshVisibleSession();
    assert.equal(harness.app.state.admin, false); assert.equal(harness.app.state.staff, null);
    assert.doesNotMatch(harness.root.innerHTML, /chat-workspace|QA_MESSAGE_READY/);
    assert.match(harness.root.innerHTML, /admin-auth/);
  } finally { harness.dispose(); dom.restore(); }
});

test('reviewing another search brief releases the previous private outreach records', async () => {
  const dialogs = [];
  const owner = { id: 'admin-session', role: 'seeker' };
  const service = ui(async path => {
    const id = path.split('/').at(-1);
    if (path.includes('/mandates/')) return { mandate: { user_id: id, user_name: id, target_roles: 'Engineer', status: 'active', allow_external: true } };
    return { items: [{ id: `record-${id}`, channel: 'Direct', opportunity: `private-${id}`, stage: 'contacted', note: `notes-${id}` }] };
  }, { admin: true, state: { user: owner }, hooks: { openDialog: html => dialogs.push(html) } });
  await service.adminMandate('first');
  service.outreachDialog('first', 'record-first');
  assert.match(dialogs.at(-1), /notes-first/);
  await service.adminMandate('second');
  assert.throws(() => service.outreachDialog('first', 'record-first'), /Refresh this brief/);
  service.outreachDialog('second', 'record-second');
  assert.match(dialogs.at(-1), /notes-second/);
  owner.id = 'new-identity';
  service.stopChat();
  assert.throws(() => service.outreachDialog('second', 'record-second'), /Refresh this brief/);
});

test('a late search brief cannot refill private caches after the view changes', async () => {
  let finish;
  const delayed = new Promise(resolve => { finish = resolve; });
  const dialogs = [];
  const service = ui(async path => path.includes('/mandates/') ? delayed : { items: [{ id: 'old-record', note: 'old-private-data' }] },
    { admin: true, hooks: { openDialog: html => dialogs.push(html) } });
  const pending = service.adminMandate('old');
  service.stopChat();
  finish({ mandate: { user_id: 'old', user_name: 'Old', target_roles: 'Engineer', status: 'active', allow_external: true } });
  await pending;
  assert.equal(dialogs.length, 0);
  assert.throws(() => service.outreachDialog('old', 'old-record'), /Refresh this brief/);
});

test('old search cursor caches expire without losing the first-page recovery control', async () => {
  const service = ui(async () => ({ ...workbenchResult([workbenchRole], 'page-two'), total: 100 }),
    { admin: true, search: '?tab=matches&job_q=original', hooks: { go: browserNavigate } });
  await service.matchesPage(true);
  await service.workbenchPage('jobs', 'next');
  const second = await service.matchesPage(true);
  assert.doesNotMatch(second.match(/<button[^>]*data-direction="previous"[^>]*>/)[0], /disabled/);
  for (let i = 0; i < 51; i++) {
    globalThis.location.search = `?tab=matches&job_q=query-${i}`;
    await service.matchesPage(true);
  }
  globalThis.location.search = '?tab=matches&job_q=original&jobs_cursor=page-two';
  const restored = await service.matchesPage(true);
  assert.match(restored.match(/<button[^>]*data-direction="previous"[^>]*>/)[0], /disabled/);
  assert.match(restored, /data-direction="first"/);
  await service.workbenchPage('jobs', 'first');
  assert.equal(new URLSearchParams(globalThis.location.search).has('jobs_cursor'), false);
});

test('long cursor browsing keeps a bounded back history and a path to the first page', async () => {
  const service = ui(async path => {
    const cursor = new URL(path, 'https://hireferry.test').searchParams.get('cursor');
    return { ...workbenchResult([workbenchRole], String(Number(cursor || 0) + 1)), total: 10000 };
  }, { admin: true, search: '?tab=matches', hooks: { go: browserNavigate } });
  await service.matchesPage(true);
  for (let i = 0; i < 105; i++) {
    await service.workbenchPage('jobs', 'next');
    await service.matchesPage(true);
  }
  let html;
  for (let i = 0; i < 99; i++) {
    await service.workbenchPage('jobs', 'previous');
    html = await service.matchesPage(true);
  }
  assert.equal(new URLSearchParams(globalThis.location.search).get('jobs_cursor'), '6');
  assert.match(html.match(/<button[^>]*data-direction="previous"[^>]*>/)[0], /disabled/);
  assert.match(html, /data-direction="first"/);
  await service.workbenchPage('jobs', 'first');
  assert.equal(new URLSearchParams(globalThis.location.search).has('jobs_cursor'), false);
});

test('pending message capacity preserves earlier idempotency keys and rejects new sends safely', async () => {
  const attempts = [];
  const rooms = Array.from({ length: 51 }, (_, i) => chatRoom(`match_pending_${i}`));
  const service = ui(async (path, options) => {
    if (path === '/api/chat/conversations') return { items: rooms };
    if (options?.method === 'POST' && path.endsWith('/messages')) {
      attempts.push({ path, ...options.body });
      throw new Error('Network uncertain');
    }
    if (path.includes('/messages?')) return historyPage([]);
    return {};
  });
  const dom = chatDom();
  try {
    const data = new FormData(); data.set('body', 'Preserve this retry');
    const form = { querySelector: () => dom.nodes['#chat-body'] };
    for (let i = 0; i < 50; i++) {
      service.stopChat();
      globalThis.location.search = `?tab=messages&conversation=match_pending_${i}`;
      await service.messagesPage(); await service.mountChat();
      await assert.rejects(service.sendMessage(form, data), /Network uncertain/);
    }
    service.stopChat();
    globalThis.location.search = '?tab=messages&conversation=match_pending_50';
    await service.messagesPage(); await service.mountChat();
    await assert.rejects(service.sendMessage(form, data), /Retry an existing pending message/);
    assert.equal(attempts.length, 50, 'capacity failure must not start another network send');
    service.stopChat();
    globalThis.location.search = '?tab=messages&conversation=match_pending_0';
    await service.messagesPage(); await service.mountChat();
    await assert.rejects(service.sendMessage(form, data), /Network uncertain/);
    assert.equal(attempts.at(-1).client_id, attempts[0].client_id, 'the oldest uncertain send must keep its original idempotency key');
    service.stopChat();
  } finally { dom.restore(); }
});

test('manager-only sessions have no browser AI tools while an explicit employee grant remains usable', () => {
  const { app, registered } = webmcp({ admin: true });
  Object.assign(app.state, { owner: false, super_admin: false, staffRole: 'manager', user: { id: 'colleague' }, staff: null });
  app.registerWebMCP();
  assert.equal(registered.size, 0);
  app.state.staff = { id: 'employee-one' };
  app.registerWebMCP();
  assert.ok(registered.has('hireferry_admin_get_user'));
  app.state.staff = null;
  app.registerWebMCP();
  assert.equal(registered.size, 0);
});

test('losing employee access clears private workbench caches even when the manager session remains', async () => {
  const identity = { admin: true, super_admin: false, user: { id: 'colleague' }, staff: { id: 'employee-one' }, staffRole: 'manager', csrf: 'same-csrf' };
  const service = ui(async path => path.includes('/mandates/')
    ? { mandate: { user_id: 'first', target_roles: 'Engineer', status: 'active' } }
    : { items: [{ id: 'record-first', note: 'Employee-only retained draft' }] }, { hooks: { state: identity } });
  await service.adminMandate('first');
  service.outreachDialog('first', 'record-first');
  identity.staff = null;
  service.stopChat();
  assert.throws(() => service.outreachDialog('first', 'record-first'), /Refresh this brief/);
});

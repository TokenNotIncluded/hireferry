import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';

const moneySource = await readFile(new URL('../static/money.js', import.meta.url), 'utf8');
const moneyHelpers = await import('data:text/javascript;base64,' + Buffer.from(moneySource).toString('base64'));
const defaultConfig = { pricing: { currency:'CNY', currency_decimals:2, allowed_currencies:['CNY','USD','JPY'], currency_options:[{currency:'CNY',currency_decimals:2,publish_fixed_minor:1000,edit_fixed_minor:100,employer_ai_min_salary_minor:300000},{currency:'USD',currency_decimals:2,publish_fixed_minor:500,edit_fixed_minor:50,employer_ai_min_salary_minor:100000},{currency:'JPY',currency_decimals:0,publish_fixed_minor:1000,edit_fixed_minor:100,employer_ai_min_salary_minor:200000}], onboarding_cents:10000,publish_mode:'salary_percent',publish_bps:1000,publish_fixed_minor:1000,edit_mode:'salary_percent',edit_bps:100,edit_fixed_minor:100,max_edits:3 }, limits:{max_job_headcount:1000000,list_page_size:50,list_max_page:10000,workbench_page_size:25,recent_list_limit:200}, providers:['stripe'] };

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
  const modelContext = { registerTool: tool => registered.set(tool.name, tool), unregisterTool: name => registered.delete(name) };
  const node = { innerHTML: '', replaceChildren() {} };
  const document = { querySelector: () => node, addEventListener() {}, removeEventListener() {}, body: { style: {} }, documentElement: {}, modelContext };
  const fetch = async (path, options) => {
    requests.push({ path, options });
    const result = Object.hasOwn(responses, path) ? responses[path] : path === '/api/ai/eligibility' ? { eligible } : path.startsWith('/api/orders/') ? order : { ok: true };
    const status = result?.httpStatus || 200;
    return { ok: status >= 200 && status < 300, status, json: async () => result?.httpStatus ? result.body : result };
  };
  const storage = new Map();
  const sessionStorage = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  const browserLocation = { pathname: '/app', search, origin: 'https://hireferry.test' };
  const app = new Function('document', 'navigator', 'window', 'localStorage', 'location', 'history', 'fetch', 'createServiceUI', 'createMessageId', 'sessionStorage', 'renderLegal', ...Object.keys(moneyHelpers), appSource + '\nreturn {state,registerWebMCP,chartSvg,orderRefundDetails,statusText,requestErrorText,paymentReturn,refundsPage,contactPage,legal,footer,safeBillingReturn,rememberBillingReturn,afterSignIn,headcountInput,newJobInput,saveJobHeadcount,headcountControl,jobBadge,myJobs,jobEditor,jobDeletionPrompt,changeJobLifecycle,payOrder,header,home,pricingSection,feeSettings,feeRule,currency,money,centsInput,adminOverview,orderTable,ordersPage,agentsPage,employerActivation,refreshJobPrice,paymentProviders,gatewayInputs,syncCurrencyFields};')(
    document, { modelContext }, { addEventListener() {} }, { getItem: () => null }, browserLocation, {}, fetch, serviceUIFactory, createMessageId, sessionStorage, legalRenderer, ...Object.values(moneyHelpers),
  );
  Object.assign(app.state, { config:structuredClone(defaultConfig), user: admin ? null : { id: 'one', role, employer_paid: true }, admin, csrf: 'synthetic-csrf', eligibility: { eligible } });
  app.registerWebMCP();
  return { app, registered, requests, location: browserLocation, setEligible(value) { eligible = value; } };
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
  assert.match(guide, /<\/p><\/div><\/div><div class="webhook-principles">/, 'the explanation grid must be outside the flex header');
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

test('net-receipt chart retains refund-only negative values below an explicit zero baseline', () => {
  const { app } = webmcp();
  app.state.lang = 'en';
  const html = app.chartSvg([{ date: '2026-10-20', revenue_cents: 60000 }, { date: '2026-10-21', revenue_cents: -45000 }]);
  const zero = Number(html.match(/class="chart-grid chart-zero"[^>]*y1="([^"]+)"/)[1]);
  const points = [...html.matchAll(/<circle[^>]*cy="([^"]+)"/g)].map(match => Number(match[1]));
  assert.equal(points.length, 2);
  assert.ok(points[0] < zero, 'payment day is above zero');
  assert.ok(points[1] > zero, 'refund-only day is below zero');
  assert.match(html, /Net -CNY\s*450\.00/);
  assert.doesNotMatch(html, /NaN|Infinity/);
  const negativeOnly = app.chartSvg([{ date: '2026-10-21', revenue_cents: -45000 }]);
  assert.match(negativeOnly, /Net -CNY\s*450\.00/);
  assert.doesNotMatch(negativeOnly, /NaN|Infinity/);
});

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
  assert.match(zh, /data-theme-select aria-label="外观"/);
  for (const label of ['跟随系统', '浅色', '深色']) assert.ok(zh.includes(label));
  assert.match(app.requestErrorText('Invalid cursor or changed search filters; restart from the first page'), /第一页/);
  assert.match(app.requestErrorText('This job is no longer available for new matches; choose another job'), /新匹配/);
  app.state.lang = 'en';
  const en = app.header();
  assert.match(en, /data-theme-select aria-label="Appearance"/);
  for (const label of ['System', 'Light', 'Dark']) assert.ok(en.includes(`>${label}</option>`));
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
  const nodes = { '#chat-scroll': { scrollTop: 0, scrollHeight: 500, clientHeight: 500 }, '#chat-messages': {}, '#conversation-list': { innerHTML: '' }, '#chat-connection': { textContent: '' }, '[data-action="chat-latest"]': { hidden: false } };
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
    assert.match(nodes['#chat-connection'].textContent, /archived/i);
    assert.ok(controls.every(control => control.disabled));
    service.stopChat();
  } finally {
    globalThis.document = originalDocument;
    globalThis.setTimeout = originalTimeout;
    globalThis.clearTimeout = originalClear;
  }
});

test('denied single-conversation metadata never produces a fabricated room or reads messages', async () => {
  for (const status of [401, 403, 404]) {
    const paths = [];
    const service = ui(async path => {
      paths.push(path);
      if (path === '/api/chat/conversations') return { items: [] };
      assert.equal(path, '/api/chat/conversations/match_denied');
      throw Object.assign(new Error('Access denied'), { status });
    }, { admin: true, search: '?tab=messages&conversation=match_denied' });
    await assert.rejects(() => service.messagesPage(true), error => error.status === status);
    assert.deepEqual(paths, ['/api/chat/conversations', '/api/chat/conversations/match_denied']);
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
  const {app,requests}=webmcp({admin:true,search:'?tab=overview&currency=JPY',responses:{'/api/admin/overview?currency=JPY':report}});
  app.state.config=usdConfig();app.state.lang='en';
  const html=await app.adminOverview();
  assert.equal(requests[0].path,'/api/admin/overview?currency=JPY');
  assert.match(html,/name="currency"/);
  assert.match(html,/JPY\s*75,000/);
  assert.match(html,/USD\s*123\.45/);
  assert.match(html,/without conversion or a combined total/);
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
  const container={querySelector:selector=>{assert.equal(selector,'form[data-form="job"]');return jobForm;},querySelectorAll:selector=>{assert.equal(selector,'select[name="salary_currency"]');return salarySelects;}};
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
  assert.match(appSource,/mountPage\(root,header\(\)\+body\+footer\(\),pageKey\);syncCurrencyFields\(\)/);
});

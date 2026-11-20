import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const core = await readFile(new URL('../static/relation-graph-core.js', import.meta.url), 'utf8');
const source = (await readFile(new URL('../static/people-jobs-ui.js', import.meta.url), 'utf8')).replace('./relation-graph-core.js', 'data:text/javascript;base64,' + Buffer.from(core).toString('base64'));
const { createPeopleJobsUI, peopleJobsUrl, graphMatchQuery, graphRelations, connectionPath, graphPreview, graphView } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const esc = value => String(value ?? '').replace(/[&<>"']/g, value => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[value]);
const people = [{ id: 'seeker', name: 'Candidate', label: 'Candidate', role: 'seeker', kind: 'user', relation_count: 1000 }, { id: 'employer', name: 'Employer', label: 'Employer', role: 'employer', kind: 'user', relation_count: 0 }];
const jobs = [{ id: 'job', owner_id: 'employer', title: 'Role', label: 'Role', kind: 'job', status: 'published', headcount: 1, salary_cents: 300000, relation_count: 5 }];
const matches = [{ id: 'match', seeker_id: 'seeker', job_id: 'job', status: 'introduced' }];
const overview = { data_rev: 'd1', query_key: 'overview-key', summary: { users: 1000, seekers: 998, jobs: 1000, edges: 5000 }, groups: { left: [{ id: 'role:seeker', label: 'Job seekers', count: 998, edges: 5000 }, { id: 'role:employer', label: 'Employers', count: 2, edges: 0 }], right: [{ id: 'status:published', label: 'Hiring', count: 1000, edges: 5000 }] }, bundles: [{ left_group_id: 'role:seeker', right_group_id: 'status:published', count: 5000, state_counts: { introduced: 5000 } }] };
const list = items => ({ items, total: items.length, limit: 100, remaining: 0, next_cursor: null, data_rev: 'd1' });

function fixture({ search = '?tab=matches', data = {}, failing = false, changes = {}, beforeRequest = async () => {} } = {}) {
  globalThis.location = new URL('https://hireferry.test/admin' + search);
  const calls = [], navigations = [], state = { admin: true, owner: true, csrf: 'owner-csrf', user: null, ...changes };
  const api = async (path, options) => {
    calls.push([path, structuredClone({ ...options, signal: undefined })]);await beforeRequest(path, options);
    if (path !== '/api/admin/relations/query') throw new Error('Unexpected path ' + path);
    const q = options.body;
    if (failing && q.mode === 'overview') throw Object.assign(new Error('Unavailable'), { status: failing === true ? 503 : failing });
    if (q.mode === 'overview') return structuredClone(data.overview || overview);
    if (q.mode === 'nodes') return structuredClone(q.side === 'left' ? data.people || list(people) : data.jobs || list(jobs));
    if (q.mode === 'neighbors') return structuredClone(data.neighbors || { ...list([{ node: jobs[0], edge: matches[0] }]), total: 1000, remaining: 999, next_cursor: 'next', focus: people[0] });
    throw new Error('Unexpected mode ' + q.mode);
  };
  const ui = createPeopleJobsUI({ state, api, esc, t: (_, en) => en, icon: value => `<i>${value}</i>`, shell: content => content, matchText: String, go: url => navigations.push(url) });
  return { ui, calls, navigations, state };
}

test('legacy directory links preserve independent filters and reject unrelated parameters', () => {
  assert.equal(peopleJobsUrl('?tab=users&q=Alex&role=seeker&status=active&page=3&csrf=secret'), '/admin?tab=matches&people_q=Alex&people_role=seeker&people_status=active&people_page=3');
  assert.equal(peopleJobsUrl('?tab=jobs&q=Rust&status=closed&page=2'), '/admin?tab=matches&job_q=Rust&job_status=closed&jobs_page=2');
  assert.equal(peopleJobsUrl('?tab=matches&people_q=Alex&people_page=2&jobs_page=3', { people_page: 1, people_q: '', jobs_page: 4, arbitrary: 'ignored' }), '/admin?tab=matches&jobs_page=4');
});
test('legacy intersection helper never turns employer ownership into a seeker match', () => {
  assert.equal(graphMatchQuery([...people, people[0]], jobs).get('seeker_ids'), 'seeker');
  assert.equal(graphMatchQuery([], []).get('seeker_ids'), '');
  assert.deepEqual(graphRelations(people, jobs, [...matches, { id: 'fake', seeker_id: 'employer', job_id: 'job' }, { id: 'off-page', seeker_id: 'seeker', job_id: 'other' }]), matches);
  assert.equal(connectionPath({ x: 10, y: 20 }, { x: 90, y: 100 }), 'M10,20 C50,20 50,100 90,100');
});
test('selection preview remains distinct from a saved relation and rejects unavailable sides', () => {
  assert.deepEqual(graphPreview(people, jobs, [], 'seeker', 'job'), { seeker_id: 'seeker', job_id: 'job' });
  assert.equal(graphPreview(people, jobs, matches, 'seeker', 'job'), null);
  assert.equal(graphPreview(people, jobs, [], 'employer', 'job'), null);
  assert.equal(graphPreview([{ ...people[0], banned: true }], jobs, [], 'seeker', 'job'), null);
  assert.equal(graphPreview([people[0], { ...people[1], banned: true }], jobs, [], 'seeker', 'job'), null);
  for (const job of [{ ...jobs[0], headcount: 0 }, { ...jobs[0], status: 'closed' }, { ...jobs[0], status: 'draft' }]) assert.equal(graphPreview(people, [job], [], 'seeker', 'job'), null);
});
test('default directory keeps collection totals and folds advanced graph controls', async () => {
  const { ui, calls } = fixture();const html = await ui.page();
  assert.equal(calls.length, 3);assert.deepEqual(calls.map(([, o]) => o.body.mode), ['overview', 'nodes', 'nodes']);
  assert.equal(calls.every(([, o]) => o.method === 'POST'), true);
  assert.equal(calls.some(([, o]) => 'seeker_ids' in o.body || 'job_ids' in o.body), false);
  assert.match(html, /5,000/);assert.match(html, /data-pj-mode="list" aria-pressed="true"/);assert.match(html, /<details class="pj-advanced" >.*Advanced views & filters/);assert.match(html, /data-graph-node="person:employer"/);
  assert.match(html, /view=search/);assert.match(html, /view=followups/);assert.doesNotMatch(html, /compatibility|\d+%/i);
});
test('directory preserves account and job actions, including employers', async () => {
  const { ui } = fixture({ search: '?tab=matches&graph_mode=list' });const html = await ui.page();
  assert.match(html, /data-graph-node="person:employer"/);assert.match(html, /data-action="admin-edit-user"/);assert.match(html, /data-action="admin-user-access"/);assert.match(html, /data-action="admin-edit-job"/);
  assert.match(html, /Page order/);
});
test('managers retain detail access without adding suspension controls', async () => {
  const { ui } = fixture({ search: '?tab=matches&graph_mode=list', changes: { owner: false, staff: null } });const html = await ui.page();
  assert.doesNotMatch(html, /data-action="admin-user-access"/);assert.match(html, /data-action="admin-edit-job" data-id="job">Details/);
});
test('a thousand-neighbor focus loads one bounded page and explicitly exposes the rest', async () => {
  const { ui, calls } = fixture({ search: '?tab=matches&graph_mode=focus&focus_side=left&focus_id=seeker' });const html = await ui.page();
  const q = calls.at(-1)[1].body;assert.equal(q.mode, 'neighbors');assert.equal(q.limit, 100);assert.deepEqual(q.focus, { side: 'left', id: 'seeker' });assert.equal(q.data_rev, 'd1');
  assert.match(html, /1,000 relations in this scope/);assert.match(html, /999 after this page/);assert.match(html, /data-page-key="neighbors"/);assert.match(html, /data-action="match-detail" data-id="match"/);
  assert.doesNotMatch(html, /data-graph-node="person:employer"/);
});
test('an empty directory does not fabricate relations or hide its management page', async () => {
  const empty = { ...overview, summary: { users: 0, jobs: 0, edges: 0 }, groups: { left: [], right: [] }, bundles: [] };
  const { ui } = fixture({ search: '?tab=matches&graph_mode=list', data: { overview: empty, people: list([]), jobs: list([]) } });const html = await ui.page();
  assert.match(html, /No visible records/);assert.doesNotMatch(html, /data-graph-node=/);assert.doesNotMatch(html, /data-action="match-detail"/);
});
test('unavailable data is distinct from zero, and auth failures are propagated', async () => {
  assert.match(await fixture({ failing: true }).ui.page(), /Could not load relation data/);
  for (const failing of [401, 403]) await assert.rejects(fixture({ failing }).ui.page(), { status: failing });
  assert.match(await fixture({ failing: 409 }).ui.page(), /pages from different revisions are not combined/);
});
test('all queries share independent user/job/group filters without exposing private profile data', async () => {
  const { ui, calls } = fixture({ search: '?tab=matches&people_q=Alex&job_q=Rust&people_role=seeker&people_status=active&job_status=published&graph_status=interview&left_group=role%3Aseeker' });await ui.page();
  for (const [, options] of calls) assert.deepEqual(options.body.filters, { user_q: 'Alex', job_q: 'Rust', role: 'seeker', account_status: 'active', job_status: 'published', match_status: 'interview', left_group_ids: ['role:seeker'] });
  assert.equal(graphView('?graph_mode=matrix&group_left=industry&group_right=owner').mode, 'matrix');
  assert.deepEqual(graphView('?group_left=made-up').grouping, { left: 'role', right: 'status' });
});
test('switching mode preserves scope and pure line visibility leaves data unchanged', async () => {
  const { ui, calls } = fixture({ search: '?tab=matches&left_group=role%3Aseeker' });await ui.page();const before = calls.length;
  await ui.applyViewCommands([{ op: 'set_mode', mode: 'matrix' }, { op: 'set_edge_display', edge_display: 'hidden' }]);
  assert.deepEqual(ui.getViewState().filters.left_group_ids, ['role:seeker']);assert.equal(ui.getViewState().edge_display, 'hidden');assert.equal(calls.length, before);
});
test('unmounted filter submission keeps the opposite directory search', async () => {
  const { ui, navigations } = fixture({ search: '?tab=matches&people_page=2&jobs_page=3&job_q=Engineer' });await ui.page();
  const data = new FormData();data.set('people_q', 'New name');data.set('people_role', 'seeker');ui.submit({ dataset: { list: 'people' } }, data);
  const p = new URL(navigations[0], 'https://hireferry.test').searchParams;assert.equal(p.has('people_page'), false);assert.equal(p.get('job_q'), 'Engineer');assert.equal(p.get('jobs_page'), '3');
});
test('changing identities while data loads discards the old account response', async () => {
  let release, started;const pending = new Promise(resolve => { release = resolve; }), loading = new Promise(resolve => { started = resolve; });
  const { ui, state } = fixture({ beforeRequest: async (_, options) => { if (options.body.mode === 'overview') { started();await pending; } } });
  const first = ui.page();await loading;state.user = { id: 'another-account' };state.csrf = 'new-csrf';release();assert.equal(await first, '');
});
test('a delayed response cannot replace a newer page', async () => {
  let release, started, requests = 0;const pending = new Promise(resolve => { release = resolve; }), loading = new Promise(resolve => { started = resolve; });
  const { ui } = fixture({ beforeRequest: async (_, options) => { if (options.body.mode === 'overview' && ++requests === 1) { started();await pending; } } });
  const first = ui.page();await loading;assert.match(await ui.page(), /data-graph-page="2"/);release();assert.equal(await first, '');
});
test('node and group labels are escaped', async () => {
  const bad = '<img src=x onerror=alert(1)>';
  const { ui } = fixture({ search: '?tab=matches&graph_mode=list', data: { people: list([{ ...people[0], label: bad, name: bad }]), jobs: list([{ ...jobs[0], label: bad, title: bad }]) } });
  const html = await ui.page();assert.match(html, /&lt;img/);assert.doesNotMatch(html, /<img src=x/);
});
test('leaving while requests are pending suppresses the stale page', async () => {
  let release;const pending = new Promise(resolve => { release = resolve; });const { ui } = fixture({ beforeRequest: () => pending });
  const first = ui.page();ui.destroy();release();assert.equal(await first, '');
});

test('SMTP email actions survive public configuration caching and stay absent when disabled', async () => {
  const cacheSource=await readFile(new URL('../static/browser-cache.js',import.meta.url),'utf8');
  const {createPublicConfigCache}=await import('data:text/javascript;base64,'+Buffer.from(cacheSource).toString('base64'));
  for(const enabled of [true,false]){
    const disk=new Map();const storage={getItem:key=>disk.get(key)||null,setItem:(key,value)=>disk.set(key,value),removeItem:key=>disk.delete(key)};
    const cache=createPublicConfigCache(async()=>({name:'HireFerry',email_notifications_enabled:enabled}),{storage});
    await cache.refresh();
    const config=createPublicConfigCache(()=>{}, {storage}).peek();
    const {ui}=fixture({changes:{config}});const html=await ui.page();
    assert.equal(html.includes('data-action="admin-email-user"'),enabled);
  }
});

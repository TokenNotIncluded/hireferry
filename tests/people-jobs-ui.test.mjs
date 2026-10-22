import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../static/people-jobs-ui.js', import.meta.url), 'utf8');
const { createPeopleJobsUI, peopleJobsUrl, graphMatchQuery, graphRelations, connectionPath, graphPreview } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const esc = value => String(value ?? '').replace(/[&<>"']/g, value => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[value]);
const people = [{ id: 'seeker', name: 'Candidate', email: 'candidate@example.invalid', role: 'seeker' }, { id: 'employer', name: 'Employer', email: 'employer@example.invalid', role: 'employer' }];
const jobs = [{ id: 'job', owner_id: 'employer', title: 'Role', status: 'published', headcount: 1, salary_cents: 300000 }];
const matches = [{ id: 'match', seeker_id: 'seeker', seeker_name: 'Candidate', job_id: 'job', job_title: 'Role', status: 'introduced' }];
const list = items => ({ items, total: items.length, page: 1, page_size: 50 });

function fixture({ search = '?tab=matches', data = {}, failing = false, changes = {}, beforeRequest = async () => {} } = {}) {
  globalThis.location = new URL('https://hireferry.test/admin' + search);
  const calls = [], navigations = [];
  const state = { admin: true, owner: true, csrf: 'owner-csrf', user: null, ...changes };
  const api = async (path, options) => {
    calls.push([path, options]);
    await beforeRequest(path);
    if (path.startsWith('/api/admin/users?')) return structuredClone(data.people || list(people));
    if (path.startsWith('/api/admin/jobs?')) return structuredClone(data.jobs || list(jobs));
    if (path.startsWith('/api/admin/workbench/matches?')) {
      if (failing) throw Object.assign(new Error('Unable to load'), { status: failing === true ? 503 : failing });
      return structuredClone(data.matches || { items: matches, total: matches.length });
    }
    throw new Error('Unexpected path ' + path);
  };
  const ui = createPeopleJobsUI({ state, api, esc, t: (_, en) => en, icon: value => `<i>${value}</i>`, avatar: () => '<i>avatar</i>', money: value => String(value), date: String, shell: content => content, badge: String, matchText: String, go: url => navigations.push(url) });
  return { ui, calls, navigations, state };
}

test('legacy directory URLs preserve their relevant filters without carrying unrelated fields', () => {
  assert.equal(peopleJobsUrl('?tab=users&q=Alex&role=seeker&status=active&page=3&csrf=secret'), '/admin?tab=matches&people_q=Alex&people_role=seeker&people_status=active&people_page=3');
  assert.equal(peopleJobsUrl('?tab=jobs&q=Rust&status=closed&page=2'), '/admin?tab=matches&job_q=Rust&job_status=closed&jobs_page=2');
  assert.equal(peopleJobsUrl('?tab=matches&people_q=Alex&people_page=2&jobs_page=3', { people_page: 1, people_q: '', jobs_page: 4, arbitrary: 'ignored' }), '/admin?tab=matches&jobs_page=4');
});

test('the graph requests only real seeker-role intersections and never turns employer ownership into a match', () => {
  const query = graphMatchQuery([...people, people[0]], jobs);
  assert.equal(query.get('seeker_ids'), 'seeker');
  assert.equal(query.get('job_ids'), 'job');
  assert.equal(query.get('limit'), '100');
  assert.equal(graphMatchQuery([], []).get('seeker_ids'), '');
  assert.deepEqual(graphRelations(people, jobs, [...matches, { id: 'fake-employer-link', seeker_id: 'employer', job_id: 'job' }, { id: 'off-page', seeker_id: 'seeker', job_id: 'other' }]), matches);
  assert.equal(connectionPath({ x: 10, y: 20 }, { x: 90, y: 100 }), 'M10,20 C50,20 50,100 90,100');
});

test('a preview joins only a selected active seeker and hiring role, and never becomes a saved relation', () => {
  const saved = structuredClone(matches);
  const allJobs = [...jobs, { ...jobs[0], id: 'new-role' }];
  assert.deepEqual(graphPreview(people, allJobs, saved, 'seeker', 'new-role'), { seeker_id: 'seeker', job_id: 'new-role' });
  assert.equal(graphPreview(people, allJobs, saved, 'seeker', 'job'), null);
  assert.equal(graphPreview(people, allJobs, saved, 'employer', 'new-role'), null);
  assert.equal(graphPreview(people, allJobs, saved, '', 'new-role'), null);
  assert.equal(graphPreview(people, allJobs, saved, 'seeker', ''), null);
  assert.deepEqual(saved, matches);
});

test('ended introductions, suspended accounts and non-hiring roles never show a new preview', () => {
  assert.equal(graphPreview(people, jobs, [{ ...matches[0], status: 'declined' }], 'seeker', 'job'), null);
  assert.equal(graphPreview([{ ...people[0], banned: true }], jobs, [], 'seeker', 'job'), null);
  assert.equal(graphPreview([people[0], { ...people[1], banned: true }], jobs, [], 'seeker', 'job'), null);
  for (const job of [{ ...jobs[0], headcount: 0 }, { ...jobs[0], status: 'closed' }, { ...jobs[0], status: 'draft' }]) assert.equal(graphPreview(people, [job], [], 'seeker', 'job'), null);
});

test('the combined page preserves profile and role actions with bounded actual introduction data', async () => {
  const { ui, calls } = fixture();
  const html = await ui.page();
  assert.equal(calls.length, 3);
  assert.equal(calls.every(([, options]) => !options?.method), true);
  assert.match(calls[2][0], /seeker_ids=seeker&job_ids=job/);
  assert.match(html, /data-graph-node="person:seeker"/);
  assert.match(html, /data-graph-node="job:job"/);
  assert.match(html, /data-action="admin-edit-user"/);
  assert.match(html, /data-action="admin-user-access"/);
  assert.match(html, /data-action="admin-edit-job"/);
  assert.match(html, /data-action="match-detail" data-id="match"/);
  assert.match(html, /view=search/);
  assert.match(html, /view=followups/);
  assert.doesNotMatch(html, /\d+%|recommended|compatibility score/i);
});

test('manager view does not add account suspension or job management controls', async () => {
  const { ui } = fixture({ changes: { owner: false, staff: null } });
  const html = await ui.page();
  assert.doesNotMatch(html, /data-action="admin-user-access"/);
  assert.match(html, /data-action="admin-edit-job" data-id="job">Details/);
});

test('no relation query is sent for an empty side, and empty pages never invent a connection', async () => {
  const { ui, calls } = fixture({ data: { jobs: list([]) } });
  const html = await ui.page();
  assert.equal(calls.length, 2);
  assert.match(html, /No roles found/);
  assert.doesNotMatch(html, /data-graph-relation="/);
  assert.match(html, /No introductions between these pages/);
});

test('a relationship load error is distinct from zero results and does not hide authentication failures', async () => {
  const { ui } = fixture({ failing: true });
  const html = await ui.page();
  assert.match(html, /Introductions could not be loaded/);
  assert.match(html, /— shown links/);
  assert.doesNotMatch(html, /Showing 0 introductions/);
  for (const failing of [401, 403]) await assert.rejects(fixture({ failing }).ui.page(), { status: failing });
});

test('pagination and a truncated relation set stay explicit; user and job search remain independent', async () => {
  const { ui, calls, navigations } = fixture({ search: '?tab=matches&people_page=2&jobs_page=3&people_q=Alex&job_q=Engineer', data: { people: { ...list(people), page: 2, page_size: 2, total: 8 }, matches: { items: matches, total: 101 } } });
  const html = await ui.page();
  assert.match(calls[0][0], /q=Alex&page=2/);
  assert.match(calls[1][0], /q=Engineer&page=3/);
  assert.match(html, /Showing 1 of 101 introductions/);
  assert.match(html, /First 1 of 101 links/);
  assert.match(html, /View all follow-ups/);
  const data = new FormData(); data.set('people_q', 'New name'); data.set('people_role', 'seeker');
  ui.submit({ dataset: { list: 'people' } }, data);
  const target = new URL(navigations[0], 'https://hireferry.test');
  assert.equal(target.searchParams.has('people_page'), false);
  assert.equal(target.searchParams.get('jobs_page'), '3');
  assert.equal(target.searchParams.get('job_q'), 'Engineer');
});

test('job keyword and status use real backend filters while retaining the independent people page', async () => {
  const { ui, calls, navigations } = fixture({ search: '?tab=matches&people_page=2&people_role=seeker&people_status=suspended&jobs_page=3&job_q=Rust&job_status=closed' });
  await ui.page();
  const peopleQuery = new URL(calls[0][0], 'https://hireferry.test').searchParams;
  const jobQuery = new URL(calls[1][0], 'https://hireferry.test').searchParams;
  assert.deepEqual(Object.fromEntries(peopleQuery), { role: 'seeker', status: 'suspended', page: '2' });
  assert.deepEqual(Object.fromEntries(jobQuery), { q: 'Rust', status: 'closed', page: '3' });
  const data = new FormData(); data.set('job_q', 'Designer'); data.set('job_status', 'published');
  ui.submit({ dataset: { list: 'jobs' } }, data);
  const target = new URL(navigations[0], 'https://hireferry.test').searchParams;
  assert.equal(target.get('people_page'), '2');
  assert.equal(target.get('people_status'), 'suspended');
  assert.equal(target.has('jobs_page'), false);
  assert.equal(target.get('job_q'), 'Designer');
  assert.equal(target.get('job_status'), 'published');
});

test('changing accounts while graph relations load discards the old account response', async () => {
  let release, started;
  const pending = new Promise(resolve => { release = resolve; });
  const loading = new Promise(resolve => { started = resolve; });
  const { ui, state } = fixture({ beforeRequest: async path => {
    if (path.startsWith('/api/admin/workbench/matches?')) { started(); await pending; }
  } });
  const first = ui.page();
  await loading;
  state.user = { id: 'another-account' }; state.csrf = 'new-csrf';
  release();
  assert.equal(await first, '');
});

test('a delayed relation response cannot replace a newer graph page', async () => {
  let release, started, requests = 0;
  const pending = new Promise(resolve => { release = resolve; });
  const loading = new Promise(resolve => { started = resolve; });
  const { ui } = fixture({ beforeRequest: async path => {
    if (path.startsWith('/api/admin/workbench/matches?') && ++requests === 1) { started(); await pending; }
  } });
  const first = ui.page();
  await loading;
  const second = await ui.page();
  assert.match(second, /data-graph-page="2"/);
  release();
  assert.equal(await first, '');
});

test('profile, role and introduction labels are escaped', async () => {
  const bad = '<img src=x onerror=alert(1)>';
  const { ui } = fixture({ data: { people: list([{ ...people[0], name: bad, email: bad }]), jobs: list([{ ...jobs[0], title: bad }]), matches: { items: [{ ...matches[0], seeker_name: bad, job_title: bad }], total: 1 } } });
  const html = await ui.page();
  assert.match(html, /&lt;img/);
  assert.doesNotMatch(html, /<img src=x/);
});

test('leaving the workspace while requests are pending suppresses the stale graph', async () => {
  let release;
  const state = { admin: true, owner: true, csrf: 'owner' };
  globalThis.location = new URL('https://hireferry.test/admin?tab=matches');
  const blocker = new Promise(resolve => { release = resolve; });
  const ui = createPeopleJobsUI({ state, api: async () => { await blocker; return list([]); } });
  const pending = ui.page();
  ui.destroy(); release();
  assert.equal(await pending, '');
});

test('saving a follow-up opened from the graph refreshes its current filters instead of changing views', async () => {
  const serviceSource = await readFile(new URL('../static/service-ui.js', import.meta.url), 'utf8');
  const { createServiceUI } = await import('data:text/javascript;base64,' + Buffer.from(serviceSource).toString('base64'));
  const events = [];
  const service = createServiceUI({ state: { admin: true }, render: async () => events.push('render'), go: async url => events.push(url) });
  globalThis.location = new URL('https://hireferry.test/admin?tab=matches&people_q=Alex&jobs_page=2');
  await service.savedFollowup();
  assert.deepEqual(events, ['render']);
  assert.equal(location.search, '?tab=matches&people_q=Alex&jobs_page=2');
  globalThis.location = new URL('https://hireferry.test/admin?tab=matches&view=followups&status=interview&matches_cursor=page-two');
  await service.savedFollowup();
  assert.equal(events[1], '/admin?tab=matches&view=followups&status=interview');
});

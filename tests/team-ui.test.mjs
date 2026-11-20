import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createNavigationHarness } from './helpers/navigation-harness.mjs';
import { fileURLToPath } from 'node:url';

const source = await readFile(new URL('../static/team-ui.js', import.meta.url), 'utf8');
const { createTeamUI, teamInvitationPath, rememberTeamInvitation, savedTeamInvitation } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const staffSource = await readFile(new URL('../static/staff-ui.js', import.meta.url), 'utf8');
const { createStaffUI } = await import('data:text/javascript;base64,' + Buffer.from(staffSource).toString('base64'));
const origin = 'https://hireferry.test';
const token = 'a'.repeat(43);
const path = `/team/invite#${token}`;
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const memory = () => {
  const values = new Map();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) };
};
const invite = changes => ({ status: 'pending', expires_at: 1900000000, name: 'Team colleague', email_hint: 'co***@example.test', email_matches: true, role: 'staff', ...changes });
const team = changes => ({ members: [], invitations: [], smtp_enabled: true, invitation_ttl_hours: 24, ai_token_ttl_days: 15, max_members: 100, ...changes });

function fixture(api, changes = {}, withManagers = false) {
  globalThis.location = new URL(origin + path);
  globalThis.sessionStorage = memory();
  const state = { user: { id: 'colleague', email: 'colleague@example.test' }, admin: false, super_admin: false, render: 1, lang: 'en', ...changes };
  const events = [];
  const dependencies = { state, api, t: (_, en) => en, esc: escape, date: value => String(value ?? '—'),
    icon: name => `<i>${name}</i>`, heading: (title, detail, action = '') => `<h1>${title}</h1><p>${detail}</p>${action}`,
    shell: content => content, notice: text => `<aside>${text}</aside>`, empty: (title, detail) => `<h2>${title}</h2><p>${detail}</p>`,
    openDialog: html => events.push(['dialog', html]), confirmDialog: (...args) => events.push(['confirm', ...args]),
    notify: text => events.push(['notify', text]), go: value => events.push(['go', value]),
    render: async () => events.push(['render']), session: async () => events.push(['session']), resetPrivateView: () => events.push(['reset']),
  };
  const managerUI = withManagers ? createStaffUI(dependencies) : null;
  const ui = createTeamUI({ ...dependencies, managerUI });
  return { ui, managerUI, state, events };
}

test('only complete same-origin fragment invitations can survive sign-in', () => {
  assert.equal(teamInvitationPath(path, origin), path);
  assert.equal(teamInvitationPath(origin + path, origin), path);
  for (const value of ['/team/invite?token=' + token, '/team/invite#short', '/team/invite/' + token, '//evil.test' + path, 'https://evil.test' + path, 'https://user:password@hireferry.test' + path, '/team/invite?return_to=/admin#' + token, '/team/invite#%2F' + token, '/admin', 'javascript:alert(1)']) {
    assert.equal(teamInvitationPath(value, origin), null, value);
  }
  const storage = memory();
  assert.equal(rememberTeamInvitation(storage, path, 1000), true);
  assert.equal(savedTeamInvitation(storage, 1500), path);
  assert.equal(savedTeamInvitation(storage, 3601000), null);
  assert.equal(rememberTeamInvitation(storage, '/admin', 1000), false);
  assert.equal(savedTeamInvitation(null), null);
});

test('invitation preview never consumes the link or puts its secret in the login URL', async () => {
  const calls = [];
  const { ui } = fixture(async (url, options) => { calls.push([url, options]); return invite({ email_matches: null, name: '<script>bad()</script>' }); }, { user: null });
  const html = await ui.reviewPage(token);
  assert.deepEqual(calls, [['/api/team/invitations/preview', { method: 'POST', body: { token } }]]);
  assert.match(html, /href="\/login"/);
  assert.doesNotMatch(html, new RegExp(token));
  assert.match(html, /&lt;script&gt;bad\(\)&lt;\/script&gt;/);
  assert.equal(ui.returnAfterLogin(), path);
});

test('the current email and unchecked confirmation are shown before joining', async () => {
  const { ui } = fixture(async () => invite());
  const html = await ui.reviewPage(token);
  assert.match(html, /colleague@example\.test/);
  assert.match(html, /name="consent" required/);
  assert.doesNotMatch(html, /name="consent"[^>]*checked/);
  assert.match(html, /data-page-key="team-invite:1"/);
  await assert.rejects(ui.submit({ dataset: { form: 'team-accept' } }, new FormData()), /Review the invitation again/);
});

test('a wrong email or owner session cannot accept an invitation', async () => {
  for (const scenario of [{ email_matches: false }, { super_admin: true }]) {
    const calls = [];
    const { ui } = fixture(async (url) => { calls.push(url); return invite({ email_matches: scenario.email_matches ?? true }); }, { super_admin: !!scenario.super_admin });
    const html = await ui.reviewPage(token);
    assert.doesNotMatch(html, /data-form="team-accept"/);
    assert.match(html, /data-action="team-switch-account"/);
    const form = new FormData(); form.set('consent', 'on');
    await assert.rejects(ui.submit({ dataset: { form: 'team-accept' } }, form));
    assert.deepEqual(calls, ['/api/team/invitations/preview']);
  }
});

test('accepted, revoked and expired invitations do not offer another acceptance', async () => {
  for (const status of ['accepted', 'revoked', 'expired']) {
    const { ui } = fixture(async () => invite({ status }));
    rememberTeamInvitation(globalThis.sessionStorage, path);
    const html = await ui.reviewPage(token);
    assert.doesNotMatch(html, /data-form="team-accept"/);
    assert.equal(ui.returnAfterLogin(), null);
  }
});

test('joining requires the currently reviewed invitation, then refreshes identity before entering the console', async () => {
  const calls = [];
  const { ui, events } = fixture(async (url, options) => { calls.push([url, options]); return url.endsWith('/preview') ? invite() : { member: { id: 'staff-one' } }; });
  await ui.reviewPage(token);
  const data = new FormData(); data.set('consent', 'on');
  globalThis.location = new URL(origin + '/team/invite#' + 'b'.repeat(43));
  await assert.rejects(ui.submit({ dataset: { form: 'team-accept' } }, data));
  assert.equal(calls.length, 1);
  globalThis.location = new URL(origin + path);
  await ui.submit({ dataset: { form: 'team-accept' } }, data);
  assert.deepEqual(calls[1], ['/api/team/invitations/accept', { method: 'POST', body: { token } }]);
  assert.deepEqual(events.slice(0, 2), [['session'], ['go', '/admin?tab=messages']]);
  assert.equal(ui.returnAfterLogin(), null);
  await assert.rejects(ui.submit({ dataset: { form: 'team-accept' } }, data));
  assert.equal(calls.length, 2);
});

test('disabled mail still permits link creation, and SMTP failure preserves the copyable link', async () => {
  const calls = [];
  const { ui, events } = fixture(async (url, options) => {
    calls.push([url, options]);
    return options ? { invitation_url: origin + path, delivery_status: 'failed' } : team({ smtp_enabled: false });
  }, { admin: true, super_admin: true });
  const page = await ui.teamPage();
  assert.match(page, /data-action="team-add-member"/);
  await ui.action('team-add-member', { dataset: {} });
  const html = events.find(event => event[0] === 'dialog')[1];
  assert.match(html, /name="send_email" disabled/);
  assert.match(html, /expires after 24 hours/);
  assert.match(html, /expires after 15 days/);
  const data = new FormData(); data.set('email', ' colleague@example.test '); data.set('send_email', 'on');
  await ui.submit({ dataset: { form: 'team-invite' } }, data);
  assert.equal(calls[1][1].body.send_email, false);
  assert.equal(calls[1][1].body.email, 'colleague@example.test');
  const dialog = events.filter(event => event[0] === 'dialog').at(-1)[1];
  assert.match(dialog, /email delivery failed/);
  assert.match(dialog, /id="team-invitation-secret"/);
  assert.match(dialog, new RegExp(token));
});

test('AI staff receive a separate one-time credential with an explicit expiry', async () => {
  const calls = [];
  const { ui, events } = fixture(async (url, options) => { calls.push([url, options]); return { member: { id: 'ai-one', name: '<AI>' }, token: 'hf_staff_private-token', expires_at: 1900000000 }; }, { admin: true, super_admin: true });
  const data = new FormData(); data.set('name', ' Follow-up AI ');
  await ui.submit({ dataset: { form: 'team-ai' } }, data);
  assert.deepEqual(calls, [['/api/admin/team/ai', { method: 'POST', body: { name: 'Follow-up AI' } }]]);
  const html = events.find(event => event[0] === 'dialog')[1];
  assert.match(html, /shown once/);
  assert.match(html, /Expires/);
  assert.match(html, /&lt;AI&gt;/);
  assert.match(html, /Authorization: Bearer &lt;token&gt;/);
});

test('staff cannot initiate team management from UI action or form handlers', async () => {
  let calls = 0;
  const { ui } = fixture(async () => { calls += 1; }, { admin: true, super_admin: false });
  await assert.rejects(ui.submit({ dataset: { form: 'team-ai' } }, new FormData()), /Only the super administrator/);
  await assert.rejects(ui.action('team-rotate-ai', { dataset: { id: 'ai-one' } }), /Only the super administrator/);
  assert.equal(calls, 0);
});

test('late credentials cannot reopen a secret after the administrator leaves or signs out', async () => {
  for (const change of ['logout', 'navigate']) {
    let resolve;
    const pending = new Promise(done => { resolve = done; });
    const { ui, state, events } = fixture(async () => pending, { admin: true, super_admin: true, csrf: 'owner-csrf' });
    const data = new FormData(); data.set('name', 'AI');
    const submitted = ui.submit({ dataset: { form: 'team-ai' } }, data);
    if (change === 'logout') { state.super_admin = false; state.csrf = null; }
    else { state.render += 1; globalThis.location = new URL(origin + '/admin?tab=messages'); }
    resolve({ token: 'must-never-appear', member: { name: 'AI' } });
    await submitted;
    assert.deepEqual(events, []);
  }
});

test('staff navigation shows their identity and blocks owner pages without fetching owner data', async () => {
  const harness = await createNavigationHarness(fileURLToPath(new URL('../static', import.meta.url)));
  try {
    harness.overrides.set('/api/session', { user: { id: 'person-one', role: 'unassigned', email: 'colleague@example.test' }, admin: true, super_admin: false, staff: { id: 'staff-one', name: 'Colleague', kind: 'human', active: true }, bootstrap: false, csrf: 'staff-csrf' });
    await harness.prime();
    for (const tab of ['team', 'keys', 'agents']) {
      await harness.route(`/admin?tab=${tab}`, 'This page is managed by the super administrator');
      assert.match(harness.root.innerHTML, /Colleague/);
      assert.doesNotMatch(harness.root.innerHTML, /href="\/admin\?tab=(team|keys|agents)"/);
      assert.doesNotMatch(harness.root.innerHTML, /href="\/oauth\/connections"/);
    }
    assert.equal(harness.network.some(call => ['/api/admin/team', '/api/admin/passkeys', '/api/admin/tokens'].includes(call.path)), false);
  } finally { harness.dispose(); }
});

test('manager and employee sessions retain different business access without becoming owners', async () => {
  const user = { id: 'colleague', name: 'QA colleague', role: 'unassigned', email: 'colleague@example.test' };
  for (const employee of [false, true]) {
    const harness = await createNavigationHarness(fileURLToPath(new URL('../static', import.meta.url)));
    try {
      harness.overrides.set('/api/session', { user, admin: employee, super_admin: false, staff_role: employee ? null : 'manager', staff: employee ? { id: 'employee-one', name: 'QA colleague', kind: 'human' } : null, csrf: 'shared-session-csrf' });
      await harness.prime();
      assert.equal(harness.app.state.admin, true);
      assert.equal(harness.app.state.owner, false);
      assert.equal(harness.app.state.super_admin, false);
      harness.overrides.set('/api/admin/relations/query', options => {
        const query = JSON.parse(options.body);
        const base = { data_rev: 1, query_key: 'fixture', limit: 100, remaining: 0, next_cursor: null };
        if (query.mode === 'overview') return { ...base, summary: { users: 1, seekers: 1, jobs: 1, edges: 0, state_counts: {} }, groups: { left: [{ id: 'seekers', label: 'Seekers', count: 1, edges: 0 }], right: [{ id: 'published', label: 'Published', count: 1, edges: 0 }] }, bundles: [] };
        const person = { id: 'customer', kind: 'user', name: 'QA customer', label: 'QA customer', role: 'seeker', group_id: 'seekers', relation_count: 0 };
        const job = { id: 'job', kind: 'job', title: 'QA role', label: 'QA role', status: 'published', salary_cents: 10000, headcount: 1, group_id: 'published', relation_count: 0 };
        return { ...base, items: query.side === 'left' ? [person] : [job], total: 1 };
      });
      await harness.route('/admin?tab=users&graph_mode=list', 'QA customer');
      assert.equal(harness.root.innerHTML.includes('data-action="admin-user-access"'), employee);
      await harness.route('/admin?tab=jobs&graph_mode=list', 'QA role');
      assert.match(harness.root.innerHTML, new RegExp(`data-action="admin-edit-job"[^>]*>${employee ? 'Manage' : 'Details'}`));
      const forbidden = ['team', 'staff', 'keys', 'agents', 'promos', ...(!employee ? ['overview', 'orders'] : [])];
      for (const tab of forbidden) {
        await harness.route(`/admin?tab=${tab}`, 'This page is managed by the super administrator');
        for (const hidden of forbidden) assert.doesNotMatch(harness.root.innerHTML, new RegExp(`href="/admin\\?tab=${hidden}"`));
      }
      for (const tab of ['overview', 'orders']) assert.equal(harness.root.innerHTML.includes(`href="/admin?tab=${tab}"`), employee);
      assert.equal(harness.network.some(call => /^\/api\/admin\/(team|staff|passkeys|tokens|promos|overview|orders)(\/|\?|$)/.test(call.path)), false);
    } finally { harness.dispose(); }
  }
});

test('root sessions use one team entry for managers, staff and AI, including the legacy route', async () => {
  const harness = await createNavigationHarness(fileURLToPath(new URL('../static', import.meta.url)));
  try {
    harness.overrides.set('/api/session', { user: null, admin: true, super_admin: true, csrf: 'root-csrf' });
    await harness.prime();
    await harness.route('/admin?tab=team', 'data-action="team-add-member"');
    await harness.route('/admin?tab=staff', 'data-action="team-add-member"');
    assert.equal(harness.app.state.owner, true);
    assert.doesNotMatch(harness.root.innerHTML, /href="\/admin\?tab=staff"/);
    for (const tab of ['team', 'keys', 'agents', 'promos']) assert.match(harness.root.innerHTML, new RegExp(`href="/admin\\?tab=${tab}"`));
    assert.ok(harness.network.some(call => call.path === '/api/admin/team'));
    assert.ok(harness.network.some(call => call.path === '/api/admin/staff'));
  } finally { harness.dispose(); }
});

test('accepting either invitation cancels old private reads and reloads the cached session', async () => {
  for (const employee of [false, true]) {
    let release, started;
    const blocked = new Promise(resolve => { release = resolve; });
    const pendingRead = new Promise(resolve => { started = resolve; });
    const harness = await createNavigationHarness(fileURLToPath(new URL('../static', import.meta.url)), { beforeResponse: path => { if (path === '/api/profile') { started(); return blocked; } } });
    try {
      await harness.prime();
      const canceled = assert.rejects(harness.app.api('/api/profile'), { name: 'AbortError' });
      await pendingRead;
      await harness.app.api(employee ? '/api/team/invitations/accept' : `/api/staff/invitations/${token}/accept`, { method: 'POST', body: employee ? { token } : {} });
      await canceled;
      harness.overrides.set('/api/session', { user: { id: 'colleague', role: 'unassigned' }, admin: employee, super_admin: false, staff_role: employee ? null : 'manager', staff: employee ? { id: 'employee-one' } : null, csrf: 'new-csrf' });
      await harness.app.session(harness.app.state.render);
      assert.equal(harness.app.state.admin, true);
      assert.equal(harness.app.state.owner, false);
      assert.equal(harness.app.state.staffRole, employee ? null : 'manager');
      assert.equal(harness.app.state.staff?.id || null, employee ? 'employee-one' : null);
      assert.ok(harness.network.some(call => call.path === '/api/profile' && call.aborted));
    } finally { release(); harness.dispose(); }
  }
});


test('one team workspace keeps all three roles and invitation systems in one roster', async () => {
  const calls = [];
  const managerData = { members: [{ user_id: 'manager-one', name: '<Manager>', email: 'manager@example.test', active: true }], invitations: [{ id: 'manager-invite', email: 'invited@example.test', expires_at: Date.now() / 1000 + 3600 }] };
  const { ui, managerUI, events } = fixture(async (url, options) => {
    calls.push([url, options]);
    if (options) return { invite_url: origin + '/staff/invite/' + token, sent: false };
    return url === '/api/admin/staff' ? managerData : team({ smtp_enabled: false, members: [{ id: 'staff-one', kind: 'human', name: 'Colleague', email: 'colleague@example.test', active: true }, { id: 'ai-one', kind: 'ai', name: 'Assistant', active: true }], invitations: [{ id: 'staff-invite', status: 'pending', email: 'colleague2@example.test' }] });
  }, { super_admin: true, csrf: 'owner-csrf' }, true);
  const html = await ui.teamPage();
  assert.deepEqual(calls.map(call => call[0]).sort(), ['/api/admin/staff', '/api/admin/team']);
  for (const role of ['human', 'manager', 'ai']) assert.match(html, new RegExp(`data-team-kind="${role}"`));
  assert.match(html, /&lt;Manager&gt;/);
  assert.match(html, /Enabled identities<\/span><strong>3/);
  assert.match(html, /Pending invitations<\/span><strong>2/);
  for (const action of ['team-toggle-member', 'team-rotate-ai', 'staff-toggle', 'team-revoke-invite', 'staff-revoke']) assert.match(html, new RegExp(`data-action="${action}"`));
  assert.equal((html.match(/data-action="team-add-member"/g) || []).length, 1);
  assert.doesNotMatch(html, /data-form="(team-invite|team-ai|staff-invite)"/);
  await ui.action('team-add-member', { dataset: {} });
  const dialog = events.find(event => event[0] === 'dialog')[1];
  for (const form of ['team-invite', 'team-ai', 'staff-invite']) assert.match(dialog, new RegExp(`data-form="${form}"`));
  assert.match(dialog, /data-team-add-kind="manager" hidden/);
  assert.match(dialog, /id="staff-send-email" name="send_email" disabled/);
  assert.doesNotMatch(dialog, /<select/);
  const data = new FormData(); data.set('email', ' manager@example.test '); data.set('role', 'super_admin');
  await managerUI.submit({ dataset: { form: 'staff-invite' } }, data);
  assert.deepEqual(calls.at(-1), ['/api/admin/staff/invitations', { method: 'POST', body: { email: 'manager@example.test', role: 'manager', send_email: false } }]);
  assert.match(events.filter(event => event[0] === 'dialog').at(-1)[1], /id="team-invitation-secret"/);
});

test('role filters update members and invitations together without another request', async () => {
  let calls = 0;
  const { ui } = fixture(async () => { calls += 1; return {}; }, { super_admin: true }, true);
  const sections = [['human', 'manager', 'ai'], ['human', 'manager']].map(kinds => {
    const rows = kinds.map(kind => ({ dataset: { teamKind: kind }, hidden: false }));
    const count = { textContent: String(rows.length) }, empty = { hidden: true };
    return { rows, count, empty, querySelectorAll: () => rows, querySelector: selector => selector === '[data-team-visible-count]' ? count : empty };
  });
  const choices = ['all', 'human', 'manager', 'ai'].map(kind => ({ dataset: { kind }, setAttribute(name, value) { this[name] = value; } }));
  const workspace = { querySelectorAll: selector => selector === '[data-team-section]' ? sections : choices };
  await ui.action('team-filter', { dataset: { kind: 'ai' }, closest: () => workspace });
  assert.deepEqual(sections[0].rows.map(row => row.hidden), [true, true, false]);
  assert.deepEqual(sections[1].rows.map(row => row.hidden), [true, true]);
  assert.equal(sections[0].count.textContent, '1');
  assert.equal(sections[1].count.textContent, '0');
  assert.equal(sections[1].empty.hidden, false);
  assert.equal(choices[3]['aria-pressed'], 'true');
  await ui.action('team-filter', { dataset: { kind: 'all' }, closest: () => workspace });
  assert.equal(sections.every(section => section.rows.every(row => !row.hidden)), true);
  assert.equal(calls, 0);
});

test('changing the add-member identity preserves fields and only exposes its own form', async () => {
  const { ui } = fixture(async () => assert.fail('Identity selection must not make a request'), { super_admin: true }, true);
  const choices = ['human', 'manager', 'ai'].map(kind => ({ dataset: { kind }, setAttribute(name, value) { this[name] = value; } }));
  const forms = choices.map(choice => ({ dataset: { teamAddKind: choice.dataset.kind }, hidden: choice.dataset.kind !== 'human', value: 'unsent draft' }));
  const dialog = { querySelectorAll: selector => selector === '[data-team-add-kind]' ? forms : choices };
  await ui.action('team-add-kind', { dataset: { kind: 'manager' }, closest: () => dialog });
  assert.deepEqual(forms.map(form => form.hidden), [true, false, true]);
  assert.equal(forms[0].value, 'unsent draft');
  assert.equal(choices[1]['aria-pressed'], 'true');
});

test('manager invitation secrets cannot reopen after navigation or sign-out', async () => {
  for (const change of ['logout', 'navigate']) {
    let resolve;
    const pending = new Promise(done => { resolve = done; });
    const { managerUI, state, events } = fixture(async () => pending, { super_admin: true, csrf: 'owner-csrf' }, true);
    const data = new FormData(); data.set('email', 'manager@example.test');
    const submitted = managerUI.submit({ dataset: { form: 'staff-invite' } }, data);
    if (change === 'logout') { state.super_admin = false; state.csrf = null; }
    else { state.render += 1; globalThis.location = new URL(origin + '/admin?tab=messages'); }
    resolve({ invite_url: origin + '/staff/invite/' + token, sent: false });
    await submitted;
    assert.deepEqual(events, []);
  }
  const { managerUI } = fixture(async () => assert.fail('Non-owner must not make a write request'), {}, true);
  await assert.rejects(managerUI.submit({ dataset: { form: 'staff-invite' } }, new FormData()), /Only the super administrator/);
  await assert.rejects(managerUI.action('staff-toggle', { dataset: {} }), /Only the super administrator/);
});

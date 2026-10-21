import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../static/humangrant-ui.js', import.meta.url), 'utf8');
const { createHumanGrantUI, humanReviewPath, rememberHumanReview, savedHumanReview } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const origin = 'https://hireferry.test';
const link = 'a'.repeat(43);
const path = `/human/${link}`;
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const memory = () => {
  const data = new Map();
  return { getItem: key => data.get(key) ?? null, setItem: (key, value) => data.set(key, value), removeItem: key => data.delete(key) };
};
const now = Math.floor(Date.now() / 1000);
const permission = { action: 'profile.read', resource: 'self' };
const writePermission = { action: 'profile.write', resource: 'self' };
const request = changes => ({
  request_id: 'request-one', phase: 'waiting_human', actor_id: 'ed25519:actor-one',
  client_id: 'hireferry-web', target_service: 'hireferry', purpose: 'Read my profile',
  human_id: 'person-one', human_name: 'A person', expected_human_id: null,
  requested_permissions: [permission, writePermission], grantable_permissions: [permission],
  expires_at: now + 600, requested_grant_expires_at: now + 3600, checked_at: now, csrf: 'review-csrf',
  ...changes,
});

function fixture(api, { state: changes = {}, ...extra } = {}) {
  globalThis.location = { origin };
  globalThis.sessionStorage = memory();
  const state = { config: { humangrant_enabled: true }, user: { id: 'person-one', role: 'seeker', name: 'A person' }, lang: 'en', admin: false, ...changes };
  const events = [];
  const ui = createHumanGrantUI({ state, api, t: (_, en) => en, esc: escape,
    icon: name => `<i data-icon="${name}"></i>`, notice: text => `<aside>${text}</aside>`,
    empty: (title, text) => `<h2>${title}</h2><p>${text}</p>`, notify: text => events.push(['notify', text]),
    go: path => events.push(['go', path]), render: async () => events.push(['render']), session: async () => {},
    confirmDialog() {}, ...extra,
  });
  return { ui, state, events };
}

function approvalData() {
  const data = new FormData();
  data.append('permission', '0');
  data.set('consent', 'on');
  data.set('expires_at', String(now + 900));
  return data;
}

test('sign-in return accepts only a complete same-site review path and expires', () => {
  assert.equal(humanReviewPath(path, origin), path);
  assert.equal(humanReviewPath(origin + path, origin), path);
  for (const bad of ['//evil.test' + path, 'https://evil.test' + path, origin + path + '?return=//evil.test', origin + path + '#other', origin + '/human/%2fadmin', origin + '/human/short', 'javascript:alert(1)', 'https://user:secret@hireferry.test' + path, '/app']) {
    assert.equal(humanReviewPath(bad, origin), null, bad);
  }
  const storage = memory();
  assert.equal(rememberHumanReview(storage, path, 1000), true);
  assert.equal(savedHumanReview(storage, 1500), path);
  assert.equal(savedHumanReview(storage, 1000 + 3600000), null);
  assert.equal(rememberHumanReview(storage, '/admin', 1000), false);
  assert.equal(savedHumanReview(null), null);
  assert.equal(rememberHumanReview(null, path), false);
});

test('request text is escaped, permissions are opt-in, and unavailable permissions cannot be selected', async () => {
  const { ui } = fixture(async () => request({ purpose: '<script>steal()</script>', human_name: '<img src=x onerror=bad()>', actor_id: 'actor <svg onload=bad()>' }));
  const html = await ui.reviewPage(link);
  assert.match(html, /&lt;script&gt;steal\(\)&lt;\/script&gt;/);
  assert.match(html, /&lt;img src=x onerror=bad\(\)&gt;/);
  assert.match(html, /actor &lt;svg onload=bad\(\)&gt;/);
  assert.doesNotMatch(html, /<script>|<img src=x|<svg onload/);
  assert.doesNotMatch(html, /<input[^>]*\bchecked\b/);
  assert.match(html, /name="permission" value="0" >/);
  assert.match(html, /name="permission" value="1" disabled/);
  assert.match(html, /name="consent" required/);
});

test('approval sends only selected permissions with the review-specific token and expiry', async () => {
  const calls = [];
  const { ui, events } = fixture(async (url, options) => {
    calls.push({ url, ...options });
    return options?.method ? { phase: 'approved' } : request();
  });
  await ui.reviewPage(link);
  await ui.submit({ dataset: { form: 'hg-approve', link } }, approvalData());
  assert.equal(calls.length, 2);
  assert.equal(calls[1].url, `/api/humangrant/review/${link}/approve`);
  assert.equal(calls[1].method, 'POST');
  assert.deepEqual(calls[1].body, { permissions: [permission], expires_at: now + 900 });
  assert.deepEqual(calls[1].headers, { 'X-HG-CSRF': 'review-csrf' });
  assert.deepEqual(events[0], ['render']);
});

test('missing consent, empty permissions, or an unavailable permission cannot submit approval', async () => {
  let writes = 0;
  const { ui } = fixture(async (_, options) => { if (options?.method) writes++; return request(); });
  await ui.reviewPage(link);
  const form = { dataset: { form: 'hg-approve', link } };
  const noConsent = approvalData(); noConsent.delete('consent');
  await assert.rejects(ui.submit(form, noConsent), /Confirm your approval/);
  const noPermissions = approvalData(); noPermissions.delete('permission');
  await assert.rejects(ui.submit(form, noPermissions), /Select at least one/);
  const unavailable = approvalData(); unavailable.set('permission', '1');
  await assert.rejects(ui.submit(form, unavailable), /unavailable/);
  assert.equal(writes, 0);
});

test('an account mismatch and a finished request never show an approval form', async () => {
  const mismatch = fixture(async () => request({ expected_human_id: 'another-person', csrf: null, grantable_permissions: [] }));
  const wrongHtml = await mismatch.ui.reviewPage(link);
  assert.match(wrongHtml, /This request is for another account/);
  assert.doesNotMatch(wrongHtml, /data-form="hg-approve"/);
  for (const phase of ['expired', 'rejected', 'cancelled', 'approved', 'succeeded']) {
    const { ui } = fixture(async () => request({ phase, csrf: null }));
    const html = await ui.reviewPage(link);
    assert.doesNotMatch(html, /data-form="hg-approve"/, phase);
    assert.doesNotMatch(html, /data-action="hg-deny"/, phase);
  }
});

test('the review uses the server-checked sign-in and remembers the link without approving', async () => {
  const calls = [];
  const { ui } = fixture(async (url, options) => { calls.push(options?.method || 'GET'); return request({ human_id: null, human_name: null, csrf: null, grantable_permissions: [] }); });
  const html = await ui.reviewPage(link);
  assert.match(html, /Sign in to review/);
  assert.doesNotMatch(html, /data-form="hg-approve"/);
  assert.equal(ui.savedReturn(), path);
  assert.deepEqual(calls, ['GET']);
});

test('revocation is confirmed and uses the grant-list token with the chosen grant ID', async () => {
  const calls = [];
  let confirm;
  const { ui } = fixture(async (url, options) => {
    calls.push({ url, ...options });
    return { human_id: 'person-one', csrf: 'list-csrf', grants: [
      { grant_id: 'grant-current', actor_id: 'actor-one', permissions: [permission], created_at: now, expires_at: now + 3600 },
      { grant_id: 'grant-revoked', actor_id: 'actor-two', permissions: [permission], created_at: now, expires_at: now + 3600, revoked_at: now },
      { grant_id: 'grant-expired', actor_id: 'actor-three', permissions: [permission], created_at: now - 100, expires_at: now - 1 },
    ] };
  }, { confirmDialog: (_title, _text, action) => { confirm = action; } });
  const html = await ui.grantsPanel();
  assert.match(html, /data-action="hg-revoke" data-id="grant-current"/);
  assert.doesNotMatch(html, /data-action="hg-revoke" data-id="grant-(revoked|expired)"/);
  await ui.action('hg-revoke', { dataset: { id: 'grant-current' } });
  assert.equal(calls.length, 1);
  await confirm();
  assert.equal(calls[1].url, '/api/humangrant/grants/grant-current');
  assert.equal(calls[1].method, 'DELETE');
  assert.deepEqual(calls[1].headers, { 'X-HG-CSRF': 'list-csrf' });
  await assert.rejects(ui.action('hg-revoke', { dataset: { id: 'other-user-grant' } }), /Refresh the access list/);
});

test('the new sign-in option uses the exact HumanGrant label and respects server configuration', () => {
  const { ui } = fixture(async () => ({}));
  assert.match(ui.loginButton(), /Continue with human beings/);
  assert.match(ui.loginButton(), /href="\/humangrant"/);
  const disabled = fixture(async () => { throw new Error('must not load'); }, { state: { config: { humangrant_enabled: false } } });
  assert.equal(disabled.ui.loginButton(), '');
  assert.match(disabled.ui.intro(), /HumanGrant is not available/);
});


test('retained navigation gives each request and signed-in account a fresh consent form', async () => {
  let person = 'person-one';
  const { ui } = fixture(async () => request({ human_id: person }));
  const first = await ui.reviewPage(link);
  const second = await ui.reviewPage('b'.repeat(43));
  person = 'person-two';
  const otherAccount = await ui.reviewPage(link);
  const pageKey = html => html.match(/<main[^>]*data-page-key="([^"]+)"/)[1];
  assert.notEqual(pageKey(first), pageKey(second));
  assert.notEqual(pageKey(first), pageKey(otherAccount));
  assert.doesNotMatch(second + otherAccount, /<input[^>]*\bchecked\b/);
});

test('late review responses cannot replace the active request or its saved login return', async () => {
  let releaseOld;
  const calls = [];
  const currentLink = 'b'.repeat(43);
  const { ui, state } = fixture(async (url, options) => {
    if (options?.method) { calls.push({ url, ...options }); return { phase: 'approved' }; }
    if (url.endsWith(link)) return new Promise(resolve => { releaseOld = resolve; });
    return request({ csrf: 'current-review-csrf' });
  }, { state: { render: 1 } });
  const obsolete = ui.reviewPage(link, 1);
  state.render = 2;
  await ui.reviewPage(currentLink, 2);
  releaseOld(request({ human_id: null, csrf: null, grantable_permissions: [] }));
  assert.equal(await obsolete, '');
  assert.equal(ui.savedReturn(), null);
  await ui.submit({ dataset: { form: 'hg-approve', link: currentLink } }, approvalData());
  assert.equal(calls[0].url, `/api/humangrant/review/${currentLink}/approve`);
  assert.equal(calls[0].headers['X-HG-CSRF'], 'current-review-csrf');
  state.render = 3;
  await assert.rejects(ui.submit({ dataset: { form: 'hg-approve', link: currentLink } }, approvalData()), /Open the review page again/);
  assert.equal(calls.length, 1);
});

test('late grant lists cannot replace the current list or its revocation token', async () => {
  let releaseOld, confirm;
  let reads = 0;
  const writes = [];
  const grants = (id, csrf) => ({ csrf, grants: [{ grant_id: id, actor_id: 'actor-one', permissions: [permission], created_at: now, expires_at: now + 3600 }] });
  const { ui, state } = fixture(async (url, options) => {
    if (options?.method) { writes.push({ url, ...options }); return {}; }
    if (++reads === 1) return new Promise(resolve => { releaseOld = resolve; });
    return grants('current-grant', 'current-list-csrf');
  }, { state: { render: 1 }, confirmDialog: (_title, _text, action) => { confirm = action; } });
  const obsolete = ui.grantsPanel();
  state.render = 2;
  await ui.grantsPanel();
  releaseOld(grants('old-grant', 'old-list-csrf'));
  assert.equal(await obsolete, '');
  await assert.rejects(ui.action('hg-revoke', { dataset: { id: 'old-grant' } }), /Refresh the access list/);
  await ui.action('hg-revoke', { dataset: { id: 'current-grant' } });
  await confirm();
  assert.equal(writes[0].url, '/api/humangrant/grants/current-grant');
  assert.equal(writes[0].headers['X-HG-CSRF'], 'current-list-csrf');
});

test('a newly signed-in member completes role selection before returning to the same review', async () => {
  const { ui, state } = fixture(async () => request({ human_id: null, csrf: null }), { state: { user: null } });
  await ui.reviewPage(link);
  state.user = { id: 'person-one', role: 'unassigned' };
  assert.equal(ui.returnAfterLogin(), '/app');
  assert.equal(ui.savedReturn(), path);
  state.user.role = 'seeker';
  assert.equal(ui.returnAfterLogin(), path);
  ui.clearReturn();
  assert.equal(ui.returnAfterLogin(), '/app');
});

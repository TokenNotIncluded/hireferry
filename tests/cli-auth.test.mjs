import test from 'node:test';
import assert from 'node:assert/strict';
import { CLI_RETURN_KEY, safeCliPath, rememberCliReturn, takeCliReturn } from '../static/cli-auth-return.js';
import { createHumanGrantUI, rememberHumanReview, savedHumanReview } from '../static/humangrant-ui.js';

const origin = 'https://hireferry.example';
const clock = 1000000;
const member = {user: {role: 'seeker'}, admin: false, bootstrap: false};
const admin = {user: null, admin: true, bootstrap: false};
function storage() {
  const values = new Map();
  return {getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key)};
}

test('CLI continuation permits only the same-site consent and connection pages', () => {
  assert.equal(safeCliPath('/oauth/device?user_code=ABCD-EFGH', origin), '/oauth/device?user_code=ABCD-EFGH');
  assert.equal(safeCliPath(origin + '/oauth/authorize?state=demo', origin), '/oauth/authorize?state=demo');
  assert.equal(safeCliPath('/oauth/connections', origin), '/oauth/connections');
  for (const path of ['//evil.example/oauth/device', 'https://hireferry.example.evil/oauth/device', 'javascript:alert(1)', 'https://user@hireferry.example/oauth/device', '/login?next=/oauth/device', '/oauth/device#fragment', '/oauth/device/extra', '/oauth/%64evice', '/\\evil.example/oauth/device']) {
    assert.equal(safeCliPath(path, origin), null, path);
  }
});

test('sign-in continuation is single-use and basic identity needs no AI role', () => {
  const store = storage();
  assert.equal(rememberCliReturn(store, '/oauth/device?user_code=DEMO', origin, false, clock), true);
  assert.equal(takeCliReturn(store, origin, {}, false, clock), null);
  assert.equal(takeCliReturn(store, origin, {user: {role: 'unassigned'}}, true, clock), '/oauth/device?user_code=DEMO');
  assert.equal(takeCliReturn(store, origin, member, false, clock), null);
});

test('an existing admin session does not skip requested passkey reauthentication', () => {
  const store = storage();
  rememberCliReturn(store, '/oauth/authorize?state=demo', origin, true, clock);
  assert.equal(takeCliReturn(store, origin, admin, false, clock), null);
  assert.equal(takeCliReturn(store, origin, {...admin, bootstrap: true}, true, clock), null);
  assert.equal(takeCliReturn(store, origin, admin, true, clock), '/oauth/authorize?state=demo');
});

test('expired, cross-origin and invalid continuations cannot redirect a user', () => {
  const store = storage();
  rememberCliReturn(store, '/oauth/device', origin, false, clock);
  assert.equal(takeCliReturn(store, origin, member, false, clock + 600000), null);
  assert.equal(store.getItem(CLI_RETURN_KEY), null);
  store.setItem(CLI_RETURN_KEY, JSON.stringify({path: 'https://evil.example/oauth/device', expiresAt: clock + 1}));
  assert.equal(takeCliReturn(store, origin, member, true, clock), null);
  store.setItem(CLI_RETURN_KEY, '{broken');
  assert.equal(takeCliReturn(store, origin, member, true, clock), null);
  store.setItem(CLI_RETURN_KEY, JSON.stringify({path: '/oauth/device', expiresAt: clock + 600001}));
  assert.equal(takeCliReturn(store, origin, member, true, clock), null);
});

test('CLI sign-in and HumanGrant preserve independent pending consent requests', () => {
  const store = storage();
  const now = Date.now();
  const review = '/human/' + 'a'.repeat(43);
  const state = {config: {humangrant_enabled: true}, user: {role: 'unassigned'}, admin: false, bootstrap: false};
  const previousStorage = globalThis.sessionStorage;
  try {
    globalThis.sessionStorage = store;
    const human = createHumanGrantUI({state});
    rememberHumanReview(store, review, now);
    rememberCliReturn(store, '/oauth/authorize?state=demo', origin, false, now);

    // Completing the CLI login returns to its consent page once. It must not
    // consume or approve the separate HumanGrant request in the same browser.
    assert.equal(takeCliReturn(store, origin, state, true, now), '/oauth/authorize?state=demo');
    assert.equal(takeCliReturn(store, origin, state, true, now), null);
    assert.equal(savedHumanReview(store, now), review);
    assert.equal(human.returnAfterLogin(), '/app');

    // HumanGrant retains its existing role-selection behavior, then resumes
    // its original review. Clearing that review cannot erase a new CLI return.
    state.user.role = 'seeker';
    assert.equal(human.returnAfterLogin(), review);
    rememberCliReturn(store, '/oauth/connections', origin, false, now);
    human.clearReturn();
    assert.equal(savedHumanReview(store, now), null);
    assert.equal(takeCliReturn(store, origin, state, true, now), '/oauth/connections');
  } finally {
    if (previousStorage === undefined) delete globalThis.sessionStorage;
    else globalThis.sessionStorage = previousStorage;
  }
});

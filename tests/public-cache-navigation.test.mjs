import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createNavigationHarness, fixtureConfig } from './helpers/navigation-harness.mjs';

const directory = fileURLToPath(new URL('../static/', import.meta.url));
const key = 'hireferry-public-config-v1';
const publicConfig = (extra = {}) => ({ ...structuredClone(fixtureConfig), public_origin: 'https://hireferry.test', support_email: 'support@example.test', cli_distribution: { release_repository: 'TokenNotIncluded/hireferry', release_tag_prefix: 'cli-v' }, ...extra });
const stored = value => [[key, JSON.stringify({ schema: 1, savedAt: Date.now(), value })]];
const deferred = () => { let resolve; const promise = new Promise(yes => { resolve = yes; }); return { promise, resolve }; };
const turn = () => new Promise(resolve => setTimeout(resolve, 1));
async function until(predicate) {
  for (let i = 0; i < 500; i++) { if (predicate()) return; await turn(); }
  assert.ok(predicate(), 'Expected request or rendered state was not reached');
}
const reads = (h, path) => h.network.filter(request => request.path === path);
const ready = h => h.mounts.filter(mount => !mount.busy);
async function finish(h, gates, navigation) {
  gates.forEach(gate => gate.resolve());
  await navigation;
  await until(() => h.network.every(request => request.finished !== null));
  await turn();
  h.dispose();
}

test('cached CLI renders before either startup response, then changed commands replace the snapshot', async () => {
  const config = deferred(), session = deferred();
  const cached = publicConfig({ public_origin: 'https://previous.hireferry.test' });
  const fresh = publicConfig({ public_origin: 'https://current.hireferry.test' });
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(cached), beforeResponse: path => path === '/api/config' ? config.promise : path === '/api/session' ? session.promise : undefined });
  let navigation;
  try {
    h.setUser(null); h.overrides.set('/api/config', fresh);
    assert.equal(h.app.state.configCached, true);
    assert.equal(h.document.documentElement.lang, 'en');
    assert.equal(h.document.title, `${cached.name} · People-first hiring`);
    navigation = h.app.go('/cli');
    await until(() => reads(h, '/api/config').length && reads(h, '/api/session').length);
    assert.match(h.root.innerHTML, /cli-guide/);
    assert.match(h.root.innerHTML, /https:\/\/previous\.hireferry\.test\/install\.sh/);
    assert.equal(reads(h, '/api/config')[0].finished, null);
    assert.equal(reads(h, '/api/session')[0].finished, null);
    assert.equal(ready(h).length, 1);
    config.resolve(); session.resolve(); await navigation;
    await until(() => h.root.innerHTML.includes('https://current.hireferry.test/install.sh'));
    assert.doesNotMatch(h.root.innerHTML, /previous\.hireferry\.test|route-error/);
    assert.equal(ready(h).length, 2);
    assert.equal(h.app.state.configCached, false);
    assert.equal(h.root.dataset.configState, 'fresh');
    assert.equal(JSON.parse(h.localStorage.getItem(key)).value.public_origin, fresh.public_origin);
  } finally { await finish(h, [config, session], navigation); }
});

test('unchanged public configuration and guest identity do not mount the page twice', async () => {
  const config = deferred(), current = publicConfig();
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(current), beforeResponse: path => path === '/api/config' ? config.promise : undefined });
  let navigation;
  try {
    h.setUser(null); h.overrides.set('/api/config', structuredClone(current));
    navigation = h.app.go('/cli'); await navigation;
    const published = h.root.innerHTML;
    assert.equal(ready(h).length, 1);
    config.resolve(); await until(() => h.root.dataset.configState === 'fresh');
    assert.equal(ready(h).length, 1);
    assert.equal(h.root.innerHTML, published);
    assert.equal(reads(h, '/api/config').length, 1);
  } finally { await finish(h, [config], navigation); }
});

test('failed public revalidation keeps the complete cached page instead of a route error', async () => {
  const config = deferred(), cached = publicConfig();
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(cached), beforeResponse: path => path === '/api/config' ? config.promise : undefined });
  let navigation;
  try {
    h.setUser(null); h.overrides.set('/api/config', { httpStatus: 503, body: { error: 'Temporarily unavailable' } });
    navigation = h.app.go('/cli'); await navigation;
    const published = h.root.innerHTML;
    config.resolve(); await until(() => h.root.dataset.configState === 'stale');
    assert.equal(h.root.innerHTML, published);
    assert.match(h.root.innerHTML, /cli-guide/);
    assert.doesNotMatch(h.root.innerHTML, /route-error/);
    assert.equal(ready(h).length, 1);
    assert.equal(h.app.configCache.peek().public_origin, cached.public_origin);
  } finally { await finish(h, [config], navigation); }
});

test('failed configuration revalidation still applies a successful session in either response order', async () => {
  for (const first of ['session', 'config']) {
    const config = deferred(), session = deferred(), cached = publicConfig();
    const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(cached), beforeResponse: path => path === '/api/config' ? config.promise : path === '/api/session' ? session.promise : undefined });
    let navigation;
    try {
      h.overrides.set('/api/config', { httpStatus: 503, body: { error: 'Temporarily unavailable' } });
      navigation = h.app.go('/cli'); await navigation;
      const body = h.root.innerHTML.match(/<main\b[\s\S]*<\/main>/)[0];
      assert.match(h.root.innerHTML, /href="\/login" data-nav>Get started/);
      (first === 'session' ? session : config).resolve();
      await until(() => reads(h, `/api/${first}`).some(request => request.finished !== null));
      assert.equal(reads(h, `/api/${first === 'session' ? 'config' : 'session'}`)[0].finished, null);
      (first === 'session' ? config : session).resolve();
      await until(() => h.app.state.user?.id === 'fixture-user' && h.root.dataset.configState === 'stale');
      assert.match(h.root.innerHTML, /href="\/app" data-nav>My workspace/);
      assert.equal(h.root.innerHTML.match(/<main\b[\s\S]*<\/main>/)[0], body);
      assert.doesNotMatch(h.root.innerHTML, /route-error/);
      assert.equal(h.app.state.configCached, true);
      assert.deepEqual(h.app.configCache.peek(), cached);
      assert.equal(ready(h).length, 2);
      assert.equal(reads(h, '/api/config')[0].aborted, false);
      assert.equal(reads(h, '/api/session').length, 1);
    } finally { await finish(h, [config, session], navigation); }
  }
});

test('a cold public page needs configuration but paints before its session check completes', async () => {
  const config = deferred(), session = deferred();
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, beforeResponse: path => path === '/api/config' ? config.promise : path === '/api/session' ? session.promise : undefined });
  let navigation;
  try {
    h.setUser(null); h.overrides.set('/api/config', publicConfig());
    navigation = h.app.go('/contact');
    await until(() => reads(h, '/api/config').length);
    assert.doesNotMatch(h.root.innerHTML, /class="prose contact-page"/);
    config.resolve();
    await until(() => reads(h, '/api/session').length);
    assert.match(h.root.innerHTML, /class="prose contact-page"/);
    assert.match(h.root.innerHTML, /support@example\.test/);
    assert.equal(reads(h, '/api/session')[0].finished, null);
    assert.equal(ready(h).length, 1);
    await navigation;
    session.resolve(); await until(() => h.root.dataset.configState === 'fresh');
    assert.equal(ready(h).length, 1);
  } finally { await finish(h, [config, session], navigation); }
});

test('login after a cold public paint waits for the shared session and redirects an existing member', async () => {
  for (const continuation of [null, '/oauth/device?user_code=ABCD-EFGH']) {
    const session = deferred();
    const h = await createNavigationHarness(directory, { useFixtureConfig: false, beforeResponse: path => path === '/api/session' ? session.promise : undefined });
    let navigation;
    try {
      h.overrides.set('/api/config', publicConfig());
      await h.app.go('/contact');
      await until(() => reads(h, '/api/session').length);
      assert.match(h.root.innerHTML, /class="prose contact-page"/);
      assert.equal(h.app.state.configCached, false);
      assert.equal(h.app.state.user, null);
      assert.equal(reads(h, '/api/session')[0].finished, null);
      if (continuation) h.localStorage.setItem('hireferry-cli-return', JSON.stringify({ path: continuation, expiresAt: Date.now() + 600000, afterAuthOnly: false }));
      navigation = h.app.go('/login');
      assert.equal(h.location.pathname, '/login');
      assert.equal(h.network.some(request => request.path.startsWith('/api/chat/')), false);
      session.resolve(); await navigation;
      if (continuation) {
        assert.equal(h.location.pathname + h.location.search, continuation);
        assert.equal(h.localStorage.getItem('hireferry-cli-return'), null);
        assert.equal(h.network.some(request => request.path.startsWith('/api/chat/')), false);
      } else {
        await until(() => h.location.pathname === '/app' && h.root.innerHTML.includes('<p>QA_MESSAGE_READY</p>'));
      }
      assert.equal(h.app.state.user.id, 'fixture-user');
      assert.doesNotMatch(h.root.innerHTML, /data-form="email-start"|contact-page|route-error/);
      assert.equal(reads(h, '/api/session').length, 1);
      assert.equal(reads(h, '/api/config').length, 1);
    } finally { await finish(h, [session], navigation); }
  }
});

test('persistent public configuration never unlocks a private route before live config and session checks', async () => {
  const config = deferred(), session = deferred();
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(publicConfig()), beforeResponse: path => path === '/api/config' ? config.promise : path === '/api/session' ? session.promise : undefined });
  let navigation;
  try {
    h.overrides.set('/api/config', publicConfig());
    navigation = h.app.go('/app?tab=profile');
    await until(() => reads(h, '/api/config').length && reads(h, '/api/session').length);
    assert.doesNotMatch(h.root.innerHTML, /QA_PROFILE_READY/);
    assert.equal(reads(h, '/api/profile').length, 0);
    assert.equal(ready(h).length, 0);
    session.resolve(); await until(() => reads(h, '/api/session')[0].finished !== null);
    assert.equal(reads(h, '/api/config')[0].finished, null);
    assert.equal(reads(h, '/api/profile').length, 0);
    assert.doesNotMatch(h.root.innerHTML, /QA_PROFILE_READY/);
    config.resolve(); await navigation;
    assert.match(h.root.innerHTML, /QA_PROFILE_READY/);
    assert.doesNotMatch(h.root.innerHTML, /route-error/);
    assert.equal(h.app.state.configCached, false);
    assert.equal(reads(h, '/api/session').length, 1);
    assert.equal(reads(h, '/api/config')[0].aborted, false);
  } finally { await finish(h, [config, session], navigation); }
});

test('rapid public navigation shares refresh reads and never republishes the old route', async () => {
  const config = deferred(), session = deferred();
  const fresh = publicConfig({ support_email: 'current-support@example.test' });
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(publicConfig()), beforeResponse: path => path === '/api/config' ? config.promise : path === '/api/session' ? session.promise : undefined });
  let navigation;
  try {
    h.setUser(null); h.overrides.set('/api/config', fresh);
    await h.app.go('/cli');
    const transition = h.mounts.length;
    navigation = h.app.go('/contact'); await navigation;
    assert.match(h.root.innerHTML, /class="prose contact-page"/);
    config.resolve(); session.resolve();
    await until(() => h.root.innerHTML.includes('current-support@example.test'));
    assert.equal(h.root.dataset.pageKey, '/contact');
    assert.doesNotMatch(h.root.innerHTML, /cli-guide|route-error/);
    assert.ok(h.mounts.slice(transition).every(mount => mount.pageKey === '/contact'));
    assert.equal(reads(h, '/api/config').length, 1);
    assert.equal(reads(h, '/api/session').length, 1);
  } finally { await finish(h, [config, session], navigation); }
});

test('cross-tab identity reset cancels a stale public refresh and cannot restore the prior session', async () => {
  const oldConfig = deferred(); let configReads = 0;
  const fresh = publicConfig({ public_origin: 'https://after-reset.hireferry.test' });
  const h = await createNavigationHarness(directory, { useFixtureConfig: false, initialStorage: stored(publicConfig()), beforeResponse: path => path === '/api/config' && ++configReads === 1 ? oldConfig.promise : undefined });
  let navigation;
  try {
    h.overrides.set('/api/config', fresh);
    navigation = h.app.go('/cli'); await navigation;
    await until(() => reads(h, '/api/session').some(request => request.finished !== null));
    assert.equal(reads(h, '/api/config')[0].finished, null);
    h.setUser(null); h.dispatch('storage', { key: 'hireferry-session-change' });
    await until(() => h.root.innerHTML.includes('https://after-reset.hireferry.test/install.sh'));
    assert.equal(h.app.state.user, null);
    assert.equal(h.root.dataset.configState, 'fresh');
    assert.equal(reads(h, '/api/config')[0].aborted, true);
    const published = h.root.innerHTML;
    oldConfig.resolve(); await turn();
    assert.equal(h.root.innerHTML, published);
    assert.equal(h.app.state.user, null);
    assert.doesNotMatch(h.root.innerHTML, /route-error/);
    const cached = JSON.parse(h.localStorage.getItem(key));
    assert.equal(cached.value.public_origin, fresh.public_origin);
    assert.equal('user' in cached.value, false);
  } finally { await finish(h, [oldConfig], navigation); }
});

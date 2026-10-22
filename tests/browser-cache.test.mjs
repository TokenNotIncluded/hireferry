import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../static/browser-cache.js', import.meta.url), 'utf8');
const { createPublicConfigCache, PUBLIC_CONFIG_CACHE_KEY } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const configuration = (name = 'HireFerry') => ({ name, default_language: 'zh', public_origin: 'https://hireferry.example', providers: [], cli_distribution: { release_repository: 'TokenNotIncluded/hireferry', release_tag_prefix: 'cli-v' }, pricing: { currency: 'CNY', allowed_currencies: ['CNY'], onboarding_cents: 10000 }, limits: { list_page_size: 50 } });
const storage = () => { const values = new Map(); return { values, getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) }; };
const deferred = () => { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };

test('a reload can read public content synchronously before a slow revalidation completes', async () => {
  const disk = storage();
  await createPublicConfigCache(async () => configuration(), { storage: disk }).refresh();
  const gate = deferred();
  const cache = createPublicConfigCache(() => gate.promise, { storage: disk });
  const snapshot = cache.peek();
  assert.equal(snapshot.name, 'HireFerry');
  assert.equal(cache.isVerified(), false);
  const refreshed = cache.refresh();
  assert.equal(cache.peek().name, 'HireFerry');
  gate.resolve(configuration('Updated HireFerry'));
  const result = await refreshed;
  assert.equal(result.previous.name, 'HireFerry');
  assert.equal(result.value.name, 'Updated HireFerry');
  assert.equal(result.changed, true);
  assert.equal(cache.isVerified(), true);
  assert.equal(createPublicConfigCache(() => {}, { storage: disk }).peek().name, 'Updated HireFerry');
});

test('simultaneous refreshes share one request and return independent data', async () => {
  const gate = deferred(); let calls = 0;
  const cache = createPublicConfigCache(() => { calls++; return gate.promise; }, { storage: storage() });
  const first = cache.refresh(), second = cache.refresh();
  gate.resolve(configuration());
  const [a, b] = await Promise.all([first, second]);
  assert.equal(calls, 1);
  assert.equal(a.changed, false);
  a.value.pricing.onboarding_cents = 1;
  assert.equal(b.value.pricing.onboarding_cents, 10000);
  const snapshot = cache.peek(); snapshot.pricing.onboarding_cents = 2;
  assert.equal(cache.peek().pricing.onboarding_cents, 10000);
});

test('an unchanged response in a different object-key order does not animate', async () => {
  const first = configuration(), second = Object.fromEntries(Object.entries(first).reverse());
  second.pricing = Object.fromEntries(Object.entries(first.pricing).reverse());
  let next = first;
  const cache = createPublicConfigCache(async () => next, { storage: storage() });
  await cache.refresh(); next = second;
  assert.equal((await cache.refresh()).changed, false);
  next = { ...second, support_email: 'support@example.test' };
  assert.equal((await cache.refresh()).changed, true);
  next = first;
  assert.equal((await cache.refresh()).changed, true, 'removing a setting counts as a change');
});

test('network failure keeps the last good snapshot, rejects, and allows a later retry', async () => {
  let fail = false;
  const cache = createPublicConfigCache(async () => { if (fail) throw new Error('Offline'); return configuration(); }, { storage: storage() });
  await cache.refresh(); fail = true;
  await assert.rejects(cache.refresh(), /Offline/);
  assert.equal(cache.peek().name, 'HireFerry');
  fail = false;
  assert.equal((await cache.refresh()).value.name, 'HireFerry');
});

test('expired data, future timestamps, schema changes and corrupted JSON are discarded', async () => {
  const disk = storage(); let clock = 100;
  const cache = createPublicConfigCache(async () => configuration(), { storage: disk, now: () => clock, maxAge: 1000 });
  await cache.refresh();
  const good = disk.getItem(PUBLIC_CONFIG_CACHE_KEY);
  clock = 1100;
  assert.equal(cache.peek(), null);
  assert.equal(cache.isVerified(), false);
  assert.equal(disk.getItem(PUBLIC_CONFIG_CACHE_KEY), null);
  for (const invalid of [good, '{broken', JSON.stringify({ schema: 999, savedAt: clock, value: configuration() }), JSON.stringify({ schema: 1, savedAt: clock + 1, value: configuration() }), JSON.stringify({ schema: 1, savedAt: clock, value: { ...configuration(), pricing: { currency_options: 'not an array' } } }), 'x'.repeat(512 * 1024 + 1)]) {
    disk.setItem(PUBLIC_CONFIG_CACHE_KEY, invalid);
    const reloaded = createPublicConfigCache(() => {}, { storage: disk, now: () => clock, maxAge: 1000 });
    assert.equal(reloaded.peek(), null);
    assert.equal(disk.getItem(PUBLIC_CONFIG_CACHE_KEY), null);
  }
});

test('disabled storage and a full quota do not break successful reads', async () => {
  for (const disk of [null, { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('full'); }, removeItem() { throw new Error('blocked'); } }]) {
    const cache = createPublicConfigCache(async () => configuration(), { storage: disk });
    assert.equal(cache.peek(), null);
    await cache.refresh();
    assert.equal(cache.peek().name, 'HireFerry');
    cache.invalidate();
    assert.equal(cache.peek(), null);
  }
});

test('only explicit public fields are stored; account and token objects never reach storage', async () => {
  const disk = storage();
  const result = { ...configuration(), csrf: 'secret-csrf', user: { email: 'private@example.test' }, access_token: 'secret-token', messages: [{ body: 'private conversation' }] };
  const cache = createPublicConfigCache(async () => result, { storage: disk });
  const refreshed = await cache.refresh();
  assert.deepEqual(refreshed.value, configuration());
  assert.doesNotMatch(disk.getItem(PUBLIC_CONFIG_CACHE_KEY), /secret|private|access_token|messages/);
  for (const privateResponse of [{ user: { name: 'Member' }, csrf: 'secret' }, { items: [] }]) {
    await assert.rejects(createPublicConfigCache(async () => privateResponse, { storage: disk }).refresh(), /Invalid public configuration/);
  }
});

test('invalidation prevents an old response from restoring stale settings or replacing a newer request', async () => {
  const gate = deferred(); let calls = 0;
  const disk = storage();
  const cache = createPublicConfigCache(() => ++calls === 1 ? gate.promise : configuration('Fresh'), { storage: disk });
  const old = cache.refresh(); await Promise.resolve();
  cache.invalidate();
  const next = await cache.refresh();
  gate.resolve(configuration('Obsolete'));
  await assert.rejects(old, { name: 'AbortError' });
  assert.equal(next.value.name, 'Fresh');
  assert.equal(cache.peek().name, 'Fresh');
  assert.equal(JSON.parse(disk.getItem(PUBLIC_CONFIG_CACHE_KEY)).value.name, 'Fresh');
});

test('a malformed server response never replaces a usable snapshot', async () => {
  let response = configuration();
  const cache = createPublicConfigCache(async () => response, { storage: storage() });
  await cache.refresh();
  response = { ...configuration('Bad'), limits: { list_page_size: 'many' } };
  await assert.rejects(cache.refresh(), /Invalid public limits/);
  assert.equal(cache.peek().name, 'HireFerry');
});

test('localized chat welcomes survive fresh loads and reloads without storing conversation data', async () => {
  const disk = storage();
  const welcome = { title: '欢迎', title_en: 'Welcome', body_md: '**说明**', body_html: '<p><strong>说明</strong></p>', body_md_en: null, prompts: [{ label: '聊聊方向', body: '我想聊聊求职方向。', label_en: 'My next role', body_en: 'My next role.', private_token: 'secret' }] };
  const cache = createPublicConfigCache(async () => ({ ...configuration(), chat: { welcome: { platform_seeker: { ...welcome, messages: ['private'] }, matched: { prompts: [] } }, conversations: ['private'] } }), { storage: disk });
  const fresh = (await cache.refresh()).value.chat;
  assert.equal(fresh.welcome.platform_seeker.title_en, 'Welcome');
  assert.equal(fresh.welcome.platform_seeker.body_html, welcome.body_html);
  assert.equal(fresh.welcome.platform_seeker.prompts[0].body_en, 'My next role.');
  assert.deepEqual(fresh.welcome.matched.prompts, []);
  assert.deepEqual(createPublicConfigCache(() => {}, { storage: disk }).peek().chat, fresh);
  assert.doesNotMatch(disk.getItem(PUBLIC_CONFIG_CACHE_KEY), /private|secret|conversations|messages/);
});

test('invalid welcome fields are rejected while bounded multilingual settings can exceed the old cache size', async () => {
  for (const welcome of [{ typo: {} }, { staff: { title: 42 } }, { staff: { prompts: [{ label: 'Hello', body: '' }] } }, { matched: { prompts: Array(7).fill({ label: 'Hello', body: 'World' }) } }]) {
    await assert.rejects(createPublicConfigCache(async () => ({ ...configuration(), chat: { welcome } }), { storage: storage() }).refresh(), /Invalid public chat/);
  }
  const welcome = Object.fromEntries(['platform_seeker', 'platform_employer', 'matched', 'staff'].map(key => [key, { body_md: '好'.repeat(4000), body_md_en: 'a'.repeat(4000), prompts: Array(6).fill({ label: 'Ask', body: '好'.repeat(2000), body_en: 'a'.repeat(2000) }) }]));
  const disk = storage();
  await createPublicConfigCache(async () => ({ ...configuration(), chat: { welcome } }), { storage: disk }).refresh();
  assert.ok(disk.getItem(PUBLIC_CONFIG_CACHE_KEY).length > 64 * 1024);
  assert.deepEqual(createPublicConfigCache(() => {}, { storage: disk }).peek().chat.welcome, welcome);
});

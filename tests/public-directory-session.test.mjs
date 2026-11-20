import test from 'node:test';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
import {createNavigationHarness, fixtureConfig} from './helpers/navigation-harness.mjs';

const directory = fileURLToPath(new URL('../static/', import.meta.url));
const configKey = 'hireferry-public-config-v1';
const config = extra => ({...structuredClone(fixtureConfig), public_origin:'https://hireferry.test', support_email:'support@example.test', ...extra});
const savedConfig = value => [[configKey, JSON.stringify({schema:1, savedAt:Date.now(), value})]];
const deferred = () => { let resolve; const promise = new Promise(yes => {resolve = yes;}); return {promise, resolve}; };
const turn = () => new Promise(resolve => setTimeout(resolve, 1));
async function until(predicate) {
  for (let i = 0; i < 500; i++) {if (predicate()) return; await turn();}
  assert.ok(predicate(), 'Expected request or rendered state was not reached');
}
const reads = (h, path) => h.network.filter(request => request.path === path);
const main = html => html.match(/<main\b[\s\S]*<\/main>/)?.[0];
const person = {id:'qa-person', handle:'qa-person', name:'PUBLIC_PERSON_READY', resume_html:'<p>Public résumé fixture.</p>', ai_profile_links:{}, created_at:1800000000};
const job = {id:'qa-role', title:'PUBLIC_ROLE_READY', jd_html:'<p>Public role fixture.</p>', employer:{name:'Example employer'}, salary_cents:1000000, currency:'CNY', currency_decimals:2, headcount:1, created_at:1800000000};
const routes = [
  {path:'/discover?q=Rust', endpoint:'/api/public/jobs?q=Rust', response:{items:[job], total:1, limit:50}, marker:job.title},
  {path:'/discover?tab=people&q=Rust', endpoint:'/api/public/people?q=Rust', response:{items:[person], total:1, limit:50}, marker:person.name},
  {path:'/discover/jobs/qa-role', endpoint:'/api/public/jobs/qa-role', response:job, marker:job.title},
  {path:'/discover/people/qa-person', endpoint:'/api/public/people/qa-person', response:person, marker:person.name},
  {path:'/u/qa-person', endpoint:'/api/public-profiles/qa-person', response:{profile:person}, marker:person.name},
];
async function dispose(h, gates) {
  gates.forEach(gate => gate.resolve());
  await until(() => h.network.every(request => request.finished !== null));
  await turn(); h.dispose();
}

test('cold public directory and profile routes paint before the session and then show the signed-in header', async () => {
  for (const route of routes) {
    const session = deferred();
    const h = await createNavigationHarness(directory, {useFixtureConfig:false, beforeResponse:path => path === '/api/session' ? session.promise : undefined});
    try {
      h.overrides.set('/api/config', config()); h.overrides.set(route.endpoint, route.response);
      await h.app.go(route.path);
      assert.ok(h.root.innerHTML.includes(route.marker), route.path);
      assert.match(h.root.innerHTML, /href="\/login" data-nav>Get started/);
      const content = main(h.root.innerHTML);
      await until(() => reads(h, '/api/session').length === 1);
      assert.equal(reads(h, '/api/session')[0].finished, null, 'public content does not wait for identity');
      session.resolve();
      await until(() => h.root.innerHTML.includes('href="/app" data-nav>My workspace'));
      assert.equal(h.app.state.user.id, 'fixture-user');
      assert.equal(main(h.root.innerHTML), content, 'identity refresh retains the public result');
      assert.equal(reads(h, route.endpoint).length, 1, 'identity refresh does not fetch public content twice');
      assert.equal(reads(h, '/api/config').length, 1);
      assert.equal(h.root.dataset.configState, 'fresh');
    } finally {await dispose(h, [session]);}
  }
});

test('cached directory configuration revalidates without replacing an edited search or expanded content', async () => {
  const configuration = deferred(), session = deferred();
  const h = await createNavigationHarness(directory, {useFixtureConfig:false, initialStorage:savedConfig(config({name:'Earlier site'})), beforeResponse:path => path === '/api/config' ? configuration.promise : path === '/api/session' ? session.promise : undefined});
  try {
    h.overrides.set('/api/config', config({name:'Updated site', support_email:'updated@example.test'}));
    h.overrides.set(routes[0].endpoint, routes[0].response);
    await h.app.go(routes[0].path);
    await until(() => reads(h, '/api/config').length && reads(h, '/api/session').length);
    // The HTML sink simulates body changes; navigation.test.mjs checks the real
    // reconciler preserves exact nodes and live input values for this option.
    h.root.innerHTML = h.root.innerHTML.replace('value="Rust"', 'value="Unsaved search draft"').replace('<details class="public-mcp-tools">', '<details class="public-mcp-tools" open>');
    const edited = main(h.root.innerHTML);
    configuration.resolve(); session.resolve();
    await until(() => h.root.dataset.configState === 'fresh');
    assert.match(h.root.innerHTML, /updated@example\.test/);
    assert.match(h.root.innerHTML, /href="\/app" data-nav>My workspace/);
    assert.equal(main(h.root.innerHTML), edited);
    assert.equal(reads(h, routes[0].endpoint).length, 1);
    assert.equal(JSON.parse(h.localStorage.getItem(configKey)).value.name, 'Updated site');
  } finally {await dispose(h, [configuration, session]);}
});

test('failed configuration revalidation still applies public directory identity and preserves its content', async () => {
  const h = await createNavigationHarness(directory, {useFixtureConfig:false, initialStorage:savedConfig(config())});
  try {
    h.overrides.set('/api/config', {httpStatus:503, body:{error:'Temporarily unavailable'}});
    h.overrides.set(routes[1].endpoint, routes[1].response);
    await h.app.go(routes[1].path);
    const content = main(h.root.innerHTML);
    await until(() => h.root.dataset.configState === 'stale');
    assert.match(h.root.innerHTML, /href="\/app" data-nav>My workspace/);
    assert.equal(main(h.root.innerHTML), content);
    assert.doesNotMatch(h.root.innerHTML, /route-error/);
  } finally {await dispose(h, []);}
});

test('a delayed public identity refresh cannot restore a prior directory query or profile', async () => {
  const session = deferred();
  const h = await createNavigationHarness(directory, {useFixtureConfig:false, beforeResponse:path => path === '/api/session' ? session.promise : undefined});
  try {
    h.overrides.set('/api/config', config());
    h.overrides.set(routes[0].endpoint, routes[0].response);
    h.overrides.set(routes[1].endpoint, routes[1].response);
    await h.app.go(routes[0].path);
    await until(() => reads(h, '/api/session').length);
    await h.app.go(routes[1].path);
    const content = main(h.root.innerHTML);
    session.resolve();
    await until(() => h.root.innerHTML.includes('href="/app" data-nav>My workspace'));
    assert.equal(h.location.pathname + h.location.search, routes[1].path);
    assert.equal(main(h.root.innerHTML), content);
    assert.match(h.root.innerHTML, /PUBLIC_PERSON_READY/);
    assert.doesNotMatch(h.root.innerHTML, /PUBLIC_ROLE_READY/);
    assert.equal(reads(h, '/api/session').length, 1);
    assert.equal(reads(h, routes[1].endpoint).length, 1);
  } finally {await dispose(h, [session]);}
});

test('failed session lookup keeps public results available without showing signed-in controls', async () => {
  const h = await createNavigationHarness(directory, {useFixtureConfig:false});
  try {
    h.overrides.set('/api/config', config());
    h.overrides.set('/api/session', {httpStatus:503, body:{error:'Identity temporarily unavailable'}});
    h.overrides.set(routes[0].endpoint, routes[0].response);
    await h.app.go(routes[0].path);
    const content = main(h.root.innerHTML);
    await until(() => h.root.dataset.configState === 'fresh');
    assert.equal(main(h.root.innerHTML), content);
    assert.match(h.root.innerHTML, /href="\/login" data-nav>Get started/);
    assert.doesNotMatch(h.root.innerHTML, /route-error/);
    assert.equal(h.app.state.user, null);
  } finally {await dispose(h, []);}
});

test('detail metadata retains its public record after configuration and identity refresh', async () => {
  const configuration = deferred(), session = deferred();
  const h = await createNavigationHarness(directory, {useFixtureConfig:false, initialStorage:savedConfig(config({name:'Earlier site'})), beforeResponse:path => path === '/api/config' ? configuration.promise : path === '/api/session' ? session.promise : undefined});
  try {
    const createElement = h.document.createElement;
    h.document.createElement = tag => ({...createElement(tag), dataset:{}});
    const catalog = h.document.createElement('meta');
    catalog.setAttribute('name', 'hireferry-seo');
    catalog.content = JSON.stringify({'/discover':[['公开目录', '公开机会'], ['Directory', 'Public opportunities']]});
    h.document.head.append(catalog);
    h.overrides.set('/api/config', config({name:'Updated site'}));
    h.overrides.set(routes[3].endpoint, routes[3].response);
    await h.app.go(routes[3].path);
    assert.equal(h.document.title, 'PUBLIC_PERSON_READY · Earlier site');
    configuration.resolve(); session.resolve();
    await until(() => h.root.dataset.configState === 'fresh');
    assert.equal(h.document.title, 'PUBLIC_PERSON_READY · Updated site');
    assert.equal(h.document.querySelector('meta[name="robots"]').content, 'index, follow, max-image-preview:large');
    assert.equal(h.document.querySelector('link[rel="canonical"]').href, 'https://hireferry.test/discover/people/qa-person');
  } finally {await dispose(h, [configuration, session]);}
});

test('cross-tab sign-out during a directory refresh cannot restore the former identity', async () => {
  const firstConfiguration = deferred(); let configReads = 0;
  const h = await createNavigationHarness(directory, {useFixtureConfig:false, initialStorage:savedConfig(config()), beforeResponse:path => path === '/api/config' && ++configReads === 1 ? firstConfiguration.promise : undefined});
  try {
    h.overrides.set('/api/config', config());
    h.overrides.set(routes[0].endpoint, routes[0].response);
    await h.app.go(routes[0].path);
    await until(() => reads(h, '/api/session').some(request => request.finished !== null));
    h.setUser(null); h.dispatch('storage', {key:'hireferry-session-change'});
    firstConfiguration.resolve();
    await until(() => h.root.dataset.configState === 'fresh' && h.network.every(request => request.finished !== null));
    assert.equal(h.app.state.user, null);
    assert.match(h.root.innerHTML, /href="\/login" data-nav>Get started/);
    assert.doesNotMatch(h.root.innerHTML, /href="\/app" data-nav>My workspace|route-error/);
    assert.match(h.root.innerHTML, /PUBLIC_ROLE_READY/);
  } finally {await dispose(h, [firstConfiguration]);}
});

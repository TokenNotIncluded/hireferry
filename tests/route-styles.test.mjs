import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source = (await readFile(new URL('../static/route-styles.js', import.meta.url), 'utf8'))
  .replace("new URL('./', import.meta.url)", "new URL('./', 'https://hireferry.test/static/revision/route-styles.js')")
  .replace('export function routeStyles', 'function routeStyles')
  .replace('export async function ensureRouteStyles', 'async function ensureRouteStyles')
  + '\nglobalThis.__routeStyles = {routeStyles, ensureRouteStyles};';

function fixture() {
  const links = [];
  const document = {
    head: {
      querySelector(selector) {
        const match = selector.match(/(?:data-route-style|href\$=)\\?"?([^"\\)]+)"?$/);
        if (!match) return null;
        const name = match[1].replace(/\\"/g, '');
        return links.find(link => link.dataset.routeStyle === name || link.href.endsWith(`/${name}`)) || null;
      },
      append(link) {
        link.sheet = {};
        links.push(link);
      },
    },
    createElement() {
      return {
        dataset: {},
        addEventListener() {},
      };
    },
  };
  const context = {
    document,
    location: {pathname: '/', search: ''},
    window: {},
    URL,
    URLSearchParams,
    Promise,
    Map,
    Set,
    console,
  };
  vm.runInNewContext(source, context);
  return {routeStyles: context.__routeStyles, links, location: context.location};
}

test('route CSS is version-relative and deduplicated across SPA navigation', async () => {
  const fixtureState = fixture();
  const home = await fixtureState.routeStyles.ensureRouteStyles('/');
  assert.deepEqual([...home.failed], []);
  assert.deepEqual(fixtureState.links.map(link => link.dataset.routeStyle), [
    'promo.css',
    'particle-type.css',
    'brand-motion.css',
  ]);
  assert.ok(fixtureState.links.every(link => link.href.startsWith('https://hireferry.test/static/revision/')));

  await fixtureState.routeStyles.ensureRouteStyles('/discover');
  await fixtureState.routeStyles.ensureRouteStyles('/discover');
  assert.deepEqual(fixtureState.links.map(link => link.dataset.routeStyle), [
    'promo.css',
    'particle-type.css',
    'brand-motion.css',
    'people-jobs.css',
    'public-directory.css',
  ]);
});

test('both role detail routes load the shared interaction stylesheet', async () => {
  for (const path of ['/discover/jobs/role-1', '/jobs/role-1']) {
    const state = fixture();
    await state.routeStyles.ensureRouteStyles(path);
    assert.ok(state.links.some(link => link.dataset.routeStyle === 'job-interactions.css'));
    assert.ok(state.links.every(link => link.href.startsWith('https://hireferry.test/static/revision/')));
  }
  const state = fixture();
  await state.routeStyles.ensureRouteStyles('/discover');
  assert.ok(state.links.every(link => link.dataset.routeStyle !== 'job-interactions.css'));
});

test('route CSS mapping keeps login light and loads workspace chat styles by tab', async () => {
  const fixtureState = fixture();
  await fixtureState.routeStyles.ensureRouteStyles('/login');
  assert.deepEqual(fixtureState.links, []);

  fixtureState.links.length = 0;
  fixtureState.location.search = '?tab=messages';
  await fixtureState.routeStyles.ensureRouteStyles('/app');
  assert.deepEqual(fixtureState.links.map(link => link.dataset.routeStyle), [
    'workspace.css',
    'people-jobs.css',
    'promo.css',
    'conversation.css',
    'chat-renderer.css',
  ]);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { createRelationTools } from '../static/relation-webmcp.js';

const definition = { name: 'hf_view_patch', description: 'Change view', inputSchema: { type: 'object' } };
const deferred = () => { let resolve;const promise = new Promise(done => { resolve = done; });return { promise, resolve }; };
function context() {
  const tools = new Map(), calls = [];
  return { tools, calls, async registerTool(tool, { signal }) {
    assert.equal(signal.aborted, false);
    assert.equal(tools.has(tool.name), false, 'A tool must never be registered twice');
    calls.push(tool.name);tools.set(tool.name, tool);
    signal.addEventListener('abort', () => { if (tools.get(tool.name) === tool) tools.delete(tool.name); }, { once: true });
  } };
}
const owner = (browser, api, errors = []) => createRelationTools({ getContext: () => browser, api, onError: error => errors.push(error) });

test('a catalog arriving after an identity change or route departure cannot register private graph tools', async () => {
  for (const invalidate of ['identity', 'route']) {
    let current = true;
    const catalog = deferred(), browser = context(), registry = owner(browser, () => catalog.promise);
    const pending = registry.sync({ scope: 'owner:a', isCurrent: () => current });
    if (invalidate === 'identity') current = false;
    else await registry.clear();
    catalog.resolve({ tools: [definition] });await pending;
    assert.equal(browser.tools.size, 0);
  }
});

test('repeated language and route renders share pending discovery and keep the same six registrations', async () => {
  const catalog = deferred(), browser = context();let reads = 0;
  const registry = owner(browser, () => { reads++;return catalog.promise; });
  const first = registry.sync({ scope: 'owner:a' }), second = registry.sync({ scope: 'owner:a' });
  const names = ['hf_view_open', 'hf_view_patch', 'hf_graph_query', 'hf_graph_read', 'hf_view_save', 'hf_view_list'];
  catalog.resolve({ tools: names.map(name => ({ ...definition, name })) });
  await Promise.all([first, second]);
  await registry.sync({ scope: 'owner:a' });
  assert.equal(reads, 1);assert.deepEqual(browser.calls, names);
  assert.equal(browser.tools.size, 6);
});

test('the current identity owns discovery even if the previous identity responds last', async () => {
  const old = deferred(), next = deferred(), browser = context();let reads = 0;
  const registry = owner(browser, () => ++reads === 1 ? old.promise : next.promise);
  const first = registry.sync({ scope: 'owner:a' });await Promise.resolve();
  const second = registry.sync({ scope: 'owner:b' });await Promise.resolve();
  next.resolve({ tools: [{ ...definition, description: 'Current owner' }] });await second;
  old.resolve({ tools: [{ ...definition, description: 'Previous owner' }] });await first;
  assert.equal(browser.tools.get(definition.name).description, 'Current owner');
  assert.equal(browser.calls.length, 1);
});

test('browser AI uses the shared service and stale callbacks cannot send a mutation', async () => {
  let current = true;
  const browser = context(), calls = [];
  const registry = owner(browser, async (path, options) => {
    calls.push({ path, options });return path.endsWith('/tools') ? { tools: [definition] } : { view_rev: 2, delivery: 'queued' };
  });
  await registry.sync({ scope: 'owner:a', isCurrent: () => current });
  const tool = browser.tools.get(definition.name);
  const result = await tool.execute({ view_id: 'example', if_view_rev: 1 });
  assert.equal(JSON.parse(result.content[0].text).delivery, 'queued');
  assert.equal(calls[1].path, '/api/admin/relations/tools/hf_view_patch');
  assert.deepEqual(calls[1].options.body, { view_id: 'example', if_view_rev: 1 });
  current = false;
  assert.equal((await tool.execute({ view_id: 'example' })).isError, true);
  assert.equal(calls.length, 2);
  await registry.clear();assert.equal(browser.tools.size, 0);
});

test('revocation suppresses an already running private result and keeps unrelated tools', async () => {
  const browser = context(), work = deferred();let writes = 0;
  browser.tools.set('unrelated', { name: 'unrelated' });
  const registry = owner(browser, async path => path.endsWith('/tools') ? { tools: [definition] } : (writes++, work.promise));
  await registry.sync({ scope: 'owner:a' });
  const tool = browser.tools.get(definition.name), running = tool.execute({});
  await registry.clear();
  assert.deepEqual([...browser.tools.keys()], ['unrelated']);
  work.resolve({ private: 'must not be returned' });
  assert.equal((await running).isError, true);
  assert.equal((await tool.execute({})).isError, true);assert.equal(writes, 1);
});

test('failed discovery reports a diagnostic, leaves the workspace usable, and can retry', async () => {
  const browser = context(), errors = [];let reads = 0;
  const registry = owner(browser, async () => {
    if (++reads === 1) throw new Error('Unavailable');
    return { tools: [definition, { ...definition, name: 'foreign_tool' }] };
  }, errors);
  await registry.sync({ scope: 'owner:a' });
  assert.equal(errors.length, 1);assert.equal(errors[0].operation, 'discover');
  await registry.sync({ scope: 'owner:a' });
  assert.deepEqual([...browser.tools.keys()], [definition.name]);
});

test('browsers without WebMCP do not request the catalog', async () => {
  const registry = owner({}, () => assert.fail('No optional discovery needed'));
  await registry.sync({ scope: 'owner:a' });
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../static/relation-webmcp.js', import.meta.url), 'utf8');
const { registerRelationTools } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const definition = { name: 'hf_view_patch', description: 'Change view', inputSchema: { type: 'object' } };

test('a catalog arriving after an identity change cannot register private graph tools', async () => {
  let current = true, finish;
  const catalog = new Promise(resolve => { finish = resolve; });
  const registered = [];
  const pending = registerRelationTools({ context: { registerTool: tool => registered.push(tool) }, api: () => catalog, isCurrent: () => current, onRegistered: () => {} });
  current = false;
  finish({ tools: [definition] });
  await pending;
  assert.deepEqual(registered, []);
});

test('browser AI uses the shared service and stale executions cannot send a view mutation', async () => {
  let current = true, tool;
  const calls = [];
  await registerRelationTools({
    context: { registerTool: value => { tool = value; } }, isCurrent: () => current, onRegistered: () => {},
    api: async (path, options) => { calls.push({ path, options }); return path.endsWith('/tools') ? { tools: [definition] } : { view_rev: 2, delivery: 'queued' }; },
  });
  const result = await tool.execute({ view_id: 'example', if_view_rev: 1 });
  assert.equal(JSON.parse(result.content[0].text).delivery, 'queued');
  assert.equal(calls[1].path, '/api/admin/relations/tools/hf_view_patch');
  assert.deepEqual(calls[1].options.body, { view_id: 'example', if_view_rev: 1 });
  current = false;
  assert.equal((await tool.execute({ view_id: 'example' })).isError, true);
  assert.equal(calls.length, 2);
});

test('failure to discover optional browser AI does not break the human workspace', async () => {
  await registerRelationTools({ context: {}, api: async () => { throw new Error('Unavailable'); }, isCurrent: () => true, onRegistered: () => { assert.fail(); } });
});

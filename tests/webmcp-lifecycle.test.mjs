import assert from 'node:assert/strict';
import test from 'node:test';
import { createWebMCPOwner } from '../static/webmcp-lifecycle.js';

const deferred = () => { let resolve;const promise = new Promise(done => { resolve = done; });return { promise, resolve }; };
const definition = (name, execute = async () => name) => ({ name, description: name, inputSchema: { type: 'object', properties: {} }, execute });
function modern({ gate } = {}) {
  const tools = new Map(), calls = [];
  return {
    tools, calls,
    async registerTool(tool, { signal }) {
      calls.push(tool.name);
      if (gate) await gate.promise;
      if (signal.aborted) throw new DOMException('Aborted', 'AbortError');
      if (tools.has(tool.name)) throw new Error('Duplicate tool name');
      tools.set(tool.name, tool);
      signal.addEventListener('abort', () => { if (tools.get(tool.name) === tool) tools.delete(tool.name); }, { once: true });
    },
  };
}
function owner(context, errors = []) { return createWebMCPOwner({ getContext: () => context, onError: error => errors.push(error) }); }

test('route and language renders preserve registrations and active executions', async () => {
  const context = modern(), registry = owner(context), work = deferred();
  await registry.sync({ scope: 'owner:a', tools: [definition('read', () => work.promise)] });
  const running = context.tools.get('read').execute();
  await registry.sync({ scope: 'owner:a', tools: [definition('read')] });
  assert.deepEqual(context.calls, ['read']);
  work.resolve('completed');assert.equal(await running, 'completed');
});

test('identity change aborts pending registrations before adding the next catalog', async () => {
  const gate = deferred(), context = modern({ gate }), errors = [], registry = owner(context, errors);
  void registry.sync({ scope: 'owner:a', tools: [definition('admin')] });
  await Promise.resolve();
  void registry.sync({ scope: 'member:b', tools: [definition('profile')] });
  gate.resolve();await registry.idle();
  assert.deepEqual([...context.tools.keys()], ['profile']);assert.deepEqual(errors, []);
});

test('revocation is synchronous and old callbacks cannot run or return private data', async () => {
  const context = modern(), registry = owner(context), work = deferred();let called = 0;
  await registry.sync({ scope: 'owner:a', tools: [definition('admin', () => { called++;return work.promise; })] });
  const previous = context.tools.get('admin'), running = previous.execute();
  const cleanup = registry.clear();
  assert.equal(context.tools.size, 0);assert.equal(registry.isCurrent('owner:a'), false);
  assert.equal((await previous.execute()).isError, true);assert.equal(called, 1);
  work.resolve('private response');assert.equal((await running).isError, true);await cleanup;
});

test('independent owners do not unregister each other', async () => {
  const context = modern(), workspace = owner(context), relations = owner(context);
  await Promise.all([
    workspace.sync({ scope: 'owner:a', tools: [definition('workspace')] }),
    relations.sync({ scope: 'owner:a', tools: [definition('relations')] }),
  ]);
  await workspace.clear();assert.deepEqual([...context.tools.keys()], ['relations']);
  assert.equal(await context.tools.get('relations').execute(), 'relations');await relations.clear();
});

test('asynchronous registration rejection is reported without stealing foreign tools', async () => {
  const context = modern(), errors = [], registry = owner(context, errors), foreign = definition('foreign');
  context.tools.set('foreign', foreign);
  await registry.sync({ scope: 'owner:a', tools: [definition('foreign'), definition('owned')] });
  assert.equal(errors.length, 1);assert.equal(errors[0].operation, 'register');
  await registry.clear();assert.equal(context.tools.get('foreign'), foreign);assert.equal(context.tools.size, 1);
});

test('legacy asynchronous unregister completes before reusing a name', async () => {
  const tools = new Map(), removed = [], gate = deferred();
  const context = {
    registerTool(tool) { assert.equal(tools.has(tool.name), false);tools.set(tool.name, tool); },
    async unregisterTool(name) { await gate.promise;removed.push(name);tools.delete(name); },
  };
  const registry = owner(context);
  await registry.sync({ scope: 'owner:a', tools: [definition('shared', async () => 'a')] });
  const before = tools.get('shared');
  void registry.sync({ scope: 'owner:b', tools: [definition('shared', async () => 'b')] });
  assert.equal((await before.execute()).isError, true);
  gate.resolve();await registry.idle();
  assert.deepEqual(removed, ['shared']);assert.equal(await tools.get('shared').execute(), 'b');
});

test('failed legacy removal stays tracked and cannot cause a duplicate registration', async () => {
  const tools = new Map(), errors = [];let blocked = true, registrations = 0;
  const context = {
    registerTool(tool) { registrations++;assert.equal(tools.has(tool.name), false);tools.set(tool.name, tool); },
    unregisterTool(name) { if (blocked) throw new Error('Removal unavailable');tools.delete(name); },
  };
  const registry = owner(context, errors);
  await registry.sync({ scope: 'owner:a', tools: [definition('read')] });
  await registry.sync({ scope: 'owner:b', tools: [definition('read')] });
  assert.equal(registrations, 1);assert.equal((await tools.get('read').execute()).isError, true);
  assert.equal(errors.length, 1);blocked = false;await registry.clear();assert.equal(tools.size, 0);
  await registry.sync({ scope: 'owner:b', tools: [definition('read')] });assert.equal(registrations, 2);
});

test('context replacement releases the old context and refreshes schema changes', async () => {
  const first = modern(), second = modern();let context = first;
  const registry = createWebMCPOwner({ getContext: () => context });
  await registry.sync({ scope: 'member:a', tools: [definition('read')] });
  context = second;await registry.sync({ scope: 'member:a', tools: [definition('read')] });
  assert.equal(first.tools.size, 0);assert.equal(second.tools.size, 1);
  await registry.sync({ scope: 'member:a', tools: [{ ...definition('read'), description: 'Updated schema description' }] });
  assert.equal(second.tools.get('read').description, 'Updated schema description');
});

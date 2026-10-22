import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../static/content-refresh.js', import.meta.url), 'utf8');
const { animateContentRefresh } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

class Events {
  listeners = new Map();
  addEventListener(type, callback) { if (!this.listeners.has(type)) this.listeners.set(type, new Set()); this.listeners.get(type).add(callback); }
  removeEventListener(type, callback) { this.listeners.get(type)?.delete(callback); }
  dispatch(type) { for (const callback of [...this.listeners.get(type) || []]) callback(); }
  get listenerCount() { return [...this.listeners.values()].reduce((total, set) => total + set.size, 0); }
}

function textNode(key, text, { form = false, controls = false } = {}) {
  const classes = new Set();
  const node = { key, textContent: text, classes, form, controls, children: [],
    getAttribute: () => key,
    classList: { add: name => classes.add(name), remove: (...names) => names.forEach(name => classes.delete(name)) },
    closest: () => node.form ? node : null,
    querySelector: () => node.controls ? {} : null,
    contains: other => other === node || node.children.includes(other),
  };
  return node;
}

function fixture(before = [['price', '100']], after = [['price', '200']]) {
  const doc = new Events(), reduced = new Events();
  reduced.matches = false;
  let nodes = before.map(([key, text, options]) => textNode(key, text, options));
  let nextNodes = after.map(([key, text, options]) => textNode(key, text, options));
  let selection = null, commits = 0;
  Object.assign(doc, { hidden: false, activeElement: null, defaultView: { matchMedia: () => reduced }, getSelection: () => selection,
    createElement(tag) { assert.equal(tag, 'template'); return { set innerHTML(_value) {}, content: { querySelectorAll: () => nextNodes } }; },
  });
  const root = { ownerDocument: doc, isConnected: true, querySelectorAll: () => nodes };
  const commit = () => { commits++; nodes = nextNodes; };
  return { root, doc, reduced, commit, get commits() { return commits; }, get nodes() { return nodes; }, get nextNodes() { return nextNodes; },
    select(node) { selection = { isCollapsed: false, rangeCount: 1, getRangeAt: () => ({ intersectsNode: candidate => candidate === node }) }; doc.dispatch('selectionchange'); },
    next(value) { nextNodes = value.map(([key, text]) => textNode(key, text)); },
  };
}

async function tick(context, milliseconds) { context.mock.timers.tick(milliseconds); await Promise.resolve(); await Promise.resolve(); }
function clock(context) { context.mock.timers.enable({ apis: ['setTimeout'] }); }

test('only changed keyed text fades out before commit and the new text fades in afterward', async t => {
  clock(t);
  const f = fixture([['price', '100'], ['label', 'Same']], [['price', '200'], ['label', 'Same']]);
  const old = f.nodes[0], untouched = f.nodes[1];
  const work = animateContentRefresh(f.root, 'next', f.commit);
  assert.ok(old.classes.has('content-refresh-out'));
  assert.equal(untouched.classes.size, 0);
  assert.equal(f.commits, 0);
  await tick(t, 199); assert.equal(f.commits, 0);
  await tick(t, 1); assert.equal(f.commits, 1);
  assert.equal(old.classes.size, 0);
  assert.ok(f.nodes[0].classes.has('content-refresh-in'));
  await tick(t, 180); assert.equal(await work, true);
  assert.equal(f.nodes[0].classes.size, 0);
  assert.equal(f.doc.listenerCount + f.reduced.listenerCount, 0);
});

test('unchanged content, hidden tabs, and reduced motion commit without a timer', async () => {
  for (const mode of ['same', 'hidden', 'reduced']) {
    const f = fixture();
    if (mode === 'same') f.next([['price', '100']]);
    if (mode === 'hidden') f.doc.hidden = true;
    if (mode === 'reduced') f.reduced.matches = true;
    const work = animateContentRefresh(f.root, 'next', f.commit);
    assert.equal(f.commits, 1);
    assert.equal(await work, true);
    assert.equal(f.doc.listenerCount, 0);
  }
});

test('focused, selected, and form content is not delayed or decorated', async () => {
  for (const mode of ['focused', 'selected', 'form', 'controls']) {
    const f = fixture();
    if (mode === 'focused') f.doc.activeElement = f.nodes[0];
    if (mode === 'selected') f.select(f.nodes[0]);
    if (mode === 'form') f.nodes[0].form = true;
    if (mode === 'controls') f.nextNodes[0].controls = true;
    const old = f.nodes[0];
    const work = animateContentRefresh(f.root, 'next', f.commit);
    assert.equal(f.commits, 1, mode);
    assert.equal(await work, true);
    assert.equal(old.classes.size, 0);
  }
});

test('selection or focus during the fade stops motion immediately and never fades the replacement', async t => {
  clock(t);
  for (const mode of ['selection', 'focus', 'hidden', 'reduced']) {
    const f = fixture(), old = f.nodes[0];
    const work = animateContentRefresh(f.root, 'next', f.commit);
    if (mode === 'selection') f.select(old);
    if (mode === 'focus') { f.doc.activeElement = old; f.doc.dispatch('focusin'); }
    if (mode === 'hidden') { f.doc.hidden = true; f.doc.dispatch('visibilitychange'); }
    if (mode === 'reduced') { f.reduced.matches = true; f.reduced.dispatch('change'); }
    assert.equal(await work, true);
    assert.equal(f.commits, 1);
    assert.equal(old.classes.size + f.nodes[0].classes.size, 0);
    assert.equal(f.doc.listenerCount + f.reduced.listenerCount, 0);
  }
});

test('obsolete navigation never commits and removes temporary classes', async t => {
  clock(t);
  const f = fixture(); let current = true;
  const work = animateContentRefresh(f.root, 'next', f.commit, { isCurrent: () => current });
  current = false;
  await tick(t, 200);
  assert.equal(await work, false);
  assert.equal(f.commits, 0);
  assert.equal(f.nodes[0].classes.size, 0);
  assert.equal(f.doc.listenerCount, 0);
  assert.equal(await animateContentRefresh(f.root, 'next', f.commit, { isCurrent: () => false }), false);
  assert.equal(f.commits, 0);
});

test('a newer update cancels the old fade before it can overwrite current content', async t => {
  clock(t);
  const f = fixture(); let oldCommits = 0;
  const old = animateContentRefresh(f.root, 'old', () => oldCommits++);
  f.next([['price', '300']]);
  const latest = animateContentRefresh(f.root, 'latest', f.commit);
  assert.equal(await old, false);
  assert.ok(f.nodes[0].classes.has('content-refresh-out'), 'old cleanup must not remove the new fade from a reused node');
  await tick(t, 200); await tick(t, 180);
  assert.equal(await latest, true);
  assert.equal(oldCommits, 0);
  assert.equal(f.nodes[0].textContent, '300');
  assert.equal(f.nodes[0].classes.size, 0);
});

test('large updates animate at most twelve labels and never delay for unmatched keys', async t => {
  clock(t);
  const before = Array.from({ length: 30 }, (_, i) => [`key-${i}`, String(i)]);
  const after = before.map(([key, text]) => [key, text + ' updated']);
  const f = fixture(before, after);
  const work = animateContentRefresh(f.root, 'next', f.commit);
  assert.equal(f.nodes.filter(node => node.classes.has('content-refresh-out')).length, 12);
  await tick(t, 200); await tick(t, 180); await work;
  const unmatched = fixture([['old', 'Text']], [['new', 'Different']]);
  const immediate = animateContentRefresh(unmatched.root, 'next', unmatched.commit);
  assert.equal(unmatched.commits, 1); await immediate;
});

test('commit errors still clean up animation classes and event listeners', async t => {
  clock(t);
  const f = fixture();
  const work = animateContentRefresh(f.root, 'next', () => { throw new Error('Render failed'); });
  const rejected = assert.rejects(work, /Render failed/);
  await tick(t, 200); await rejected;
  assert.equal(f.nodes[0].classes.size, 0);
  assert.equal(f.doc.listenerCount + f.reduced.listenerCount, 0);
});

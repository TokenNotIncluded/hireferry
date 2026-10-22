import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../static/disclosure.js', import.meta.url), 'utf8');
const { createDisclosureUI } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

class Events {
  listeners = new Map();
  addEventListener(type, fn, options) {
    const entries = this.listeners.get(type) || new Map();
    entries.set(fn, options);
    this.listeners.set(type, entries);
  }
  removeEventListener(type, fn) { this.listeners.get(type)?.delete(fn); }
  dispatch(type, target = this) {
    for (const fn of this.listeners.get(type)?.keys() || []) fn({ target, preventDefault() {} });
  }
}

function fixture({ storage } = {}) {
  const root = new Events(), doc = new Events(), scroller = new Events();
  const classes = new Set(), frames = new Map();
  let nextFrame = 0;
  root.classList = { add: name => classes.add(name), remove: name => classes.delete(name) };
  root.dataset = { pageKey: 'app:orders' };
  const header = { offsetHeight: 64, contains: node => node?.inHeader === true };
  Object.assign(scroller, { scrollTop: 0, scrollHeight: 2000, clientHeight: 600 });
  let currentScroller = scroller, details = [], panels = [], menuOpen = false, modalOpen = false;
  root.querySelector = selector => {
    if (selector === '.workspace-header') return currentScroller ? header : null;
    if (selector === '.workspace-layout > .app-content') return currentScroller;
    if (selector === '.theme-menu[open], .workspace-more[open]') return menuOpen ? {} : null;
    if (selector.startsWith('[data-workspace-panel="inbox"]')) return panels.find(button => button.dataset.workspacePanel === 'inbox' && button.restore === !selector.includes(':not'));
    throw new Error(`Unexpected selector: ${selector}`);
  };
  root.querySelectorAll = selector => selector === '[data-workspace-panel]' ? panels : details;
  doc.querySelector = () => modalOpen ? {} : null;
  const win = {
    localStorage: storage,
    requestAnimationFrame(fn) { frames.set(++nextFrame, fn); return nextFrame; },
    cancelAnimationFrame(id) { frames.delete(id); },
  };
  const ui = createDisclosureUI({ root, doc, win });
  const flush = () => {
    const callbacks = [...frames.values()];
    frames.clear();
    for (const fn of callbacks) fn();
  };
  return {
    root, doc, scroller, ui, frames, flush,
    get collapsed() { return classes.has('workspace-chrome-collapsed'); },
    panelCollapsed: name => classes.has(`workspace-${name}-collapsed`),
    setPanels(value) { panels = value; },
    scroll(y, paint = true) { scroller.scrollTop = y; scroller.dispatch('scroll'); if (paint) flush(); },
    setDetails(value) { details = value; },
    setScroller(value) { currentScroller = value; },
    setMenu(value) { menuOpen = value; },
    setModal(value) { modalOpen = value; },
    sync(scope = 'account-a') { ui.sync({ scope }); },
  };
}

function panel(name, restore = false) {
  return {
    dataset: { workspacePanel: name, labelExpand: `Expand ${name}`, labelCollapse: `Collapse ${name}` },
    restore, attributes: {}, focused: false,
    setAttribute(key, value) { this.attributes[key] = value; },
    closest() { return this; },
    focus() { this.focused = true; },
  };
}

test('navigation and inbox fold independently and preferences survive a new page load', () => {
  const values = new Map(), storage = { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) };
  const f = fixture({ storage }), nav = panel('navigation'), inbox = panel('inbox'), restore = panel('inbox', true);
  f.setPanels([nav, inbox, restore]); f.sync();
  f.root.dispatch('click', nav);
  assert.equal(f.panelCollapsed('navigation'), true);
  assert.equal(f.panelCollapsed('inbox'), false);
  assert.equal(nav.attributes['aria-expanded'], 'false');
  assert.equal(nav.attributes['aria-label'], 'Expand navigation');
  f.root.dispatch('click', inbox);
  assert.equal(f.panelCollapsed('inbox'), true);
  assert.equal(restore.focused, true, 'focus moves out of the hidden list');
  f.root.dispatch('click', restore);
  assert.equal(f.panelCollapsed('inbox'), false);
  assert.equal(inbox.focused, true);
  f.ui.reset(); f.sync('another-account');
  assert.equal(f.panelCollapsed('navigation'), true, 'layout preferences contain no account data');
  const next = fixture({ storage }), replacement = panel('navigation');
  next.setPanels([replacement]); next.sync();
  assert.equal(next.panelCollapsed('navigation'), true);
  assert.equal(next.panelCollapsed('inbox'), false);
  assert.equal(replacement.attributes['aria-expanded'], 'false');
  f.ui.dispose(); next.ui.dispose();
});

test('folding still works when browser storage is blocked or contains invalid data', () => {
  const blocked = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
  for (const storage of [blocked, { getItem: () => '{invalid' }, { getItem: () => '{"inbox":"false"}' }]) {
    const f = fixture({ storage }), button = panel('navigation');
    f.setPanels([button]); f.sync();
    assert.equal(f.panelCollapsed('inbox'), false);
    f.root.dispatch('click', button);
    assert.equal(f.panelCollapsed('navigation'), true);
    f.ui.dispose();
    assert.equal(f.root.listeners.get('click').size, 0);
  }
});

function disclosure(key, open = false, parent = null) {
  const node = {
    dataset: { disclosure: key }, open,
    parentElement: parent && { closest: () => parent },
    matches: selector => selector === 'details[data-disclosure]',
  };
  return node;
}

test('manual disclosure choices survive rerenders and return visits without changing content', () => {
  const f = fixture(), first = disclosure('history', true);
  f.setDetails([first]); f.sync();
  first.open = false; f.root.dispatch('toggle', first);
  const replacement = disclosure('history', true);
  f.setDetails([replacement]); f.sync();
  assert.equal(replacement.open, false);
  f.root.dataset.pageKey = 'app:jobs'; f.setDetails([]); f.sync();
  f.root.dataset.pageKey = 'app:orders';
  const returned = disclosure('history', true);
  f.setDetails([returned]); f.sync();
  assert.equal(returned.open, false);
  f.ui.dispose();
});

test('disclosure choices are isolated by page and cleared when the account changes', () => {
  const f = fixture(), first = disclosure('details');
  f.setDetails([first]); f.sync();
  first.open = true; f.root.dispatch('toggle', first);
  f.root.dataset.pageKey = 'app:jobs';
  const job = disclosure('details'); f.setDetails([job]); f.sync();
  assert.equal(job.open, false);
  f.root.dataset.pageKey = 'app:orders';
  const otherAccount = disclosure('details'); f.setDetails([otherAccount]); f.sync('account-b');
  assert.equal(otherAccount.open, false);
  f.ui.dispose();
});

test('workspace scrolling folds chrome, preserves disclosure choices and batches scroll events', () => {
  const f = fixture(), details = disclosure('history', true);
  f.setDetails([details]); f.sync(); f.sync();
  assert.equal(f.scroller.listeners.get('scroll').size, 1);
  assert.deepEqual([...f.scroller.listeners.get('scroll').values()], [{ passive: true }]);
  f.scroll(65, false); f.scroll(130, false); f.scroll(180, false);
  assert.equal(f.frames.size, 1);
  f.flush();
  assert.equal(f.collapsed, true);
  assert.equal(details.open, true);
  f.scroll(172); assert.equal(f.collapsed, true);
  f.scroll(154); assert.equal(f.collapsed, false);
  f.ui.dispose();
});

test('near-top scrolling and touch overscroll reveal chrome without flickering', () => {
  const f = fixture(); f.sync(); f.scroll(160);
  assert.equal(f.collapsed, true);
  f.scroll(44); assert.equal(f.collapsed, false);
  f.scroll(-8); f.scroll(0); f.scroll(12); f.scroll(40);
  assert.equal(f.collapsed, false);
  f.ui.dispose();
});

test('a short page or a jump to the bottom cannot collapse the viewport and clamp its scroll position', () => {
  const f = fixture(); f.sync();
  f.scroller.scrollHeight = 820;
  f.scroll(150);
  assert.equal(f.collapsed, false);
  assert.equal(f.scroller.scrollTop, 150);
  f.ui.dispose();
});

test('keyboard focus, active editing, menus and dialogs keep the chrome available', () => {
  const f = fixture(); f.sync(); f.scroll(160);
  assert.equal(f.collapsed, true);
  f.doc.activeElement = { inHeader: true };
  f.doc.dispatch('focusin');
  assert.equal(f.collapsed, false);
  f.scroll(240); assert.equal(f.collapsed, false);
  f.doc.activeElement = { closest: () => ({ tagName: 'TEXTAREA' }) };
  f.scroll(320); assert.equal(f.collapsed, false);
  f.doc.activeElement = null;
  f.setMenu(true); f.scroll(400); assert.equal(f.collapsed, false);
  f.setMenu(false); f.setModal(true); f.scroll(480); assert.equal(f.collapsed, false);
  f.setModal(false); f.scroll(560); assert.equal(f.collapsed, true);
  f.ui.dispose();
});

test('opening the native theme menu reveals an already folded header immediately', () => {
  const f = fixture(); f.sync(); f.scroll(180);
  assert.equal(f.collapsed, true);
  f.root.dispatch('toggle', { open: true, matches: selector => selector === '.theme-menu, .workspace-more' });
  assert.equal(f.collapsed, false);
  f.ui.dispose();
});

test('native validation opens every containing disclosure before the browser focuses the field', () => {
  const f = fixture(), outer = disclosure('profile'), inner = disclosure('contact', false, outer);
  f.setDetails([outer, inner]); f.sync();
  f.root.dispatch('invalid', { closest: () => inner });
  assert.equal(outer.open, true);
  assert.equal(inner.open, true);
  const nextOuter = disclosure('profile'), nextInner = disclosure('contact');
  f.setDetails([nextOuter, nextInner]); f.sync();
  assert.equal(nextOuter.open, true);
  assert.equal(nextInner.open, true);
  f.ui.dispose();
});

test('leaving the workspace detaches its scroller and cancels an outstanding frame', () => {
  const f = fixture(); f.sync(); f.scroll(200, false);
  assert.equal(f.frames.size, 1);
  f.setScroller(null); f.root.dataset.pageKey = '/'; f.sync();
  assert.equal(f.frames.size, 0);
  assert.equal(f.scroller.listeners.get('scroll').size, 0);
  f.scroll(500); assert.equal(f.collapsed, false);
  f.ui.dispose();
  assert.equal(f.root.listeners.get('toggle').size, 0);
  assert.equal(f.doc.listeners.get('focusin').size, 0);
});

test('changing routes or resetting the session restores the header', () => {
  const f = fixture(); f.sync(); f.scroll(180);
  assert.equal(f.collapsed, true);
  f.root.dataset.pageKey = 'app:jobs'; f.sync();
  assert.equal(f.collapsed, false);
  f.scroll(300); assert.equal(f.collapsed, true);
  f.ui.reset();
  assert.equal(f.collapsed, false);
  assert.equal(f.scroller.listeners.get('scroll').size, 0);
  f.ui.dispose();
});

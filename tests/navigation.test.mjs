import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('../static/navigation.js', import.meta.url), 'utf8');
const { reconcileChildren, pageIdentity, pagePlaceholder } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

// A small DOM fixture keeps these identity tests runnable without a browser dependency.
class Element {
  constructor(name, attributes = {}, children = []) {
    this.nodeType = 1; this.nodeName = this.tagName = name.toUpperCase();
    this.attrs = new Map(Object.entries(attributes)); this.childNodes = []; this.parentNode = null;
    this.value = attributes.value || ''; this.checked = 'checked' in attributes;
    this.replaceChildren(...children);
  }
  get id() { return this.getAttribute('id') || ''; }
  get attributes() { return [...this.attrs].map(([name, value]) => ({name, value})); }
  get firstChild() { return this.childNodes[0] || null; }
  get nextSibling() { return this.parentNode?.childNodes[this.parentNode.childNodes.indexOf(this) + 1] || null; }
  getAttribute(name) { return this.attrs.get(name) ?? null; }
  hasAttribute(name) { return this.attrs.has(name); }
  setAttribute(name, value) { this.attrs.set(name, String(value)); }
  removeAttribute(name) { this.attrs.delete(name); }
  isEqualNode(other) { return this.nodeName === other.nodeName && JSON.stringify(this.attributes) === JSON.stringify(other.attributes) && this.childNodes.length === other.childNodes.length && this.childNodes.every((child, index) => child.isEqualNode(other.childNodes[index])); }
  insertBefore(node, before) { node.remove(); const index = before ? this.childNodes.indexOf(before) : this.childNodes.length; assert.ok(index >= 0); this.childNodes.splice(index, 0, node); node.parentNode = this; }
  remove() { if (this.parentNode) this.parentNode.childNodes.splice(this.parentNode.childNodes.indexOf(this), 1); this.parentNode = null; }
  replaceWith(node) { const parent = this.parentNode; parent.insertBefore(node, this); this.remove(); }
  replaceChildren(...children) { for (const child of [...this.childNodes]) child.remove(); for (const child of children) this.insertBefore(child, null); }
}
const e = (name, attributes, children) => new Element(name, attributes, children);

test('switching workspace pages preserves the exact header, sidebar and navigation links', () => {
  const link = e('a', {href:'/app?tab=profile', class:'active'});
  const sidebar = e('aside', {'data-dom-key':'sidebar'}, [link]);
  const content = e('section', {'data-page-key':'app:profile'}, [e('input', {name:'name', value:'Saved'})]);
  content.childNodes[0].value = 'Unsaved draft';
  const header = e('header', {'data-dom-key':'site-header'}, [e('select', {'data-theme-select':''})]);
  header.childNodes[0].value = 'dark';
  const main = e('main', {id:'main'}, [sidebar, content]);
  const root = e('div', {}, [header, main]);
  const next = e('div', {}, [e('header', {'data-dom-key':'site-header'}, [e('select', {'data-theme-select':''})]), e('main', {id:'main'}, [e('aside', {'data-dom-key':'sidebar'}, [e('a', {href:'/app?tab=profile', class:''})]), e('section', {'data-page-key':'app:jobs'}, [e('input', {name:'name', value:'New role'})])])]);
  reconcileChildren(root, next);
  assert.equal(root.childNodes[0], header);
  assert.equal(header.childNodes[0].value, 'dark');
  assert.equal(root.childNodes[1], main);
  assert.equal(main.childNodes[0], sidebar);
  assert.equal(sidebar.childNodes[0], link);
  assert.equal(link.getAttribute('class'), '');
  assert.equal(main.childNodes[1], content);
  assert.equal(content.childNodes[0].value, 'New role');
});

test('a same-page refresh preserves an unchanged live input and applies changed server defaults', () => {
  const input = e('input', {value:'Saved'}); input.value = 'In progress';
  const root = e('div', {}, [input]);
  reconcileChildren(root, e('div', {}, [e('input', {value:'Saved'})]));
  assert.equal(root.firstChild, input);
  assert.equal(input.value, 'In progress');
  reconcileChildren(root, e('div', {}, [e('input', {value:'Confirmed'})]));
  assert.equal(input.value, 'Confirmed');
});

test('inserting a banner and changing navigation order never recreates existing keyed nodes', () => {
  const a = e('a', {href:'/a'}), b = e('a', {href:'/b'}), footer = e('footer', {'data-dom-key':'footer'});
  const root = e('div', {}, [a, b, footer]);
  reconcileChildren(root, e('div', {}, [e('aside', {'data-dom-key':'notice'}), e('a', {href:'/b'}), e('a', {href:'/a'}), e('footer', {'data-dom-key':'footer'})]));
  assert.equal(root.childNodes[1], b);
  assert.equal(root.childNodes[2], a);
  assert.equal(root.childNodes[3], footer);
  reconcileChildren(root, e('div', {}, [e('a', {href:'/a'}), e('footer', {'data-dom-key':'footer'})]));
  assert.deepEqual(root.childNodes, [a, footer]);
});

test('switching conversation resources clears the old draft and keeps the conversation list', () => {
  const sidebar=e('aside', {id:'conversation-list'}, [e('a', {href:'/app?tab=messages&conversation=a'})]);
  const textarea=e('textarea', {id:'chat-body'}); textarea.value='Private draft for A';
  const thread=e('section', {class:'chat-thread','data-page-key':'conversation:a'}, [textarea]);
  const root=e('div', {}, [sidebar,thread]);
  const view=id=>e('div', {}, [e('aside', {id:'conversation-list'}, [e('a', {href:'/app?tab=messages&conversation=a'})]),e('section', {class:'chat-thread','data-page-key':`conversation:${id}`}, [e('textarea', {id:'chat-body'})])]);
  reconcileChildren(root,view('a'));
  assert.equal(thread.firstChild,textarea);
  assert.equal(textarea.value,'Private draft for A');
  reconcileChildren(root,view('b'));
  assert.equal(root.firstChild,sidebar);
  assert.equal(root.childNodes[1],thread);
  assert.notEqual(thread.firstChild,textarea);
  assert.equal(thread.firstChild.value,'');
});

test('page placeholders follow the route layout without a fullscreen loading screen', () => {
  assert.equal(pageIdentity('/admin', null), 'admin:overview');
  assert.equal(pageIdentity('/app', 'jobs'), 'app:jobs');
  for (const tab of ['messages','matches','mandate','orders','overview']) {
    const html = pagePlaceholder(tab, 'Loading content');
    assert.match(html, /role="status"/);
    assert.match(html, /aria-hidden="true"/);
    assert.doesNotMatch(html, /loading-screen|class="loader"/);
  }
  assert.match(pagePlaceholder('messages', 'Loading'), /skeleton-chat/);
  assert.match(pagePlaceholder('mandate', 'Loading'), /skeleton-form/);
});

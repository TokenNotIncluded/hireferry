import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { gzipSync } from 'node:zlib';
import { JSDOM } from 'jsdom';
import { draftAction, safeImage, safeLink } from '../frontend/security.js';

const dom = new JSDOM('<!doctype html><html><body></body></html>', { url: 'https://hireferry.test/inbox', pretendToBeVisual: true });
for (const name of ['window', 'document', 'navigator', 'location', 'HTMLElement', 'HTMLInputElement', 'HTMLTextAreaElement', 'Element', 'Node', 'CustomEvent', 'MutationObserver', 'DOMParser', 'Event', 'MouseEvent']) {
  Object.defineProperty(globalThis, name, { configurable: true, value: dom.window[name] });
}
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame.bind(dom.window);
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame.bind(dom.window);
let backgroundRequests = 0;
globalThis.fetch = async () => { backgroundRequests++; throw new Error('Unexpected renderer network request'); };
const { mount, capabilities } = await import('../static/chat-renderer.js');
const nextFrame = () => new Promise((resolve) => setTimeout(resolve, 35));
async function waitFor(predicate) {
  const deadline = Date.now() + 2000;
  while (!predicate() && Date.now() < deadline) await nextFrame();
  assert.ok(predicate(), 'rendered state did not arrive');
}
function fixture(props) {
  const element = document.createElement('div');
  document.body.append(element);
  const handle = mount(element, props);
  return { element, handle, dispose() { handle.unmount(); element.remove(); } };
}

test('rich Markdown renders GFM, code and math while remaining self hosted', async () => {
  const view = fixture({ format: 'markdown', body: '# Welcome\n\n| Role | Status |\n| --- | --- |\n| Rust | Open |\n\n- [x] Read\n\n```javascript\nconst message = "<script>alert(1)</script>";\n```\n\n$E=mc^2$' });
  try {
    await waitFor(() => view.element.querySelector('.katex'));
    assert.equal(view.element.querySelector('h1').textContent, 'Welcome');
    assert.equal(view.element.querySelectorAll('table tbody tr').length, 1);
    assert.equal(view.element.querySelector('input[type="checkbox"]').disabled, true);
    assert.ok(view.element.querySelector('.hljs-keyword'));
    assert.equal(view.element.querySelector('script'), null);
    assert.match(view.element.querySelector('code').textContent, /<script>/u);
    assert.equal(view.element.querySelector('link,iframe'), null);
    assert.equal(backgroundRequests, 0, 'renderer must not initialize a telemetry sink');
  } finally { view.dispose(); }
});

test('Markdown omits raw HTML, unsafe links and automatic external images', async () => {
  const view = fixture({ format: 'markdown', body: '[unsafe](javascript:alert%281%29)\n\n[good](https://example.com/jobs)\n\n![Remote](https://tracker.test/image.png)\n\n<img src=x onerror="alert(1)">\n<script>alert(1)</script>' });
  try {
    await waitFor(() => view.element.textContent.includes('Remote'));
    assert.equal(view.element.querySelector('img,script,iframe,[onerror]'), null);
    for (const link of view.element.querySelectorAll('a')) {
      assert.ok(link.href.startsWith('https://'));
      assert.equal(link.rel, 'noopener noreferrer');
    }
    assert.equal(view.element.querySelectorAll('a').length, 2);
  } finally { view.dispose(); }
});

test('OpenUI actually parses the exported contract and renders its interactive example', async () => {
  const metadata = JSON.parse(await readFile(new URL('../static/chat-openui.json', import.meta.url)));
  const view = fixture({ format: 'openui', body: metadata.example, status: 'completed' });
  let detail;
  view.element.addEventListener('hireferry:chat-action', (event) => { detail = event.detail; });
  try {
    await waitFor(() => view.element.querySelector('button'));
    assert.match(view.element.textContent, /面试安排/u);
    assert.equal(view.element.querySelectorAll('table tbody tr').length, 2);
    view.element.querySelector('button').click();
    await waitFor(() => detail);
    assert.equal(detail.type, 'draft');
    assert.equal(detail.text, '我想选择周三 14:00 面试');
    assert.equal(metadata.library, capabilities.library);
    assert.ok(metadata.schema.$defs || metadata.schema.components || metadata.schema.$schema);
  } finally { view.dispose(); }
});

test('stream updates preserve the root, disable unfinished actions and support cancellation', async () => {
  const view = fixture({ format: 'openui', body: 'root = Col([title])\ntitle = Title("Start")', status: 'streaming' });
  try {
    await waitFor(() => view.element.querySelector('.hfy-rich-message'));
    const original = view.element.querySelector('.hfy-rich-message');
    view.handle.update({ body: 'root = Col([title, button])\ntitle = Title("Updated")\nbutton = Button("Review", @ToAssistant("Draft only"))' });
    await waitFor(() => view.element.querySelector('button'));
    assert.equal(view.element.querySelector('.hfy-rich-message'), original);
    assert.equal(view.element.querySelector('button').disabled, true);
    view.handle.update({ status: 'cancelled' });
    await waitFor(() => !view.element.querySelector('button').disabled);
    assert.equal(view.element.querySelector('.hfy-rich-message'), original);
    assert.equal(mount(view.element, { status: 'completed' }), view.handle);
  } finally { view.dispose(); }
});

test('OpenUI local form state survives updates and only user review drafts leave the island', async () => {
  const body = '$name = "Initial"\nroot = Col([form])\nform = Form([field, change, review])\nfield = Input($name, "Name")\nchange = Button("Change locally", @Set($name, "Edited"))\nreview = Button("Review", @ToAssistant("Review my answer"))';
  const view = fixture({ format: 'openui', body, status: 'complete' });
  let detail;
  view.element.addEventListener('hireferry:chat-action', (event) => { detail = event.detail; });
  try {
    await waitFor(() => view.element.querySelector('input'));
    const input = view.element.querySelector('input');
    assert.equal(input.value, 'Initial');
    view.element.querySelector('button').click();
    await waitFor(() => input.value === 'Edited');
    assert.equal(detail, undefined, 'local state actions must stay local');
    view.handle.update({ body: `${body}\ncaption = Caption("Saved locally")` });
    await nextFrame();
    assert.equal(view.element.querySelector('input'), input);
    assert.equal(input.value, 'Edited');
    view.element.querySelectorAll('button')[1].click();
    await waitFor(() => detail);
    assert.equal(detail.text, 'Review my answer');
    assert.ok(Object.values(detail.formState).includes('Edited'));
  } finally { view.dispose(); }
});

test('incomplete Markdown streams remain readable and complete without rebuilding the root', async () => {
  const view = fixture({ format: 'markdown', body: '```rust\nlet value = "<script>', status: 'streaming' });
  try {
    await waitFor(() => view.element.querySelector('code'));
    const original = view.element.querySelector('.hfy-rich-message');
    assert.match(view.element.textContent, /let value/u);
    assert.equal(view.element.querySelector('script'), null);
    view.handle.update({ body: '```rust\nlet value = "<script>";\n```\n\nDone **now**.', status: 'complete' });
    await waitFor(() => view.element.querySelector('strong'));
    assert.equal(view.element.querySelector('.hfy-rich-message'), original);
    assert.equal(view.element.querySelector('strong').textContent, 'now');
  } finally { view.dispose(); }
});

test('OpenUI actions cannot open URLs, fetch tools, or submit automatically', async () => {
  let requests = 0, opened = 0, actions = 0;
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => { requests++; throw new Error('Unexpected network'); };
  const originalOpen = window.open;
  window.open = () => { opened++; };
  const view = fixture({ format: 'openui', body: 'root = Col([b])\nb = Button("Open", @OpenUrl("javascript:alert(1)"))' });
  view.element.addEventListener('hireferry:chat-action', () => { actions++; });
  try {
    await waitFor(() => view.element.querySelector('button'));
    view.element.querySelector('button').click();
    await nextFrame();
    assert.equal(opened, 0);
    assert.equal(actions, 0);
    view.handle.update({ body: 'root = Col([Text("No access")])\nsecret = Query("private_data", {})' });
    await nextFrame();
    assert.equal(requests, 0);
  } finally { view.dispose(); globalThis.fetch = originalFetch; window.open = originalOpen; }
});

test('malformed OpenUI and oversized messages have a readable fallback', async () => {
  const view = fixture({ format: 'openui', body: 'root = HtmlView("<script>evil()</script>")', status: 'completed' });
  try {
    await waitFor(() => view.element.querySelector('.hfy-rich-fallback'));
    assert.match(view.element.textContent, /HtmlView/u);
    assert.equal(view.element.querySelector('iframe,script'), null);
    view.handle.update({ body: 'x'.repeat(65537) });
    await waitFor(() => view.element.textContent.includes('超出'));
    assert.equal(view.element.querySelector('pre').textContent.length, 65536);
  } finally { view.dispose(); }
});

test('plain text stays plain and unmount cancels queued updates', async () => {
  const view = fixture({ format: 'plain', body: '**raw** <img src=x>' });
  await waitFor(() => view.element.textContent.includes('**raw**'));
  assert.equal(view.element.querySelector('img,strong'), null);
  view.handle.update({ body: 'must not mount again' });
  view.handle.unmount();
  await nextFrame();
  assert.equal(view.element.textContent, '');
  view.dispose();
});

test('URL and draft boundaries reject hidden authority and keep only bounded scalar fields', () => {
  for (const value of ['javascript:alert(1)', 'data:text/html,test', '\u0009javascript:alert(1)', 'https://user:password@example.com']) assert.equal(safeLink(value), undefined);
  assert.equal(safeImage('https://remote.test/image.png'), undefined);
  assert.equal(safeImage('data:image/svg+xml;base64,PHN2Zz4='), undefined);
  assert.ok(safeImage('/static/local.png'));
  assert.equal(draftAction({ type: 'open_url', humanFriendlyMessage: 'Open' }), null);
  assert.equal(draftAction({ type: 'continue_conversation', humanFriendlyMessage: 'x'.repeat(4097) }), null);
  const event = draftAction({ type: 'continue_conversation', humanFriendlyMessage: 'Review', params: { hiddenContext: 'execute' }, formState: { nested: { token: 'secret' }, label: 'x'.repeat(2048), n: 1, flag: true } });
  assert.equal(event.formState.label.length, 1024);
  assert.equal(event.formState.nested, undefined);
  assert.equal(event.params, undefined);
});

test('generated assets use local fonts and a bounded lazy JavaScript payload', async () => {
  const css = await readFile(new URL('../static/chat-renderer.css', import.meta.url), 'utf8');
  assert.ok(css.includes('url(chat-renderer-fonts/'));
  assert.doesNotMatch(css, /url\((?:https?:|\/\/|fonts\/)/u);
  const style = document.createElement('style');
  style.textContent = css;
  document.head.append(style);
  assert.equal([...style.sheet.cssRules].filter((rule) => rule.type === window.CSSRule.FONT_FACE_RULE).length, 20, 'all local KaTeX font-face rules must remain intact');
  style.remove();
  const bundle = await readFile(new URL('../static/chat-renderer.js', import.meta.url));
  assert.ok(gzipSync(bundle).length < 450_000, 'chat JavaScript gzip exceeds 450 KiB budget');
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../static/legal-content.js', import.meta.url), 'utf8');
const { renderLegal } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const escape = value => String(value).replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));

// A template snapshot supplies the DOM operations used by the renderer without
// adding a browser or HTML parser dependency to the Node test suite.
function element(name, text, attributes = {}) {
  return {
    name, textContent: text, attributes: new Map(Object.entries(attributes)), removed: false,
    get id() { return this.getAttribute('id') || ''; },
    set id(value) { this.setAttribute('id', value); },
    getAttribute(key) { return this.attributes.get(key) ?? null; },
    setAttribute(key, value) { this.attributes.set(key, value); },
    remove() { this.removed = true; },
    html() { return this.removed ? '' : `<${name}${[...this.attributes].map(([key,value]) => ` ${key}="${escape(value)}"`).join('')}>${escape(this.textContent)}</${name}>`; },
  };
}

function renderSnapshot(nodes, metadata = {}, language = 'en') {
  const previous = globalThis.document;
  let received;
  const template = {
    set innerHTML(value) { received = value; },
    get innerHTML() { return nodes.map(node => node.html()).join(''); },
    content: {
      querySelector(selector) { assert.equal(selector, 'h1'); return nodes.find(node => node.name === 'h1') || null; },
      querySelectorAll(selector) {
        assert.ok(['h2', 'a[href]'].includes(selector));
        return nodes.filter(node => selector === 'h2' ? node.name === 'h2' : node.name === 'a' && node.getAttribute('href') !== null);
      },
    },
  };
  globalThis.document = { createElement(name) { assert.equal(name, 'template'); return template; } };
  try {
    const html = renderLegal({ html: 'server-sanitized-document', ...metadata }, language);
    assert.equal(received, 'server-sanitized-document');
    return html;
  } finally {
    if (previous === undefined) delete globalThis.document;
    else globalThis.document = previous;
  }
}

test('legal content keeps root-relative navigation in the app and preserves fragment, protocol, and external links', () => {
  const hrefs = ['/privacy', '/privacy#retention', '/contact', '#operator', '//outside.example/privacy', '/\\outside.example/privacy', 'https://outside.example/privacy', 'mailto:support@example.test', 'relative-document'];
  const links = hrefs.map(href => element('a', 'Read', {href}));
  const html = renderSnapshot([element('h1', 'Terms'), element('h2', 'Operator', {id:'operator'}), ...links]);
  links.forEach((link,index) => assert.equal(link.attributes.has('data-nav'), index < 3, hrefs[index]));
  assert.match(html, /href="\/privacy#retention" data-nav=""/);
  assert.match(html, /<a href="#operator">1\. Operator<\/a>/);
  assert.doesNotMatch(html, /href="#operator" data-nav/);
});

test('legal document headings, metadata, and table of contents remain escaped and localized', () => {
  const title = element('h1', 'Terms & <privacy>');
  const first = element('h2', 'Fees & currency', {id:'pricing'});
  const second = element('h2', '<Contact>', {id:'Invalid Id'});
  const html = renderSnapshot([title,first,second], {version:'v2 <draft>',effective_date:'2026-10-21'}, 'en');
  assert.equal(title.removed,true);
  assert.equal((html.match(/<h1>/g)||[]).length,1);
  assert.match(html, /<h1>Terms &amp; &lt;privacy&gt;<\/h1>/);
  assert.match(html, /Version v2 &lt;draft&gt; · Updated and effective: 2026-10-21/);
  assert.match(html, /href="#pricing">1\. Fees &amp; currency/);
  assert.equal(second.id,'section-2');
  assert.match(html, /href="#section-2">2\. &lt;Contact&gt;/);
  const missingTitle = renderSnapshot([], {}, 'zh');
  assert.match(missingTitle, /<h1>服务说明<\/h1>/);
  assert.match(missingTitle, /aria-label="文档目录"/);
  assert.doesNotMatch(missingTitle, /class="legal-meta"/);
});

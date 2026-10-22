'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function client(saved) {
  const values = new Map(saved ? [['hireferry.desktop.server', saved]] : []);
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, { value: '', textContent: '', disabled: false, dataset: {},
      classList: { toggle() {} }, checkValidity: () => true, reportValidity() {},
      addEventListener() {}, setAttribute() {}, replaceChildren() {} });
    return elements.get(id);
  };
  const invokes = [];
  const window = { __HIREFERRY_MOBILE__: true, __TAURI__: { core: { invoke: async (...args) => invokes.push(args) } } };
  const context = vm.createContext({ URL, window, navigator: { language: 'en-US', userAgent: 'Android' },
    document: { documentElement: { dataset: {} }, getElementById: element, querySelectorAll: () => [] },
    localStorage: { getItem: key => values.get(key) || null, setItem: (key,value) => values.set(key,value), removeItem: key => values.delete(key) } });
  for (const name of ['server-origin.js', 'app.js']) vm.runInContext(fs.readFileSync(path.join(__dirname, '../desktop/ui', name), 'utf8'), context);
  return { context, values, invokes, element };
}

test('mobile does not persist or invoke credential-bearing and non-origin input', async () => {
  for (const input of ['https://user:secret@example.test/', 'https://example.test/?token=secret',
    'https://example.test/#session=secret', 'https://example.test/?', 'https://example.test/#',
    'https://example.test/login', 'https://example.test:0', 'http://remote.test/', 'file:///secret']) {
    const app = client('https://safe.example');
    app.element('server').value = input;
    await app.context.run('connect');
    assert.equal(app.values.get('hireferry.desktop.server'), 'https://safe.example', input);
    assert.equal(app.invokes.length, 0, input);
  }
});

test('mobile persists only the canonical origin before navigation', async () => {
  const app = client();
  app.element('server').value = ' https://EXAMPLE.test:443/ ';
  await app.context.run('connect');
  assert.equal(app.values.get('hireferry.desktop.server'), 'https://example.test');
  assert.equal(app.invokes[0][1].server, 'https://example.test');
});

test('legacy credential-bearing preferences are removed on startup', () => {
  const app = client('https://user:secret@example.test/?token=secret');
  assert.equal(app.values.has('hireferry.desktop.server'), false);
  assert.equal(app.element('server').value, 'https://hireferry.lmm.best');
});

test('loopback validation matches actual loopback hosts', async () => {
  for (const input of ['http://localhost:3000', 'http://127.0.0.2:3000', 'http://[::1]:3000']) {
    const app = client();
    app.element('server').value = input;
    await app.context.run('connect');
    assert.equal(app.invokes.length, 1, input);
  }
  for (const input of ['http://localhost.example:3000', 'http://192.168.0.2:3000', 'http://[::ffff:127.0.0.1]:3000']) {
    const app = client();
    app.element('server').value = input;
    await app.context.run('connect');
    assert.equal(app.invokes.length, 0, input);
  }
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {safeFeedbackPage} from '../static/feedback-ui.js';

test('feedback references only a short local path, never query strings or secrets', () => {
  assert.equal(safeFeedbackPage('/oauth/connections'), '/oauth/connections');
  assert.equal(safeFeedbackPage('/app'), '/app');
  for (const path of ['//evil.example', '/app?token=secret', '/app#private', '/\\outside', 'https://outside.example', '/'.repeat(161), '/control\n']) {
    assert.equal(safeFeedbackPage(path), '', path);
  }
});

test('main app and standalone OAuth pages share saved Material 3 themes and feedback', async () => {
  for (const name of ['index.html','cli-auth.html','cli-connections.html']) {
    const html = await readFile(new URL(`../static/${name}`, import.meta.url), 'utf8');
    for (const asset of ['theme.js','theme-palettes.css','theme.css','feedback.css','feedback-ui.js']) {
      assert.ok(html.includes(`/static/${asset}`), `${name}: missing ${asset}`);
    }
  }
  const css = await readFile(new URL('../static/cli-auth.css', import.meta.url), 'utf8');
  for (const variable of ['--paper','--surface','--ink','--accent','--on-accent','--line']) {
    assert.ok(css.includes(`var(${variable})`), `CLI page must use ${variable}`);
  }
  assert.doesNotMatch(css, /#[0-9a-fA-F]{6}\b/, 'no standalone hard-coded theme colors');
});

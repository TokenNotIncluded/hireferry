import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {mountQuickFeedback, safeFeedbackPage} from '../static/feedback-ui.js';

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

function feedbackFixture({fetch} = {}) {
  const events = () => ({
    listeners:new Map(),
    addEventListener(type, listener) {
      if (!this.listeners.has(type)) this.listeners.set(type, []);
      this.listeners.get(type).push(listener);
    },
    dispatch(type, event = {}) { return Promise.all((this.listeners.get(type) || []).map(listener => listener({target:this, detail:0, preventDefault() {}, ...event}))); },
  });
  const doc = {...events(), activeElement:null, controls:[], documentElement:{lang:'zh-CN'}};
  const element = (rect = {left:16, top:16, right:60, bottom:60, width:44, height:44}) => ({
    ...events(), style:{}, attributes:{}, dataset:{}, value:'', textContent:'', hidden:false, isConnected:true, rect, rectReads:0,
    classList:{add() {}, remove() {}},
    setAttribute(name, value) { this.attributes[name] = value; },
    focus() { doc.activeElement = this; },
    getClientRects() { return this.hidden || !this.isConnected ? [] : [this.rect]; },
    getBoundingClientRect() { this.rectReads++; return this.rect; },
    closest() { return null; },
  });
  const selectors = new Map(['#feedback-launch', '#feedback-panel', '#feedback-form', '#feedback-category', '#feedback-message', '#feedback-result', '.feedback-send', '[data-feedback-close]', '#feedback-length'].map(selector => [selector, element()]));
  const translated = new Map(['open','title','category','bug','idea','other','message','count','hint'].map(key => [key, {...element(), dataset:{feedback:key}}]));
  selectors.get('.feedback-send').dataset.feedback = 'submit';translated.set('submit', selectors.get('.feedback-send'));
  selectors.get('#feedback-category').value = 'bug';
  Object.assign(selectors.get('#feedback-form'), {reportValidity:() => true, reset() {selectors.get('#feedback-message').value = '';selectors.get('#feedback-category').value = 'bug';}});
  const launch = selectors.get('#feedback-launch'), panel = selectors.get('#feedback-panel');
  panel.hidden = true;
  panel.rect = {left:0, top:0, right:300, bottom:300, width:300, height:300};
  const root = {...element(), querySelector:selector => selectors.get(selector), querySelectorAll:() => [...translated.values()], contains:target => target === root || [...selectors.values()].includes(target)};
  let mounted = false;
  Object.assign(doc, {
    getElementById:() => mounted ? launch : null,
    createElement:() => root,
    body:{append() { mounted = true; }},
    querySelectorAll:() => doc.controls,
  });
  const observers = [], storage = new Map();
  const win = {...events(), innerWidth:400, innerHeight:800, location:{pathname:'/cli'}, fetch,
    localStorage:{getItem:key => storage.get(key) || null, setItem:(key, value) => storage.set(key,value)},
    MutationObserver:class {constructor(callback) {observers.push(callback);}observe() {}},
  };
  mountQuickFeedback(doc, win);
  return {doc, win, selectors, launch, panel, element, translated, setLanguage(language) {doc.documentElement.lang = language;for (const callback of observers) callback();}};
}

test('inline chat feedback uses visible anchors and returns focus across SPA entries', () => {
  const f = feedbackFixture();
  f.launch.hidden = true;
  const summary = f.element({left:340, top:6, right:384, bottom:50, width:44, height:44});
  const menu = {open:true, querySelector:selector => selector === 'summary' ? summary : null};
  const menuTrigger = f.element();
  menuTrigger.closest = selector => selector === '[data-chat-feedback]' ? menuTrigger : selector === 'details.chat-tools' ? menu : null;
  f.doc.controls = [summary, menuTrigger];
  f.doc.dispatch('click', {target:menuTrigger});
  assert.equal(menu.open, false, 'the menu closes before measuring its visible summary');
  assert.equal(f.panel.hidden, false);
  assert.equal(f.panel.style.top, '62px');
  assert.equal(f.doc.activeElement, f.selectors.get('#feedback-message'));
  assert.equal(f.launch.rectReads, 0, 'the hidden floating control is never measured');
  f.doc.dispatch('keydown', {key:'Escape'});
  assert.equal(f.panel.hidden, true);
  assert.equal(f.doc.activeElement, summary);

  // A newly rendered list entry works without mounting the shared widget again.
  const listTrigger = f.element({left:280, top:700, right:384, bottom:744, width:104, height:44});
  listTrigger.closest = selector => selector === '[data-chat-feedback]' ? listTrigger : null;
  f.doc.controls = [listTrigger];
  mountQuickFeedback(f.doc, f.win);
  assert.equal(f.doc.listeners.get('click').length, 1);
  f.doc.dispatch('click', {target:{closest:selector => listTrigger.closest(selector)}});
  assert.equal(f.panel.hidden, false);
  assert.equal(f.panel.style.top, '388px', 'the panel opens above the list entry');
  f.selectors.get('[data-feedback-close]').dispatch('click');
  assert.equal(f.doc.activeElement, listTrigger);
  assert.equal(f.panel.hidden, true);

  f.launch.hidden = false;
  f.launch.dispatch('click');
  assert.equal(f.panel.hidden, false, 'the public floating entry still opens the same panel');
  f.doc.dispatch('keydown', {key:'Escape'});
  assert.equal(f.doc.activeElement, f.launch);
});

test('language changes translate the existing open or closed panel without losing draft or category', () => {
  const f = feedbackFixture(), message = f.selectors.get('#feedback-message'), category = f.selectors.get('#feedback-category');
  f.setLanguage('en');assert.equal(f.launch.attributes['aria-label'], 'Quick feedback');assert.equal(f.panel.hidden, true);
  f.launch.dispatch('click');message.value = 'Synthetic draft';category.value = 'idea';
  const languageButton = {closest:selector => selector.includes('language') ? languageButton : null};
  f.doc.dispatch('pointerdown', {target:languageButton});f.setLanguage('zh-CN');
  assert.equal(f.panel.hidden, false);assert.equal(message.value, 'Synthetic draft');assert.equal(category.value, 'idea');
  assert.equal(f.launch.title, '轻点反馈，拖动调整位置');assert.equal(f.translated.get('idea').textContent, '改进建议');
  assert.equal(f.selectors.get('[data-feedback-close]').attributes['aria-label'], '关闭');
  f.selectors.get('[data-feedback-close]').dispatch('click');f.setLanguage('en');
  assert.equal(message.placeholder, 'What happened? What would you like us to change?');assert.equal(f.panel.hidden, true);
});

test('changing locale preserves the pending state and retranslates a retained error', async () => {
  let release, started;const pending = new Promise(resolve => {release = resolve;}), requested = new Promise(resolve => {started = resolve;});
  const f = feedbackFixture({fetch:async path => {
    if (path === '/api/session') return {ok:true,json:async () => ({csrf:'synthetic'})};
    started();await pending;return {ok:false,status:429};
  }});
  const message = f.selectors.get('#feedback-message'), send = f.selectors.get('.feedback-send');message.value = 'Synthetic report';
  const submitting = f.selectors.get('#feedback-form').dispatch('submit');await requested;
  assert.equal(send.disabled, true);assert.equal(send.textContent, '提交中…');f.setLanguage('en');
  assert.equal(send.disabled, true);assert.equal(send.textContent, 'Sending…');
  release();await submitting;assert.equal(send.disabled, false);assert.equal(send.textContent, 'Send feedback');
  assert.equal(f.selectors.get('#feedback-result').textContent, 'Too many reports. Please try again later.');
  f.setLanguage('zh-CN');assert.equal(f.selectors.get('#feedback-result').textContent, '反馈提交过于频繁，请稍后再试。');assert.equal(message.value, 'Synthetic report');
});

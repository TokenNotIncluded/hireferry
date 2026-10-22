// HF-QA-001: real DOM regression for the shared feedback widget and enhanced
// selects. Only synthetic guest sessions and an isolated localhost fixture.
// HFY_PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node tests/feedback-language.browser.mjs
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..'), staticRoot = resolve(root, 'static');
const moduleName = process.env.HFY_PLAYWRIGHT_MODULE || 'playwright';
const { chromium } = await import(isAbsolute(moduleName) ? pathToFileURL(moduleName).href : moduleName);
const errors = [], unexpected = [], submissions = [], external = [];
const config = { name: 'HireFerry synthetic feedback fixture', default_language: 'zh', google_enabled: false, humangrant_enabled: false, email_enabled: true, development: false, cli_distribution: { release_repository: 'TokenNotIncluded/hireferry', release_tag_prefix: 'cli-v' }, pricing: { currency: 'CNY', currency_decimals: 2 }, limits: { list_page_size: 50 }, providers: [] };
const session = { user: null, admin: false, super_admin: false, staff: null, staff_role: null, bootstrap: false, csrf: null };
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml' };
let heldFeedback = null, feedbackStatus = 200, origin, browser, page, stage = 'setup';
const server = createServer(async (request, response) => {
  try {
    const url = new URL(request.url, 'http://fixture.local');
    const json = (value, status = 200) => { response.writeHead(status, { 'content-type': 'application/json', 'cache-control': 'no-store' });response.end(JSON.stringify(value)); };
    if (url.pathname === '/api/config') return json(config);
    if (url.pathname === '/api/session') return json(session);
    if (url.pathname === '/api/feedback' && request.method === 'POST') {
      submissions.push(JSON.parse(Buffer.concat(await Array.fromAsync(request)).toString()));
      const held = heldFeedback;heldFeedback = null;
      if (held) { held.started();await held.wait; }
      return json(feedbackStatus === 200 ? { id: 42 } : { error: 'Synthetic temporary failure' }, feedbackStatus);
    }
    if (url.pathname.startsWith('/api/')) { unexpected.push(url.pathname);return json({ error: 'Unexpected fixture endpoint' }, 404); }
    const named = url.pathname === '/oauth/device' ? 'cli-auth.html' : url.pathname === '/oauth/connections' ? 'cli-connections.html' : 'index.html';
    const file = url.pathname.startsWith('/static/') ? resolve(staticRoot, decodeURIComponent(url.pathname.slice(8))) : resolve(staticRoot, named);
    if (!file.startsWith(staticRoot + '/')) throw new Error('Invalid fixture path');
    response.writeHead(200, { 'content-type': types[extname(file)] || 'application/octet-stream', 'cache-control': 'no-store' });response.end(await readFile(file));
  } catch (error) { errors.push(error.message);response.writeHead(500);response.end('Synthetic fixture error'); }
});
const words = {
  zh: { open: '快速反馈', title: '快速反馈', move: '轻点反馈，拖动调整位置', category: '反馈类型', idea: '改进建议', options: ['遇到问题', '改进建议', '其他'], message: '详细描述', placeholder: '你遇到了什么？希望我们如何改进？', hint: '仅填写问题或建议。', close: '关闭', send: '提交反馈', sending: '提交中…', error: '反馈暂不可用，请稍后重试。', success: '已收到反馈 · #42' },
  en: { open: 'Quick feedback', title: 'Quick feedback', move: 'Click for feedback, drag to move', category: 'Category', idea: 'Suggestion', options: ['Something is broken', 'Suggestion', 'Other'], message: 'Describe it', placeholder: 'What happened? What would you like us to change?', hint: 'Describe the problem or suggestion.', close: 'Close', send: 'Send feedback', sending: 'Sending…', error: 'Feedback is unavailable. Please try again later.', success: 'Feedback received · #42' },
};
const frame = () => page.evaluate(() => new Promise(resolve_ => requestAnimationFrame(() => requestAnimationFrame(resolve_))));
const languageButton = () => page.locator('[data-action="language"], #language').first();
async function switchLanguage(language) {
  await languageButton().click();
  await page.waitForFunction(value => document.documentElement.lang.startsWith(value), language === 'en' ? 'en' : 'zh');
  await page.waitForFunction(value => document.querySelector('#feedback-launch')?.getAttribute('aria-label') === value, words[language].open);
  await frame();
}
async function translated(language, { open = true, enhanced = false, pending = false } = {}) {
  const t = words[language];
  assert.equal(await page.locator('#feedback-launch').getAttribute('aria-label'), t.open);
  assert.equal(await page.locator('#feedback-launch').getAttribute('title'), t.move);
  assert.equal(await page.locator('#feedback-panel').isVisible(), open);
  assert.equal(await page.locator('#feedback-title').textContent(), t.title);
  assert.equal(await page.locator('[data-feedback-close]').getAttribute('aria-label'), t.close);
  assert.equal(await page.locator('label[for="feedback-category"]').textContent(), t.category);
  assert.equal(await page.locator('label[for="feedback-message"]').textContent(), t.message);
  assert.equal(await page.locator('#feedback-message').getAttribute('placeholder'), t.placeholder);
  assert.equal(await page.locator('.feedback-hint').textContent(), t.hint);
  assert.deepEqual(await page.locator('#feedback-category option').allTextContents(), t.options);
  assert.equal(await page.locator('.feedback-send').textContent(), pending ? t.sending : t.send);
  if (open) assert.equal(await page.getByRole('dialog', { name: t.title, exact: true }).count(), 1);
  if (enhanced) {
    const selected = await page.locator('#feedback-category').evaluate(select => select.selectedOptions[0].label);
    assert.equal(await page.locator('#feedback-form .choice-value').textContent(), selected);
    assert.equal(await page.locator('#feedback-form .choice-trigger').getAttribute('aria-label'), `${t.category}: ${selected}`);
  }
}
async function exercise(path, enhanced) {
  stage = path + ': open and closed language changes';
  await page.goto(origin + path, { waitUntil: 'domcontentloaded' });
  await page.locator('#feedback-launch').waitFor();
  if (enhanced) await page.waitForFunction(() => document.querySelector('#root')?.dataset.navigationState === 'ready');
  await translated('zh', { open: false, enhanced });
  await switchLanguage('en');await translated('en', { open: false, enhanced });
  await page.locator('#feedback-launch').click();
  const draft = 'Synthetic feedback draft / 演示草稿';
  await page.locator('#feedback-message').fill(draft);
  if (enhanced) { await page.locator('#feedback-form .choice-trigger').click();await page.getByRole('option', { name: 'Suggestion', exact: true }).click(); }
  else await page.locator('#feedback-category').selectOption('idea');
  const rect = await page.locator('#feedback-launch').boundingBox();
  await page.mouse.move(rect.x + rect.width / 2, rect.y + rect.height / 2);await page.mouse.down();await page.mouse.move(rect.x + rect.width / 2 - 130, rect.y + rect.height / 2 - 45, { steps: 5 });await page.mouse.up();
  const position = await page.evaluate(() => localStorage.getItem('hireferry-feedback-position'));
  assert.ok(position, 'The drag should persist only a relative position');
  for (const language of ['zh', 'en']) {
    await switchLanguage(language);await translated(language, { enhanced });
    assert.equal(await page.locator('#feedback-message').inputValue(), draft);
    assert.equal(await page.locator('#feedback-category').inputValue(), 'idea');
    assert.equal(await page.locator('#feedback-length').textContent(), String([...draft].length));
    assert.equal(await page.evaluate(() => localStorage.getItem('hireferry-feedback-position')), position);
  }
  await page.locator('[data-feedback-close]').click();await switchLanguage('zh');await translated('zh', { open: false, enhanced });
  await page.locator('#feedback-launch').click();await translated('zh', { enhanced });
  assert.equal(await page.locator('#feedback-message').inputValue(), draft);

  stage = path + ': sending and result language changes';
  let release, started;const requested = new Promise(resolve_ => { started = resolve_; });
  heldFeedback = { started, wait: new Promise(resolve_ => { release = resolve_; }) };feedbackStatus = 503;
  await page.locator('.feedback-send').click();await requested;
  await translated('zh', { enhanced, pending: true });assert.equal(await page.locator('.feedback-send').isDisabled(), true);
  await switchLanguage('en');await translated('en', { enhanced, pending: true });assert.equal(await page.locator('.feedback-send').isDisabled(), true);
  release();await page.waitForFunction(value => document.querySelector('#feedback-result').textContent === value, words.en.error);
  assert.equal(await page.locator('#feedback-message').inputValue(), draft);
  await switchLanguage('zh');assert.equal(await page.locator('#feedback-result').textContent(), words.zh.error);
  feedbackStatus = 200;await page.locator('.feedback-send').click();
  await page.waitForFunction(value => document.querySelector('#feedback-result').textContent === value, words.zh.success);
  await switchLanguage('en');await translated('en', { enhanced });
  assert.equal(await page.locator('#feedback-result').textContent(), words.en.success);
  assert.equal(await page.locator('#feedback-message').inputValue(), '');
  process.stdout.write(`ok - ${path}: closed/open locales, accessible labels, draft/category/position, pending and result states\n`);
}

try {
  await new Promise((resolve_, reject) => { server.once('error', reject);server.listen(0, '127.0.0.1', resolve_); });
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true, ...(process.env.HFY_BROWSER_EXECUTABLE ? { executablePath: process.env.HFY_BROWSER_EXECUTABLE } : {}) });
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: 'reduce', locale: 'zh-CN' });
  await context.addInitScript(() => { localStorage.setItem('hireferry-language', 'zh');localStorage.setItem('hireferry-theme', 'light');localStorage.setItem('hireferry-palette', 'neutral'); });
  await context.route('**/*', route => { if (route.request().url().startsWith(origin + '/')) return route.continue();external.push(route.request().url());return route.abort(); });
  page = await context.newPage();page.setDefaultTimeout(10000);page.on('pageerror', error => errors.push(error.message));
  for (const path of ['/cli', '/oauth/device', '/oauth/connections']) await exercise(path, path === '/cli');

  stage = 'open custom listbox relabeling';
  await page.goto(origin + '/cli', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#root')?.dataset.navigationState === 'ready');
  await page.evaluate(() => {
    const section = document.createElement('section');section.id = 'synthetic-control-fixture';section.style.cssText = 'position:fixed;top:90px;left:100px;width:280px;z-index:90;background:white;padding:16px';
    const label = document.createElement('label');label.htmlFor = 'synthetic-category';label.textContent = '演示类型';
    const select = document.createElement('select');select.id = 'synthetic-category';
    for (let i = 0; i < 50; i++) { const option = document.createElement('option');option.value = String(i);option.textContent = `演示选项 ${i}`;select.append(option); }
    section.append(label, select);document.body.append(section);
  });
  const trigger = page.locator('#synthetic-control-fixture .choice-trigger');await trigger.click();await trigger.press('End');
  const highlighted = await trigger.getAttribute('aria-activedescendant'), before = await page.locator('.choice-menu').evaluate(element => element.scrollTop);
  assert.ok(before > 0, 'The list must be scrolled before relabeling');
  await page.evaluate(() => {
    const select = document.querySelector('#synthetic-category');select.labels[0].firstChild.textContent = 'Synthetic category';
    for (const option of select.options) option.firstChild.textContent = `Synthetic option ${option.value}`;
    select.options[10].label = 'Explicit translated label';document.documentElement.lang = 'en';
  });
  await page.waitForFunction(() => document.querySelector('.choice-menu [data-index="49"]')?.textContent === 'Synthetic option 49');
  assert.equal(await trigger.getAttribute('aria-activedescendant'), highlighted);
  assert.equal(await page.locator('.choice-menu').getAttribute('aria-label'), 'Synthetic category');
  assert.equal(await trigger.getAttribute('aria-label'), 'Synthetic category: Synthetic option 0');
  assert.equal(await page.locator('.choice-menu [data-index="10"]').textContent(), 'Explicit translated label');
  assert.equal(await page.locator('#synthetic-category').inputValue(), '0');
  assert.equal(await page.locator('.choice-menu').evaluate(element => element.scrollTop), before);
  await trigger.press('Enter');assert.equal(await page.locator('#synthetic-category').inputValue(), '49');
  process.stdout.write('ok - open listbox observes native text/label changes and preserves highlighted option and scroll\n');
  assert.equal(submissions.length, 6);assert.ok(submissions.every(value => value.message === 'Synthetic feedback draft / 演示草稿' && value.category === 'idea'));
  assert.deepEqual(unexpected, []);assert.deepEqual(external, []);assert.deepEqual(errors, []);
  process.stdout.write('Passed HF-QA-001 feedback language regression with synthetic data only.\n');
} catch (error) {
  process.stderr.write(JSON.stringify({ stage, errors, unexpected, visible: (await page?.locator('#feedback-widget').innerText().catch(() => '') || '').slice(0, 1600) }) + '\n');throw error;
} finally {
  await browser?.close();await new Promise(resolve_ => { server.closeAllConnections();server.close(resolve_); });
}

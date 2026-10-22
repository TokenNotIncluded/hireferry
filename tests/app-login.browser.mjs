// Run against the real Rust routes and a temporary SQLite database. The two browser
// contexts model a WebView and its separate system browser, not native OS integration.
import assert from 'node:assert/strict';
import { dirname, isAbsolute, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { startAppLoginFixture } from './helpers/app-login-browser-fixture.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const modulePath = process.env.HFY_PLAYWRIGHT_MODULE || 'playwright';
const { chromium } = await import(isAbsolute(modulePath) ? pathToFileURL(modulePath).href : modulePath);
const fixture = await startAppLoginFixture(root);
const contexts = [], pageErrors = [], externalRequests = [];
const delay = milliseconds => new Promise(resolve_ => setTimeout(resolve_, milliseconds));
let browser;

function deferred() {
  let resolve_;
  const promise = new Promise(resolve => { resolve_ = resolve; });
  return { promise, resolve: resolve_ };
}
async function within(promise, label, timeout = 10000) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(label)), timeout); })]); }
  finally { clearTimeout(timer); }
}
const matches = (response, path) => new URL(response.url()).pathname === path && response.request().method() === 'POST';
const responseFor = (page, path) => page.waitForResponse(response => matches(response, path));

try {
  browser = await chromium.launch({
    headless: true, executablePath: process.env.HFY_BROWSER_EXECUTABLE || undefined,
    args: ['--no-sandbox', '--disk-cache-size=1048576', '--media-cache-size=1048576']
  });
  async function createContext({ source = false, language = 'zh' } = {}) {
    const context = await browser.newContext({ viewport: { width: 390, height: 900 }, locale: language === 'en' ? 'en-US' : 'zh-CN', reducedMotion: 'reduce' });
    contexts.push(context);
    await context.addInitScript(language_ => {
      if (!localStorage.getItem('hireferry-language')) localStorage.setItem('hireferry-language', language_);
      localStorage.setItem('hireferry-theme', 'light');
      localStorage.setItem('hireferry-palette', 'neutral');
    }, language);
    await context.route('**/*', route => {
      const url = new URL(route.request().url());
      if (url.origin === fixture.origin) return route.continue();
      externalRequests.push(url.origin + url.pathname);
      return route.abort();
    });
    if (source) await context.addCookies([{
      name: 'hf_session', value: fixture.sourceSession, url: fixture.origin,
      httpOnly: true, sameSite: 'Lax', expires: fixture.sourceExpires
    }]);
    const page = await context.newPage();
    page.setDefaultTimeout(10000);
    page.on('pageerror', error => pageErrors.push(error.message));
    return { context, page };
  }
  async function api(page, path, body, csrf) {
    return page.evaluate(async ({ path_, body_, csrf_ }) => {
      const response = await fetch(path_, {
        method: body_ === undefined ? 'GET' : 'POST', credentials: 'same-origin', cache: 'no-store',
        headers: { ...(body_ === undefined ? {} : { 'content-type': 'application/json' }), ...(csrf_ ? { 'x-csrf-token': csrf_ } : {}) },
        ...(body_ === undefined ? {} : { body: JSON.stringify(body_) })
      });
      return { status: response.status, body: await response.json() };
    }, { path_: path, body_: body, csrf_: csrf });
  }
  async function cookie(context, name) { return (await context.cookies()).find(item => item.name === name); }
  async function signedIn(page) { return (await api(page, '/api/session')).body; }
  async function assertAnonymous(page) { assert.equal((await signedIn(page)).user, null, 'App must remain signed out'); }
  async function beginApp(app) {
    const response = responseFor(app.page, '/api/auth/app/start');
    await app.page.goto(fixture.origin + '/auth/app');
    const result = await response;
    assert.equal(result.status(), 200);
    assert.equal(result.headers()['cache-control'], 'no-store');
    const data = await result.json();
    await app.page.locator('#app-flow').waitFor({ state: 'visible' });
    assert.match(data.request, /^[A-Za-z0-9_-]{43}$/);
    assert.match(data.comparison_code, /^[A-Z0-9]{4}-[A-Z0-9]{4}$/);
    const link = new URL(await app.page.locator('#open-browser').getAttribute('href'));
    assert.equal(link.origin, fixture.origin);
    assert.equal(link.pathname, '/auth/app/authorize');
    assert.deepEqual([...link.searchParams.keys()], ['request']);
    assert.equal(link.searchParams.get('request'), data.request);
    assert.equal(link.hash, '');
    const proof = await cookie(app.context, 'hf_app_login');
    assert.ok(proof, 'Only the initiating context receives a proof cookie');
    assert.equal(proof.httpOnly, true);
    assert.equal(proof.sameSite, 'Strict');
    assert.equal(proof.path, '/api/auth/app');
    assert.ok(!JSON.stringify(data).includes(proof.value), 'Proof must not be serialized to page JSON');
    assert.ok(!link.href.includes(proof.value), 'Proof must not enter the public URL');
    assert.ok(!(await app.page.evaluate(() => document.cookie)).includes('hf_app_login'));
    assert.equal(await cookie(app.context, 'hf_session'), undefined);
    await assertAnonymous(app.page);
    return data;
  }
  async function openConsent(system, request) {
    await system.page.goto(request.verification_uri);
    await system.page.locator('#consent').waitFor({ state: 'visible' });
    assert.equal(await system.page.locator('#browser-code').textContent(), request.comparison_code);
    assert.equal(await system.page.locator('#confirm').isChecked(), false);
    assert.equal(await system.page.locator('#approve').isDisabled(), true);
    assert.equal(await cookie(system.context, 'hf_app_login'), undefined);
  }
  async function approve(system) {
    const response = responseFor(system.page, '/api/auth/app/decide');
    await system.page.locator('#confirm').check();
    await system.page.locator('#approve').click();
    const result = await response;
    assert.equal(result.status(), 200);
    assert.deepEqual(await result.json(), { approved: true });
    await system.page.locator('#finished').waitFor({ state: 'visible' });
    assert.equal(await system.page.locator('#return-app').getAttribute('href'), 'hireferry://login-return');
  }
  async function wake(app) {
    await app.page.bringToFront();
    await app.page.evaluate(() => window.dispatchEvent(new Event('hireferry-app-return')));
  }
  async function complete(app) {
    await wake(app);
    await app.page.waitForURL(url => url.pathname === '/app');
    const identity = await signedIn(app.page);
    assert.equal(identity.user.id, fixture.member.id);
    assert.equal(identity.admin, false);
    const session = await cookie(app.context, 'hf_session');
    assert.ok(session?.httpOnly);
    assert.notEqual(session.value, fixture.sourceSession);
    assert.equal(await cookie(app.context, 'hf_app_login'), undefined);
    return session;
  }
  async function assertSourceIntact(system) {
    assert.equal((await cookie(system.context, 'hf_session')).value, fixture.sourceSession);
    assert.equal((await signedIn(system.page)).user.id, fixture.member.id);
    assert.ok(fixture.session(fixture.sourceSession), 'The browser source session must remain live');
  }
  async function assertLayout(page, label) {
    const bounds = await page.evaluate(() => ({ viewport: innerWidth, document: document.documentElement.scrollWidth, body: document.body.scrollWidth }));
    assert.ok(bounds.document <= bounds.viewport + 1 && bounds.body <= bounds.viewport + 1, `${label}: horizontal overflow ${JSON.stringify(bounds)}`);
  }
  async function languagesAndWidths(page, flow) {
    for (const width of [320, 390]) {
      await page.setViewportSize({ width, height: 900 });
      for (const language of ['zh-CN', 'en']) {
        if (await page.locator('html').getAttribute('lang') !== language) await page.locator('#language').click();
        assert.equal(await page.locator('html').getAttribute('lang'), language);
        assert.equal(await page.locator('.card').getAttribute('aria-label'), language === 'en' ? 'App sign-in' : '应用登录');
        const primary = page.locator(flow === 'app' ? '#open-browser' : '#approve');
        assert.equal(await primary.textContent(), flow === 'app' ? (language === 'en' ? 'Confirm in browser' : '打开浏览器确认') : (language === 'en' ? 'Confirm app sign-in' : '确认登录应用'));
        await assertLayout(page, `${flow}, ${language}, ${width}px`);
      }
    }
  }

  const system = await createContext({ source: true });
  {
    const app = await createContext();
    const request = await beginApp(app);
    await openConsent(system, request);
    await languagesAndWidths(app.page, 'app');
    await languagesAndWidths(system.page, 'browser');
    assert.equal(fixture.grant(request.request).status, 'pending', 'Opening the public link must not grant consent');
    assert.equal(fixture.sessionCount(), 1);
    const withoutProof = await api(system.page, '/api/auth/app/redeem', { request: request.request });
    assert.equal(withoutProof.status, 401, 'Even an authenticated browser cannot redeem the public ID without the initiating proof');
    await assertSourceIntact(system);
    await approve(system);
    const alsoWithoutProof = await api(system.page, '/api/auth/app/redeem', { request: request.request });
    assert.equal(alsoWithoutProof.status, 401);
    const appSession = await complete(app);
    const stored = fixture.session(appSession.value);
    assert.equal(stored.user_id, fixture.member.id);
    assert.equal(stored.recent_auth_at, fixture.sourceRecent, 'Handoff must not refresh recent authentication');
    assert.ok(stored.expires_at <= fixture.sourceExpires, 'Handoff must not extend the source session lifetime');
    assert.equal(fixture.sessionCount(), 2);
    const replay = await api(app.page, '/api/auth/app/redeem', { request: request.request });
    assert.ok([401, 409].includes(replay.status));
    assert.equal(fixture.sessionCount(), 2);
    await assertSourceIntact(system);
    await app.context.close();
    process.stdout.write('ok - explicit consent, isolated HttpOnly proof, one-time app session, source lifetime and source browser preserved\n');
    process.stdout.write('ok - app and consent screens in Chinese/English at 320px and 390px without horizontal overflow\n');
  }

  {
    const app = await createContext({ language: 'en' });
    const baseline = fixture.sessionCount(), request = await beginApp(app);
    await openConsent(system, request);
    const response = responseFor(system.page, '/api/auth/app/decide');
    await system.page.locator('#deny').click();
    assert.deepEqual(await (await response).json(), { approved: false });
    await wake(app);
    await app.page.locator('#finished').waitFor({ state: 'visible' });
    assert.equal(await app.page.locator('#status').textContent(), 'Connection declined');
    await assertAnonymous(app.page);
    assert.equal(await cookie(app.context, 'hf_app_login'), undefined);
    assert.equal(fixture.sessionCount(), baseline);
    await assertSourceIntact(system);
    await app.context.close();
    process.stdout.write('ok - browser decline leaves the app signed out and consumes no session\n');
  }

  {
    const app = await createContext({ language: 'en' });
    const baseline = fixture.sessionCount(), request = await beginApp(app);
    await openConsent(system, request);
    const response = responseFor(app.page, '/api/auth/app/cancel');
    await app.page.locator('#cancel').click();
    assert.equal((await response).status(), 200);
    await app.page.locator('#finished').waitFor({ state: 'visible' });
    assert.equal(await app.page.locator('#status').textContent(), 'Connection cancelled');
    assert.equal(fixture.grant(request.request), undefined);
    await assertAnonymous(app.page);
    const lateApproval = await api(system.page, '/api/auth/app/decide', { request: request.request, approve: true }, fixture.sourceCsrf);
    assert.equal(lateApproval.status, 400);
    assert.equal(fixture.sessionCount(), baseline);
    await app.context.close();
    process.stdout.write('ok - cancellation invalidates the outstanding browser approval link\n');
  }

  {
    const app = await createContext({ language: 'en' }), emailBrowser = await createContext({ language: 'en' });
    const request = await beginApp(app);
    await emailBrowser.page.goto(request.verification_uri);
    await emailBrowser.page.locator('#email-form').waitFor({ state: 'visible' });
    assert.equal(await emailBrowser.page.locator('#consent').isVisible(), false);
    const started = responseFor(emailBrowser.page, '/api/auth/email/start');
    await emailBrowser.page.locator('#email').fill(fixture.member.email);
    await emailBrowser.page.locator('#email-form button').click();
    const startResponse = await started;
    assert.equal(startResponse.status(), 200);
    // The development-only response is read inside the test, never rendered or logged.
    const code = (await startResponse.json()).dev_code;
    assert.match(code, /^\d{8}$/);
    await emailBrowser.page.locator('#otp').fill(code);
    const verified = responseFor(emailBrowser.page, '/api/auth/email/verify');
    await emailBrowser.page.locator('#otp-form button').click();
    assert.equal((await verified).status(), 200);
    await emailBrowser.page.locator('#consent').waitFor({ state: 'visible' });
    assert.equal(await emailBrowser.page.locator('#confirm').isChecked(), false);
    assert.equal(await emailBrowser.page.locator('#approve').isDisabled(), true);
    assert.equal(await emailBrowser.page.locator('#otp').inputValue(), '');
    assert.ok(!emailBrowser.page.url().includes(code));
    assert.equal(fixture.grant(request.request).status, 'pending');
    await assertAnonymous(app.page);
    await assertLayout(emailBrowser.page, 'email sign-in to consent');
    await approve(emailBrowser);
    const appSession = await complete(app), browserSession = await cookie(emailBrowser.context, 'hf_session');
    assert.notEqual(appSession.value, browserSession.value);
    assert.equal((await signedIn(emailBrowser.page)).user.id, fixture.member.id);
    assert.equal(fixture.userCount(), 1, 'Email OTP must reuse the one synthetic member');
    await assertSourceIntact(system);
    await app.context.close(); await emailBrowser.context.close();
    process.stdout.write('ok - real development email OTP reaches unchecked consent before a separate app session is issued\n');
  }

  {
    const app = await createContext({ language: 'en' });
    const heldRedeem = deferred(), releaseRedeem = deferred(), heldCancel = deferred(), releaseCancel = deferred();
    const baseline = fixture.sessionCount();
    await app.page.route('**/api/auth/app/redeem', async route => {
      heldRedeem.resolve(); await releaseRedeem.promise; await route.continue();
    });
    await app.page.route('**/api/auth/app/cancel', async route => {
      heldCancel.resolve(); await releaseCancel.promise; await route.continue();
    });
    try {
      const request = await beginApp(app);
      await wake(app); await within(heldRedeem.promise, 'The pending redeem was not intercepted');
      const cancelResponse = responseFor(app.page, '/api/auth/app/cancel');
      await app.page.locator('#cancel').click();
      await within(heldCancel.promise, 'Cancel must be sent while redeem is still pending');
      assert.equal(await app.page.locator('#status').textContent(), 'Cancelling. Please wait…');
      assert.equal(await app.page.locator('#finished').isVisible(), false, 'Cancellation must not claim success before server acknowledgement');
      assert.ok(fixture.grant(request.request));
      releaseCancel.resolve(); assert.equal((await cancelResponse).status(), 200);
      await app.page.locator('#finished').waitFor({ state: 'visible' });
      const redeemResponse = responseFor(app.page, '/api/auth/app/redeem');
      releaseRedeem.resolve(); assert.equal((await redeemResponse).status(), 401);
      assert.equal(await app.page.locator('#status').textContent(), 'Connection cancelled');
      assert.equal(new URL(app.page.url()).pathname, '/auth/app');
      await assertAnonymous(app.page);
      assert.equal(fixture.grant(request.request), undefined);
      assert.equal(fixture.sessionCount(), baseline);
      process.stdout.write('ok - hanging redeem does not block cancel; cancellation is shown only after the real server confirms it\n');
    } finally { releaseRedeem.resolve(); releaseCancel.resolve(); await app.context.close(); }
  }

  {
    const app = await createContext({ language: 'en' });
    const committed = deferred(), releaseResponse = deferred(), delivered = deferred();
    const baseline = fixture.sessionCount();
    await app.page.route('**/api/auth/app/redeem', async route => {
      const response = await route.fetch();
      const data = await response.json();
      if (data.status === 'complete') { committed.resolve(); await releaseResponse.promise; }
      await route.fulfill({ response });
      if (data.status === 'complete') delivered.resolve();
    });
    try {
      const request = await beginApp(app);
      await openConsent(system, request); await approve(system); await wake(app);
      await within(committed.promise, 'Approved redemption did not reach the real server');
      assert.equal(fixture.grant(request.request).status, 'consumed');
      assert.equal(fixture.sessionCount(), baseline + 1);
      const cancelled = responseFor(app.page, '/api/auth/app/cancel');
      await app.page.locator('#cancel').click(); assert.equal((await cancelled).status(), 200);
      await app.page.locator('#finished').waitFor({ state: 'visible' });
      assert.equal(fixture.grant(request.request), undefined);
      assert.equal(fixture.sessionCount(), baseline, 'Cancel must revoke the newly committed app session');
      const late = responseFor(app.page, '/api/auth/app/redeem');
      releaseResponse.resolve(); assert.equal((await late).status(), 200);
      await within(delivered.promise, 'Late redeem response was not delivered');
      await delay(100);
      assert.equal(new URL(app.page.url()).pathname, '/auth/app', 'A late successful response must not navigate after cancellation');
      assert.equal(await app.page.locator('#status').textContent(), 'Connection cancelled');
      await assertAnonymous(app.page);
      await assertSourceIntact(system);
      process.stdout.write('ok - cancel after redeem commit revokes the app session; a late successful response cannot sign the app back in\n');
    } finally { releaseResponse.resolve(); await app.context.close(); }
  }

  assert.deepEqual(pageErrors, []);
  assert.deepEqual(externalRequests, [], 'All authentication traffic must stay on the isolated fixture origin');
  process.stdout.write('ok - no page exceptions, external authentication traffic, or production data; temporary server/database cleaned up\n');
} finally {
  await Promise.allSettled(contexts.map(context => context.close()));
  await browser?.close();
  await fixture.close();
}

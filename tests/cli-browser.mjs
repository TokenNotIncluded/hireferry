// Optional real-browser gate. It is deliberately separate from *.test.mjs.
// Build the service and CLI, install Playwright + Chromium, then run:
//   HFY_SERVER_BIN=target/debug/hireferry HFY_CLI_BIN=target/debug/hfy \
//     node tests/cli-browser.mjs
// Optional: HFY_PLAYWRIGHT_MODULE, HFY_BROWSER_EXECUTABLE,
// HFY_BROWSER_ARGS_MODULE (a module exporting a Chromium class with .args).
// No accounts, cookies, authorization codes, state values, or token values are
// printed. All server data and CLI configuration are created in a temporary dir.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { createConnection, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { setTimeout as pause } from 'node:timers/promises';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const children = new Set();
const clients = [];
const stdoutSecrets = new Set();
const credentialSecrets = new Set();
let temporary;
let browser;
let origin;
let stage = 'setup';
let passed = 0;

class GateError extends Error {}
function check(condition, message) {
  if (!condition) throw new GateError(message);
}
async function step(name, action) {
  stage = name;
  await action();
  passed += 1;
  process.stdout.write(`ok - ${name}\n`);
}
async function requireResponseSuccess(response, label) {
  if (response.ok()) return;
  const publicErrors = new Set([
    'Request origin is not allowed',
    'Enter a valid email address',
    'Email sign-in is not configured',
    'Too many attempts; please try again later',
    'Internal error. Please try again.',
    'We could not send the email; try again later',
    'Request verification failed; reload and try again',
  ]);
  const body = await response.json().catch(() => null);
  const reason = publicErrors.has(body?.error) ? body.error : 'Unrecognized server error';
  const sentOrigin = await response.request().headerValue('origin');
  const originState = sentOrigin === null ? 'missing' : sentOrigin === origin ? 'matches configured issuer' : 'different from configured issuer';
  throw new GateError(`${label} (HTTP ${response.status()}; ${reason}; request Origin ${originState})`);
}
async function loadModule(specifier) {
  return import(isAbsolute(specifier) ? pathToFileURL(specifier).href : specifier);
}
function browserArguments(arguments_) {
  // Serverless browser bundles sometimes disable the browser's origin checks.
  // Keep their process/sandbox compatibility flags, but test authentication with
  // ordinary web origin enforcement and site-isolation settings.
  const unsafeFlags = new Set(['--disable-web-security', '--disable-site-isolation-trials', '--allow-running-insecure-content']);
  const originFeatures = new Set(['IsolateOrigins', 'site-per-process', 'SitePerProcess']);
  return arguments_.flatMap(argument => {
    if (unsafeFlags.has(argument.split('=')[0])) return [];
    if (!argument.startsWith('--disable-features=')) return [argument];
    const features = argument.slice('--disable-features='.length).split(',').filter(feature => !originFeatures.has(feature));
    return features.length ? [`--disable-features=${features.join(',')}`] : [];
  });
}
async function within(promise, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new GateError(message)), milliseconds); }),
    ]);
  } finally { clearTimeout(timer); }
}
async function eventually(condition, message, milliseconds = 10000) {
  const deadline = Date.now() + milliseconds;
  while (Date.now() < deadline) {
    const value = await condition();
    if (value) return value;
    await pause(25);
  }
  throw new GateError(message);
}
function start(binary, arguments_, environment) {
  const child = spawn(binary, arguments_, { cwd: root, env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
  const record = { child, stdout: '', stderr: '', closed: false, overflow: false };
  children.add(record);
  for (const stream of ['stdout', 'stderr']) {
    child[stream].setEncoding('utf8');
    child[stream].on('data', chunk => {
      record[stream] += chunk;
      if (record[stream].length > 128 * 1024) {
        record.overflow = true;
        child.kill('SIGKILL');
      }
    });
  }
  record.done = new Promise(resolve_ => {
    child.once('error', () => { record.spawnFailed = true; });
    child.once('close', (code, signal) => {
      record.closed = true;
      record.code = code;
      record.signal = signal;
      resolve_(record);
    });
  });
  return record;
}
function cleanEnvironment() {
  return Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('HFY_')));
}
function cli(directory, command, scope = 'identity') {
  const record = start(resolve(root, process.env.HFY_CLI_BIN || 'target/debug/hfy'), [
    '--json', '--server', origin, '--client-id', 'hireferry-cli', '--scope', scope,
    '--storage', 'file', '--config-dir', directory, '--poll-interval-seconds', '1',
    '--request-timeout-seconds', '5', ...command,
  ], cleanEnvironment());
  clients.push(record);
  return record;
}
function safeOutput(record, result) {
  const visit = value => {
    if (!value || typeof value !== 'object') return;
    for (const [key, child] of Object.entries(value)) {
      check(!/^(?:access_token|refresh_token|device_code|user_code|code_verifier|client_secret|id_token|cookie|authorization_code)$/i.test(key), 'CLI JSON contained a credential field');
      visit(child);
    }
  };
  visit(result);
  for (const secret of stdoutSecrets) {
    check(!record.stdout.includes(secret), 'CLI stdout exposed sign-in data');
  }
  for (const secret of credentialSecrets) {
    check(!record.stdout.includes(secret) && !record.stderr.includes(secret), 'CLI output exposed a saved credential');
  }
}
async function finish(record, expectedError = null, timeout = 20000) {
  await within(record.done, timeout, 'CLI did not terminate within the test deadline');
  check(!record.spawnFailed && !record.overflow, 'CLI process could not run safely');
  check(record.signal === null, 'CLI exited by signal instead of producing a structured result');
  check(record.code === (expectedError ? 1 : 0), 'CLI returned an unexpected exit status');
  let result;
  try { result = JSON.parse(record.stdout); } catch { throw new GateError('CLI stdout was not one complete JSON document'); }
  check(result && typeof result === 'object' && !Array.isArray(result), 'CLI returned an invalid JSON result');
  if (expectedError) check(result.error?.code === expectedError, 'CLI returned an unexpected structured error');
  else check(!result.error, 'CLI returned an unexpected structured error');
  safeOutput(record, result);
  return result;
}
async function signInLink(record, flow) {
  const prefix = `${origin}/oauth/${flow === 'oauth' ? 'authorize?' : 'device'}`;
  const link = await eventually(() => {
    const line = record.stderr.split(/\r?\n/).slice(0, -1).find(line => line.startsWith(prefix));
    const lifetimeReady = flow !== 'device' || /Code expires in [1-9][0-9]* seconds/.test(record.stderr);
    if (line && lifetimeReady) return line;
    check(!record.closed, 'CLI exited before displaying a sign-in link');
    return null;
  }, 'CLI did not display a sign-in link');
  const url = new URL(link);
  for (const name of ['state', 'code_challenge', 'user_code']) {
    if (url.searchParams.get(name)) stdoutSecrets.add(url.searchParams.get(name));
  }
  if (flow === 'device') {
    check(/Code expires in [1-9][0-9]* seconds/.test(record.stderr), 'Device login did not display its lifetime');
  }
  return url;
}
async function savedCredential(directory) {
  const files = await readdir(directory).catch(() => []);
  const file = files.find(name => name.endsWith('.credential'));
  if (!file) return null;
  // Windows file storage is DPAPI protected; do not decrypt it in this test.
  if (process.platform === 'win32') return { protected: true };
  let value;
  try { value = JSON.parse(await readFile(join(directory, file), 'utf8')); }
  catch { throw new GateError('Private credential file was invalid'); }
  for (const field of ['access_token', 'refresh_token']) {
    check(typeof value[field] === 'string' && value[field].length > 16, 'Saved credential was incomplete');
    credentialSecrets.add(value[field]);
  }
  return value;
}
async function freePort() {
  const server = createServer();
  await new Promise((resolve_, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve_);
  });
  const port = server.address().port;
  await new Promise((resolve_, reject) => server.close(error => error ? reject(error) : resolve_()));
  return port;
}
async function callbackClosed(url) {
  const address = new URL(url.searchParams.get('redirect_uri'));
  const connected = await within(new Promise(resolve_ => {
    const socket = createConnection({ host: address.hostname, port: Number(address.port) });
    socket.once('connect', () => { socket.destroy(); resolve_(true); });
    socket.once('error', () => { socket.destroy(); resolve_(false); });
    socket.setTimeout(1000, () => { socket.destroy(); resolve_(true); });
  }), 1500, 'Callback listener did not close');
  check(!connected, 'OAuth callback listener remained open after login finished');
}
async function consent(page, link) {
  await page.goto(link.href, { waitUntil: 'domcontentloaded' });
  await page.locator('#consent').waitFor({ state: 'visible' });
  check((await page.locator('#scope').textContent()).trim() === 'identity', 'Consent displayed an unexpected scope');
  check((await page.locator('#site').textContent()).trim() === origin, 'Consent displayed the wrong issuer');
  check((await page.locator('#permissions').textContent()).includes('Basic sign-in does not grant'), 'Consent did not distinguish basic identity from business permissions');
  if (link.pathname === '/oauth/device') {
    check((await page.locator('#display-code').textContent()).trim() === link.searchParams.get('user_code'), 'Consent did not display the terminal device code');
  }
}
async function cleanup() {
  if (browser) await browser.close().catch(() => {});
  for (const record of children) if (!record.closed) record.child.kill('SIGTERM');
  await Promise.all([...children].map(async record => {
    if (record.closed) return;
    await within(record.done, 2000, 'Process cleanup deadline').catch(() => record.child.kill('SIGKILL'));
  }));
  if (temporary) await rm(temporary, { recursive: true, force: true });
}

try {
  const serverBinary = resolve(root, process.env.HFY_SERVER_BIN || 'target/debug/hireferry');
  const clientBinary = resolve(root, process.env.HFY_CLI_BIN || 'target/debug/hfy');
  check((await stat(serverBinary)).isFile() && (await stat(clientBinary)).isFile(), 'Build the service and CLI before running this gate');
  const { chromium } = await loadModule(process.env.HFY_PLAYWRIGHT_MODULE || 'playwright');
  const launch = { headless: true };
  if (process.env.HFY_BROWSER_EXECUTABLE) launch.executablePath = process.env.HFY_BROWSER_EXECUTABLE;
  if (process.env.HFY_BROWSER_ARGS_MODULE) {
    const module = await loadModule(process.env.HFY_BROWSER_ARGS_MODULE);
    const bundled = module.default || module;
    launch.args = browserArguments(bundled.args);
  }
  browser = await chromium.launch(launch);
  const context = await browser.newContext();
  context.setDefaultTimeout(10000);
  await context.addInitScript(() => {
    try { localStorage.setItem('hireferry-language', 'en'); } catch { /* Optional in the callback page. */ }
  });
  const page = await context.newPage();
  page.on('request', request => {
    const url = new URL(request.url());
    if (url.pathname === '/callback') {
      for (const key of ['code', 'state']) if (url.searchParams.get(key)) stdoutSecrets.add(url.searchParams.get(key));
    }
  });
  temporary = await mkdtemp(join(tmpdir(), 'hireferry-cli-browser-'));
  const port = await freePort();
  origin = `http://localhost:${port}`;
  const configuration = join(temporary, 'server.toml');
  await writeFile(configuration, [
    `bind = "127.0.0.1:${port}"`, `public_url = "${origin}"`, 'webauthn_rp_id = "localhost"',
    `database = ${JSON.stringify(join(temporary, 'service.sqlite3'))}`, 'development = true',
    '[google]', 'enabled = false', '[smtp]', 'enabled = false', '[webhooks]', 'enabled = false',
    '[cli_auth]', 'poll_seconds = 5', 'grant_minutes = 1', 'rate_limit_requests = 1000',
  ].join('\n'), { mode: 0o600 });
  const service = start(serverBinary, ['--config', configuration, 'serve'], { ...cleanEnvironment(), RUST_LOG: 'error' });
  await eventually(async () => {
    check(!service.closed, 'Test service exited before becoming ready');
    try { return (await fetch(`http://127.0.0.1:${port}/healthz`, { signal: AbortSignal.timeout(1000) })).status === 204; }
    catch { return false; }
  }, 'Test service did not become ready', 20000);
  const oauthDirectory = join(temporary, 'oauth-client');
  let userId;

  await step('OAuth browser sign-in resumes after real email verification', async () => {
    const process_ = cli(oauthDirectory, ['auth', 'login', '--oauth', '--no-browser', '--timeout', '60']);
    const link = await signInLink(process_, 'oauth');
    await page.goto(link.href, { waitUntil: 'domcontentloaded' });
    await page.locator('#member-login').waitFor({ state: 'visible' });
    await page.locator('#member-login').click();
    await page.waitForURL(url => url.pathname === '/login');
    await page.locator('#login-email').fill('cli-browser@example.test');
    const [response] = await Promise.all([
      page.waitForResponse(response => new URL(response.url()).pathname === '/api/auth/email/start' && response.request().method() === 'POST'),
      page.locator('form[data-form="email-start"] button[type="submit"]').click(),
    ]);
    await requireResponseSuccess(response, 'Email login start failed');
    const startData = await response.json();
    check(typeof startData.dev_code === 'string', 'Development email login did not return a code');
    stdoutSecrets.add(startData.dev_code);
    await page.locator('#email-code').fill(startData.dev_code);
    await page.locator('form[data-form="email-verify"] button[type="submit"]').click();
    await page.waitForURL(url => url.pathname === '/oauth/authorize' && url.searchParams.get('state') === link.searchParams.get('state'));
    await page.locator('#consent').waitFor({ state: 'visible' });
    check((await page.locator('#scope').textContent()).trim() === 'identity', 'Resumed authorization changed the requested scope');
    check((await page.locator('#permissions').textContent()).includes('Basic sign-in does not grant'), 'Basic identity consent included unexpected business permissions');
    await page.locator('#approve').click();
    const result = await finish(process_);
    check(result.authenticated && !result.reused, 'OAuth login did not create a new session');
    check(result.identity?.user?.role === 'unassigned', 'Basic identity login unexpectedly required role selection');
    check(result.identity.capabilities?.identity && !result.identity.capabilities.mcp && !result.identity.capabilities.admin, 'Basic login granted unexpected capabilities');
    userId = result.identity.user.id;
    const saved = await savedCredential(oauthDirectory);
    check(saved !== null, 'OAuth login did not persist credentials');
    if (!saved.protected) check(saved.account?.kind === 'user' && saved.account.id === userId, 'Saved credentials were not bound to the verified account');
    await callbackClosed(link);
  });

  await step('Status, identity, capabilities, cached login and token rotation', async () => {
    const status = await finish(cli(oauthDirectory, ['auth', 'status']));
    check(status.authenticated && status.identity.user.id === userId, 'Remote status did not verify the current account');
    const who = await finish(cli(oauthDirectory, ['whoami']));
    check(who.identity.user.id === userId, 'whoami returned another account');
    const capabilities = await finish(cli(oauthDirectory, ['capabilities']));
    check(capabilities.scope === 'identity' && capabilities.capabilities.identity && !capabilities.capabilities.mcp, 'Capabilities did not reflect the basic grant');
    const reused = cli(oauthDirectory, ['auth', 'login', '--oauth', '--no-browser']);
    check((await finish(reused)).reused === true, 'Valid login was not reused');
    check(!reused.stderr.includes('/oauth/authorize?') && !reused.stderr.includes('/oauth/device'), 'Cached login started a new consent flow');
    const before = await savedCredential(oauthDirectory);
    check(before !== null, 'Saved session disappeared before refresh');
    const refreshed = await finish(cli(oauthDirectory, ['auth', 'refresh']));
    check(refreshed.refreshed && refreshed.identity.user.id === userId, 'Explicit refresh did not retain the account');
    const after = await savedCredential(oauthDirectory);
    check(after !== null, 'Refresh did not persist credentials');
    if (!before.protected) {
      check(before.access_token !== after.access_token && before.refresh_token !== after.refresh_token, 'Refresh did not rotate both saved credentials');
      check(after.refresh_expires_at <= before.refresh_expires_at, 'Refresh extended the absolute authorization deadline');
    }
  });

  await step('Real connection page renders dates and revokes the token family', async () => {
    await page.goto(`${origin}/oauth/connections`);
    const button = page.locator('#connections .connection button').first();
    await button.waitFor({ state: 'visible' });
    const dates = await page.locator('#connections .connection .hint').first().textContent();
    check(!dates.includes('1970') && dates.includes(String(new Date().getFullYear())), 'Connection dates were not rendered from Unix seconds');
    const [revoked] = await Promise.all([
      page.waitForResponse(response => new URL(response.url()).pathname.startsWith('/api/cli/connections/') && response.request().method() === 'DELETE'),
      button.click(),
    ]);
    await requireResponseSuccess(revoked, 'Connection revocation failed');
    await eventually(async () => (await page.locator('#connections .connection-status').first().textContent()) === 'Revoked', 'Connection UI did not show revocation');
    const status = await finish(cli(oauthDirectory, ['auth', 'status']));
    check(status.authenticated === false, 'Revoked family remained authenticated');
    const logout = await finish(cli(oauthDirectory, ['auth', 'logout']));
    check(logout.local_removed === true, 'Logout did not remove the revoked local session');
  });

  await step('Device approval signs in without requiring an AI-qualified role', async () => {
    const directory = join(temporary, 'device-client');
    const process_ = cli(directory, ['auth', 'login', '--device', '--no-browser', '--timeout', '45']);
    const link = await signInLink(process_, 'device');
    await consent(page, link);
    await page.locator('#approve').click();
    const result = await finish(process_);
    check(result.authenticated && result.identity.user.id === userId, 'Device login did not use the confirmed account');
    check(result.identity.scope === 'identity' && !result.identity.capabilities.mcp, 'Device login granted an unexpected scope');
    await savedCredential(directory);
    const logout = await finish(cli(directory, ['auth', 'logout']));
    check(logout.local_removed && logout.remote_revoked, 'Device logout did not revoke and remove credentials');
    const again = await finish(cli(directory, ['auth', 'logout']));
    check(again.already_logged_out === true, 'Logout was not idempotent');
  });

  await step('Ineligible MCP consent retains account details, denial and account switching', async () => {
    const directory = join(temporary, 'mcp-ineligible');
    const process_ = cli(directory, ['auth', 'login', '--oauth', '--no-browser', '--timeout', '45'], 'mcp');
    const link = await signInLink(process_, 'oauth');
    const [response] = await Promise.all([
      page.waitForResponse(response => new URL(response.url()).pathname === '/api/cli/authorization' && response.request().method() === 'GET'),
      page.goto(link.href, { waitUntil: 'domcontentloaded' }),
    ]);
    await requireResponseSuccess(response, 'Ineligible consent details failed');
    const details = await response.json();
    check(details.can_approve === false && typeof details.approval_error === 'string' && details.approval_error.length > 0, 'Ineligible consent did not explain its approval restriction');
    await page.locator('#consent').waitFor({ state: 'visible' });
    check((await page.locator('#scope').textContent()).trim() === 'identity mcp', 'Ineligible consent hid or changed the requested scope');
    check((await page.locator('#account').textContent()).includes('cli-browser@example.test'), 'Ineligible consent hid the current account');
    check(await page.locator('#approve').isDisabled(), 'Ineligible consent allowed approval');
    for (const selector of ['#deny', '#switch-account']) {
      check(await page.locator(selector).isVisible() && await page.locator(selector).isEnabled(), 'Ineligible consent disabled denial or account switching');
    }
    await page.locator('#deny').click();
    await finish(process_, 'access_denied');
    check(await savedCredential(directory) === null, 'Denied ineligible consent saved credentials');
    await callbackClosed(link);
  });

  for (const flow of ['oauth', 'device']) {
    await step(`${flow === 'oauth' ? 'OAuth' : 'Device'} denial returns a safe structured error`, async () => {
      const directory = join(temporary, `${flow}-denied`);
      const process_ = cli(directory, ['auth', 'login', `--${flow}`, '--no-browser', '--timeout', '45']);
      const link = await signInLink(process_, flow);
      await consent(page, link);
      await page.locator('#deny').click();
      await finish(process_, 'access_denied');
      check(await savedCredential(directory) === null, 'Denied authorization saved credentials');
      if (flow === 'oauth') await callbackClosed(link);
    });
  }

  for (const flow of ['oauth', 'device']) {
    await step(`${flow === 'oauth' ? 'OAuth' : 'Device'} Ctrl-C closes the flow without credentials`, async () => {
      const directory = join(temporary, `${flow}-cancelled`);
      const process_ = cli(directory, ['auth', 'login', `--${flow}`, '--no-browser', '--timeout', '45']);
      const link = await signInLink(process_, flow);
      process_.child.kill('SIGINT');
      await finish(process_, 'cancelled', 5000);
      check(await savedCredential(directory) === null, 'Cancelled authorization saved credentials');
      if (flow === 'oauth') await callbackClosed(link);
    });
  }

  await step('Unanswered OAuth consent has a finite timeout and closes its listener', async () => {
    const directory = join(temporary, 'oauth-timeout');
    const started = Date.now();
    const process_ = cli(directory, ['auth', 'login', '--oauth', '--no-browser', '--timeout', '10']);
    const link = await signInLink(process_, 'oauth');
    await finish(process_, 'login_timeout', 15000);
    check(Date.now() - started < 15000, 'Login exceeded its configured timeout');
    await callbackClosed(link);
    check(await savedCredential(directory) === null, 'Timed-out authorization saved credentials');
  });

  await step('Every CLI result stays structured and excludes all observed credentials', async () => {
    for (const record of clients) safeOutput(record, JSON.parse(record.stdout));
  });
  process.stdout.write(`Passed ${passed} real server/CLI/browser checks.\n`);
} catch (error) {
  // Playwright errors may include URL query strings, so never print their raw
  // messages, call logs, request bodies, or stacks from authentication tests.
  const detail = error instanceof GateError ? error.message : 'Browser or process operation failed; sign-in diagnostics were withheld.';
  process.stderr.write(`not ok - ${stage}: ${detail}\n`);
  process.exitCode = 1;
} finally {
  await cleanup();
}

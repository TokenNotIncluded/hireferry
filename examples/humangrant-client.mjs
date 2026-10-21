#!/usr/bin/env node
/** A small HumanGrant 0.1 client. Uses Node.js built-ins; requires Node.js 22+. */
import { createPrivateKey, createPublicKey, generateKeyPairSync, KeyObject, randomBytes, createHash, sign } from 'node:crypto';
import { constants } from 'node:fs';
import { mkdir, open, rename, unlink } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const COMPONENTS = '("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization")';
const ACTIONS = new Set(['profile.read', 'profile.write', 'jobs.read', 'jobs.write', 'people.read', 'orders.read', 'chat.read', 'chat.write', 'matches.read', 'mandate.read']);
const COMMANDS = new Set(['request', 'status', 'claim', 'session', 'call', 'cancel']);
const SAFE_ID = /^[A-Za-z0-9_-]{1,64}$/;
const TOKEN = /^[A-Za-z0-9_-]{43}$/;

export const HELP = `HumanGrant client for HireFerry (Node.js 22+)

Create a request and forward its human_authorization_url to the account owner:
  node examples/humangrant-client.mjs request --origin https://hireferry.lmm.best \\
    --state "$HOME/.config/hireferry/agent.json" \\
    --permission profile.read:self --purpose "Read my profile"

After the human signs in and explicitly approves:
  node examples/humangrant-client.mjs status  --state "$HOME/.config/hireferry/agent.json"
  node examples/humangrant-client.mjs claim   --state "$HOME/.config/hireferry/agent.json"
  node examples/humangrant-client.mjs session --state "$HOME/.config/hireferry/agent.json"
  node examples/humangrant-client.mjs call    --state "$HOME/.config/hireferry/agent.json" \\
    --tool get_profile --arguments '{}'

Cancel an unclaimed request:
  node examples/humangrant-client.mjs cancel --state "$HOME/.config/hireferry/agent.json"

Options:
  --state FILE        Required. Private JSON file containing the signing key and credential.
  --origin URL        Required for a new file. HTTPS, or HTTP on a loopback host for development.
                      If supplied later, it must match the file's original service exactly.
  --permission A:R    Repeat for each action/resource. For example profile.read:self or
                      chat.read:platform_<account-id>. No wildcard permissions.
  --purpose TEXT      Required for request; plain text, at most 240 characters.
  --client-id ID      request only; defaults to hireferry-cli. Must be allowed by the service.
  --ttl SECONDS       request only; 60–86400, within the service's maximum.
                      Omit to use the service's default (at most 3600 seconds).
  --tool NAME         call only; an allowed MCP tool, for example get_profile.
  --arguments JSON    call only; a JSON object, defaults to {}.
  --help              Show this help.

The owner confirms in their browser. This client never approves, opens a browser,
sends email, retries a claim, or automatically polls. Wait for Retry-After after HTTP 429.
Credentials are written with mode 0600 and never printed. Keep this file private.
A claim can succeed only once. If its response is lost, revoke the grant in HireFerry
and create a new request with a new state file. Read results are printed, not saved.
`;

export function normalizeOrigin(input) {
  if (typeof input !== 'string' || /[\s\\?#@]/.test(input)) throw new Error('Use an origin without credentials, path, query, fragment, or spaces.');
  let url;
  try { url = new URL(input); } catch { throw new Error('Invalid service origin.'); }
  const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (url.pathname !== '/' || url.username || url.password || url.search || url.hash ||
      !(url.protocol === 'https:' || (url.protocol === 'http:' && loopback))) {
    throw new Error('Use HTTPS, or HTTP on localhost, 127.0.0.1, or [::1], with no path.');
  }
  return url.origin;
}

function privateKey(value) {
  const key = value instanceof KeyObject ? value : createPrivateKey(value);
  if (key.type !== 'private' || key.asymmetricKeyType !== 'ed25519') throw new Error('The state must contain an Ed25519 private key.');
  return key;
}

/** The exact HumanGrant 0.1 signature profile; body bytes must be sent unchanged. */
export function signHeaders({ key, method, origin, path, body = Buffer.alloc(0), token = 'anonymous', created = Math.floor(Date.now() / 1000), nonce = randomBytes(24) }) {
  origin = normalizeOrigin(origin);
  if (!['GET', 'POST'].includes(method) || typeof path !== 'string' || !path.startsWith('/') ||
      path.includes('//') || /[%\\?#]/.test(path) || path.endsWith('/') || path.length > 512) {
    throw new Error('Invalid signed method or path.');
  }
  if (!Buffer.isBuffer(body)) body = Buffer.from(body);
  if (method === 'GET' && body.length) throw new Error('Signed GET requests must have an empty body.');
  if (!Number.isSafeInteger(created) || created < 0 || !Buffer.isBuffer(nonce) || nonce.length !== 24) throw new Error('Invalid signature time or nonce.');
  if (token !== 'anonymous' && (!TOKEN.test(token) || Buffer.from(token, 'base64url').toString('base64url') !== token)) throw new Error('Invalid stored credential.');
  const signingKey = privateKey(key);
  const publicKey = createPublicKey(signingKey).export({ format: 'jwk' }).x;
  const actorId = `ed25519:${createHash('sha256').update(Buffer.from(publicKey, 'base64url')).digest('base64url')}`;
  const digest = `sha-256=:${createHash('sha256').update(body).digest('base64')}:`;
  const authorization = `HumanGrant ${token}`;
  const params = `${COMPONENTS};created=${created};expires=${created + 60};nonce="${nonce.toString('base64url')}";keyid="${actorId}";alg="ed25519";tag="humangrant-0.1"`;
  const base = `"@method": ${method}\n"@target-uri": ${origin}${path}\n"content-digest": ${digest}\n"content-type": application/json\n"x-hg-key": ${publicKey}\n"authorization": ${authorization}\n"@signature-params": ${params}`;
  return {
    'Content-Type': 'application/json',
    'Content-Digest': digest,
    'X-HG-Key': publicKey,
    Authorization: authorization,
    'Signature-Input': `hg=${params}`,
    Signature: `hg=:${sign(null, Buffer.from(base), signingKey).toString('base64')}:`,
  };
}

function validateState(state) {
  if (state?.version !== 1 || normalizeOrigin(state.origin) !== state.origin || typeof state.private_key !== 'string') throw new Error('Invalid state file. Do not replace an existing identity to recover from an error.');
  privateKey(state.private_key);
  if (state.request_id !== undefined && !SAFE_ID.test(state.request_id)) throw new Error('Invalid request ID in state file.');
  if (state.access_token !== undefined && (!TOKEN.test(state.access_token) || Buffer.from(state.access_token, 'base64url').toString('base64url') !== state.access_token)) throw new Error('Invalid credential in state file.');
  return state;
}

function checkFile(info) {
  if (!info.isFile() || info.size > 65_536) throw new Error('The state must be a small regular file.');
  if (process.platform !== 'win32' && ((info.mode & 0o077) !== 0 || (process.getuid && info.uid !== process.getuid()))) {
    throw new Error('The state file must be owned by you and readable only by you (chmod 600).');
  }
}

export async function loadState(file) {
  const handle = await open(file, constants.O_RDONLY | (constants.O_NOFOLLOW || 0));
  try {
    checkFile(await handle.stat());
    let state;
    try { state = JSON.parse(await handle.readFile('utf8')); } catch { throw new Error('The state file is not valid JSON. Preserve it and check the file.'); }
    return validateState(state);
  } finally { await handle.close(); }
}

export async function saveState(file, state) {
  validateState(state);
  await mkdir(dirname(file), { recursive: true, mode: 0o700 });
  const temporary = `${file}.${randomBytes(12).toString('hex')}.tmp`;
  let handle;
  try {
    handle = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | (constants.O_NOFOLLOW || 0), 0o600);
    await handle.chmod(0o600);
    await handle.writeFile(`${JSON.stringify(state, null, 2)}\n`, 'utf8');
    await handle.sync();
    await handle.close();
    handle = undefined;
    await rename(temporary, file);
  } finally {
    if (handle) await handle.close();
    await unlink(temporary).catch(error => { if (error.code !== 'ENOENT') throw error; });
  }
}

export function parseArguments(argv) {
  const result = { permissions: [] };
  const options = new Set(['origin', 'state', 'permission', 'purpose', 'client-id', 'ttl', 'tool', 'arguments']);
  for (let index = 0; index < argv.length; index++) {
    const value = argv[index];
    if (value === '--help' || value === '-h') { result.help = true; continue; }
    if (!value.startsWith('--')) {
      if (result.command || !COMMANDS.has(value)) throw new Error('Choose one command: request, status, claim, session, call, or cancel.');
      result.command = value;
      continue;
    }
    const at = value.indexOf('=');
    const name = value.slice(2, at === -1 ? undefined : at);
    if (!options.has(name)) throw new Error(`Unknown option: --${name}`);
    const input = at === -1 ? argv[++index] : value.slice(at + 1);
    if (input === undefined || input.startsWith('--')) throw new Error(`Missing value for --${name}.`);
    if (name === 'permission') result.permissions.push(input);
    else {
      if (Object.hasOwn(result, name)) throw new Error(`Duplicate option: --${name}`);
      result[name] = input;
    }
  }
  if (result.help) return result;
  if (!result.command || !result.state) throw new Error('A command and --state FILE are required. Use --help for examples.');
  const requestOnly = ['purpose', 'client-id', 'ttl'];
  if (result.command !== 'request' && (result.permissions.length || requestOnly.some(name => Object.hasOwn(result, name)))) throw new Error('Permission, purpose, client ID, and TTL options are for request only.');
  if (result.command !== 'call' && (result.tool !== undefined || result.arguments !== undefined)) throw new Error('Tool and arguments options are for call only.');
  return result;
}

function requestBody(options) {
  if (!options.permissions.length || options.permissions.length > 16) throw new Error('Request between 1 and 16 explicit --permission action:resource values.');
  const permissions = options.permissions.map(value => {
    const [action, resource, extra] = value.split(':');
    if (extra !== undefined || !ACTIONS.has(action) || !SAFE_ID.test(resource || '')) throw new Error('Invalid permission. Use a supported action and self or an exact resource ID.');
    return { action, resource };
  });
  if (new Set(options.permissions).size !== permissions.length) throw new Error('Do not repeat a permission.');
  const purpose = options.purpose;
  if (typeof purpose !== 'string' || !purpose.trim() || [...purpose].length > 240 || /[\u0000-\u001f\u007f]/.test(purpose)) throw new Error('Supply --purpose as plain text of 1–240 characters.');
  const clientId = options['client-id'] || 'hireferry-cli';
  if (!SAFE_ID.test(clientId)) throw new Error('Invalid client ID.');
  const body = { client_id: clientId, target_service: 'hireferry', purpose, requested_permissions: permissions };
  if (options.ttl !== undefined) {
    const ttl = Number(options.ttl);
    if (!Number.isInteger(ttl) || ttl < 60 || ttl > 86400) throw new Error('TTL must be an integer from 60 to 86400 seconds.');
    body.session_ttl_seconds = ttl;
  }
  return body;
}

async function send(state, method, path, payload, token, fetchImpl) {
  const body = method === 'GET' ? Buffer.alloc(0) : Buffer.from(JSON.stringify(payload));
  const headers = signHeaders({ key: state.private_key, method, origin: state.origin, path, body, token });
  const response = await fetchImpl(`${state.origin}${path}`, {
    method, headers, body: method === 'GET' ? undefined : body,
    redirect: 'manual', signal: AbortSignal.timeout(10_000),
  });
  if (response.status >= 300 && response.status < 400) throw new Error('The service returned a redirect. No credential was forwarded; check the configured origin.');
  let value;
  try { value = await response.json(); } catch { throw new Error(`The service returned invalid JSON (HTTP ${response.status}).`); }
  if (!response.ok) {
    const code = typeof value.error === 'string' && /^[a-z_]{1,64}$/.test(value.error) ? ` (${value.error})` : '';
    const rawRetry = response.headers.get('retry-after') || value.retry_after_seconds;
    const retry = Number(rawRetry);
    const wait = response.status === 429 && Number.isFinite(retry) && retry > 0 ? ` Wait at least ${Math.ceil(retry)} seconds before retrying.` : '';
    throw new Error(`HumanGrant request failed: HTTP ${response.status}${code}.${wait}`);
  }
  return value;
}

export async function run(argv, { fetchImpl = globalThis.fetch, log = console.log } = {}) {
  const options = parseArguments(argv);
  if (options.help) { log(HELP); return; }
  const file = resolve(options.state);
  await mkdir(dirname(file), { recursive: true, mode: 0o700 });
  let lock;
  try { lock = await open(`${file}.lock`, 'wx', 0o600); }
  catch (error) {
    if (error.code === 'EEXIST') throw new Error('This state file is in use. If a previous process stopped, check it before removing the .lock file.');
    throw error;
  }
  try {
    let state;
    try { state = await loadState(file); }
    catch (error) {
      if (error.code !== 'ENOENT' || options.command !== 'request') throw error;
      if (!options.origin) throw new Error('A new identity requires --origin URL.');
      state = { version: 1, origin: normalizeOrigin(options.origin), private_key: generateKeyPairSync('ed25519').privateKey.export({ type: 'pkcs8', format: 'pem' }) };
    }
    if (options.origin && normalizeOrigin(options.origin) !== state.origin) throw new Error('The state file belongs to another origin. Use a separate state file for each service.');
    if (options.command === 'request') {
      if (state.access_token || (state.request_id && !['cancelled', 'rejected', 'expired'].includes(state.phase))) throw new Error('This file already has a request or grant. Finish or cancel the request, or use a new private state file after revoking the grant.');
      const body = requestBody(options);
      await saveState(file, state); // Persist the signing identity before any network operation.
      const value = await send(state, 'POST', '/v1/requests', body, 'anonymous', fetchImpl);
      const human = new URL(value.human_authorization_url);
      if (!SAFE_ID.test(value.request_id || '') || human.origin !== state.origin || !/^\/human\/[A-Za-z0-9_-]{1,64}$/.test(human.pathname) || human.search || human.hash) throw new Error('The service returned an invalid authorization link.');
      Object.assign(state, { request_id: value.request_id, phase: value.phase, request_expires_at: value.expires_at });
      await saveState(file, state);
      log(JSON.stringify({ request_id: value.request_id, human_authorization_url: human.href, expires_at: value.expires_at, poll_after_seconds: value.poll_after_seconds }, null, 2));
      return;
    }
    if (['status', 'claim', 'cancel'].includes(options.command) && !state.request_id) throw new Error('The state file has no authorization request.');
    if (options.command === 'status') {
      const value = await send(state, 'GET', `/v1/requests/${state.request_id}`, undefined, 'anonymous', fetchImpl);
      if (typeof value.phase === 'string') { state.phase = value.phase; await saveState(file, state); }
      log(JSON.stringify(value, null, 2));
    } else if (options.command === 'claim') {
      if (state.access_token) throw new Error('A credential is already stored. Use session or call; do not claim again.');
      const value = await send(state, 'POST', `/v1/requests/${state.request_id}/claim`, {}, 'anonymous', fetchImpl);
      if (value.token_type !== 'HumanGrant' || typeof value.access_token !== 'string' || !TOKEN.test(value.access_token) || !SAFE_ID.test(value.grant_id || '') || !Number.isSafeInteger(value.expires_at)) throw new Error('The service returned an invalid credential. Revoke the grant and start a new request.');
      Object.assign(state, { access_token: value.access_token, grant_id: value.grant_id, grant_expires_at: value.expires_at, phase: 'succeeded' });
      try { await saveState(file, state); }
      catch { throw new Error('The credential was issued but could not be saved. Revoke the grant and start a new request with a new state file.'); }
      log(JSON.stringify({ grant_id: value.grant_id, expires_at: value.expires_at, credential_saved_to: file }, null, 2));
    } else if (options.command === 'cancel') {
      if (state.access_token) throw new Error('A claimed grant must be revoked in the HireFerry AI connections page.');
      const value = await send(state, 'POST', `/v1/requests/${state.request_id}/cancel`, {}, 'anonymous', fetchImpl);
      state.phase = 'cancelled';
      await saveState(file, state);
      log(JSON.stringify(value, null, 2));
    } else {
      if (!state.access_token) throw new Error('No credential is stored. Ask the human to approve, then use claim.');
      if (options.command === 'session') {
        log(JSON.stringify(await send(state, 'GET', '/v1/session', undefined, state.access_token, fetchImpl), null, 2));
      } else {
        if (!/^[a-z][a-z0-9_]{0,63}$/.test(options.tool || '')) throw new Error('Supply --tool with an MCP tool name.');
        let args;
        try { args = JSON.parse(options.arguments || '{}'); } catch { throw new Error('--arguments must be a valid JSON object.'); }
        if (!args || typeof args !== 'object' || Array.isArray(args)) throw new Error('--arguments must be a JSON object.');
        const result = await send(state, 'POST', '/mcp', { jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: options.tool, arguments: args } }, state.access_token, fetchImpl);
        log(JSON.stringify(result, null, 2));
        if (result.error || result.result?.isError) throw new Error('The MCP tool returned an error. Check the granted action and resource.');
      }
    }
  } finally {
    await lock.close();
    await unlink(`${file}.lock`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  run(process.argv.slice(2)).catch(error => {
    // Do not print requests, raw server error bodies, keys, or credentials.
    console.error(error?.name === 'TimeoutError' ? 'The request timed out. Do not automatically repeat a claim.' : (error?.message || 'HumanGrant client failed.'));
    process.exitCode = 1;
  });
}

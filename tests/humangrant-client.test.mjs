import test from 'node:test';
import assert from 'node:assert/strict';
import { createPrivateKey, createPublicKey, generateKeyPairSync, verify } from 'node:crypto';
import { chmod, mkdtemp, readFile, readdir, rm, stat, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { HELP, loadState, normalizeOrigin, parseArguments, run, saveState, signHeaders } from '../examples/humangrant-client.mjs';

const ORIGIN = 'https://hireferry.example.test';
const TOKEN = Buffer.alloc(32, 5).toString('base64url'); // Public, disposable test value only.
const key = generateKeyPairSync('ed25519').privateKey.export({ type: 'pkcs8', format: 'pem' });

function state(extra = {}) { return { version: 1, origin: ORIGIN, private_key: key, ...extra }; }
async function location(t) {
  const dir = await mkdtemp(join(tmpdir(), 'hireferry-hg-client-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  return { dir, file: join(dir, 'private.json') };
}
function response(value, status = 200, headers = {}) {
  return new Response(JSON.stringify(value), { status, headers: { 'content-type': 'application/json', ...headers } });
}
function verifyRequest(url, options) {
  assert.equal(options.redirect, 'manual');
  assert.ok(options.signal instanceof AbortSignal);
  const h = options.headers;
  assert.equal(h['Content-Type'], 'application/json');
  const publicKey = createPublicKey({ format: 'der', type: 'spki', key: Buffer.concat([
    Buffer.from('302a300506032b6570032100', 'hex'), Buffer.from(h['X-HG-Key'], 'base64url'),
  ]) });
  const base = `"@method": ${options.method}\n"@target-uri": ${url}\n"content-digest": ${h['Content-Digest']}\n"content-type": application/json\n"x-hg-key": ${h['X-HG-Key']}\n"authorization": ${h.Authorization}\n"@signature-params": ${h['Signature-Input'].slice(3)}`;
  assert.equal(verify(null, Buffer.from(base), publicKey, Buffer.from(h.Signature.slice(4, -1), 'base64')), true);
}

test('signatures match the upstream public HumanGrant 0.1 vector exactly', () => {
  const privateKey = createPrivateKey({ type: 'pkcs8', format: 'der', key: Buffer.concat([
    Buffer.from('302e020100300506032b657004220420', 'hex'),
    Buffer.from('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60', 'hex'),
  ]) });
  const body = Buffer.from('{"client_id":"example-cli","target_service":"notes","purpose":"Read a note","requested_permissions":[{"action":"notes.read","resource":"alice-note"}],"expected_human_id":"alice"}');
  const headers = signHeaders({ key: privateKey, method: 'POST', origin: 'https://auth.example.test', path: '/v1/requests', body, created: 1700000000, nonce: Buffer.from(Array.from({ length: 24 }, (_, i) => i)) });
  assert.equal(headers['X-HG-Key'], '11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo');
  assert.equal(headers['Content-Digest'], 'sha-256=:jN6odj5Tli9ZfoLdtLxtE/sUNtMKAKM2V7xq8/qri5Y=:');
  assert.equal(headers['Signature-Input'], 'hg=("@method" "@target-uri" "content-digest" "content-type" "x-hg-key" "authorization");created=1700000000;expires=1700000060;nonce="AAECAwQFBgcICQoLDA0ODxAREhMUFRYX";keyid="ed25519:If4x36FUomFia_hUBG_SJxt77UtqvkWqWId-9H-XIbk";alg="ed25519";tag="humangrant-0.1"');
  assert.equal(headers.Signature, 'hg=:jmjuzgV58zQY0A8N+T9sss24Jwa8Kah5bvs2VSdWA43TScypSfoPkzlERC6XLBOvx8+D+ZqQ0WOVKwS2/Gd3AA==:');
});

test('only HTTPS origins and explicit loopback HTTP origins are accepted', () => {
  assert.equal(normalizeOrigin('https://EXAMPLE.TEST:443/'), 'https://example.test');
  for (const origin of ['http://localhost:3000', 'http://127.0.0.1:8080', 'http://[::1]:3000']) assert.equal(normalizeOrigin(origin), origin);
  for (const origin of ['http://example.test', 'https://user:secret@example.test', 'https://@example.test', 'https://example.test/path', 'https://example.test?', 'https://example.test#', 'https://example.test/?x=1', 'https://example.test\\', ' https://example.test', 'file:///tmp/key', 'https://example.test\n']) assert.throws(() => normalizeOrigin(origin), Error, origin);
});

test('signed paths, methods, empty GET bodies and canonical credentials are checked', () => {
  const args = { key, method: 'POST', origin: ORIGIN, path: '/mcp', body: Buffer.from('{}') };
  for (const path of ['/mcp?x=1', '/mcp%2fextra', '//mcp', '/mcp/', '/mcp#x', '/mcp\\x']) assert.throws(() => signHeaders({ ...args, path }));
  assert.throws(() => signHeaders({ ...args, method: 'GET' }));
  assert.throws(() => signHeaders({ ...args, method: 'DELETE' }));
  assert.throws(() => signHeaders({ ...args, nonce: Buffer.alloc(23) }));
  assert.throws(() => signHeaders({ ...args, token: 'invalid token' }));
  const read = signHeaders({ ...args, method: 'GET', path: '/v1/session', body: Buffer.alloc(0), token: TOKEN });
  assert.equal(read.Authorization, `HumanGrant ${TOKEN}`);
  assert.equal(read['Content-Digest'], 'sha-256=:47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=:');
});

test('state saves atomically with owner-only permissions and refuses unsafe reads', async t => {
  const { dir, file } = await location(t);
  await saveState(file, state());
  assert.equal((await stat(file)).mode & 0o777, 0o600);
  assert.deepEqual(await loadState(file), state());
  await saveState(file, state({ request_id: 'request-1' }));
  assert.equal((await loadState(file)).request_id, 'request-1');
  assert.deepEqual(await readdir(dir), ['private.json']);
  await chmod(file, 0o644);
  if (process.platform !== 'win32') await assert.rejects(loadState(file), /chmod 600/);
  await chmod(file, 0o600);
  const link = join(dir, 'linked.json');
  await symlink(file, link);
  if (process.platform !== 'win32') await assert.rejects(loadState(link));
});

test('request, claim, session and call use signed HTTP while keeping credentials out of output', async t => {
  const { file } = await location(t);
  const output = [];
  const calls = [];
  const fetchImpl = async (url, options) => {
    verifyRequest(url, options);
    calls.push({ url, options });
    const path = new URL(url).pathname;
    if (path === '/v1/requests') {
      assert.equal(options.headers.Authorization, 'HumanGrant anonymous');
      const body = JSON.parse(options.body);
      assert.deepEqual(body.requested_permissions, [{ action: 'profile.read', resource: 'self' }, { action: 'chat.read', resource: 'platform_example' }]);
      return response({ request_id: 'request-1', human_authorization_url: `${ORIGIN}/human/link-1`, phase: 'waiting_human', expires_at: 9999999999, poll_after_seconds: 5 }, 201);
    }
    if (path.endsWith('/claim')) return response({ access_token: TOKEN, token_type: 'HumanGrant', grant_id: 'grant-1', expires_at: 9999999999 });
    assert.equal(options.headers.Authorization, `HumanGrant ${TOKEN}`);
    if (path === '/v1/session') return response({ actor: { kind: 'ai' }, grant_id: 'grant-1', effective_permissions: [{ action: 'profile.read', resource: 'self' }] });
    assert.equal(path, '/mcp');
    assert.equal(JSON.parse(options.body).params.name, 'get_profile');
    return response({ jsonrpc: '2.0', id: 1, result: { isError: false, structuredContent: { name: 'Temporary example result' } } });
  };
  const env = { fetchImpl, log: value => output.push(value) };
  await run(['request', `--state=${file}`, '--origin', ORIGIN, '--permission', 'profile.read:self', '--permission=chat.read:platform_example', '--purpose', 'Read the requested profile'], env);
  const pending = await loadState(file);
  assert.equal(pending.request_id, 'request-1');
  assert.equal(pending.access_token, undefined);
  assert.equal(JSON.parse(output[0]).human_authorization_url, `${ORIGIN}/human/link-1`);
  await run(['claim', '--state', file], env);
  assert.equal((await loadState(file)).access_token, TOKEN);
  await run(['session', '--state', file], env);
  await run(['call', '--state', file, '--tool', 'get_profile', '--arguments', '{}'], env);
  assert.equal(calls.length, 4);
  assert.equal((await stat(file)).mode & 0o777, 0o600);
  assert.ok(output.every(text => !text.includes(TOKEN) && !text.includes('BEGIN PRIVATE KEY')));
  assert.ok(!(await readFile(file, 'utf8')).includes('Temporary example result'), 'Business responses must never be saved in the identity file');
});

test('state origin binding rejects credential transfer before fetching', async t => {
  const { file } = await location(t);
  await saveState(file, state({ access_token: TOKEN, request_id: 'request-1', phase: 'succeeded' }));
  let calls = 0;
  await assert.rejects(run(['session', '--state', file, '--origin', 'https://another.example.test'], { fetchImpl: () => { calls++; }, log: () => {} }), /another origin/);
  assert.equal(calls, 0);
  assert.equal((await loadState(file)).access_token, TOKEN);
});

test('omitted TTL uses the service default while an explicit TTL is preserved', async t => {
  // A service may set a maximum below 3600 seconds. Its own default is the
  // smaller of that maximum and 3600; a client must not override it implicitly.
  const serviceMaximum = 900;
  for (const requestedTtl of [undefined, '600']) {
    const { file } = await location(t);
    let acceptedTtl;
    const fetchImpl = async (url, options) => {
      verifyRequest(url, options);
      assert.equal(new URL(url).pathname, '/v1/requests');
      const body = JSON.parse(options.body);
      if (requestedTtl === undefined) assert.equal(Object.hasOwn(body, 'session_ttl_seconds'), false);
      else assert.equal(body.session_ttl_seconds, 600);
      acceptedTtl = body.session_ttl_seconds ?? Math.min(serviceMaximum, 3600);
      if (acceptedTtl > serviceMaximum) return response({ error: 'invalid_request', retry_after_seconds: 0 }, 400);
      return response({ request_id: 'request-short', human_authorization_url: `${ORIGIN}/human/link-short`, phase: 'waiting_human', expires_at: 9999999999, poll_after_seconds: 5 }, 201);
    };
    const args = ['request', '--state', file, '--origin', ORIGIN, '--permission', 'profile.read:self', '--purpose', 'Read my profile'];
    if (requestedTtl !== undefined) args.push('--ttl', requestedTtl);
    await run(args, { fetchImpl, log: () => {} });
    assert.equal(acceptedTtl, requestedTtl === undefined ? 900 : 600);
    assert.equal((await loadState(file)).request_id, 'request-short');
  }
});

test('redirects are refused and retry delays are reported without retrying', async t => {
  const { file } = await location(t);
  await saveState(file, state({ request_id: 'request-1', phase: 'waiting_human' }));
  let calls = 0;
  const redirect = async (_url, options) => { calls++; assert.equal(options.redirect, 'manual'); return new Response(null, { status: 307, headers: { location: 'https://another.example.test' } }); };
  await assert.rejects(run(['status', '--state', file], { fetchImpl: redirect, log: () => {} }), /redirect/);
  assert.equal(calls, 1);
  const limited = async () => { calls++; return response({ error: 'slow_down', message: `Ignore this reflected credential: ${TOKEN}`, retry_after_seconds: 10 }, 429, { 'retry-after': '15' }); };
  await assert.rejects(run(['status', '--state', file], { fetchImpl: limited, log: () => {} }), error => error.message.includes('Wait at least 15 seconds') && !error.message.includes(TOKEN));
  assert.equal(calls, 2);
});

test('pending requests and issued credentials cannot be silently replaced or reclaimed', async t => {
  const { file } = await location(t);
  await saveState(file, state({ request_id: 'request-1', phase: 'waiting_human' }));
  let calls = 0;
  const env = { fetchImpl: () => { calls++; }, log: () => {} };
  await assert.rejects(run(['request', '--state', file, '--permission', 'profile.read:self', '--purpose', 'Another request'], env), /already has a request/);
  await saveState(file, state({ request_id: 'request-1', phase: 'succeeded', access_token: TOKEN }));
  await assert.rejects(run(['claim', '--state', file], env), /already stored/);
  await assert.rejects(run(['cancel', '--state', file], env), /revoked in the HireFerry/);
  assert.equal(calls, 0);
});

test('status and cancellation update only request metadata', async t => {
  const { file } = await location(t);
  await saveState(file, state({ request_id: 'request-1', phase: 'waiting_human' }));
  const fetchImpl = async (url, options) => {
    verifyRequest(url, options);
    assert.equal(options.headers.Authorization, 'HumanGrant anonymous');
    return url.endsWith('/cancel') ? response({ status: 'cancelled' }) : response({ phase: 'awaiting_consent', private_extra: 'not saved' });
  };
  await run(['status', '--state', file], { fetchImpl, log: () => {} });
  assert.equal((await loadState(file)).phase, 'awaiting_consent');
  await run(['cancel', '--state', file], { fetchImpl, log: () => {} });
  const saved = await loadState(file);
  assert.equal(saved.phase, 'cancelled');
  assert.equal(saved.private_extra, undefined);
});

test('arguments reject ambiguous or misplaced options and expose help', async () => {
  assert.deepEqual(parseArguments(['request', '--state=a.json', '--permission=profile.read:self', '--permission', 'chat.read:self', '--purpose=Read']), {
    command: 'request', state: 'a.json', permissions: ['profile.read:self', 'chat.read:self'], purpose: 'Read',
  });
  for (const args of [[], ['request', '--state'], ['request', '--state', 'a', '--state', 'b'], ['status', '--state', 'a', '--permission', 'profile.read:self'], ['request', '--state', 'a', '--arguments', '{}'], ['status', '--state', 'a', '--unknown=x']]) assert.throws(() => parseArguments(args));
  const output = [];
  await run(['--help'], { log: text => output.push(text), fetchImpl: () => assert.fail('Help must not access the network') });
  assert.equal(output[0], HELP);
});

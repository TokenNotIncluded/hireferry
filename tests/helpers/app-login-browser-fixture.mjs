// A real Rust server with a disposable database and one ordinary synthetic member.
// No repository config, production identity, SMTP or external authentication is used.
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const delay = milliseconds => new Promise(resolve_ => setTimeout(resolve_, milliseconds));
const hash = value => createHash('sha256').update(value).digest('hex');

export async function startAppLoginFixture(root) {
  const directory = await mkdtemp(join(tmpdir(), 'hireferry-app-login-'));
  let child;
  const running = () => child && child.exitCode === null && child.signalCode === null;
  const close = async () => {
    if (running()) {
      const exited = new Promise(resolve_ => child.once('exit', resolve_));
      child.kill('SIGTERM');
      await Promise.race([exited, delay(2000)]);
      if (running()) { child.kill('SIGKILL'); await exited; }
    }
    await rm(directory, { recursive: true, force: true });
  };
  try {
    const listener = createServer();
    await new Promise((resolve_, reject) => {
      listener.once('error', reject);
      listener.listen(0, '127.0.0.1', resolve_);
    });
    const port = listener.address().port;
    await new Promise(resolve_ => listener.close(resolve_));
    const origin = `http://localhost:${port}`, database = join(directory, 'fixture.sqlite3');
    const config = join(directory, 'fixture.toml');
    await writeFile(config, `bind = "127.0.0.1:${port}"
public_url = "${origin}"
webauthn_rp_id = "localhost"
database = ${JSON.stringify(database)}
development = true
session_hours = 24
[site]
name = "HireFerry · Synthetic app-login acceptance"
default_language = "zh"
[google]
enabled = false
[smtp]
enabled = false
[webhooks]
enabled = false
`, { mode: 0o600 });
    child = spawn(process.env.HFY_SERVER_BINARY || resolve(root, 'target/debug/hireferry'), ['--config', config, 'serve'], {
      cwd: directory, stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, RUST_LOG: 'error' }
    });
    let output = '', failure;
    child.on('error', error => { failure = error; });
    child.stdout.on('data', value => { output = (output + value).slice(-6000); });
    child.stderr.on('data', value => { output = (output + value).slice(-6000); });
    let ready = false;
    for (let attempt = 0; attempt < 150; attempt++) {
      if (failure || !running()) throw failure || new Error(output || 'Fixture server exited');
      try { ready = (await fetch(`${origin}/healthz`, { signal: AbortSignal.timeout(1000) })).ok; } catch { /* Starting. */ }
      if (ready) break;
      await delay(100);
    }
    if (!ready) throw new Error('Fixture server did not become ready: ' + output);

    const member = { id: 'app-login-member', email: 'app-login-member@example.invalid', name: 'Synthetic member' };
    const sourceSession = randomBytes(32).toString('hex'), sourceCsrf = randomBytes(24).toString('hex');
    const now = Math.floor(Date.now() / 1000), sourceExpires = now + 3600, sourceRecent = now - 600;
    const db = new DatabaseSync(database);
    try {
      db.exec('PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; BEGIN;');
      db.prepare('INSERT INTO users(id,email,name,role,created_at,updated_at) VALUES(?,?,?,?,?,?)')
        .run(member.id, member.email, member.name, 'seeker', now, now);
      db.prepare('INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?,?,0,0,?,?,?)')
        .run(hash(sourceSession), member.id, sourceCsrf, sourceExpires, sourceRecent);
      db.exec('COMMIT;');
    } finally { db.close(); }
    function read(sql, ...parameters) {
      const connection = new DatabaseSync(database, { readOnly: true });
      try { connection.exec('PRAGMA busy_timeout=5000;'); return connection.prepare(sql).get(...parameters); }
      finally { connection.close(); }
    }
    return {
      origin, member, sourceSession, sourceCsrf, sourceExpires, sourceRecent, close,
      session: raw => read('SELECT token_hash,user_id,admin,bootstrap,expires_at,recent_auth_at FROM sessions WHERE token_hash=?', hash(raw)),
      grant: request => read('SELECT status,issued_session_hash FROM app_login_grants WHERE request_id=?', request),
      sessionCount: () => read('SELECT COUNT(*) AS count FROM sessions WHERE user_id=?', member.id).count,
      userCount: () => read('SELECT COUNT(*) AS count FROM users').count
    };
  } catch (error) { await close(); throw error; }
}

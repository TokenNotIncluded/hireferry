/* Isolated real-server fixture: synthetic identities only; never touches config.toml or data/. */
import { createServer } from 'node:net';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const delay = milliseconds => new Promise(resolve_ => setTimeout(resolve_, milliseconds));
const hash = value => createHash('sha256').update(value).digest('hex');

export async function startRelationsFixture(root) {
  const directory = await mkdtemp(join(tmpdir(), 'hireferry-relations-'));
  const listener = createServer();
  await new Promise((resolve_, reject) => { listener.once('error', reject); listener.listen(0, '127.0.0.1', resolve_); });
  const port = listener.address().port;
  await new Promise(resolve_ => listener.close(resolve_));
  const origin = `http://localhost:${port}`, database = join(directory, 'fixture.sqlite3');
  const config = join(directory, 'fixture.toml');
  await writeFile(config, `bind = "127.0.0.1:${port}"\npublic_url = "${origin}"\nwebauthn_rp_id = "localhost"\ndatabase = ${JSON.stringify(database)}\ndevelopment = true\n[site]\nname = "HireFerry · 合成验收数据"\ndefault_language = "zh"\n`);
  const process_ = spawn(process.env.HFY_SERVER_BINARY || resolve(root, 'target/debug/hireferry'), ['--config', config, 'serve'], { cwd: directory, stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '', failure;
  process_.on('error', error => { failure = error; });
  process_.stdout.on('data', value => { output = (output + value).slice(-6000); });
  process_.stderr.on('data', value => { output = (output + value).slice(-6000); });
  const close = async () => {
    if (process_.exitCode === null) {
      process_.kill('SIGTERM');
      await Promise.race([new Promise(resolve_ => process_.once('exit', resolve_)), delay(2000)]);
      if (process_.exitCode === null) process_.kill('SIGKILL');
    }
    await rm(directory, { recursive: true, force: true });
  };
  try {
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      if (failure || process_.exitCode !== null) throw failure || new Error(output || 'Fixture server exited');
      try { ready = (await fetch(`${origin}/healthz`)).ok; } catch { /* Starting. */ }
      if (ready) break;
      await delay(100);
    }
    if (!ready) throw new Error('Fixture server did not become ready: ' + output);
    const db = new DatabaseSync(database);
    db.exec('PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; BEGIN;');
    const timestamp = Math.floor(Date.now() / 1000);
    const session = 'synthetic-relation-browser-session', csrf = 'synthetic-relation-browser-csrf';
    const token = 'hf_admin_' + 'b'.repeat(64);
    db.prepare('INSERT INTO sessions(token_hash,user_id,admin,bootstrap,csrf,expires_at,recent_auth_at) VALUES(?,NULL,1,0,?,?,?)').run(hash(session), csrf, timestamp + 3600, timestamp);
    db.prepare('INSERT INTO admin_api_tokens(id,name,token_hash,created_at,expires_at) VALUES(?,?,?,?,?)').run('synthetic-graph-ai', 'Synthetic browser acceptance', hash(token), timestamp, timestamp + 3600);
    const user = db.prepare('INSERT INTO users(id,email,name,role,employer_paid,resume_md,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)');
    const mandate = db.prepare('INSERT INTO mandates(user_id,target_industry,target_roles,location,preferences,created_at,updated_at) VALUES(?,?,?,?,?,?,?)');
    const job = db.prepare('INSERT INTO jobs(id,owner_id,title,jd_md,salary_cents,headcount,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)');
    const match = db.prepare('INSERT INTO matches(id,seeker_id,job_id,status,note,created_at,updated_at) VALUES(?,?,?,?,?,?,?)');
    const industries = ['软件技术', '设计创意', '产品运营', '数据分析', '金融服务', '智能制造', '教育服务', '医疗健康'];
    const id = (prefix, index) => prefix + String(index).padStart(4, '0');
    for (let index = 0; index < 8; index++) user.run(id('e', index), `employer-${index}@example.invalid`, `示例招聘团队 ${index + 1}`, 'employer', 1, '', timestamp, timestamp);
    for (let index = 0; index < 992; index++) {
      user.run(id('s', index), `candidate-${index}@example.invalid`, `示例候选人 ${String(index).padStart(4, '0')}`, 'seeker', 0, 'PRIVATE_RESUME_SHOULD_NOT_APPEAR', timestamp - index, timestamp);
      mandate.run(id('s', index), industries[index % 8], ['工程师', '设计师', '运营专家', '分析师'][index % 4], index % 2 ? '上海' : '远程', '', timestamp, timestamp);
    }
    for (let index = 0; index < 1000; index++) job.run(id('j', index), id('e', index % 8), `${industries[index % 8]} · 示例岗位 ${String(index).padStart(4, '0')}`, 'PRIVATE_JOB_DESCRIPTION_SHOULD_NOT_APPEAR', 1200000 + index * 1000, 2, index % 17 ? 'published' : 'closed', timestamp - index, timestamp);
    let edges = 0;
    const states = ['introduced', 'interview', 'hired', 'declined'];
    for (let index = 0; index < 991; index++) {
      const neighbors = index === 0 ? 1000 : 10;
      for (let offset = 0; offset < neighbors; offset++) {
        const role = index === 0 ? offset : (index * 7 + offset * 8) % 1000;
        match.run(`m-${index}-${role}`, id('s', index), id('j', role), states[(index + role) % 4], 'PRIVATE_MATCH_NOTE_SHOULD_NOT_APPEAR', timestamp, timestamp);
        edges++;
      }
    }
    db.exec('COMMIT;');
    db.close();
    async function api(path, body, method = body === undefined ? 'GET' : 'POST') {
      const response = await fetch(origin + path, { method, headers: { cookie: `hf_session=${session}`, 'x-csrf-token': csrf, 'content-type': 'application/json', origin }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
      const result = await response.json();
      if (!response.ok) throw Object.assign(new Error(`${response.status}: ${JSON.stringify(result)}`), { status: response.status });
      return result;
    }
    async function mcp(name, arguments_) {
      const response = await fetch(origin + '/mcp', { method: 'POST', headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json', accept: 'application/json, text/event-stream', 'mcp-protocol-version': '2025-11-25' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: arguments_ } }) });
      const envelope = await response.json();
      if (!response.ok || envelope.error || envelope.result?.isError) throw new Error(JSON.stringify(envelope));
      return envelope.result.structuredContent || JSON.parse(envelope.result.content[0].text);
    }
    return { origin, session, csrf, database, edges, api, mcp, close };
  } catch (error) { await close(); throw error; }
}

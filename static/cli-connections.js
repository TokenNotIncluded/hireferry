import { rememberCliReturn } from './cli-auth-return.js';

const $ = id => document.getElementById(id);
let language = 'zh';
try { language = localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh'; } catch { /* Optional preference. */ }
const words = {
  title: ['命令行连接', 'Command-line connections'],
  intro: ['管理已授权的终端连接。', 'Manage your authorized terminals.'],
  account: ['当前账户', 'Current account'],
  checking: ['正在检查登录状态…', 'Checking your sign-in…'],
  signIn: ['登录后管理连接。', 'Sign in to manage connections.'],
  ready: ['', ''],
  empty: ['还没有命令行连接。', 'No command-line connections yet.'],
  failed: ['连接加载失败。请重试。', 'Connections failed to load. Try again.'],
  memberLogin: ['Google / 邮箱登录', 'Google / email sign-in'],
  adminLogin: ['管理员 Passkey 登录', 'Administrator passkey sign-in'],
  administrator: ['平台管理员', 'Platform administrator'],
  refresh: ['刷新列表', 'Refresh list'],
  workspace: ['打开工作台', 'Open workspace'],
  created: ['连接时间', 'Connected'], expires: ['到期', 'Expires'],
  revoked: ['已撤销', 'Revoked'], expired: ['已过期', 'Expired'], active: ['有效', 'Active'],
  revoke: ['撤销连接', 'Revoke connection'],
  revokedNotice: ['已撤销。再次连接需要重新授权。', 'Revoked. Authorize again to reconnect.'],
  returnFailed: ['返回位置未保存。登录后请重新打开本页。', 'Return page not saved. Reopen it after signing in.'],
};
const text = key => words[key][language === 'en' ? 1 : 0];
let identity = null;
let items = [];
let busy = false;
let statusKey = 'checking';

function element(tag, value, className) {
  const el = document.createElement(tag);
  if (value !== undefined) el.textContent = value;
  if (className) el.className = className;
  return el;
}

function draw() {
  document.documentElement.lang = language === 'en' ? 'en' : 'zh-CN';
  document.title = text('title') + ' · HireFerry';
  document.querySelectorAll('[data-text]').forEach(el => { el.textContent = text(el.dataset.text); });
  $('language').textContent = language === 'en' ? '中文' : 'EN';
  $('status').textContent = text(statusKey);
  if (identity?.admin) $('account').textContent = text('administrator');
  else if (identity?.user) $('account').textContent = [identity.user.name, identity.user.email].filter(Boolean).join(' · ');
  const fragment = document.createDocumentFragment();
  for (const item of items) {
    const row = element('article', undefined, 'connection');
    const heading = element('div', undefined, 'connection-heading');
    heading.append(element('h3', item.client_id));
    const expired = Number(item.expires_at) * 1000 <= Date.now();
    heading.append(element('span', text(item.revoked ? 'revoked' : expired ? 'expired' : 'active'),
      'connection-status' + (item.revoked || expired ? ' inactive' : '')));
    row.append(heading);
    row.append(element('p', item.scope, 'connection-scope'));
    const dates = element('div', undefined, 'connection-meta');
    for (const [key, value] of [['created', item.created_at], ['expires', item.expires_at]]) {
      const date = new Date(Number(value) * 1000);
      const readable = Number.isFinite(date.getTime()) ? date.toLocaleString(language === 'en' ? 'en-US' : 'zh-CN') : String(value);
      dates.append(element('span', text(key) + ': ' + readable));
    }
    row.append(dates);
    if (!item.revoked && !expired) {
      const actions = element('div', undefined, 'connection-actions');
      const button = element('button', text('revoke'), 'button secondary small');
      button.type = 'button';
      button.disabled = busy;
      button.addEventListener('click', () => revoke(item.id));
      actions.append(button);
      row.append(actions);
    }
    fragment.append(row);
  }
  $('connections').replaceChildren(fragment);
  $('refresh').disabled = busy;
}

async function request(path, options = {}) {
  const response = await fetch(path, {credentials: 'same-origin', cache: 'no-store', redirect: 'error', ...options});
  if (response.status === 204) return {};
  const data = await response.json();
  if (!response.ok) throw new Error(typeof data.error === 'string' ? data.error : text('failed'));
  return data;
}

function fail(error) {
  $('error').textContent = error.message || text('failed');
  $('error').hidden = false;
  statusKey = 'failed';
}

async function load(notice = null) {
  if (busy) return;
  busy = true;
  $('error').hidden = true;
  $('sign-in').hidden = true;
  statusKey = 'checking';
  draw();
  try {
    identity = await request('/api/session');
    if ((!identity.user && !identity.admin) || identity.bootstrap) {
      items = [];
      statusKey = 'signIn';
      $('sign-in').hidden = false;
    } else {
      const data = await request('/api/cli/connections');
      items = data.items;
      statusKey = notice || (items.length ? 'ready' : 'empty');
      $('workspace').href = identity.admin ? '/admin?tab=agents' : '/app?tab=agents';
    }
  } catch (error) { fail(error); }
  finally { busy = false; draw(); }
}

async function revoke(id) {
  if (busy || !identity?.csrf) return;
  busy = true;
  $('error').hidden = true;
  draw();
  try {
    await request('/api/cli/connections/' + encodeURIComponent(id), {
      method: 'DELETE', headers: {'X-CSRF-Token': identity.csrf},
    });
    busy = false;
    await load('revokedNotice');
  } catch (error) { fail(error); }
  finally { busy = false; draw(); }
}

for (const id of ['member-login', 'admin-login']) {
  $(id).addEventListener('click', () => {
    try { rememberCliReturn(sessionStorage, '/oauth/connections', location.origin); }
    catch { fail(new Error(text('returnFailed'))); draw(); }
  });
}
$('refresh').addEventListener('click', () => load());
$('language').addEventListener('click', () => {
  language = language === 'en' ? 'zh' : 'en';
  try { localStorage.setItem('hireferry-language', language); } catch { /* Optional preference. */ }
  draw();
});
draw();
load();

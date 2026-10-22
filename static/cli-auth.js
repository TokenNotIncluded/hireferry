import { CLI_RETURN_KEY, rememberCliReturn } from './cli-auth-return.js';

const $ = id => document.getElementById(id);
let language = 'zh';
try { language = localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh'; } catch { /* Optional preference. */ }
const words = {
  title: ['连接 hfy', 'Connect hfy'],
  intro: ['确认账户与权限后继续。', 'Review access for this terminal.'],
  requestTitle: ['授权详情', 'Access details'],
  codeLabel: ['输入终端显示的设备码', 'Enter the device code shown in your terminal'],
  continue: ['继续', 'Continue'],
  signInHelp: ['登录后确认本次连接。', 'Sign in to approve this connection.'],
  memberLogin: ['Google / 邮箱登录', 'Google / email sign-in'],
  adminLogin: ['管理员 Passkey 登录', 'Administrator passkey sign-in'],
  siteLabel: ['网站', 'Website'], accountLabel: ['账户', 'Account'], clientLabel: ['客户端', 'Client'], scopeLabel: ['权限', 'Scopes'],
  compareCode: ['请确认设备码与你的终端一致', 'Match this code with your terminal'],
  ownRequest: ['请确认请求来自你正在登录的终端。', 'Confirm this request came from the terminal you are signing in to.'],
  revokeHelp: ['运行 hfy auth logout 或撤销连接即可结束授权。', 'Run hfy auth logout or revoke the connection to end access.'],
  manageConnections: ['管理连接', 'Manage connections'],
  approve: ['允许连接', 'Allow access'], deny: ['拒绝', 'Deny'], switchAccount: ['切换账户', 'Use another account'],
  retry: ['重新检查', 'Check again'], workspace: ['打开工作台', 'Open workspace'], reauth: ['重新验证 Passkey', 'Verify your passkey again'],
  checking: ['正在检查登录状态…', 'Checking your sign-in…'],
  signIn: ['请先登录。', 'Please sign in first.'],
  checkDetails: ['', ''],
  approvalUnavailable: ['当前账户无法批准此权限。请切换账户或拒绝请求。', 'This account cannot approve these permissions. Switch accounts or decline.'],
  identityPermissions: ['查看账户、AI 资格和现有权限。不含 AI 工具或业务操作权限。', 'Read account, AI eligibility and granted permissions. No AI tools or business actions.'],
  memberPermissions: ['允许按你现有的 AI 权限管理资料、委托、岗位、订单、匹配和会话。收费操作按订单结算。', 'Use your existing AI permissions to manage profiles, briefs, jobs, orders, introductions and conversations. Paid actions use the normal checkout.'],
  adminPermissions: ['允许管理全平台业务资料、用户、匹配和会话。范围不含 Passkey、凭据签发与服务器密钥。', 'Manage platform records, users, introductions and conversations. Passkeys, credential issuance and server secrets are outside this scope.'],
  administrator: ['平台管理员', 'Platform administrator'],
  deciding: ['正在提交你的选择…', 'Saving your choice…'],
  approved: ['已批准。请回到终端继续。', 'Approved. Return to your terminal to continue.'],
  denied: ['已拒绝。', 'Declined.'],
  failed: ['连接失败。请重试或在终端重新登录。', 'Connection failed. Retry or sign in again from your terminal.'],
  invalid: ['请求无效。请在终端重新运行 hfy auth login。', 'Invalid request. Run hfy auth login again.'],
  enterCode: ['输入终端显示的设备码。', 'Enter the code shown in your terminal.'],
  storage: ['返回位置未保存。登录后请重新打开终端中的授权链接。', 'Return page not saved. Reopen the terminal’s authorization link after signing in.'],
};
const text = key => words[key][language === 'en' ? 1 : 0];
let identity = null;
let details = null;
let statusKey = 'checking';
let completed = false;
let busy = false;
let version = 0;
const params = new URLSearchParams(location.search);
const flow = location.pathname === '/oauth/device' ? 'device' : 'oauth';
const requestKeys = ['client_id', 'response_type', 'redirect_uri', 'scope', 'state', 'code_challenge', 'code_challenge_method'];

function translate() {
  document.documentElement.lang = language === 'en' ? 'en' : 'zh-CN';
  document.title = text('title') + ' · HireFerry';
  document.querySelectorAll('[data-text]').forEach(el => { el.textContent = text(el.dataset.text); });
  $('language').textContent = language === 'en' ? '中文' : 'EN';
  $('status').textContent = text(statusKey);
  if (details) {
    const scopes = new Set(details.scope.split(' '));
    $('permissions').textContent = text(scopes.has('admin') ? 'adminPermissions' : scopes.has('mcp') ? 'memberPermissions' : 'identityPermissions');
    $('account').textContent = details.admin ? text('administrator') : [details.user?.name, details.user?.email].filter(Boolean).join(' · ');
    $('client').textContent = details.client_id;
    $('scope').textContent = details.scope;
  }
  $('approve').disabled = busy || details?.can_approve === false;
}

async function request(path, options = {}) {
  const response = await fetch(path, {credentials: 'same-origin', cache: 'no-store', redirect: 'error', ...options});
  if (response.status === 204) return {};
  const data = await response.json();
  if (!response.ok) {
    const error = new Error(typeof data.error === 'string' ? data.error : text('failed'));
    error.status = response.status;
    throw error;
  }
  return data;
}

function showError(error) {
  $('error').textContent = error.message || text('failed');
  $('error').hidden = false;
  $('recover').hidden = false;
  $('reauth').hidden = !identity?.admin;
  $('workspace').href = identity?.admin ? '/admin?tab=agents' : '/app?tab=agents';
  statusKey = 'failed';
  translate();
}

function requestParameters() {
  if (flow === 'device') return new URLSearchParams({flow, user_code: params.get('user_code') || ''});
  const query = new URLSearchParams();
  for (const key of requestKeys) {
    if (params.getAll(key).length > 1) throw new Error(text('invalid'));
    if (params.has(key)) query.set(key, params.get(key));
  }
  return query;
}

async function load() {
  if (busy || completed) return;
  const current = ++version;
  details = null;
  $('error').hidden = true;
  $('recover').hidden = true;
  $('consent').hidden = true;
  $('sign-in').hidden = true;
  statusKey = 'checking';
  translate();
  try {
    if (flow === 'device' && !params.get('user_code')) {
      $('code-form').hidden = false;
      statusKey = 'enterCode';
      translate();
      return;
    }
    $('code-form').hidden = true;
    if (flow === 'device' && params.getAll('user_code').length > 1) throw new Error(text('invalid'));
    identity = await request('/api/session');
    if (current !== version) return;
    if ((!identity.user && !identity.admin) || identity.bootstrap) {
      statusKey = 'signIn';
      $('sign-in').hidden = false;
      translate();
      return;
    }
    details = await request('/api/cli/authorization?' + requestParameters());
    if (current !== version) return;
    $('site').textContent = location.origin;
    $('code-confirm').hidden = flow !== 'device';
    $('display-code').textContent = details.user_code || '';
    $('consent').hidden = false;
    statusKey = 'checkDetails';
    if (details.can_approve === false) {
      $('error').textContent = details.approval_error || text('approvalUnavailable');
      $('error').hidden = false;
      $('recover').hidden = false;
      $('reauth').hidden = !details.requires_recent_auth;
      $('workspace').href = identity?.admin ? '/admin?tab=agents' : '/app?tab=agents';
      statusKey = 'approvalUnavailable';
    }
    translate();
  } catch (error) {
    if (current === version) showError(error);
  }
}

function remember(afterAuthOnly = false) {
  try {
    return rememberCliReturn(sessionStorage, location.pathname + location.search, location.origin, afterAuthOnly);
  } catch {
    showError(new Error(text('storage')));
    return false;
  }
}

async function decide(approve) {
  if (busy || completed || !identity?.csrf || !details) return;
  if (approve && details.can_approve === false) return;
  busy = true;
  $('error').hidden = true;
  $('recover').hidden = true;
  for (const id of ['approve', 'deny', 'switch-account']) $(id).disabled = true;
  statusKey = 'deciding';
  translate();
  try {
    const body = flow === 'device'
      ? {flow, user_code: params.get('user_code'), approve}
      : {flow, ...Object.fromEntries(requestParameters()), approve};
    const result = await request('/api/cli/authorization', {
      method: 'POST',
      headers: {'Content-Type': 'application/json', 'X-CSRF-Token': identity.csrf},
      body: JSON.stringify(body),
    });
    if (flow === 'oauth') {
      // Navigate only to the exact local callback the user just reviewed.
      const expected = new URL(details.redirect_uri);
      const target = new URL(result.redirect_uri);
      if (target.origin !== expected.origin || target.pathname !== expected.pathname || target.username || target.password || target.hash) throw new Error(text('invalid'));
      if (target.searchParams.get('state') !== params.get('state') || target.searchParams.get('iss') !== location.origin) throw new Error(text('invalid'));
      location.assign(target.href);
    }
    completed = true;
    try { sessionStorage.removeItem(CLI_RETURN_KEY); } catch { /* No credentials are stored here. */ }
    $('consent').hidden = true;
    statusKey = approve ? 'approved' : 'denied';
    $('status').classList.add('complete');
    translate();
  } catch (error) { showError(error); }
  finally {
    busy = false;
    for (const id of ['approve', 'deny', 'switch-account']) $(id).disabled = false;
    $('approve').disabled = details?.can_approve === false;
  }
}

$('language').addEventListener('click', () => {
  language = language === 'en' ? 'zh' : 'en';
  try { localStorage.setItem('hireferry-language', language); } catch { /* Optional preference. */ }
  translate();
});
$('code-form').addEventListener('submit', event => {
  event.preventDefault();
  const code = $('user-code').value.trim().toUpperCase();
  params.set('user_code', code);
  history.replaceState(null, '', '/oauth/device?' + new URLSearchParams({user_code: code}));
  load();
});
for (const id of ['member-login', 'admin-login']) $(id).addEventListener('click', () => { remember(); });
$('reauth').addEventListener('click', () => { remember(true); });
$('retry').addEventListener('click', load);
$('approve').addEventListener('click', () => { decide(true); });
$('deny').addEventListener('click', () => { decide(false); });
$('switch-account').addEventListener('click', async () => {
  if (busy || completed) return;
  busy = true;
  try {
    await request('/api/auth/logout', {method: 'POST', headers: {'Content-Type': 'application/json', 'X-CSRF-Token': identity.csrf}, body: '{}'});
    // Keep the request visible while allowing either member or administrator
    // sign-in. The next choice saves the bounded return path before navigating.
    location.reload();
  } catch (error) { showError(error); }
  finally { busy = false; }
});
translate();
load();

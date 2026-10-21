import { CLI_RETURN_KEY, rememberCliReturn } from './cli-auth-return.js';

const $ = id => document.getElementById(id);
let language = 'zh';
try { language = localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh'; } catch { /* Optional preference. */ }
const words = {
  title: ['连接你的命令行', 'Connect your command line'],
  intro: ['确认账户和权限后，回到终端继续。', 'Check the account and permissions, then return to your terminal.'],
  requestTitle: ['授权 hfy', 'Authorize hfy'],
  codeLabel: ['输入终端显示的设备码', 'Enter the device code shown in your terminal'],
  continue: ['继续', 'Continue'],
  signInHelp: ['先在聘渡网站登录。完成后会回到本页确认授权。', 'Sign in on HireFerry first. You will return here to confirm access.'],
  memberLogin: ['使用 Google 或邮箱登录', 'Sign in with Google or email'],
  adminLogin: ['管理员 Passkey 登录', 'Administrator passkey sign-in'],
  siteLabel: ['网站', 'Website'], accountLabel: ['当前账户', 'Account'], clientLabel: ['客户端', 'Client'], scopeLabel: ['权限范围', 'Scopes'],
  compareCode: ['请核对终端上的设备码', 'Check this code against your terminal'],
  ownRequest: ['只有你刚在自己的设备上运行了 hfy auth login，才应批准。不要批准他人发来的设备码或链接。', 'Approve only if you just ran hfy auth login on your own device. Do not approve codes or links sent by other people.'],
  revokeHelp: ['授权有有效期。运行 hfy auth logout 或在命令行连接页面撤销后，访问令牌和刷新令牌都会失效。', 'Access expires. Running hfy auth logout or revoking a command-line connection invalidates both its access and refresh tokens.'],
  manageConnections: ['管理命令行连接', 'Manage command-line connections'],
  approve: ['允许连接', 'Allow access'], deny: ['拒绝', 'Deny'], switchAccount: ['切换账户', 'Use another account'],
  retry: ['重新检查', 'Check again'], workspace: ['打开工作台', 'Open workspace'], reauth: ['重新验证 Passkey', 'Verify your passkey again'],
  footer: ['登录操作在聘渡网站完成。终端不会索取你的 Google 密码、邮箱验证码或 Passkey。', 'Sign-in happens on HireFerry. The terminal does not ask for your Google password, email code, or passkey.'],
  checking: ['正在检查登录状态…', 'Checking your sign-in…'],
  signIn: ['请先登录。', 'Please sign in first.'],
  checkDetails: ['请核对下面的账户和权限。', 'Check the account and permissions below.'],
  approvalUnavailable: ['当前账户暂不能批准此权限。你仍可拒绝请求或切换账户。', 'This account cannot approve these permissions. You can still decline the request or switch accounts.'],
  identityPermissions: ['仅允许客户端查看当前账户、AI 使用资格和已授权能力。基础登录不授予 AI 工具、MCP 或管理员业务操作权限。', 'Allow the client to check your account, AI eligibility, and granted capabilities. Basic sign-in does not grant AI tool, MCP, or administrator business access.'],
  memberPermissions: ['除查看当前账户和资格外，允许客户端按你的现有 AI 权限访问你的资料、委托、岗位、订单、匹配及会话，并执行你已有权限的业务操作。服务端会持续检查 AI 使用资格，收费操作仍需完成支付。', 'In addition to account and eligibility checks, allow the client to use your existing AI access to profiles, search briefs, jobs, orders, introductions, and conversations. The server checks AI eligibility on every operation. Paid actions still require payment.'],
  adminPermissions: ['这是管理员授权。客户端将获得平台 AI 业务权限，可读取和管理全平台业务资料、用户、匹配及会话。它不能管理 Passkey、签发其他凭据或读取服务器密钥。', 'This is administrator access. The client can read and manage platform business records, users, introductions, and conversations. It cannot manage passkeys, issue other credentials, or read server secrets.'],
  administrator: ['平台管理员', 'Platform administrator'],
  deciding: ['正在提交你的选择…', 'Saving your choice…'],
  approved: ['已批准。请回到终端继续。', 'Approved. Return to your terminal to continue.'],
  denied: ['已拒绝。本次请求没有获得访问权限。', 'Denied. This request has no access.'],
  failed: ['暂时无法继续。请重试或回到终端重新登录。', 'Unable to continue. Retry or start a new login in your terminal.'],
  invalid: ['登录请求无效，请在终端重新运行 hfy auth login。', 'This request is invalid. Run hfy auth login again.'],
  enterCode: ['在你的终端找到设备码，然后输入。', 'Find the device code in your terminal and enter it here.'],
  storage: ['浏览器无法保存返回位置。登录后请重新打开终端显示的授权链接。', 'This browser cannot save the return page. After signing in, reopen the authorization link shown in your terminal.'],
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

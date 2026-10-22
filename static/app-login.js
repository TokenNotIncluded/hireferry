const $ = id => document.getElementById(id);
const messages = {
  zh: {
    eyebrow:'连接你的账户', title:'继续登录聘渡应用', intro:'在浏览器中确认账户，应用会接着完成登录。', cardLabel:'应用登录',
    compareApp:'在浏览器中核对这个编号', compareBrowser:'核对聘渡应用中的编号',
    appHelp:'确认后返回此应用。编号五分钟内有效，登录凭据不会放进链接。', openBrowser:'打开浏览器确认', cancel:'取消',
    site:'网站', account:'账户', signInHelp:'先登录，再确认本次应用连接。', google:'使用 Google 登录', email:'邮箱', sendCode:'发送验证码', otp:'8 位验证码', verifyCode:'登录并继续',
    permission:'此应用将以显示的账户登录网站，可执行该账户原有的操作。仅确认你自己刚在应用中发起的请求。',
    confirm:'我已核对编号，这是我正在登录的应用', approve:'确认登录应用', deny:'拒绝', returnApp:'返回聘渡应用', website:'继续使用网站', retry:'重新开始',
    footnote:'应用和浏览器各自保留登录会话。团队员工、经理和总管理员请直接在应用中登录。',
    loading:'正在准备登录…', waiting:'等待你在浏览器中确认', choose:'请核对账户和编号', signedOut:'尚未登录', sent:'验证码已发送，请检查邮箱',
    approved:'已确认。请返回聘渡应用。', denied:'已拒绝本次连接', cancelling:'正在取消，请稍候…', cancelFailed:'尚未确认取消，请重试。', cancelled:'已取消本次连接', complete:'登录完成，正在打开工作台…',
    returnHelp:'切回刚才的聘渡应用，它会自动完成登录。此网页不会收到应用的登录凭据。',
    stopHelp:'没有为此请求创建应用会话。你仍可继续使用网站。', expired:'编号已过期，请在应用中重新开始。',
    admin:'此回跳仅支持普通账户。团队员工、经理和总管理员请直接在应用中登录。', network:'暂时无法连接，请检查网络后重试。'
  },
  en: {
    eyebrow:'CONNECT YOUR ACCOUNT', title:'Continue to HireFerry', intro:'Confirm your account in the browser. The app will finish signing you in.', cardLabel:'App sign-in',
    compareApp:'Match this number in your browser', compareBrowser:'Match the number shown in your app',
    appHelp:'Return to the app after confirming. This number expires in five minutes; sign-in credentials stay out of links.', openBrowser:'Confirm in browser', cancel:'Cancel',
    site:'Website', account:'Account', signInHelp:'Sign in first, then confirm this app connection.', google:'Continue with Google', email:'Email', sendCode:'Send code', otp:'8-digit code', verifyCode:'Sign in and continue',
    permission:'The app will sign in as the displayed account and can perform its existing website actions. Confirm only a request you just started in your own app.',
    confirm:'The number matches the app I am signing in to', approve:'Confirm app sign-in', deny:'Decline', returnApp:'Return to HireFerry', website:'Continue on website', retry:'Start again',
    footnote:'The app and browser keep separate sessions. Staff, managers and administrators must sign in directly in the app.',
    loading:'Preparing sign-in…', waiting:'Waiting for your browser confirmation', choose:'Check your account and the number', signedOut:'Not signed in', sent:'Code sent. Please check your email.',
    approved:'Confirmed. Return to the HireFerry app.', denied:'Connection declined', cancelling:'Cancelling. Please wait…', cancelFailed:'Cancellation is not confirmed. Please try again.', cancelled:'Connection cancelled', complete:'Signed in. Opening your workspace…',
    returnHelp:'Switch back to the HireFerry app you just opened. It will finish signing you in. This webpage never receives the app session credential.',
    stopHelp:'No app session was created for this request. You can still use the website.', expired:'This number expired. Start again in the app.',
    admin:'This return flow supports ordinary accounts. Staff, managers and administrators must sign in directly in the app.', network:'Unable to connect. Check your connection and try again.'
  }
};
let language = 'zh';
try { language = localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh'; } catch {}
let statusKey = 'loading', finishKey = '', request = '', csrf = '', expires = 0, timer, busy = false, polling = false, cancelling = false, cancelInFlight = false, done = false;
const browser = location.pathname === '/auth/app/authorize';
const t = key => messages[language][key] || key;
function translate() {
  document.documentElement.lang = language === 'en' ? 'en' : 'zh-CN';
  document.title = `${t('title')} · HireFerry`;
  document.querySelectorAll('[data-text]').forEach(node => { node.textContent = t(node.dataset.text); });
  document.querySelectorAll('[data-label]').forEach(node => { node.setAttribute('aria-label', t(node.dataset.label)); });
  $('language').textContent = language === 'en' ? '中文' : 'EN';
  $('language').setAttribute('aria-label', language === 'en' ? '切换到中文' : 'Switch to English');
  $('status').textContent = t(statusKey);
  if (finishKey) $('finished-help').textContent = t(finishKey);
}
function status(key) { statusKey = key; $('status').textContent = t(key); }
function error(message) { $('error').textContent = message; $('error').hidden = !message; }
async function api(path, body, token) {
  const controller = new AbortController(), deadline = setTimeout(() => controller.abort(), 15000);
  try {
    const response = await fetch(path, { method: body === undefined ? 'GET' : 'POST', credentials:'same-origin', cache:'no-store', signal:controller.signal,
      headers: { ...(body === undefined ? {} : {'content-type':'application/json'}), ...(token ? {'x-csrf-token':token} : {}) },
      ...(body === undefined ? {} : { body:JSON.stringify(body) }) });
    const data = await response.json();
    if (!response.ok) { const failure = new Error(data.error || data.message || t('network')); failure.status = response.status; throw failure; }
    return data;
  } catch (failure) { if (failure.name === 'AbortError') throw new Error(t('network')); throw failure; }
  finally { clearTimeout(deadline); }
}
function finish(key, help) {
  done = true; clearTimeout(timer); status(key); finishKey = help;
  $('app-flow').hidden = true; $('browser-flow').hidden = true; $('finished').hidden = false;
  $('return-app').hidden = key !== 'approved';
  $('finished-help').textContent = t(help);
}
async function poll() {
  clearTimeout(timer);
  if (done || polling || cancelling || browser || !request) return;
  if (Date.now() / 1000 >= expires) { finish('expired','stopHelp'); $('retry').hidden = false; return; }
  if (document.hidden) { timer = setTimeout(poll, 3000); return; }
  polling = true;
  try {
    const result = await api('/api/auth/app/redeem', {request});
    if (done || cancelling) return;
    error('');
    if (result.status === 'complete') { done = true; status('complete'); location.replace('/app'); return; }
    if (result.status === 'denied') { finish('denied','stopHelp'); $('retry').hidden = false; return; }
    status('waiting');
  } catch (failure) {
    if (done || cancelling) return;
    error(failure.message);
    if (failure.status && failure.status !== 429 && failure.status < 500) { done = true; $('retry').hidden = false; }
  } finally {
    polling = false;
    if (!done && !cancelling) timer = setTimeout(poll, 3000);
  }
}
async function start() {
  done = false; cancelling = false; error(''); status('loading'); $('retry').hidden = true; $('finished').hidden = true;
  try {
    const data = await api('/api/auth/app/start', {});
    request = data.request; expires = data.expires_at;
    $('app-code').textContent = data.comparison_code;
    // Only this exact same-origin route is eligible for the native browser hop.
    const link = new URL(data.verification_uri, location.origin);
    if (link.origin !== location.origin || link.pathname !== '/auth/app/authorize' || link.searchParams.get('request') !== request) throw new Error(t('network'));
    $('open-browser').href = link.href; $('app-flow').hidden = false; status('waiting');
    timer = setTimeout(poll, 3000);
  } catch (failure) { error(failure.message); $('retry').hidden = false; }
}
async function loadContext() {
  try {
    const context = await api(`/api/auth/app/context?request=${encodeURIComponent(request)}`);
    csrf = context.csrf || '';
    $('browser-code').textContent = context.comparison_code;
    $('site').textContent = context.site;
    $('account').textContent = context.user ? `${context.user.name ? context.user.name + ' · ' : ''}${context.user.email}` : t('signedOut');
    $('sign-in').hidden = context.can_approve || context.admin_session;
    $('google').hidden = !context.google_enabled;
    $('google').href = `/api/auth/google/start?app_request=${encodeURIComponent(request)}`;
    $('email-form').hidden = !context.email_enabled || context.can_approve;
    $('consent').hidden = !context.can_approve;
    $('browser-flow').hidden = false;
    status('choose');
    if (context.admin_session) error(t('admin'));
  } catch (failure) { error(failure.message); status('expired'); }
}
async function decide(approve) {
  if (busy || (approve && !$('confirm').checked)) return;
  busy = true; $('approve').disabled = true; $('deny').disabled = true; error('');
  try { await api('/api/auth/app/decide', {request,approve}, csrf); finish(approve ? 'approved' : 'denied', approve ? 'returnHelp' : 'stopHelp'); }
  catch (failure) { error(failure.message); }
  finally { busy = false; $('approve').disabled = !$('confirm').checked; $('deny').disabled = false; }
}
$('language').addEventListener('click', () => { language = language === 'en' ? 'zh' : 'en'; try { localStorage.setItem('hireferry-language',language); } catch {} translate(); });
$('retry').addEventListener('click', start);
$('confirm').addEventListener('change', () => { $('approve').disabled = busy || !$('confirm').checked; });
$('approve').addEventListener('click', () => decide(true));
$('deny').addEventListener('click', () => decide(false));
$('cancel').addEventListener('click', async () => {
  if (cancelInFlight || done) return; cancelling = true; cancelInFlight = true; clearTimeout(timer); $('cancel').disabled = true; status('cancelling');
  try { await api('/api/auth/app/cancel', {request}); finish('cancelled','stopHelp'); $('retry').hidden = false; }
  catch (failure) { error(failure.message); status('cancelFailed'); }
  finally { cancelInFlight = false; $('cancel').disabled = false; }
});
$('email-form').addEventListener('submit', async event => {
  event.preventDefault(); if (busy) return; busy = true; const button = event.currentTarget.querySelector('button'); button.disabled = true; error('');
  try { await api('/api/auth/email/start', {email:$('email').value.trim()}); $('otp-form').hidden = false; status('sent'); $('otp').focus(); }
  catch (failure) { error(failure.message); } finally { busy = false; button.disabled = false; }
});
$('otp-form').addEventListener('submit', async event => {
  event.preventDefault(); if (busy) return; busy = true; const button = event.currentTarget.querySelector('button'); button.disabled = true; error('');
  try { await api('/api/auth/email/verify', {email:$('email').value.trim(),code:$('otp').value.trim()}); $('otp-form').hidden = true; $('otp').value = ''; await loadContext(); }
  catch (failure) { error(failure.message); } finally { busy = false; button.disabled = false; }
});
document.addEventListener('visibilitychange', () => { if (!document.hidden) poll(); });
window.addEventListener('hireferry-app-return', poll);
window.addEventListener('pageshow', () => { if (!browser && request) poll(); });
window.addEventListener('pagehide', () => clearTimeout(timer));
translate();
if (browser) { request = new URLSearchParams(location.search).get('request') || ''; loadContext(); } else start();

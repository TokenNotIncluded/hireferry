'use strict';

const messages = {
  zh: {
    eyebrow:'你的桌面工作台', heading:'让人才与机会，\n在此相遇。',
    lead:'一扇轻巧的窗口，连接你的聘渡。\n继续每一次沟通，开启下一段旅程。',
    ready:'准备就绪', connectTitle:'从这里出发',
    connectDescription:'使用官方服务，或连接你自己的聘渡站点。',
    serverLabel:'服务地址', serverHelp:'使用 HTTPS 地址。本机开发可使用 http://localhost:3000。',
    connectButton:'进入工作台', browserButton:'在浏览器中打开',
    loginNote:'桌面内可使用邮箱验证码登录。Google 登录请在浏览器中完成；浏览器与桌面的登录状态各自独立。',
    footer:'人类 HR 与 AI，协作同行。', footerHelp:'连接设置可随时在应用菜单中重新打开。',
    opening:'正在打开工作台…', opened:'工作台已打开。若页面未加载，可在应用菜单中刷新或切换服务地址。',
    browserOpened:'已在默认浏览器中打开。', invalid_server:'请输入完整的 HTTPS 站点地址，不包含账号、路径、查询参数或片段。本机允许 HTTP。',
    not_allowed:'只能在聘渡桌面版的连接页面执行此操作。',
    window_failed:'暂时无法创建窗口，请重试或选择在浏览器中打开。',
    browser_failed:'无法打开默认浏览器，请检查系统的默认浏览器设置。',
    unavailable:'请在 HireFerry 桌面版中打开此页面。', unexpected:'操作未完成，请重试。'
  },
  en: {
    eyebrow:'YOUR DESKTOP WORKSPACE', heading:'People. Possibilities.\nA place to meet.',
    lead:'A lighter window into HireFerry.\nPick up the conversation. Start your next chapter.',
    ready:'Ready to connect', connectTitle:'Your next port of call',
    connectDescription:'Use the official service or connect to your own HireFerry deployment.',
    serverLabel:'Server address', serverHelp:'Use HTTPS. Local development can use http://localhost:3000.',
    connectButton:'Open workspace', browserButton:'Open in browser',
    loginNote:'Email-code sign-in works in the app. Use your browser for Google sign-in. Browser and desktop sessions stay separate.',
    footer:'Human HR and AI, working together.', footerHelp:'Reopen connection settings from the application menu.',
    opening:'Opening your workspace…', opened:'Workspace opened. Use the application menu to reload or change servers if the page does not load.',
    browserOpened:'Opened in your default browser.', invalid_server:'Enter a full HTTPS origin without credentials, paths, query parameters or fragments. HTTP is allowed on loopback only.',
    not_allowed:'This action is available only from the desktop connection screen.',
    window_failed:'The window could not be opened. Try again or open the service in your browser.',
    browser_failed:'Your browser could not be opened. Check the default browser in system settings.',
    unavailable:'Open this page in the HireFerry desktop application.', unexpected:'The action did not complete. Please try again.'
  }
};
const storage = {
  get(key) { try { return localStorage.getItem(key); } catch { return null; } },
  set(key, value) { try { localStorage.setItem(key, value); } catch { /* A session still works without storage. */ } }
};
let language = storage.get('hireferry.desktop.language') || (navigator.language.startsWith('zh') ? 'zh' : 'en');
if (!messages[language]) language = 'en';
let statusKey = '', statusError = false;
const server = document.getElementById('server');
server.value = storage.get('hireferry.desktop.server') || 'https://hireferry.lmm.best';

function render() {
  document.documentElement.lang = language === 'zh' ? 'zh-CN' : 'en';
  for (const element of document.querySelectorAll('[data-i18n]')) {
    const lines = messages[language][element.dataset.i18n].split('\n');
    element.replaceChildren(...lines.flatMap((text, index) => index ? [document.createElement('br'), document.createTextNode(text)] : [document.createTextNode(text)]));
  }
  const toggle = document.getElementById('language');
  toggle.textContent = language === 'zh' ? 'EN' : '中文';
  toggle.setAttribute('aria-label', language === 'zh' ? 'Switch to English' : '切换为简体中文');
  document.getElementById('status').textContent = statusKey ? messages[language][statusKey] : '';
  document.getElementById('status').classList.toggle('error', statusError);
}

function status(key, error = false) { statusKey = key; statusError = error; render(); }

async function run(command) {
  if (!server.checkValidity()) { server.reportValidity(); return; }
  const invoke = window.__TAURI__?.core?.invoke;
  if (!invoke) { status('unavailable', true); return; }
  const buttons = [document.getElementById('connect'), document.getElementById('browser')];
  buttons.forEach(button => { button.disabled = true; });
  if (command === 'connect') status('opening');
  try {
    const address = server.value.trim();
    await invoke(command, { server: address });
    storage.set('hireferry.desktop.server', address);
    status(command === 'connect' ? 'opened' : 'browserOpened');
  } catch (error) {
    const code = typeof error === 'string' && Object.prototype.hasOwnProperty.call(messages.en, error) ? error : 'unexpected';
    status(code, true);
  } finally { buttons.forEach(button => { button.disabled = false; }); }
}

document.getElementById('language').addEventListener('click', () => {
  language = language === 'zh' ? 'en' : 'zh';
  storage.set('hireferry.desktop.language', language);
  render();
});
document.getElementById('connect-form').addEventListener('submit', event => { event.preventDefault(); run('connect'); });
document.getElementById('browser').addEventListener('click', () => run('open_browser'));
render();

// No content is persisted in the browser. Only a local pathname (no search or
// fragment) accompanies a plain-text report. The server enforces every limit.
export const safeFeedbackPage = pathname => typeof pathname === 'string' &&
  pathname.startsWith('/') && !pathname.startsWith('//') &&
  !/[?#\\\x00-\x1f]/.test(pathname) && new TextEncoder().encode(pathname).length <= 160 ? pathname : '';

const translations = {
  zh: {
    open:'快速反馈', title:'告诉我们哪里需要改进', description:'页面出问题、操作不顺手，或者有个好主意？两分钟就能反馈。',
    category:'反馈类型', bug:'遇到问题', idea:'改进建议', other:'其他', message:'详细描述',
    placeholder:'你遇到了什么？希望我们如何改进？', hint:'请勿填写密码、验证码、令牌或完整简历。反馈将在保留期结束或达到容量上限时自动删除。',
    submit:'提交反馈', sending:'提交中…', close:'关闭', success:'已收到反馈', limit:'反馈提交过于频繁，请稍后再试。',
    error:'提交失败，请稍后重试。', count:'字',
  },
  en: {
    open:'Quick feedback', title:'Help us make HireFerry better', description:'Something broken or awkward? Have an idea? Send a short note.',
    category:'Category', bug:'Something is broken', idea:'Suggestion', other:'Other', message:'Describe it',
    placeholder:'What happened? What would you like us to change?', hint:'Do not enter passwords, one-time codes, access tokens or full résumés. Reports are deleted at the retention deadline or capacity limit.',
    submit:'Send feedback', sending:'Sending…', close:'Close', success:'Feedback received', limit:'Too many reports. Please try again later.',
    error:'Could not send the report. Please try again.', count:'characters',
  },
};

export function mountQuickFeedback(doc = document, win = window) {
  if (doc.getElementById('feedback-launch')) return;
  const root = doc.createElement('div');
  root.id = 'feedback-widget';
  root.innerHTML = `<button id="feedback-launch" class="feedback-launch" type="button" aria-expanded="false" aria-controls="feedback-panel"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11.5a8 8 0 0 1-8.5 8.5H4l-3 3V11.5A8.5 8.5 0 0 1 9.5 3h3a8.5 8.5 0 0 1 8.5 8.5Z"/><path d="M6 9h10M6 13h7"/></svg><span data-feedback="open"></span></button>
    <section id="feedback-panel" class="feedback-panel" role="dialog" aria-modal="false" aria-labelledby="feedback-title" hidden><div class="feedback-panel-head"><h2 id="feedback-title" data-feedback="title"></h2><button class="feedback-close" type="button" data-feedback-close aria-label="Close">×</button></div><p class="feedback-description" data-feedback="description"></p><form id="feedback-form"><label for="feedback-category" data-feedback="category"></label><select id="feedback-category" name="category"><option value="bug" data-feedback="bug"></option><option value="idea" data-feedback="idea"></option><option value="other" data-feedback="other"></option></select><label for="feedback-message" data-feedback="message"></label><textarea id="feedback-message" name="message" minlength="2" maxlength="2000" required></textarea><small class="feedback-count"><span id="feedback-length">0</span>/2000 <span data-feedback="count"></span></small><p class="feedback-hint" data-feedback="hint"></p><button class="feedback-send" type="submit" data-feedback="submit"></button></form><p id="feedback-result" class="feedback-result" role="status" aria-live="polite"></p></section>`;
  doc.body.append(root);
  const $ = selector => root.querySelector(selector);
  const launch = $('#feedback-launch'), panel = $('#feedback-panel'), form = $('#feedback-form');
  const message = $('#feedback-message'), result = $('#feedback-result'), send = $('.feedback-send');
  let pending = false;
  const words = () => {
    try { return translations[win.localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh']; }
    catch { return translations.zh; }
  };
  function translate() {
    const t = words();
    root.querySelectorAll('[data-feedback]').forEach(el => { el.textContent = t[el.dataset.feedback]; });
    message.placeholder = t.placeholder;
    $('[data-feedback-close]').ariaLabel = t.close;
  }
  function toggle(open) {
    panel.hidden = !open;
    launch.setAttribute('aria-expanded', String(open));
    if (open) { translate(); message.focus(); }
    else launch.focus();
  }
  translate();
  launch.addEventListener('click', () => toggle(panel.hidden));
  $('[data-feedback-close]').addEventListener('click', () => toggle(false));
  doc.addEventListener('keydown', event => { if (event.key === 'Escape' && !panel.hidden) toggle(false); });
  doc.addEventListener('pointerdown', event => {
    if (!panel.hidden && !root.contains(event.target)) toggle(false);
  });
  message.addEventListener('input', () => { $('#feedback-length').textContent = String([...message.value].length); });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (pending || !form.reportValidity()) return;
    pending = true;
    send.disabled = true;
    send.textContent = words().sending;
    result.textContent = '';
    result.classList.remove('error');
    try {
      // A logged-in browser reports as its actual account and must send CSRF;
      // visitors without a session can still report login-page failures.
      const sessionResponse = await win.fetch('/api/session', {credentials:'same-origin', cache:'no-store', redirect:'error'});
      if (!sessionResponse.ok) throw new Error('session');
      const session = await sessionResponse.json();
      const headers = {'content-type':'application/json'};
      if (session.csrf) headers['x-csrf-token'] = session.csrf;
      const response = await win.fetch('/api/feedback', {
        method:'POST', credentials:'same-origin', cache:'no-store', redirect:'error', headers,
        body:JSON.stringify({category:$('#feedback-category').value,message:message.value,
          page:safeFeedbackPage(win.location.pathname)}),
      });
      if (response.status === 429) throw new Error('limit');
      if (!response.ok) throw new Error('submit');
      const data = await response.json();
      result.textContent = `${words().success} · #${data.id}`;
      form.reset();
      $('#feedback-length').textContent = '0';
    } catch (error) {
      result.textContent = error.message === 'limit' ? words().limit : words().error;
      result.classList.add('error');
    } finally {
      pending = false;
      send.disabled = false;
      send.textContent = words().submit;
    }
  });
}

if (typeof document !== 'undefined') mountQuickFeedback(document, window);

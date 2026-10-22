// No feedback content is persisted in the browser. Only a local pathname (no search or
// fragment) accompanies a plain-text report. The server enforces every limit.
export const safeFeedbackPage = pathname => typeof pathname === 'string' &&
  pathname.startsWith('/') && !pathname.startsWith('//') &&
  !/[?#\\\x00-\x1f]/.test(pathname) && new TextEncoder().encode(pathname).length <= 160 ? pathname : '';

const translations = {
  zh: {
    open:'快速反馈', title:'快速反馈', move:'轻点反馈，拖动调整位置',
    category:'反馈类型', bug:'遇到问题', idea:'改进建议', other:'其他', message:'详细描述',
    placeholder:'你遇到了什么？希望我们如何改进？', hint:'仅填写问题或建议。',
    submit:'提交反馈', sending:'提交中…', close:'关闭', success:'已收到反馈', limit:'反馈提交过于频繁，请稍后再试。',
    duplicate:'这条反馈已收到。', unavailable:'反馈暂不可用，请稍后重试。', error:'提交失败，请稍后重试。', count:'字',
  },
  en: {
    open:'Quick feedback', title:'Quick feedback', move:'Click for feedback, drag to move',
    category:'Category', bug:'Something is broken', idea:'Suggestion', other:'Other', message:'Describe it',
    placeholder:'What happened? What would you like us to change?', hint:'Describe the problem or suggestion.',
    submit:'Send feedback', sending:'Sending…', close:'Close', success:'Feedback received', limit:'Too many reports. Please try again later.',
    duplicate:'This feedback was already received.', unavailable:'Feedback is unavailable. Please try again later.', error:'Could not send the report. Please try again.', count:'characters',
  },
};

export function mountQuickFeedback(doc = document, win = window) {
  if (doc.getElementById('feedback-launch')) return;
  const root = doc.createElement('div');
  root.id = 'feedback-widget';
  root.innerHTML = `<button id="feedback-launch" class="feedback-launch" type="button" aria-expanded="false" aria-controls="feedback-panel"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M20 11.5a8 8 0 0 1-8.5 8.5H4l-3 3V11.5A8.5 8.5 0 0 1 9.5 3h3a8.5 8.5 0 0 1 8.5 8.5Z"/><path d="M6 9h10M6 13h7"/></svg><span data-feedback="open"></span></button>
    <section id="feedback-panel" class="feedback-panel" role="dialog" aria-modal="false" aria-labelledby="feedback-title" hidden><div class="feedback-panel-head"><h2 id="feedback-title" data-feedback="title"></h2><button class="feedback-close" type="button" data-feedback-close aria-label="Close">×</button></div><form id="feedback-form"><label for="feedback-category" data-feedback="category"></label><select id="feedback-category" name="category"><option value="bug" data-feedback="bug"></option><option value="idea" data-feedback="idea"></option><option value="other" data-feedback="other"></option></select><label for="feedback-message" data-feedback="message"></label><textarea id="feedback-message" name="message" minlength="2" maxlength="2000" required></textarea><small class="feedback-count"><span id="feedback-length">0</span>/2000 <span data-feedback="count"></span></small><p class="feedback-hint" data-feedback="hint"></p><button class="feedback-send" type="submit" data-feedback="submit"></button></form><p id="feedback-result" class="feedback-result" role="status" aria-live="polite"></p></section>`;
  doc.body.append(root);
  const $ = selector => root.querySelector(selector);
  const launch = $('#feedback-launch'), panel = $('#feedback-panel'), form = $('#feedback-form');
  const message = $('#feedback-message'), result = $('#feedback-result'), send = $('.feedback-send');
  const positionKey = 'hireferry-feedback-position';
  const clamp = (value, min, max) => Math.max(min, Math.min(value, Math.max(min, max)));
  const visible = element => element?.isConnected && element.getClientRects().length > 0;
  let panelAnchor = launch;
  let position = null, drag = null, dragged = false;
  try {
    const saved = JSON.parse(win.localStorage.getItem(positionKey));
    if (saved && Number.isFinite(saved.x) && Number.isFinite(saved.y)) {
      position = {x:clamp(saved.x, 0, 1), y:clamp(saved.y, 0, 1)};
    }
  } catch { /* Moving the button also works without browser storage. */ }
  const viewport = () => {
    const view = win.visualViewport;
    const left = (view?.offsetLeft ?? 0) + 16, top = (view?.offsetTop ?? 0) + 16;
    return {left, top, width:Math.max(0, (view?.width ?? win.innerWidth) - 32),
      height:Math.max(0, (view?.height ?? win.innerHeight) - 32)};
  };
  function layout() {
    const area = viewport();
    if (position) {
      Object.assign(launch.style, {right:'auto', bottom:'auto',
        left:`${area.left + position.x * Math.max(0, area.width - launch.offsetWidth)}px`,
        top:`${area.top + position.y * Math.max(0, area.height - launch.offsetHeight)}px`});
    }
    if (panel.hidden) return;
    const anchorElement = visible(panelAnchor) ? panelAnchor : visible(launch) ? launch : null;
    const anchor = anchorElement?.getBoundingClientRect() || {top:area.top, bottom:area.top, right:area.left + area.width};
    const above = clamp(anchor.top - area.top - 12, 0, area.height);
    const below = clamp(area.top + area.height - anchor.bottom - 12, 0, area.height);
    const openAbove = above >= below;
    panel.style.maxHeight = `${Math.min(640, openAbove ? above : below)}px`;
    panel.style.maxWidth = `${area.width}px`;
    const size = panel.getBoundingClientRect();
    const top = openAbove ? anchor.top - size.height - 12 : anchor.bottom + 12;
    Object.assign(panel.style, {right:'auto', bottom:'auto',
      left:`${clamp(anchor.right - size.width, area.left, area.left + area.width - size.width)}px`,
      top:`${clamp(top, area.top, area.top + area.height - size.height)}px`});
  }
  launch.addEventListener('pointerdown', event => {
    if (!event.isPrimary || event.button !== 0 || drag) return;
    const rect = launch.getBoundingClientRect();
    dragged = false;
    drag = {id:event.pointerId, x:event.clientX, y:event.clientY, left:rect.left, top:rect.top};
    launch.setPointerCapture(event.pointerId);
  });
  launch.addEventListener('pointermove', event => {
    if (!drag || event.pointerId !== drag.id) return;
    const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
    if (!dragged && Math.hypot(dx, dy) < 6) return;
    dragged = true;
    event.preventDefault();
    launch.classList.add('is-dragging');
    const area = viewport();
    const width = Math.max(0, area.width - launch.offsetWidth), height = Math.max(0, area.height - launch.offsetHeight);
    position = {x:width ? clamp((drag.left + dx - area.left) / width, 0, 1) : 0,
      y:height ? clamp((drag.top + dy - area.top) / height, 0, 1) : 0};
    layout();
  });
  function finishDrag(event) {
    if (!drag || event.pointerId !== drag.id) return;
    drag = null;
    launch.classList.remove('is-dragging');
    if (launch.hasPointerCapture(event.pointerId)) launch.releasePointerCapture(event.pointerId);
    if (dragged) {
      // Store only relative button coordinates, so resizing keeps it in view.
      try { win.localStorage.setItem(positionKey, JSON.stringify(position)); } catch { /* Optional preference. */ }
    }
  }
  for (const type of ['pointerup','pointercancel','lostpointercapture']) launch.addEventListener(type, finishDrag);
  win.addEventListener('resize', layout);
  win.visualViewport?.addEventListener('resize', layout);
  win.visualViewport?.addEventListener('scroll', layout);
  if (win.ResizeObserver) {
    const observer = new win.ResizeObserver(layout);
    observer.observe(launch);
    observer.observe(panel);
  }
  let pending = false, resultKey = '', resultId = null;
  const words = () => {
    const language = doc.documentElement?.lang;
    if (language) return translations[language.toLowerCase().startsWith('en') ? 'en' : 'zh'];
    try { return translations[win.localStorage.getItem('hireferry-language') === 'en' ? 'en' : 'zh']; }
    catch { return translations.zh; }
  };
  function translate() {
    const t = words();
    root.querySelectorAll('[data-feedback]').forEach(el => {
      const value = t[el.dataset.feedback === 'submit' && pending ? 'sending' : el.dataset.feedback];
      if (el.textContent !== value) el.textContent = value;
    });
    message.placeholder = t.placeholder;
    launch.title = t.move;
    launch.setAttribute('aria-label', t.open);
    $('[data-feedback-close]').setAttribute('aria-label', t.close);
    result.textContent = resultKey ? t[resultKey] + (resultKey === 'success' ? ` · #${resultId}` : '') : '';
  }
  function toggle(open, anchor = launch) {
    panel.hidden = !open;
    launch.setAttribute('aria-expanded', String(open));
    if (open) { panelAnchor = anchor; translate(); layout(); message.focus(); }
    else {
      // The originating route may have changed while this shared panel was open.
      const target = [panelAnchor, ...doc.querySelectorAll('.chat-tools > summary, [data-chat-feedback]'), launch].find(visible);
      target?.focus();
    }
  }
  translate();
  layout();
  // The SPA and standalone OAuth pages both publish their active locale here.
  // Translate the existing widget so drafts, selections and dragged positions survive.
  if (doc.documentElement && win.MutationObserver) {
    const languageObserver = new win.MutationObserver(translate);
    languageObserver.observe(doc.documentElement, {attributes:true, attributeFilter:['lang']});
  }
  launch.addEventListener('click', event => {
    // Keyboard and assistive-technology clicks have no pointer click count.
    if (dragged && event.detail !== 0) { dragged = false; event.preventDefault(); return; }
    dragged = false;
    toggle(panel.hidden);
  });
  doc.addEventListener('click', event => {
    const trigger = event.target.closest?.('[data-chat-feedback]');
    if (!trigger) return;
    event.preventDefault();
    const menu = trigger.closest('details.chat-tools');
    const anchor = menu?.querySelector('summary') || trigger;
    trigger.setAttribute('aria-haspopup', 'dialog');
    trigger.setAttribute('aria-controls', 'feedback-panel');
    if (menu) menu.open = false;
    toggle(true, anchor);
  });
  $('[data-feedback-close]').addEventListener('click', () => toggle(false));
  doc.addEventListener('keydown', event => { if (event.key === 'Escape' && !panel.hidden) toggle(false); });
  doc.addEventListener('pointerdown', event => {
    if (!panel.hidden && !root.contains(event.target) && !event.target.closest?.('[data-action="language"], #language')) toggle(false);
  });
  message.addEventListener('input', () => { $('#feedback-length').textContent = String([...message.value].length); });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (pending || !form.reportValidity()) return;
    pending = true;
    send.disabled = true;
    resultKey = '';resultId = null;translate();
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
      if (response.status === 409) throw new Error('duplicate');
      if (response.status === 503) throw new Error('unavailable');
      if (!response.ok) throw new Error('submit');
      const data = await response.json();
      resultKey = 'success';resultId = data.id;
      form.reset();
      $('#feedback-length').textContent = '0';
    } catch (error) {
      resultKey = ['limit','duplicate','unavailable'].includes(error.message) ? error.message : 'error';
      result.classList.add('error');
    } finally {
      pending = false;
      send.disabled = false;
      translate();
    }
  });
}

if (typeof document !== 'undefined') mountQuickFeedback(document, window);

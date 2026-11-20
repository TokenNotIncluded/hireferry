const PAGE_SIZE = 20;
const count = value => Number.isSafeInteger(value) && value >= 0 ? value : 0;
const lastOffset = total => Math.floor(Math.max(0, count(total) - 1) / PAGE_SIZE) * PAGE_SIZE;
const boundedOffset = (value, total) => Math.min(lastOffset(total), Math.floor(count(value) / PAGE_SIZE) * PAGE_SIZE);

export function supportsJobInteractions(job, employer = job?.employer) {
  return job?.status === 'published' && Number(job.headcount) > 0 && !employer?.banned;
}

export function createJobInteractionsUI({state, api, t, esc, date, ensureSession}) {
  let panel, controller, summary, busy = false, error = '', unavailable = false, draft = '', retryOffset = 0, authorized = false, generation = 0, summaryIdentity = '', sessionError = '';
  const identity = () => `${state.user?.id || ''}:${state.csrf || ''}`;
  const live = target => panel === target && target.isConnected && !controller?.signal.aborted;

  function placeholder(job, employer) {
    if (!supportsJobInteractions(job, employer)) return '';
    return `<section class="job-interactions" data-job-interactions="${esc(job.id)}" aria-labelledby="job-interactions-title" aria-busy="true"><h2 id="job-interactions-title">${t('岗位互动','Role discussion')}</h2><p class="help" role="status">${t('正在加载互动…','Loading discussion…')}</p></section>`;
  }

  function paint() {
    if (!panel) return;
    const focused = panel.ownerDocument.activeElement;
    const editing = focused?.matches('[name="body"]') && panel.contains(focused);
    const selection = editing ? [focused.selectionStart, focused.selectionEnd] : null;
    const action = panel.contains(focused) ? focused?.dataset.jobInteraction : null;
    const actionId = focused?.dataset.commentId;
    const disabled = busy ? ' disabled' : '';
    const signedIn = authorized && !!state.user && !!state.csrf;
    const votes = summary ? `<div class="job-votes" role="group" aria-label="${t('岗位评价','Role votes')}">${[1,-1].map(vote => `<button type="button" class="button secondary small job-vote" data-job-interaction="vote" data-vote="${vote}" aria-pressed="${signedIn && summary.my_vote === vote}"${busy || !signedIn ? ' disabled' : ''}>${vote === 1 ? t('赞','Upvote') : t('踩','Downvote')} <span>${count(vote === 1 ? summary.upvotes : summary.downvotes)}</span></button>`).join('')}</div>` : '';
    const comments = summary ? `<h3>${t('公开评论','Public comments')} · ${count(summary.total)}</h3><div class="job-comments">${summary.comments.length ? summary.comments.map(comment => `<article class="job-comment"><div class="job-comment-meta"><strong>${esc(comment.author_name)}</strong><span>${esc(date(comment.created_at))}</span>${signedIn && comment.can_delete ? `<button type="button" class="text-button danger-text" data-job-interaction="delete" data-comment-id="${esc(comment.id)}"${disabled}>${t('删除','Delete')}</button>` : ''}</div><p class="job-comment-body">${esc(comment.body)}</p></article>`).join('') : `<p class="help">${t('还没有评论。','No comments yet.')}</p>`}</div>${summary.total > PAGE_SIZE || summary.offset > 0 ? `<nav class="job-comments-pages" aria-label="${t('评论分页','Comment pages')}"><button type="button" class="button secondary small" data-job-interaction="previous"${busy || summary.offset <= 0 ? ' disabled' : ''}>${t('上一页','Previous')}</button><span class="help">${t(`第 ${Math.floor(summary.offset / PAGE_SIZE) + 1} / ${Math.floor(lastOffset(summary.total) / PAGE_SIZE) + 1} 页`,`Page ${Math.floor(summary.offset / PAGE_SIZE) + 1} of ${Math.floor(lastOffset(summary.total) / PAGE_SIZE) + 1}`)}</span><button type="button" class="button secondary small" data-job-interaction="next"${busy || summary.offset >= lastOffset(summary.total) ? ' disabled' : ''}>${t('下一页','Next')}</button></nav>` : ''}` : '';
    panel.setAttribute('aria-busy', String(busy));
    panel.innerHTML = `<h2 id="job-interactions-title">${t('岗位互动','Role discussion')}</h2><p class="help" id="job-comment-disclaimer">${t('投票和评论面向公众。评论会公开显示，请勿发布联系方式或其他敏感信息。','Votes and comments are public. Comments are visible to everyone; do not post contact details or other sensitive information.')}</p>${sessionError ? `<p class="notice error" role="alert">${esc(sessionError)}</p><button type="button" class="text-button" data-job-interaction="session"${disabled}>${t('重试登录状态','Retry sign-in check')}</button>` : ''}${unavailable ? `<p class="help" role="status">${t('这个岗位已停止公开互动。','Public discussion is no longer available for this role.')}</p>` : `${votes}${busy ? `<p class="help" role="status">${t('正在加载或保存…','Loading or saving…')}</p>` : ''}${error ? `<div class="notice error" role="alert"><p>${esc(error)}</p><button type="button" class="text-button" data-job-interaction="retry"${disabled}>${t('刷新互动','Refresh discussion')}</button></div>` : ''}${comments}${signedIn && summary ? `<form data-job-comment-form><div class="field"><label for="job-comment-body">${t('发表评论','Add a comment')}</label><textarea class="input" id="job-comment-body" name="body" rows="4" aria-describedby="job-comment-disclaimer job-comment-limit" required${disabled}>${esc(draft)}</textarea><p class="help" id="job-comment-limit">${t('去除首尾空白后，1–2000 个字符。','1–2000 characters after trimming whitespace.')}</p></div><button type="submit" class="button small"${disabled}>${t('公开发表','Post publicly')}</button></form>` : !busy && authorized && !state.user ? `<a class="button secondary small" href="/login" data-nav>${t('登录后投票或评论','Sign in to vote or comment')}</a>` : ''}`}`;
    if (editing) {
      const field = panel.querySelector('[name="body"]');
      if (field && !field.disabled) { field.focus({preventScroll:true});field.setSelectionRange(...selection); }
    } else if (action) {
      const button = [...panel.querySelectorAll('[data-job-interaction]')].find(item => item.dataset.jobInteraction === action && item.dataset.commentId === actionId && item.dataset.vote === focused.dataset.vote);
      if (button && !button.disabled) button.focus({preventScroll:true});
    }
  }

  function normalize(value) {
    return {...value, total:count(value.total), offset:Math.floor(count(value.offset) / PAGE_SIZE) * PAGE_SIZE, comments:Array.isArray(value.comments) ? value.comments.slice(0, PAGE_SIZE) : []};
  }

  async function request({method = 'GET', path, body, offset = 0} = {}) {
    if (!panel || busy || unavailable) return;
    const target = panel, scope = identity(), version = generation, signal = controller.signal;
    const current = () => live(target) && generation === version;
    if (method !== 'GET' && (!authorized || !state.user || !state.csrf)) return;
    if (method !== 'GET' && summaryIdentity !== scope) { void mount(target.parentNode);return; }
    busy = true;error = '';retryOffset = offset;paint();
    try {
      const base = `/api/public/jobs/${encodeURIComponent(target.dataset.jobInteractions)}/interactions`;
      let result = normalize(await api(path || `${base}?offset=${offset}`, {method, ...(body === undefined ? {} : {body}), fresh:true, ...(method === 'GET' ? {signal} : {})}));
      if (!current() || identity() !== scope) return;
      // A deletion on another page may leave the requested last page empty.
      const corrected = boundedOffset(result.offset, result.total);
      if (method === 'GET' && corrected !== result.offset) {
        result = normalize(await api(`${base}?offset=${corrected}`, {fresh:true, signal}));
        if (!current() || identity() !== scope) return;
      }
      summary = result;summaryIdentity = scope;
      if (method === 'POST') draft = '';
      retryOffset = boundedOffset(summary.offset, summary.total);
    } catch (failure) {
      if (!current() || identity() !== scope) return;
      unavailable = failure.status === 404;
      error = failure.message || t('互动暂时无法加载，请稍后重试。','Discussion could not be loaded. Try again later.');
    } finally {
      if (current()) {
        busy = false;
        if (identity() !== scope) {
          summary = null;draft = '';authorized = false;
          void mount(target.parentNode);
        } else paint();
      }
    }
  }

  const onInput = event => { if (event.target.matches('[name="body"]')) draft = event.target.value; };
  const onClick = event => {
    const button = event.target.closest('[data-job-interaction]');
    if (!button || !panel?.contains(button) || button.disabled || busy) return;
    event.preventDefault();
    const action = button.dataset.jobInteraction;
    if (action === 'session') void mount(panel.parentNode);
    else if (action === 'retry') void request({offset:summary ? boundedOffset(retryOffset, summary.total) : 0});
    else if (action === 'previous' || action === 'next') {
      if (summary) void request({offset:boundedOffset(summary.offset + (action === 'next' ? PAGE_SIZE : -PAGE_SIZE), summary.total)});
    } else if (authorized && state.user && state.csrf && summary) {
      const base = `/api/jobs/${encodeURIComponent(panel.dataset.jobInteractions)}`;
      if (action === 'vote') {
        const vote = Number(button.dataset.vote);
        if (vote === 1 || vote === -1) void request({method:'PUT', path:`${base}/vote`, body:{vote:summary.my_vote === vote ? 0 : vote}});
      } else if (action === 'delete' && summary.comments.some(comment => String(comment.id) === button.dataset.commentId && comment.can_delete)) {
        void request({method:'DELETE', path:`${base}/comments/${encodeURIComponent(button.dataset.commentId)}`});
      }
    }
  };
  const onSubmit = event => {
    if (!event.target.matches('[data-job-comment-form]')) return;
    event.preventDefault();
    if (busy || !authorized || !state.user || !state.csrf || !summary) return;
    draft = event.target.querySelector('[name="body"]').value;
    const body = draft.trim(), length = Array.from(body).length;
    if (length < 1 || length > 2000) { error = t('评论须为 1–2000 个字符。','Use 1–2000 characters for your comment.');paint();return; }
    void request({method:'POST', path:`/api/jobs/${encodeURIComponent(panel.dataset.jobInteractions)}/comments`, body:{body}});
  };

  function destroy() {
    generation++;
    controller?.abort();
    panel?.removeEventListener('click', onClick);
    panel?.removeEventListener('submit', onSubmit);
    panel?.removeEventListener('input', onInput);
    panel = null;controller = null;
  }

  async function mount(root) {
    destroy();
    const target = root.querySelector('[data-job-interactions]');
    if (!target) return;
    panel = target;controller = new AbortController();
    const version = generation;
    summary = null;busy = false;error = '';unavailable = false;draft = '';retryOffset = 0;authorized = false;sessionError = '';summaryIdentity = '';
    panel.addEventListener('click', onClick);
    panel.addEventListener('submit', onSubmit);
    panel.addEventListener('input', onInput);
    try {
      await ensureSession();
      if (!live(target) || generation !== version) return;
      authorized = true;
    } catch {
      if (!live(target) || generation !== version) return;
      sessionError = t('登录状态暂时无法确认，仍可浏览公开互动。','Sign-in status could not be checked. You can still read public discussion.');
    }
    await request();
  }

  return {placeholder, mount, destroy};
}

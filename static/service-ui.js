/* HireFerry service workspace: private conversations, mandates, and human matching. */
export function createMessageId(cryptoApi = globalThis.crypto) {
  const bytes = new Uint8Array(16);
  cryptoApi.getRandomValues(bytes);
  return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

export function createServiceUI({ state, api, t, esc, icon, avatar, money, date, heading, shell, empty, notice, badge, matchText, openDialog, closeDialog, notify, go, render, centsInput }) {
  let polling = null;
  let generation = 0;
  let chat = null;
  const matchCache = new Map();
  const outreachCache = new Map();
  const mandateCache = new Map();
  const pendingMessages = new WeakMap();
  const statuses = ['introduced', 'interview', 'hired', 'declined'];
  const outreachStages = ['identified', 'contacted', 'replied', 'interview', 'offer', 'closed'];
  const query = () => new URLSearchParams(location.search);
  const route = (admin, tab, extra = {}) => (admin ? '/admin' : '/app') + '?' + new URLSearchParams({ tab, ...extra });
  const chatUrl = (admin, id) => route(admin, 'messages', { conversation: id });
  const humanTime = value => value ? new Date(Number(value) < 1e12 ? Number(value) * 1000 : value).toLocaleString(state.lang === 'en' ? 'en-GB' : 'zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '';
  const mandateStatus = value => ({ active: t('寻找中', 'Active'), paused: t('已暂停', 'Paused'), closed: t('已结束', 'Closed') })[value] || value;
  const outreachText = value => ({ identified: t('发现机会', 'Identified'), contacted: t('已联系', 'Contacted'), replied: t('有回复', 'Replied'), interview: t('面试中', 'Interview'), offer: t('收到意向', 'Offer'), closed: t('已结束', 'Closed') })[value] || value;
  const safeLink = value => { try { const u = new URL(value); return ['https:', 'http:'].includes(u.protocol) ? u.href : ''; } catch { return ''; } };
  const salaryRange = m => m.salary_min_cents && m.salary_max_cents ? `${money(m.salary_min_cents)} – ${money(m.salary_max_cents)}` : m.salary_min_cents ? `${money(m.salary_min_cents)} +` : m.salary_max_cents ? `${t('最高', 'Up to')} ${money(m.salary_max_cents)}` : t('可以沟通', 'Open to discussion');

  function stopChat() {
    generation += 1;
    clearTimeout(polling);
    polling = null;
    chat = null;
  }

  function conversationLabel(c) {
    if (c.kind === 'platform') {
      if (!state.admin) return t('与聘渡平台', 'Your HireFerry team');
      const person = c.participants?.find(p => p.id === c.user_id) || c.participants?.[0];
      return person?.name || c.title || t('平台私聊', 'Private platform chat');
    }
    return c.title || t('匹配会话', 'Introduction room');
  }

  function conversationList(items, selected, admin) {
    const groups = [['platform', t('平台私聊', 'Private conversations')], ['matched', t('匹配后会话', 'Introduction rooms')]];
    return groups.map(([kind, label]) => {
      const list = items.filter(c => c.kind === kind);
      if (!list.length) return '';
      return `<section class="conversation-group"><h3>${label}<span>${list.length}</span></h3>${list.map(c => `<a href="${esc(chatUrl(admin, c.id))}" data-nav class="conversation-item ${c.id === selected ? 'active' : ''}" ${c.id === selected ? 'aria-current="page"' : ''}><span class="conversation-avatar">${icon(kind === 'platform' ? 'shield' : 'link')}</span><span class="conversation-copy"><strong>${esc(conversationLabel(c))}</strong><span>${esc(c.last_message_preview || (kind === 'platform' ? t('从一句你好开始。', 'Start with a hello.') : t('新的对话，从这里开始。', 'Your conversation starts here.')))}</span><small>${humanTime(c.last_message_at)}</small></span>${c.unread_count ? `<span class="unread-badge" aria-label="${c.unread_count} ${t('条未读消息', 'unread messages')}">${c.unread_count > 99 ? '99+' : c.unread_count}</span>` : ''}</a>`).join('')}</section>`;
    }).join('') || `<p class="conversation-empty">${t('还没有会话。', 'No conversations yet.')}</p>`;
  }

  function messageHtml(m, admin) {
    const own = admin ? m.sender_kind === 'platform' : m.sender_id === state.user?.id;
    const author = m.sender_kind === 'platform' ? t('聘渡平台', 'HireFerry team') : m.sender_name || t('参与者', 'Participant');
    return `<article class="chat-message ${own ? 'own' : ''} ${m.sender_kind === 'platform' ? 'from-platform' : ''}" data-message-id="${Number(m.id)}"><div class="message-meta"><strong>${esc(author)}</strong>${m.via_ai ? `<span class="ai-label">${icon('bot')} ${t('AI 代发', 'via AI')}</span>` : ''}<time>${humanTime(m.created_at)}</time></div><div class="message-body markdown">${m.body_html || esc(m.body).replace(/\n/g, '<br>')}</div></article>`;
  }

  async function messagesPage(admin = false) {
    if (!admin && state.user?.role === 'unassigned') return shell(heading(t('先认识一下彼此。', 'Let’s get acquainted.'), t('选择身份后，和平台一起准备下一站。', 'Choose your role, then plan your next step with us.')), 'messages');
    if (!admin) await api('/api/chat/platform', { method: 'POST', body: {} });
    const index = await api('/api/chat/conversations');
    const items = index.items || [];
    const requested = query().get('conversation');
    const selected = requested || (!admin ? items.find(c => c.kind === 'platform' && c.user_id === state.user.id)?.id : null) || items[0]?.id;
    let current = items.find(c => c.id === selected);
    let messages = { items: [], next_cursor: 0, first_cursor: 0, has_older: false };
    if (selected) {
      messages = await api(`/api/chat/conversations/${encodeURIComponent(selected)}/messages?limit=100`);
      if (!current) {
        const refreshed = await api('/api/chat/conversations');
        items.splice(0, items.length, ...(refreshed.items || []));
        current = items.find(c => c.id === selected);
      }
      if (!current) throw new Error(t('这个会话当前不可访问。', 'This conversation is not available.'));
    }
    const seen = new Set((messages.items || []).map(m => m.id));
    chat = { id: selected, admin, items, current, cursor: messages.next_cursor || 0, first: messages.first_cursor || messages.items?.[0]?.id || 0, hasOlder: !!messages.has_older, seen, page: generation };
    const privacy = current?.kind === 'matched' ? t('仅匹配双方与平台可见。平台私聊记录不会转入这里。', 'Only the matched parties and HireFerry can see this room. Private platform history stays separate.') : admin ? t('仅这位用户与平台可见。不会向任何匹配方展示。', 'Only this user and HireFerry can see this conversation. It is never shared with a match.') : t('仅你与聘渡平台可见。先说说期待，我们认真去找。', 'Only you and HireFerry can see this conversation. Tell us what you need; we’ll take it from there.');
    return shell(`${heading(admin ? t('每个需求，都有人接住。', 'Keep every conversation moving.') : t('先聊聊，我们陪你往前。', 'Let’s talk about what comes next.'), admin ? t('平台私聊与匹配会话分开，沟通记录各自保密。', 'Private conversations and introduction rooms have separate histories.') : t('与平台一对一沟通。匹配建立后，再和对方认识。', 'Speak with HireFerry first. Meet the other side after an introduction.'), '', 'CONVERSATIONS')}<section class="chat-workspace"><aside class="conversation-sidebar"><div class="conversation-sidebar-heading"><h2>${t('会话', 'Conversations')}</h2><span class="live-dot" aria-label="${t('每 5 秒更新', 'Updates every 5 seconds')}"></span></div><div id="conversation-list">${conversationList(items, selected, admin)}</div></aside><section class="chat-thread" aria-label="${t('消息内容', 'Messages')}">${current ? `<header class="chat-thread-header"><div><h2>${esc(conversationLabel(current))}</h2><p>${current.writable === false ? t('历史会话 · 只读', 'Archived conversation · read-only') : current.kind === 'matched' ? t('匹配双方 · 聘渡平台', 'Matched parties · HireFerry') : t('一对一平台私聊', 'Private conversation with HireFerry')}</p></div><span class="pill">${icon(current.kind === 'matched' ? 'link' : 'shield')}${current.kind === 'matched' ? t('已建立匹配', 'Introduced') : t('私密', 'Private')}</span></header><div class="chat-privacy">${icon('shield')}<span>${privacy}</span></div><div class="chat-scroll" id="chat-scroll"><button type="button" class="chat-history text-button" data-action="chat-older" ${messages.has_older ? '' : 'hidden'}>${t('加载更早的消息', 'Load earlier messages')}</button><div id="chat-messages" role="log" aria-live="polite" aria-relevant="additions">${messages.items?.length ? messages.items.map(m => messageHtml(m, admin)).join('') : `<div id="chat-empty" class="chat-welcome"><span>${icon('message')}</span><h3>${current.kind === 'platform' ? t('一句你好，就是开始。', 'A hello is a good beginning.') : t('在这里，认识一下彼此。', 'Get to know each other here.')}</h3><p>${current.kind === 'platform' ? t('告诉我们你在找什么，也可以随时补充想法。', 'Tell us what you’re looking for. Add questions or thoughts at any time.') : t('这是一段全新的对话，不包含任何历史私聊。', 'This is a fresh conversation. No private chat history is included.')}</p></div>`}</div></div><button type="button" data-action="chat-latest" class="chat-new-message" hidden>${t('有新消息 · 回到最新', 'New messages · Jump to latest')} ↓</button><form data-form="chat" class="chat-composer"><label class="sr-only" for="chat-body">${t('写一条消息', 'Write a message')}</label><textarea id="chat-body" name="body" rows="2" maxlength="10000" required ${current.writable === false ? 'disabled' : ''} placeholder="${esc(t('想说的，写在这里。', 'Write what’s on your mind.'))}"></textarea><div class="chat-composer-footer"><span id="chat-connection" class="help" role="status">${current.writable === false ? t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.') : t('消息会自动更新', 'Messages update automatically')}</span><button class="button small" type="submit" ${current.writable === false ? 'disabled' : ''}>${t('发送', 'Send')}${icon('send')}</button></div></form>` : empty(t('等待第一句你好。', 'Waiting for the first hello.'), t('新的平台会话和匹配会话会显示在这里。', 'Private conversations and introduction rooms will appear here.'), '', 'message')}</section></section>`, 'messages', admin);
  }

  function scrollLatest() {
    const scroll = document.querySelector('#chat-scroll');
    if (scroll) scroll.scrollTop = scroll.scrollHeight;
    const button = document.querySelector('[data-action="chat-latest"]');
    if (button) button.hidden = true;
  }

  async function markRead(current) {
    if (!current?.id || !current.cursor) return;
    await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/read`, { method: 'POST', body: { message_id: current.cursor } });
  }

  function appendMessages(current, messages, forceScroll = false) {
    const container = document.querySelector('#chat-messages');
    const scroll = document.querySelector('#chat-scroll');
    if (!container || !scroll || chat !== current) return;
    const nearBottom = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 150;
    const fresh = (messages || []).filter(m => !current.seen.has(m.id));
    if (!fresh.length) return;
    document.querySelector('#chat-empty')?.remove();
    for (const m of fresh) { current.seen.add(m.id); container.insertAdjacentHTML('beforeend', messageHtml(m, current.admin)); }
    current.first ||= fresh[0].id;
    if (nearBottom || forceScroll) scrollLatest();
    else { const button = document.querySelector('[data-action="chat-latest"]'); if (button) button.hidden = false; }
  }

  function mountChat() {
    const current = chat;
    if (!current?.id || !document.querySelector('#chat-scroll')) return;
    const activeGeneration = generation;
    scrollLatest();
    markRead(current).catch(() => {});
    const poll = async () => {
      if (generation !== activeGeneration || chat !== current) return;
      try {
        if (!document.hidden) {
          const [messages, index] = await Promise.all([
            api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages?after=${current.cursor}&limit=100`),
            api('/api/chat/conversations'),
          ]);
          if (generation !== activeGeneration || chat !== current) return;
          appendMessages(current, messages.items);
          current.cursor = Math.max(current.cursor, messages.next_cursor || 0);
          current.items = index.items || [];
          const selected = current.items.find(c => c.id === current.id);
          if (selected) { selected.unread_count = 0; current.current = selected; }
          const list = document.querySelector('#conversation-list');
          if (list) list.innerHTML = conversationList(current.items, current.id, current.admin);
          const status = document.querySelector('#chat-connection');
          if (status) status.textContent = current.current?.writable === false ? t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.') : t('消息已同步', 'Messages are up to date');
          document.querySelectorAll('.chat-composer textarea,.chat-composer button').forEach(el => { el.disabled = current.current?.writable === false || el.getAttribute('aria-busy') === 'true'; });
          await markRead(current);
        }
      } catch (error) {
        if (generation === activeGeneration) {
          const status = document.querySelector('#chat-connection');
          if (status) status.textContent = error.status === 404 ? t('这个会话已不可访问，请返回会话列表。', 'This conversation is no longer available. Return to your conversation list.') : t('连接暂时中断，正在重试。', 'Connection interrupted. Retrying.');
        }
      } finally {
        if (generation === activeGeneration && chat === current) polling = setTimeout(poll, 5000);
      }
    };
    polling = setTimeout(poll, 5000);
  }

  async function sendMessage(form, data) {
    const current = chat;
    if (!current?.id) throw new Error(t('请先选择一个会话。', 'Choose a conversation first.'));
    if (current.current?.writable === false) throw new Error(t('这个会话已归档，只能查看历史。', 'This room is archived and read-only.'));
    const body = String(data.get('body') || '').trim();
    if (!body) throw new Error(t('写一点内容再发送吧。', 'Write a message before sending.'));
    const previous = pendingMessages.get(form);
    const client_id = previous?.body === body ? previous.id : createMessageId();
    pendingMessages.set(form, { id: client_id, body });
    const message = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages`, { method: 'POST', body: { body, client_id } });
    pendingMessages.delete(form);
    if (chat !== current) return;
    const textarea = form.querySelector('textarea');
    if (textarea.value.trim() === body) textarea.value = '';
    appendMessages(current, [message], true);
    // The poll cursor advances only from the incremental endpoint so concurrent replies are never skipped.
    await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/read`, { method: 'POST', body: { message_id: message.id } });
    textarea.focus();
  }

  async function olderMessages() {
    const current = chat;
    if (!current?.first || !current.hasOlder) return;
    const result = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages?before=${current.first}&limit=100`);
    if (chat !== current) return;
    const scroll = document.querySelector('#chat-scroll');
    const container = document.querySelector('#chat-messages');
    const oldHeight = scroll.scrollHeight;
    const oldTop = scroll.scrollTop;
    const fresh = (result.items || []).filter(m => !current.seen.has(m.id));
    fresh.forEach(m => current.seen.add(m.id));
    container.insertAdjacentHTML('afterbegin', fresh.map(m => messageHtml(m, current.admin)).join(''));
    current.first = result.first_cursor || current.first;
    current.hasOlder = !!result.has_older;
    document.querySelector('[data-action="chat-older"]').hidden = !current.hasOlder;
    scroll.scrollTop = oldTop + scroll.scrollHeight - oldHeight;
  }

  function mandateFields(m = {}, admin = false) {
    return `<div class="grid-2"><div class="field"><label for="mandate-industry">${t('想去的行业', 'Target industry')}</label><input class="input" id="mandate-industry" name="target_industry" maxlength="200" value="${esc(m.target_industry)}" placeholder="${esc(t('例如：教育、软件、消费品牌', 'For example: education, software, consumer brands'))}"></div><div class="field"><label for="mandate-roles">${t('想做的岗位', 'Target roles')}</label><input class="input" id="mandate-roles" name="target_roles" maxlength="500" value="${esc(m.target_roles)}" placeholder="${esc(t('例如：产品设计师、Rust 工程师', 'For example: product designer, Rust engineer'))}" required></div></div><div class="grid-2"><div class="field"><label for="mandate-min">${t('期待月薪底薪 · 最低人民币', 'Expected monthly base · minimum CNY')}</label><input class="input" id="mandate-min" name="salary_min" type="number" min="1" step="0.01" value="${m.salary_min_cents ? m.salary_min_cents / 100 : ''}" placeholder="${esc(t('可以留空', 'Optional'))}"></div><div class="field"><label for="mandate-max">${t('期待月薪底薪 · 最高人民币', 'Expected monthly base · maximum CNY')}</label><input class="input" id="mandate-max" name="salary_max" type="number" min="1" step="0.01" value="${m.salary_max_cents ? m.salary_max_cents / 100 : ''}" placeholder="${esc(t('可以留空', 'Optional'))}"></div></div><div class="grid-2"><div class="field"><label for="mandate-location">${t('地点与工作方式', 'Location and work arrangement')}</label><input class="input" id="mandate-location" name="location" maxlength="300" value="${esc(m.location)}" placeholder="${esc(t('例如：上海 / 远程 / 混合办公', 'For example: Shanghai / remote / hybrid'))}"></div><div class="field"><label for="mandate-status">${t('委托状态', 'Search status')}</label><select class="input" id="mandate-status" name="status">${['active', 'paused', 'closed'].map(s => `<option value="${s}" ${s === (admin && m.banned && m.status === 'active' ? 'paused' : m.status || 'active') ? 'selected' : ''} ${admin && s === 'active' && (m.banned || m.status !== 'active') ? 'disabled' : ''}>${mandateStatus(s)}</option>`).join('')}</select></div></div><div class="field"><label for="mandate-preferences">${t('还有什么对你很重要？', 'What else matters to you?')}</label><textarea class="input" id="mandate-preferences" name="preferences" rows="5" maxlength="5000" placeholder="${esc(t('团队、成长方向、工作节奏，以及你希望我们了解的事。', 'The team, growth, working rhythm, and anything else we should know.'))}">${esc(m.preferences)}</textarea></div>${admin ? `<div class="notice">${icon('shield')}<div>${m.allow_external ? t('本人已授权多渠道寻找。管理员不能扩大这项授权。', 'The job seeker has authorized external outreach. Administrators cannot expand this consent.') : t('本人尚未授权外部联系。请先在平台私聊中沟通。', 'External outreach is not authorized. Discuss it in the private platform conversation first.')}${m.consented_at ? `<br>${t('授权时间', 'Consent recorded')}: ${humanTime(m.consented_at)}` : ''}</div></div>` : `<label class="toggle-row mandate-consent"><input type="checkbox" name="allow_external" ${m.allow_external ? 'checked' : ''}><span><strong>${t('授权聘渡通过多渠道为我寻找机会', 'Authorize HireFerry to seek opportunities across channels')}</strong><small>${t('由平台围绕这份委托开展外部联系，并在这里记录进展。你可以随时取消授权或暂停委托。', 'HireFerry may contact relevant parties for this search and record progress here. You can withdraw consent or pause at any time.')}</small></span></label>`}`;
  }

  function outreachList(items, admin = false, userId = '') {
    if (!items?.length) return empty(t('进展，会认真记在这里。', 'Every step will be recorded here.'), t('平台完成实际寻找或联系后，会更新机会与跟进情况。', 'The team will update opportunities and follow-ups after real research or contact.'), '', 'target');
    return `<div class="outreach-timeline">${items.map(record => {
      outreachCache.set(record.id, record);
      const link = safeLink(record.source_url);
      return `<article class="outreach-record"><div class="outreach-point"></div><div class="outreach-record-top"><span class="status ${record.stage === 'closed' ? 'closed' : ''}">${outreachText(record.stage)}</span>${record.via_ai ? `<span class="ai-label">${icon('bot')} ${t('AI 记录', 'via AI')}</span>` : ''}<time>${humanTime(record.updated_at || record.created_at)}</time></div><h3>${esc(record.opportunity)}</h3><div class="list-meta"><span>${esc(record.channel)}</span>${link ? `<a class="help-link" href="${esc(link)}" target="_blank" rel="noopener noreferrer">${t('来源', 'Source')} ↗</a>` : ''}</div>${record.note ? `<p>${esc(record.note)}</p>` : ''}${admin ? `<button type="button" class="text-button" data-action="edit-outreach" data-id="${esc(record.id)}" data-user="${esc(userId)}">${t('更新进展', 'Update progress')} →</button>` : ''}</article>`;
    }).join('')}</div>`;
  }

  async function mandatePage() {
    if (state.user?.role !== 'seeker') return shell(empty(t('委托属于求职者。', 'Search mandates are for job seekers.'), t('招聘需求可以直接在岗位和平台私聊中说明。', 'Share hiring needs through your roles and private platform conversation.'), `<a href="/app?tab=messages" data-nav class="button">${t('与平台聊聊', 'Talk to HireFerry')}${icon('message')}</a>`, 'target'), 'mandate');
    const [result, progress] = await Promise.all([api('/api/mandate'), api('/api/outreach')]);
    const m = result.mandate || {};
    return shell(`${heading(t('你的下一站，我们一起找。', 'Let’s find your next chapter.'), t('告诉我们方向。平台持续寻找，你随时看见进展。', 'Tell us your direction. We keep looking, and you can follow the progress.'), '', 'YOUR SEARCH MANDATE')}<section class="panel"><div class="panel-header"><div><h2>${t('一份明确的委托', 'A clear search brief')}</h2><p>${t('只有平台可以查看这份委托，不会成为公开列表。', 'Only HireFerry can view this brief. It never becomes a public listing.')}</p></div>${m.user_id ? `<span class="status ${m.status === 'active' ? '' : 'closed'}">${mandateStatus(m.status)}</span>` : `<span class="pill">${t('尚未提交', 'Not submitted')}</span>`}</div><form data-form="mandate">${mandateFields(m)}<div class="form-actions"><p class="help">${t('求职服务免费。提交后，你仍可以随时修改期待。', 'The service is free for job seekers. You can update your expectations at any time.')}</p><button class="button" type="submit">${m.user_id ? t('保存委托', 'Save search brief') : t('委托平台帮我找', 'Start my search')}${icon('arrow')}</button></div></form></section><section class="panel"><div class="panel-header"><div><h2>${t('寻找与跟进', 'Search and follow-up')}</h2><p>${t('每一条都是实际记录的进展。', 'Each entry is a recorded step by the team.')}</p></div><a class="text-button" href="/app?tab=messages" data-nav>${t('与平台沟通', 'Talk to the team')} →</a></div>${outreachList(progress.items)}</section>`, 'mandate');
  }

  function mandatePayload(data, admin = false) {
    const min = String(data.get('salary_min') || '').trim();
    const max = String(data.get('salary_max') || '').trim();
    const body = { target_industry: String(data.get('target_industry') || '').trim(), target_roles: String(data.get('target_roles') || '').trim(), salary_min_cents: min ? centsInput(min) : null, salary_max_cents: max ? centsInput(max) : null, location: String(data.get('location') || '').trim(), preferences: String(data.get('preferences') || '').trim(), status: data.get('status') };
    if (body.salary_min_cents && body.salary_max_cents && body.salary_min_cents > body.salary_max_cents) throw new Error(t('最低期待薪资不能高于最高薪资。', 'Minimum salary cannot exceed maximum salary.'));
    if (!admin) body.allow_external = data.has('allow_external');
    return body;
  }

  async function saveMandate(form, data, admin = false) {
    const body = mandatePayload(data, admin);
    if (admin) body.allow_external = form.dataset.allowExternal === 'true';
    await api(admin ? `/api/admin/mandates/${encodeURIComponent(form.dataset.id)}` : '/api/mandate', { method: admin ? 'PATCH' : 'PUT', body });
    closeDialog();
    await render();
    notify(t('委托已保存。我们会按你的方向继续跟进。', 'Search brief saved. We’ll follow your direction.'));
  }

  async function adminMandates() {
    const p = new URLSearchParams();
    for (const key of ['q', 'status', 'page']) if (query().get(key)) p.set(key, query().get(key));
    const d = await api('/api/admin/mandates?' + p);
    return shell(`${heading(t('认真寻找，不止于站内。', 'The search goes beyond this site.'), t('以用户委托为起点，记录多渠道寻找与真实跟进。', 'Start with each person’s brief. Record research and real follow-ups across channels.'), '', 'SEARCH MANDATES')}<section class="panel"><form data-form="mandate-filter" class="filter-row mandate-filter"><div class="search-field">${icon('search')}<input class="input" name="q" value="${esc(query().get('q'))}" aria-label="${t('搜索委托', 'Search briefs')}" placeholder="${esc(t('姓名、行业或岗位', 'Name, industry, or role'))}"></div><select class="input" name="status" aria-label="${t('委托状态', 'Search status')}"><option value="">${t('全部状态', 'All statuses')}</option>${['active', 'paused', 'closed'].map(s => `<option value="${s}" ${query().get('status') === s ? 'selected' : ''}>${mandateStatus(s)}</option>`).join('')}</select><button class="button secondary small" type="submit">${t('查找', 'Find')}</button></form>${d.items?.length ? `<div class="mandate-list">${d.items.map(m => `<article class="mandate-card"><div class="mandate-card-top"><div><h3>${esc(m.user_name)}${m.banned ? ` <span class="status banned">${t('已封禁', 'Suspended')}</span>` : ''}</h3><p>${esc(m.target_roles)}</p></div><span class="status ${m.status === 'active' ? '' : 'closed'}">${mandateStatus(m.status)}</span></div><div class="list-meta"><span>${esc(m.target_industry || t('行业开放', 'Open industry'))}</span><span>${esc(m.location || t('地点开放', 'Open location'))}</span><span>${salaryRange(m)} / ${t('月', 'month')}</span></div><p class="mandate-summary">${esc(m.preferences || t('可以在私聊中进一步了解期待。', 'Discuss their expectations in the private conversation.'))}</p><div class="mandate-card-footer"><span class="consent-label ${m.allow_external ? 'allowed' : ''}">${icon('shield')}${m.allow_external ? t('已授权多渠道寻找', 'External outreach authorized') : t('仅平台内沟通', 'Platform conversation only')}</span><div class="row-actions"><a href="${esc(chatUrl(true, 'platform_' + m.user_id))}" data-nav class="text-button">${t('平台私聊', 'Private chat')}</a><button class="text-button" data-action="admin-mandate" data-id="${esc(m.user_id)}">${t('查看与跟进', 'Review & follow up')} →</button></div></div></article>`).join('')}</div><p class="help">${d.total ?? d.items.length} ${t('份委托', 'search briefs')}</p>${mandatePagination(d.total)}` : empty(t('等待一份新的期待。', 'Waiting for a new direction.'), t('求职者主动提交的委托会显示在这里。', 'Search briefs appear here when job seekers submit them.'), '', 'target')}</section>`, 'mandates', true);
  }

  function mandatePagination(total) {
    const page = Math.max(1, Number(query().get('page')) || 1);
    const pages = Math.ceil(Number(total || 0) / 50);
    if (pages < 2) return '';
    const link = n => { const p = query(); p.set('page', n); return '/admin?' + p; };
    return `<nav class="pagination" aria-label="${t('分页', 'Pagination')}">${page > 1 ? `<a href="${esc(link(page - 1))}" data-nav class="button secondary small">${t('上一页', 'Previous')}</a>` : '<span></span>'}<span>${page} / ${pages}</span>${page < pages ? `<a href="${esc(link(page + 1))}" data-nav class="button secondary small">${t('下一页', 'Next')}</a>` : '<span></span>'}</nav>`;
  }

  async function adminMandate(userId) {
    const [d, progress] = await Promise.all([api(`/api/admin/mandates/${encodeURIComponent(userId)}`), api(`/api/admin/outreach/${encodeURIComponent(userId)}`)]);
    const m = d.mandate;
    if (m) mandateCache.set(userId, m);
    if (!m) throw new Error(t('用户尚未提交委托。', 'This person has not submitted a search brief.'));
    openDialog(`<div class="eyebrow">SEARCH & FOLLOW THROUGH</div><h2 id="dialog-title">${esc(m.user_name)}</h2><p class="dialog-intro">${t('委托只对平台可见。外部联系遵循本人授权。', 'This brief is private to HireFerry. External contact follows the person’s consent.')}</p><form data-form="admin-mandate" data-id="${esc(userId)}" data-allow-external="${m.allow_external ? 'true' : 'false'}">${mandateFields(m, true)}<div class="form-actions"><button class="button" type="submit">${t('保存委托', 'Save brief')}${icon('check')}</button></div></form><hr><div class="panel-header"><h2>${t('真实寻找记录', 'Search records')}</h2><button class="button secondary small" type="button" data-action="new-outreach" data-user="${esc(userId)}" ${m.allow_external && m.status === 'active' && !m.banned ? '' : 'disabled'}>${icon('plus')}${t('记录进展', 'Record progress')}</button></div>${!m.allow_external || m.status !== 'active' || m.banned ? `<p class="help">${t('新增外部联系记录需要本人授权、委托正在寻找且账户有效。已有记录可以结束。', 'New outreach requires consent, an active search brief, and an active account. Existing records can be closed.')}</p>` : ''}${outreachList(progress.items, true, userId)}`, { wide: true });
  }

  function outreachDialog(userId, id = '') {
    const r = id ? outreachCache.get(id) : {};
    const mandate = mandateCache.get(userId);
    const allowed = !!(mandate?.allow_external && mandate.status === 'active' && !mandate.banned);
    if (!r) throw new Error(t('记录已过期，请重新打开委托。', 'Refresh this brief to load the record again.'));
    openDialog(`<div class="eyebrow">A REAL STEP FORWARD</div><h2 id="dialog-title">${id ? t('更新寻找进展', 'Update search progress') : t('记下这次进展', 'Record a step forward')}</h2><p class="dialog-intro">${t('仅记录实际发生的寻找或联系。内容会对这位求职者可见。', 'Record research or contact that actually happened. The job seeker can see this entry.')}</p><form data-form="outreach" data-id="${esc(id)}" data-user="${esc(userId)}"><div class="grid-2"><div class="field"><label>${t('渠道', 'Channel')}<input class="input" name="channel" value="${esc(r.channel)}" maxlength="120" placeholder="${esc(t('例如：企业官网、行业社区、直接联系', 'For example: company website, community, direct contact'))}" required></label></div><div class="field"><label>${t('当前进度', 'Current stage')}<select class="input" name="stage">${outreachStages.map(s => `<option value="${s}" ${s === (!allowed ? 'closed' : r.stage || 'identified') ? 'selected' : ''} ${!allowed && s !== 'closed' ? 'disabled' : ''}>${outreachText(s)}</option>`).join('')}</select></label></div></div><div class="field"><label>${t('机会或联系对象', 'Opportunity or contact')}<input class="input" name="opportunity" value="${esc(r.opportunity)}" maxlength="500" required></label></div><div class="field"><label>${t('来源链接，可选', 'Source link, optional')}<input class="input" name="source_url" type="url" value="${esc(r.source_url)}" maxlength="2000" placeholder="https://"></label></div><div class="field"><label>${t('跟进记录', 'Follow-up note')}<textarea class="input" name="note" rows="5" maxlength="5000">${esc(r.note)}</textarea></label></div><div class="form-actions"><button class="button" type="submit">${t('保存记录', 'Save record')}${icon('check')}</button></div></form>`, { wide: true });
  }

  async function saveOutreach(form, data) {
    const body = Object.fromEntries(['channel', 'opportunity', 'source_url', 'stage', 'note'].map(k => [k, String(data.get(k) || '').trim()]));
    const url = form.dataset.id ? `/api/admin/outreach/records/${encodeURIComponent(form.dataset.id)}` : `/api/admin/outreach/${encodeURIComponent(form.dataset.user)}`;
    await api(url, { method: form.dataset.id ? 'PATCH' : 'POST', body });
    await adminMandate(form.dataset.user);
    notify(t('进展已记录。', 'Progress recorded.'));
  }

  function matchCard(m, admin) {
    return `<article class="match-card"><div class="match-card-top"><span class="match-person">${icon('person')}${esc(m.seeker_name)}</span><time>${date(m.updated_at || m.created_at)}</time></div><h3>${esc(m.job_title)}</h3><p class="match-company">${esc(m.employer_name)}</p>${m.note ? `<p class="match-note">${esc(m.note)}</p>` : ''}<div class="match-card-actions">${admin ? `<button class="text-button" data-action="match-detail" data-id="${esc(m.id)}">${t('详情与跟进', 'Details & follow-up')} →</button>` : `<span class="status ${m.status === 'declined' ? 'closed' : ''}">${matchText(m.status)}</span>`}${admin || m.status !== 'declined' ? `<a href="${esc(chatUrl(admin, 'match_' + m.id))}" data-nav class="text-button">${icon('message')}${m.status === 'declined' ? t('历史会话', 'History') : t('匹配会话', 'Room')}</a>` : ''}</div>${admin ? `<div class="match-private-links"><a href="${esc(chatUrl(true, 'platform_' + m.seeker_id))}" data-nav>${t('求职者私聊', 'Seeker chat')} ↗</a><a href="${esc(chatUrl(true, 'platform_' + m.employer_id))}" data-nav>${t('招聘方私聊', 'Employer chat')} ↗</a></div>` : `<div class="match-private-links">${state.user?.role === 'seeker' ? `<a href="/jobs/${encodeURIComponent(m.job_id)}" data-nav>${t('查看岗位', 'View role')} →</a>` : `<a href="/people/${encodeURIComponent(m.seeker_id)}" data-nav>${t('查看简历', 'View résumé')} →</a>`}</div>`}</article>`;
  }

  async function matchesPage(admin = false) {
    const d = await api(admin ? '/api/admin/matches' : '/api/matches');
    const matches = d.items || [];
    matchCache.clear();
    matches.forEach(m => matchCache.set(m.id, m));
    const head = heading(admin ? t('把合适的两端，连接起来。', 'Connect the right two sides.') : t('认真了解，再相互认识。', 'A thoughtful introduction comes first.'), admin ? t('从介绍到结果，跟进每一次真实匹配。', 'Follow every real introduction through to an outcome.') : t('只有平台建立匹配后，双方才可以查看资料并进入独立会话。', 'Profiles and a shared room become available after HireFerry makes an introduction.'), admin ? `<button class="button" data-action="new-match">${icon('plus')}${t('建立匹配', 'Make an introduction')}</button>` : '', 'HUMAN INTRODUCTIONS');
    if (!admin) return shell(`${head}${matches.length ? `<div class="user-match-grid">${matches.map(m => matchCard(m, false)).join('')}</div>` : `<section class="panel">${empty(t('值得聊的人，我们认真找。', 'We’re looking for a conversation worth having.'), t('先和平台说清期待。有合适的匹配，会在这里为你介绍。', 'Tell the team what matters. Your introductions will appear here.'), `<a href="/app?tab=messages" data-nav class="button secondary">${t('与平台沟通', 'Talk to the team')}${icon('message')}</a>`, 'link')}</section>`}`, 'matches');
    return shell(`${head}<div class="match-board-summary"><span>${icon('board')}${matches.length} ${t('次真实匹配', 'real introductions')}</span><span>${icon('shield')}${t('私聊独立保密，匹配后再共享', 'Private chats stay separate; sharing starts with an introduction')}</span></div><section class="match-board" aria-label="${t('匹配进度看板', 'Introduction progress board')}">${statuses.map(status => { const items = matches.filter(m => m.status === status); return `<section class="match-column ${status}" aria-labelledby="match-column-${status}"><header><h2 id="match-column-${status}">${matchText(status)}</h2><span>${items.length}</span></header><div class="match-column-body">${items.length ? items.map(m => matchCard(m, true)).join('') : `<div class="match-column-empty">${t('暂无匹配', 'No introductions')}</div>`}</div></section>`; }).join('')}</section>`, 'matches', true);
  }

  async function matchDetail(id) {
    let m = matchCache.get(id);
    if (!m) m = (await api('/api/admin/matches')).items?.find(x => x.id === id);
    if (!m) throw new Error(t('未找到这次匹配。', 'Introduction not found.'));
    openDialog(`<div class="eyebrow">A CONNECTION, IN CONTEXT</div><h2 id="dialog-title">${t('一起，推进下一步。', 'Keep the next step moving.')}</h2><p class="dialog-intro">${t('双方资料在匹配后可见。平台私聊历史始终分开。', 'Profiles are shared after an introduction. Private conversation histories remain separate.')}</p><div class="match-map"><article><span class="match-map-icon">${icon('person')}</span><small>${t('求职者', 'Job seeker')}</small><h3>${esc(m.seeker_name)}</h3><button class="text-button" data-action="admin-edit-user" data-id="${esc(m.seeker_id)}">${t('查看资料', 'View profile')} →</button><a href="${esc(chatUrl(true, 'platform_' + m.seeker_id))}" data-nav class="button secondary small">${t('平台私聊', 'Private chat')}</a></article><div class="match-map-platform"><span class="match-map-icon">${icon('link')}</span><small>HIREFERRY</small><h3>${t('平台协调', 'Coordinated by us')}</h3><span class="status ${m.status === 'declined' ? 'closed' : ''}">${matchText(m.status)}</span><a href="${esc(chatUrl(true, 'match_' + m.id))}" data-nav class="button small">${m.status === 'declined' ? t('查看历史会话', 'View room history') : t('进入匹配会话', 'Open shared room')}</a></div><article><span class="match-map-icon">${icon('briefcase')}</span><small>${t('招聘方', 'Employer')}</small><h3>${esc(m.employer_name)}</h3><p>${esc(m.job_title)}</p><button class="text-button" data-action="admin-edit-job" data-id="${esc(m.job_id)}">${t('查看岗位', 'View role')} →</button><a href="${esc(chatUrl(true, 'platform_' + m.employer_id))}" data-nav class="button secondary small">${t('平台私聊', 'Private chat')}</a></article></div><form data-form="match" data-id="${esc(m.id)}"><div class="field"><label>${t('跟进进度', 'Progress')}<select class="input" name="status">${statuses.map(s => `<option value="${s}" ${s === m.status ? 'selected' : ''}>${matchText(s)}</option>`).join('')}</select></label></div><div class="field"><label>${t('跟进说明 · 匹配双方可见', 'Follow-up note · visible to both sides')}<textarea class="input" name="note" rows="4" maxlength="2000">${esc(m.note)}</textarea></label></div><div class="form-actions"><p class="help">${t('结束后，双方不再访问资料和会话。管理员保留只读历史。', 'Ending an introduction closes participant access. Administrators retain read-only history.')}</p><button class="button" type="submit">${t('保存跟进', 'Save follow-up')}${icon('check')}</button></div></form>`, { wide: true });
  }

  const seekerOptions = seekers => `<option value="">${t('选择求职者', 'Choose a job seeker')}</option>${seekers.filter(u => u.role === 'seeker' && !u.banned).map(u => `<option value="${esc(u.id)}">${esc(u.name)} · ${esc(u.public_email || u.email || '')}</option>`).join('')}`;
  const roleOptions = roles => `<option value="">${t('选择正在招聘的岗位', 'Choose an active role')}</option>${roles.filter(j => j.status === 'published').map(j => `<option value="${esc(j.id)}">${esc(j.title)} · ${money(j.salary_cents)}</option>`).join('')}`;

  async function newMatch() {
    const [people, jobs] = await Promise.all([api('/api/admin/users?role=seeker'), api('/api/admin/jobs?status=published')]);
    openDialog(`<div class="eyebrow">A THOUGHTFUL INTRODUCTION</div><h2 id="dialog-title">${t('让合适的双方，开始聊。', 'Make a thoughtful introduction.')}</h2><p class="dialog-intro">${t('先在各自的平台私聊中了解期待。匹配建立后，双方获得资料访问与一个新的共同会话。', 'Understand each side in private first. Creating an introduction shares their profiles and opens a fresh room.')}</p><form data-form="match"><div class="field"><label>${t('查找双方', 'Find both sides')}</label><div class="filter-row match-filters"><input class="input" id="match-people-search" aria-label="${t('求职者姓名', 'Job seeker name')}" placeholder="${esc(t('求职者姓名', 'Job seeker name'))}"><input class="input" id="match-job-search" aria-label="${t('岗位关键词', 'Role keyword')}" placeholder="${esc(t('岗位关键词', 'Role keyword'))}"><button class="button secondary small" type="button" data-action="match-search">${t('搜索', 'Search')}</button></div><p class="help">${t('每次显示最多 50 项；可用关键词缩小范围。', 'Up to 50 results at a time. Use keywords to narrow the list.')}</p></div><div class="field"><label>${t('求职者', 'Job seeker')}<select class="input" name="seeker_id" required>${seekerOptions(people.items || [])}</select></label></div><div class="field"><label>${t('岗位', 'Role')}<select class="input" name="job_id" required>${roleOptions(jobs.items || [])}</select></label></div><div class="field"><label>${t('介绍说明 · 双方可见', 'Introduction note · visible to both sides')}<textarea class="input" name="note" maxlength="2000" rows="4" placeholder="${esc(t('为什么这次匹配值得认真聊聊？', 'Why is this introduction worth a conversation?'))}"></textarea></label></div><div class="form-actions"><button class="button" type="submit">${t('建立匹配与会话', 'Create introduction and room')}${icon('arrow')}</button></div></form>`, { wide: true });
  }

  async function searchMatch() {
    const peopleQuery = document.querySelector('#match-people-search').value;
    const jobQuery = document.querySelector('#match-job-search').value;
    const [people, jobs] = await Promise.all([api('/api/admin/users?role=seeker&q=' + encodeURIComponent(peopleQuery)), api('/api/admin/jobs?status=published&q=' + encodeURIComponent(jobQuery))]);
    const form = document.querySelector('[data-form="match"]');
    if (!form) return;
    form.querySelector('[name="seeker_id"]').innerHTML = seekerOptions(people.items || []);
    form.querySelector('[name="job_id"]').innerHTML = roleOptions(jobs.items || []);
  }

  async function adminTokens() {
    const d = await api('/api/admin/tokens');
    return shell(`${heading(t('让平台 AI，协助每次连接。', 'Give your platform AI a clear role.'), t('独立的管理员授权，用于平台业务与跟进。', 'Separate administrator authorization for service operations and follow-ups.'), '', 'PLATFORM AI')}<section class="panel"><div class="panel-header"><div><h2>${t('平台业务访问令牌', 'Platform business access token')}</h2><p>${t('与求职者、招聘方的个人令牌分开管理。', 'Managed separately from job seekers’ and employers’ personal tokens.')}</p></div><span class="pill">${icon('shield')} ADMIN MCP</span></div>${notice(t('此令牌可管理用户资料、岗位、匹配、委托、寻找记录和平台消息，并查看订单统计。它代表平台发言，AI 发送的消息会明确标记。', 'This token can manage profiles, roles, introductions, search briefs, outreach records, and platform messages, and read order statistics. It speaks for HireFerry; AI messages are clearly labeled.'))}<div class="field"><label>${t('MCP 服务地址', 'MCP server URL')}</label><div class="code-box">${esc(location.origin)}/mcp</div></div><p class="help">${t('使用 HTTP 连接，并将令牌作为 Authorization: Bearer 请求头。系统配置与通行密钥管理不对 AI 开放。', 'Connect over HTTP with an Authorization: Bearer header. System configuration and passkey management are not available to AI.')}</p><form data-form="admin-token" class="token-create-form"><div class="field"><label for="admin-token-name">${t('连接名称', 'Connection name')}</label><input class="input" id="admin-token-name" name="name" maxlength="80" required placeholder="${esc(t('例如：平台招聘助手', 'For example: Platform hiring assistant'))}"></div><button class="button secondary" type="submit">${icon('plus')}${t('创建平台令牌', 'Create platform token')}</button></form><div class="token-reauth"><p class="help">${t('创建令牌需要在最近 10 分钟内完成通行密钥验证。', 'Creating a token requires a passkey check within the last 10 minutes.')}</p><button class="text-button" data-action="admin-reauth">${icon('key')}${t('重新验证通行密钥', 'Verify passkey again')}</button></div><div id="new-token"></div></section><section class="panel"><div class="panel-header"><h2>${t('已授权的平台 AI', 'Authorized platform AI')}</h2></div>${d.items?.length ? `<div class="list">${d.items.map(k => `<div class="list-row"><div><h3>${esc(k.name)}</h3><div class="list-meta"><span>${t('创建于', 'Created')} ${date(k.created_at)}</span><span>${t('到期', 'Expires')} ${date(k.expires_at)}</span><span>${t('最近使用', 'Last used')} ${date(k.last_used_at)}</span></div></div><button class="text-button" data-action="revoke-admin-token" data-id="${esc(k.id)}">${t('立即撤销', 'Revoke now')}</button></div>`).join('')}</div>` : empty(t('尚未授权平台 AI。', 'No platform AI is authorized yet.'), t('为可信的助手单独创建令牌，并妥善保管。', 'Create a separate token for a trusted assistant and keep it private.'), '', 'bot')}</section>`, 'agents', true);
  }

  return { messagesPage, mountChat, stopChat, sendMessage, olderMessages, scrollLatest, mandatePage, saveMandate, adminMandates, adminMandate, outreachDialog, saveOutreach, matchesPage, matchDetail, newMatch, searchMatch, adminTokens };
}

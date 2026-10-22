/* HireFerry service workspace: private conversations, mandates, and human matching. */
export function createMessageId(cryptoApi = globalThis.crypto) {
  const bytes = new Uint8Array(16);
  cryptoApi.getRandomValues(bytes);
  return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('');
}

export function filterConversations(items, search = '', filter = 'all') {
  const term = search.trim().toLocaleLowerCase();
  return items.filter(item => (filter !== 'unread' || Number(item.unread_count) > 0) &&
    (!term || [item.title, item.last_message_preview, ...(item.participants || []).map(person => person.name)]
      .some(value => String(value || '').toLocaleLowerCase().includes(term))));
}

export function chatSendShortcut(event) {
  return event.key === 'Enter' && !!(event.ctrlKey || event.metaKey) && !event.shiftKey && !event.isComposing && event.keyCode !== 229;
}

export function messageDateKey(value) {
  const date = new Date(Number(value) < 1e12 ? Number(value) * 1000 : value);
  return Number.isNaN(date.getTime()) ? '' : `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export function createChatDateFormatter(locale, now = Date.now) {
  const formats = new Map();
  const options = {
    detail: { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' },
    clock: { hour: '2-digit', minute: '2-digit' },
    shortDate: { month: 'short', day: 'numeric' },
    day: { year: 'numeric', month: 'long', day: 'numeric' },
  };
  let timeZone;
  // Resolve the system zone once per update, not once for every message/row.
  // This also keeps a long-lived tab correct after a system time-zone change.
  const sync = () => {
    const next = new Intl.DateTimeFormat().resolvedOptions().timeZone;
    if (next !== timeZone) { timeZone = next; formats.clear(); }
    return timeZone;
  };
  const format = (value, kind) => {
    const date = new Date(Number(value) < 1e12 ? Number(value) * 1000 : value);
    if (Number.isNaN(date.getTime())) return 'Invalid Date';
    if (timeZone === undefined) sync();
    const language = locale(), key = `${language}:${kind}`;
    if (!formats.has(key)) formats.set(key, new Intl.DateTimeFormat(language, { ...options[kind], timeZone }));
    return formats.get(key).format(date);
  };
  return {
    sync,
    detail: value => value ? format(value, 'detail') : '',
    compact: value => value ? format(value, messageDateKey(value) === messageDateKey(now()) ? 'clock' : 'shortDate') : '',
    day: value => format(value, 'day'),
  };
}

export function createServiceUI({ state, api, t, esc, icon, avatar, money, currency, currencies, currencySelect, amountValue, amountStep, date, heading, shell, empty, notice, badge, matchText, openDialog, closeDialog, notify, go, render, centsInput }) {
  let polling = null;
  let generation = 0;
  let workbenchGeneration = 0;
  let candidateBatch = null;
  let bulkReview = null;
  let bulkReviewSequence = 0;
  let matchDetailEpoch = 0;
  let activeMatchDetail = null;
  let chat = null;
  let chatCleanup = null;
  let chatOwner = '';
  let conversationFilter = 'all';
  let conversationSearch = '';
  const chatDrafts = new Map();
  const outreachCache = new Map();
  const mandateCache = new Map();
  const pendingMessages = new Map();
  const webhookCache = new Map();
  let webhookSettings = { enabled: false, max_endpoints: 5, max_attempts: 5 };
  const statuses = ['introduced', 'interview', 'hired', 'declined'];
  const outreachStages = ['identified', 'contacted', 'replied', 'interview', 'offer', 'closed'];
  const query = () => new URLSearchParams(location.search);
  const route = (admin, tab, extra = {}) => (admin ? '/admin' : '/app') + '?' + new URLSearchParams({ tab, ...extra });
  const chatUrl = (admin, id) => route(admin, 'messages', { conversation: id });
  const chatDates = createChatDateFormatter(() => state.lang === 'en' ? 'en-GB' : 'zh-CN');
  const humanTime = value => chatDates.detail(value);
  const mandateStatus = value => ({ active: t('寻找中', 'Active'), paused: t('已暂停', 'Paused'), closed: t('已结束', 'Closed') })[value] || value;
  const outreachText = value => ({ identified: t('发现机会', 'Identified'), contacted: t('已联系', 'Contacted'), replied: t('有回复', 'Replied'), interview: t('面试中', 'Interview'), offer: t('收到意向', 'Offer'), closed: t('已结束', 'Closed') })[value] || value;
  const safeLink = value => { try { const u = new URL(value); return ['https:', 'http:'].includes(u.protocol) ? u.href : ''; } catch { return ''; } };
  const salaryRange = m => m.salary_min_cents != null && m.salary_max_cents != null ? `${money(m.salary_min_cents,m)} – ${money(m.salary_max_cents,m)}` : m.salary_min_cents != null ? `${money(m.salary_min_cents,m)} +` : m.salary_max_cents != null ? `${t('最高', 'Up to')} ${money(m.salary_max_cents,m)}` : t('可以沟通', 'Open to discussion');

  const ownerKey = () => `${state.admin ? 'admin' : 'member'}:${state.super_admin ? 'owner' : ''}:${state.staff?.id || ''}:${state.staffRole || ''}:${state.user?.id || ''}:${state.csrf || ''}`;
  function syncChatOwner() {
    const owner = ownerKey();
    if (chatOwner !== owner || (!state.user && !state.admin)) {
      chatDrafts.clear(); pendingMessages.clear();
      outreachCache.clear(); mandateCache.clear(); webhookCache.clear();
      workbenchPages.clear(); workbenchHistory.clear();
      conversationFilter = 'all'; conversationSearch = ''; chatOwner = owner;
    }
    return owner;
  }

  function saveChatDraft(current = chat) {
    if (!current?.id || current.owner !== ownerKey() || current.denied || current.loading) return;
    const textarea = globalThis.document?.querySelector('#chat-body');
    const scroll = globalThis.document?.querySelector('#chat-scroll');
    const previous = chatDrafts.get(current.id) || {};
    chatDrafts.set(current.id, { body: textarea?.value ?? previous.body ?? '', top: scroll?.scrollTop ?? previous.top, atBottom: scroll ? scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 80 : previous.atBottom });
    if (chatDrafts.size > 50) chatDrafts.delete(chatDrafts.keys().next().value);
  }

  function stopChat() {
    syncChatOwner();
    saveChatDraft();
    chat?.controller?.abort();
    chatCleanup?.(); chatCleanup = null;
    generation += 1;
    workbenchGeneration += 1;
    candidateBatch = null;
    bulkReview = null;
    matchDetailEpoch += 1;
    activeMatchDetail = null;
    clearTimeout(polling); polling = null; chat = null;
  }

  function conversationLabel(c) {
    if (c.kind === 'platform') {
      if (!state.admin) return t('聘渡团队', 'HireFerry team');
      const person = c.participants?.find(p => p.id === c.user_id) || c.participants?.[0];
      return person?.name || c.title || t('平台私聊', 'Private conversation');
    }
    return c.title || t('匹配会话', 'Introduction room');
  }

  function conversationAvatar(c, admin = false) {
    if (c.kind === 'platform' && !admin) return `<span class="conversation-avatar team-avatar" aria-hidden="true">${icon('leaf')}</span>`;
    const person = c.participants?.find(p => p.id !== state.user?.id && p.id !== 'platform') || c.participants?.[0];
    return `<span class="conversation-avatar ${c.kind === 'matched' ? 'room-avatar' : ''}" aria-hidden="true">${esc((person?.name || conversationLabel(c)).slice(0, 1).toUpperCase())}</span>`;
  }

  function compactTime(value) {
    return chatDates.compact(value);
  }

  function conversationView(items, selected, admin, cache = new Map()) {
    const timeZone = chatDates.sync();
    const today = messageDateKey(Date.now());
    const filtered = filterConversations(items, conversationSearch, conversationFilter);
    const groups = [], rows = new Map();
    for (const [kind, label] of [['platform', t('平台沟通', 'WITH HIREFERRY')], ['matched', t('匹配会话', 'INTRODUCTIONS')]]) {
      const members = filtered.filter(c => c.kind === kind);
      if (!members.length) continue;
      const groupRows = members.map(c => {
        const signature = JSON.stringify([state.lang, timeZone, today, admin, c.id === selected, c.kind, c.title, c.user_id, c.participants, c.last_message_at, c.last_message_preview, c.unread_count]);
        let row = cache.get(c.id);
        if (row?.signature !== signature) {
          const html = `<a href="${esc(chatUrl(admin, c.id))}" data-nav data-conversation-id="${esc(c.id)}" class="conversation-item ${c.id === selected ? 'active' : ''} ${c.unread_count ? 'has-unread' : ''}"${c.id === selected ? ' aria-current="page"' : ''}>${conversationAvatar(c, admin)}<span class="conversation-copy"><span class="conversation-row-top"><strong>${esc(conversationLabel(c))}</strong><time title="${esc(humanTime(c.last_message_at))}">${esc(compactTime(c.last_message_at))}</time></span><span class="conversation-row-bottom"><span class="conversation-preview">${esc(c.last_message_preview || t('还没有消息', 'No messages yet'))}</span>${c.unread_count ? `<span class="unread-badge" aria-label="${Number(c.unread_count)} ${t('条未读消息', 'unread messages')}">${c.unread_count > 99 ? '99+' : Number(c.unread_count)}</span>` : ''}</span></span></a>`;
          row = { id: c.id, signature, html }; cache.set(c.id, row);
        }
        rows.set(c.id, row); return row;
      });
      groups.push({ kind, label, rows: groupRows });
    }
    const known = new Set(items.map(item => item.id));
    for (const id of cache.keys()) if (!known.has(id)) cache.delete(id);
    const html = groups.map(group => `<section class="conversation-group" data-conversation-group="${group.kind}"><h3>${group.label}<span>${group.rows.length}</span></h3>${group.rows.map(row => row.html).join('')}</section>`).join('') || `<div class="conversation-empty">${icon(conversationFilter === 'unread' ? 'check' : 'search')}<strong>${conversationFilter === 'unread' && !conversationSearch ? t('没有未读消息', 'You’re all caught up') : t('没有找到会话', 'No conversations found')}</strong><span>${conversationSearch ? t('试试其他名称或消息关键词。', 'Try another name or message keyword.') : t('收到的新消息会显示在这里。', 'New conversations will appear here.')}</span></div>`;
    return { html, groups, rows };
  }

  function patchConversationList(list, before, next) {
    if (!next.groups.length || !list.querySelectorAll || !document.createElement) { list.innerHTML = next.html; return; }
    const oldGroups = new Map([...list.querySelectorAll('[data-conversation-group]')].map(node => [node.dataset.conversationGroup, node]));
    const oldRows = new Map([...list.querySelectorAll('[data-conversation-id]')].map(node => [node.dataset.conversationId, node]));
    const usedGroups = new Set(), usedRows = new Set();
    const scrollTop = list.scrollTop;
    const template = document.createElement('template');
    let groupPosition = list.firstChild;
    for (const group of next.groups) {
      let section = oldGroups.get(group.kind);
      if (!section) {
        section = document.createElement('section'); section.className = 'conversation-group'; section.dataset.conversationGroup = group.kind;
        section.innerHTML = `<h3>${group.label}<span>${group.rows.length}</span></h3>`;
      }
      usedGroups.add(section);
      if (section !== groupPosition) list.insertBefore(section, groupPosition);
      groupPosition = section.nextSibling;
      const heading = section.firstElementChild;
      if (heading.firstChild.nodeValue !== group.label) heading.firstChild.nodeValue = group.label;
      if (heading.lastElementChild.textContent !== String(group.rows.length)) heading.lastElementChild.textContent = String(group.rows.length);
      let rowPosition = heading.nextSibling;
      for (const row of group.rows) {
        let element = oldRows.get(row.id);
        if (!element || before?.rows.get(row.id)?.html !== row.html) {
          template.innerHTML = row.html;
          const replacement = template.content.firstElementChild;
          if (!element) element = replacement;
          else {
            for (const attribute of [...element.attributes]) if (!replacement.hasAttribute(attribute.name)) element.removeAttribute(attribute.name);
            for (const attribute of [...replacement.attributes]) if (element.getAttribute(attribute.name) !== attribute.value) element.setAttribute(attribute.name, attribute.value);
            // The link itself retains focus; only this changed row's non-interactive children change.
            element.replaceChildren(...replacement.childNodes);
          }
        }
        usedRows.add(element);
        if (element !== rowPosition) section.insertBefore(element, rowPosition);
        rowPosition = element.nextSibling;
      }
    }
    for (const element of oldRows.values()) if (!usedRows.has(element)) element.remove();
    for (const element of [...list.children]) if (!usedGroups.has(element)) element.remove();
    list.scrollTop = scrollTop;
  }

  function messageHtml(m, admin) {
    const own = admin ? (typeof m.is_own === 'boolean' ? m.is_own : m.sender_kind === 'platform') : m.sender_id === state.user?.id;
    const staffLabels = { '超级管理员': t('超级管理员', 'Super administrator'), '平台AI': t('平台 AI', 'Platform AI'), '员工': t('员工', 'Staff') };
    const staffName = admin && m.staff_name ? (Object.hasOwn(staffLabels, m.staff_name) ? staffLabels[m.staff_name] : m.staff_name) : t('聘渡团队', 'HireFerry team');
    const author = m.sender_kind === 'platform' ? staffName : m.sender_name || t('参与者', 'Participant');
    return `<article class="chat-message ${own ? 'own' : ''} ${m.sender_kind === 'platform' ? 'from-platform' : ''}" data-message-id="${Number(m.id)}"><div class="message-meta"><strong>${esc(author)}</strong>${m.via_ai ? `<span class="ai-label">${icon('bot')} ${t('AI 代发', 'via AI')}</span>` : ''}</div><div class="message-body markdown">${m.body_html || esc(m.body).replace(/\n/g, '<br>')}</div><time class="message-time" title="${esc(humanTime(m.created_at))}">${esc(compactTime(m.created_at))}</time></article>`;
  }

  function messagesHtml(items, admin, previousDate = '') {
    chatDates.sync();
    let day = previousDate;
    return (items || []).map(message => {
      const key = messageDateKey(message.created_at);
      const separator = key && key !== day ? `<div class="chat-date" data-date="${esc(key)}"><span>${key === messageDateKey(Date.now()) ? t('今天', 'Today') : esc(chatDates.day(message.created_at))}</span></div>` : '';
      day = key;
      return separator + messageHtml(message, admin);
    }).join('');
  }

  function chatPrompts(current, admin) {
    if (admin) return [[t('确认需求', 'Clarify the brief'), t('你好，我想进一步了解你的需求。你目前最看重哪些条件？', 'Hello! I’d like to understand your brief. What matters most to you?'), 'target'], [t('跟进进展', 'Share an update'), t('你好，关于这次寻找，我想和你同步一下进展：', 'Hello! Here is an update on your search: '), 'message']];
    if (current.kind === 'matched') return [[t('打个招呼', 'Say hello'), t('你好，很高兴认识你！想和你进一步聊聊这个岗位。', 'Hello, nice to meet you! I’d love to discuss this role.'), 'message'], [t('了解岗位', 'Ask about the role'), t('想请教一下，这个岗位日常主要负责哪些工作？', 'Could you tell me more about the day-to-day work for this role?'), 'briefcase']];
    return state.user?.role === 'employer' ? [[t('说明招聘需求', 'Describe a role'), t('你好，我们正在招聘。想和你聊聊岗位要求、团队情况和招聘计划。', 'Hello! We’re hiring and would like to discuss the role, team, and hiring plans.'), 'briefcase'], [t('询问招聘进展', 'Check progress'), t('你好，想了解一下目前的招聘进展。', 'Hello! Could you share an update on our hiring search?'), 'target']] : [[t('聊聊求职方向', 'Talk about my search'), t('你好，我想找一份新工作。希望和你聊聊我的经历以及下一份工作的期待。', 'Hello! I’m looking for a new role and would like to discuss my experience and what I’m looking for next.'), 'target'], [t('请帮我看看简历', 'Discuss my résumé'), t('你好，想请你帮我看看简历，并聊聊哪些方向比较适合。', 'Hello! I’d appreciate your thoughts on my résumé and which directions might be a good fit.'), 'file']];
  }

  function chatWelcome(current, admin) {
    const introduced = current.kind === 'matched';
    return `<div id="chat-empty" class="chat-welcome"><span class="chat-welcome-mark">${icon(introduced ? 'link' : 'message')}</span><div class="chat-welcome-kicker">${introduced ? t('新的匹配', 'A NEW INTRODUCTION') : t('你的专属沟通空间', 'YOUR CONVERSATION SPACE')}</div><h3>${introduced ? t('从这里，认识彼此。', 'Meet your next connection.') : t('下一步，聊出来。', 'Let’s find your next step.')}</h3><p>${introduced ? t('围绕这个岗位聊聊。你与平台的私聊不会出现在这里。', 'Talk about this role. Your private conversations stay separate.') : t('工作期待、招聘需求，或者还没想清楚的问题。都可以从这里开始。', 'Your next role, a hiring brief, or a question you’re still figuring out. Start here.')}</p>${current.writable === false ? '' : `<div class="chat-prompts">${chatPrompts(current, admin).map(([label, body, glyph]) => `<button type="button" data-chat-prompt="${esc(body)}">${icon(glyph)}<span>${label}</span>${icon('arrow')}</button>`).join('')}</div><span class="chat-prompt-note">${t('选一句开始，编辑后再发送', 'Choose a starting point, then make it your own')}</span>`}</div>`;
  }

  function chatContextLink(current, admin) {
    if (current.kind === 'matched' && current.job_id && !admin && state.user?.role === 'seeker') return `<a href="/jobs/${encodeURIComponent(current.job_id)}" data-nav class="chat-context-link">${icon('briefcase')}${t('岗位详情', 'View role')}${icon('arrow')}</a>`;
    if (current.kind === 'matched' && current.match_id && admin) return `<button type="button" class="chat-context-link" data-action="match-detail" data-id="${esc(current.match_id)}">${icon('link')}${t('匹配详情', 'Introduction')}${icon('arrow')}</button>`;
    if (current.kind === 'platform' && !admin) return `<a href="${state.user?.role === 'employer' ? '/app?tab=jobs' : '/app?tab=mandate'}" data-nav class="chat-context-link">${icon(state.user?.role === 'employer' ? 'briefcase' : 'target')}${state.user?.role === 'employer' ? t('我的招聘', 'My roles') : t('寻找委托', 'My search')}${icon('arrow')}</a>`;
    return '';
  }

  const chatPlaceholder = () => `<div class="chat-message-placeholder" role="status"><span class="sr-only">${t('正在加载消息', 'Loading messages')}</span><div aria-hidden="true"><i></i><i></i><i></i></div></div>`;

  function showPendingChat(requested) {
    const thread = document.querySelector('.chat-thread');
    const selected = requested || (!state.admin && state.user ? 'platform_' + state.user.id : '');
    const workspace = document.querySelector('.chat-workspace');
    workspace?.classList.toggle('thread-selected', !!requested && query().get('view') !== 'list');
    if (!thread || !selected || thread.getAttribute('data-page-key') === `conversation:${selected}`) return;
    thread.setAttribute('data-page-key', `conversation:${selected}`);
    thread.innerHTML = `<div class="chat-pending-header">${inboxToggle(true)}<span aria-hidden="true"></span><i aria-hidden="true"></i></div><div class="chat-scroll">${chatPlaceholder()}</div><div class="chat-pending-composer" aria-hidden="true"></div>`;
    document.querySelectorAll('.conversation-item').forEach(link => {
      const active = new URL(link.href, location.origin).searchParams.get('conversation') === selected;
      link.classList.toggle('active', active);
      if (active) link.setAttribute('aria-current', 'page'); else link.removeAttribute('aria-current');
    });
  }

  function inboxToggle(restore = false) {
    const collapse = t('收起会话列表', 'Collapse conversations');
    const expand = t('展开会话列表', 'Expand conversations');
    return `<button type="button" class="icon-button workspace-panel-toggle${restore ? ' inbox-restore' : ''}" data-workspace-panel="inbox" ${restore ? 'data-panel-restore' : ''} aria-controls="conversation-sidebar" aria-expanded="${!restore}" aria-label="${restore ? expand : collapse}" title="${restore ? expand : collapse}" data-label-collapse="${collapse}" data-label-expand="${expand}">${icon(restore ? 'message' : 'panel')}${restore ? `<span>${t('会话', 'Conversations')}</span>` : ''}</button>`;
  }

  function chatThread(current) {
    const room = current.current;
    if (!room) return `<div class="chat-unselected">${inboxToggle(true)}${icon('message')}<h2>${t('选择一个会话', 'Choose a conversation')}</h2><p>${t('在左侧继续沟通，消息和资料各自保密。', 'Pick a conversation to continue. Each room keeps its own history.')}</p></div>`;
    const privacy = room.kind === 'matched' ? t('仅匹配双方与平台可见。平台私聊记录不会转入这里。', 'Only the matched parties and HireFerry can see this room. Private platform history stays separate.') : current.admin ? t('仅这位用户与平台可见，不向匹配方共享。', 'Only this user and HireFerry can see this conversation. It is never shared with a match.') : t('仅你与聘渡团队可见', 'Only you and HireFerry can see this conversation');
    const readOnly = room.writable === false;
    const disabled = readOnly || current.loading;
    const draft = current.loading ? '' : chatDrafts.get(current.id)?.body || '';
    return `<header class="chat-thread-header">${inboxToggle(true)}<button type="button" class="icon-button chat-back" data-chat-back aria-label="${t('返回会话列表', 'Back to conversations')}">${icon('back')}</button>${conversationAvatar(room, current.admin)}<div class="chat-thread-title"><h2>${esc(conversationLabel(room))}</h2><p>${readOnly ? t('历史会话 · 只读', 'Archived conversation · read-only') : room.kind === 'matched' ? t('匹配双方与聘渡团队', 'An introduction with HireFerry') : t('一对一沟通 · 持续跟进', 'Private conversation · Ongoing support')}</p></div>${chatContextLink(room, current.admin)}</header><div class="chat-privacy">${icon('shield')}<span>${privacy}</span></div><div class="chat-scroll" id="chat-scroll" tabindex="0" aria-label="${t('聊天记录', 'Conversation history')}" aria-busy="${current.loading}"><button type="button" class="chat-history text-button" data-action="chat-older" ${current.hasOlder ? '' : 'hidden'}>${t('加载更早的消息', 'Load earlier messages')}</button><div id="chat-messages" role="log" aria-live="polite" aria-relevant="additions" aria-label="${t('消息', 'Messages')}">${current.loading ? chatPlaceholder() : current.messages?.length ? messagesHtml(current.messages, current.admin) : chatWelcome(room, current.admin)}</div></div><button type="button" data-action="chat-latest" class="chat-new-message" hidden>${t('有新消息 · 回到最新', 'New messages · Jump to latest')} ↓</button><form data-form="chat" class="chat-composer"><div class="chat-composer-surface"><label class="sr-only" for="chat-body">${t('写一条消息', 'Write a message')}</label><textarea id="chat-body" name="body" rows="2" maxlength="10000" required ${disabled ? 'disabled' : ''} placeholder="${esc(readOnly ? t('此会话已归档', 'This conversation is archived') : t('写下你的想法…', 'Write a message…'))}">${esc(draft)}</textarea><div class="chat-composer-footer"><span class="chat-keyboard-hint">${t('换行 Enter · 发送 Ctrl / ⌘ Enter', 'Enter for a new line · Ctrl / ⌘ Enter to send')}</span><button class="button small" type="submit" ${disabled ? 'disabled' : ''}>${t('发送', 'Send')}${icon('send')}</button></div></div><div class="chat-composer-status"><span id="chat-connection" class="help" role="status">${readOnly ? t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.') : current.loading ? t('正在加载消息…', 'Loading messages…') : t('消息会自动更新', 'Messages update automatically')}</span><span>${t('仅本会话可见', 'Private to this conversation')}</span></div></form>`;
  }

  async function messagesPage(admin = false) {
    const requestGeneration = generation;
    const owner = syncChatOwner();
    const requested = query().get('conversation');
    if (!admin && state.user?.role === 'unassigned') return shell(heading(t('先选择你的身份', 'Choose your role'), t('选择后即可与平台沟通。', 'Then start a conversation with the team.')), 'messages');
    let selected = requested || (!admin ? 'platform_' + state.user.id : null);
    let prepared;
    // Both GET endpoints authorize access and lazily ensure the platform room.
    // Loading the index does not need a separate creation POST or block the thread.
    const loadMessages = id => api(`/api/chat/conversations/${encodeURIComponent(id)}/messages?limit=100`).then(result => ({ result }), error => ({ error }));
    let pending = selected ? loadMessages(selected).then(result => { prepared = result; return result; }) : null;
    const index = await api('/api/chat/conversations');
    if (requestGeneration !== generation || owner !== ownerKey()) return '';
    const items = [...(index.items || [])];
    selected ||= items[0]?.id;
    let room = items.find(c => c.id === selected);
    if (selected && !room) {
      try { room = await api(`/api/chat/conversations/${encodeURIComponent(selected)}`); }
      catch (error) {
        if ([401, 403, 404].includes(error.status)) { chatDrafts.delete(selected); pendingMessages.delete(selected); }
        throw error;
      }
      if (!room || room.id !== selected) throw new Error(t('这个会话当前不可访问。', 'This conversation is not available.'));
      items.unshift(room);
    }
    if (requestGeneration !== generation || owner !== ownerKey()) return '';
    if (selected && !pending) pending = loadMessages(selected).then(result => { prepared = result; return result; });
    // Consume an already completed request without waiting for a slower history response.
    await Promise.resolve();
    if (requestGeneration !== generation || owner !== ownerKey()) return '';
    const result = prepared?.result;
    const messages = result?.items || [];
    chat = { id: selected, admin, items, current: room, owner, cursor: result?.next_cursor || 0, first: result?.first_cursor || messages[0]?.id || 0, hasOlder: !!result?.has_older, seen: new Set(messages.map(message => message.id)), messages, lastDate: messageDateKey(messages.at(-1)?.created_at), historyReady: !!result, loading: !!pending && !result, pending, error: prepared?.error, page: generation, hasMore: !!result?.has_more, language: state.lang, location: (admin ? '/admin' : '/app') + location.search, rowCache: new Map(), controller: new AbortController() };
    chat.listSnapshot = conversationView(items, selected, admin, chat.rowCache);
    const unread = items.reduce((count, item) => count + Number(item.unread_count || 0), 0);
    const activation=(!admin&&state.user?.role==='employer'&&!state.user.employer_paid)?`<div class="promo-activation-banner"><span>${t('招聘账号尚未激活','Your employer account is not active')}</span><button type="button" class="text-button" data-action="role-employer">${t('开通账户','Activate account')}</button><a class="text-button" href="/app?tab=promos" data-nav>${t('使用优惠码 / 申请优惠','Use a code / Apply for a promotion')}</a></div>`:'';
    return shell(`${activation}<section class="chat-workspace ${requested && query().get('view') !== 'list' ? 'thread-selected' : ''}" aria-label="${t('消息工作台', 'Conversation workspace')}"><aside id="conversation-sidebar" class="conversation-sidebar"><div class="conversation-sidebar-heading"><div><span class="inbox-eyebrow">${t('保持联系', 'STAY IN TOUCH')}</span><h1>${t('消息', 'Inbox')}<span class="inbox-count">${items.length}</span></h1></div>${inboxToggle()}</div><div class="conversation-tools"><label class="conversation-search">${icon('search')}<input id="conversation-search" type="search" value="${esc(conversationSearch)}" placeholder="${esc(t(admin ? '搜索近期会话' : '搜索会话', admin ? 'Search recent conversations' : 'Search conversations'))}" aria-label="${t('按名称或消息搜索会话', 'Search conversations by name or message')}" autocomplete="off"></label><div class="conversation-filters" aria-label="${t('会话筛选', 'Conversation filters')}"><button type="button" data-chat-filter="all" aria-pressed="${conversationFilter === 'all'}">${t('全部', 'All')}</button><button type="button" data-chat-filter="unread" aria-pressed="${conversationFilter === 'unread'}">${t('未读', 'Unread')}<span data-chat-unread-count ${unread ? '' : 'hidden'}>${unread > 99 ? '99+' : unread}</span></button></div></div><div id="conversation-list">${chat.listSnapshot.html}</div><div class="conversation-sidebar-note">${icon('shield')}<span>${admin ? t('显示近期会话，按关键词筛选', 'Search within recent conversations') : t('每一段对话，都单独保密', 'Every conversation stays separate')}</span></div></aside><section class="chat-thread" data-page-key="conversation:${esc(selected || '')}" aria-label="${t('消息内容', 'Messages')}">${chatThread(chat)}</section></section>`, 'messages', admin);
  }

  function navigateChat(next, { onlyIfChanged = false } = {}) {
    const previous = chat;
    const workspace = document.querySelector('.chat-workspace');
    const thread = document.querySelector('.chat-thread');
    const pathname = previous?.admin ? '/admin' : '/app';
    if (!previous?.mounted || !previous.historyReady || previous.loading || previous.error || previous.denied ||
        previous.owner !== ownerKey() || previous.language !== state.lang || state.bootstrap ||
        !workspace || !thread || next.pathname !== pathname || next.hash ||
        (previous.admin && next.searchParams.get('tab') !== 'messages') ||
        (next.searchParams.get('tab') && next.searchParams.get('tab') !== 'messages') ||
        [...next.searchParams.keys()].some(key => !['tab', 'conversation', 'view'].includes(key))) return null;
    const target = next.pathname + next.search;
    if (onlyIfChanged && previous.location === target) return null;
    const listOnly = next.searchParams.get('view') === 'list';
    const requested = next.searchParams.get('conversation');
    const selected = requested || (listOnly ? previous.id : previous.admin ? previous.id : 'platform_' + state.user.id);
    const room = previous.items.find(item => item.id === selected);
    if (!room) return null;
    workspace.classList.toggle('thread-selected', !!requested && !listOnly);
    if (selected === previous.id) {
      previous.location = target;
      // This is only a view change. Keep the message nodes, draft, scroll and poll controller.
      return Promise.resolve();
    }
    stopChat();
    const controller = new AbortController();
    const current = chat = {
      id: selected, admin: previous.admin, items: previous.items, current: room, owner: previous.owner,
      cursor: 0, first: 0, hasOlder: false, hasMore: false, seen: new Set(), messages: [], lastDate: '',
      historyReady: false, loading: true, language: state.lang, location: target, page: generation, controller,
      rowCache: previous.rowCache, listSnapshot: previous.listSnapshot, listElement: previous.listElement, listHtml: previous.listHtml,
    };
    // Previously displayed metadata may supply the heading. History always needs a new authorized read.
    current.pending = api(`/api/chat/conversations/${encodeURIComponent(selected)}/messages?limit=100`, { fresh: true, signal: controller.signal, group: `chat:${generation}` }).then(result => ({ result }), error => ({ error }));
    thread.setAttribute('data-page-key', `conversation:${selected}`);
    thread.innerHTML = chatThread(current);
    refreshConversationList(current);
    const ready = mountChat();
    // A slow index must not hold up the selected room. The current room is checked again if omitted.
    void refreshChatIndex(current).catch(error => {
      if (chat === current && current.owner === ownerKey() && !current.denied && [401, 403, 404].includes(error.status)) setChatAccess(current, error);
    });
    return ready;
  }

  async function refreshChatIndex(current, result) {
    const valid = () => chat === current && current.page === generation && current.owner === ownerKey() && !current.denied;
    const index = result || await api('/api/chat/conversations', { fresh: true, signal: current.controller.signal, group: `chat:${current.page}` });
    if (!valid()) return;
    const items = [...(index.items || [])];
    let selected = items.find(item => item.id === current.id);
    if (!selected) {
      selected = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}`, { fresh: true, signal: current.controller.signal, group: `chat:${current.page}` });
      if (!selected || selected.id !== current.id) throw Object.assign(new Error(t('这个会话当前不可访问。', 'This conversation is not available.')), { status: 404 });
      items.unshift(selected);
    }
    if (!valid()) return;
    if (current.historyReady && chatIsVisible()) selected.unread_count = 0;
    current.current = selected; current.items = items;
    refreshConversationList(current);
    document.querySelectorAll('.chat-composer textarea,.chat-composer button').forEach(element => { element.disabled = current.loading || selected.writable === false || element.getAttribute('aria-busy') === 'true'; });
    const status = document.querySelector('#chat-connection');
    if (status && selected.writable === false) status.textContent = t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.');
  }

  function scrollLatest() {
    const scroll = document.querySelector('#chat-scroll');
    if (scroll) scroll.scrollTop = scroll.scrollHeight;
    const button = document.querySelector('[data-action="chat-latest"]');
    if (button) button.hidden = true;
  }

  async function markRead(current) {
    if (!current?.id || chat !== current || current.page !== generation || current.owner !== ownerKey() || current.denied || !current.cursor || current.readCursor >= current.cursor || !chatIsVisible()) return;
    const cursor = current.cursor;
    await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/read`, { method: 'POST', body: { message_id: cursor } });
    current.readCursor = cursor;
    if (chat === current && current.page === generation && current.owner === ownerKey() && !current.denied && chatIsVisible()) {
      const selected = current.items.find(item => item.id === current.id);
      if (selected?.unread_count) { selected.unread_count = 0; refreshConversationList(current); }
    }
  }

  function chatIsVisible() {
    return !document.hidden && (!globalThis.matchMedia?.('(max-width: 760px)').matches || !!document.querySelector('.chat-workspace')?.classList.contains('thread-selected'));
  }

  function appendMessages(current, messages, forceScroll = false) {
    if (chat !== current || current.page !== generation || current.owner !== ownerKey() || current.denied) return;
    const container = document.querySelector('#chat-messages');
    const scroll = document.querySelector('#chat-scroll');
    if (!container || !scroll) return;
    const nearBottom = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 150;
    const fresh = (messages || []).filter(m => !current.seen.has(m.id));
    if (!fresh.length) return;
    document.querySelector('#chat-empty')?.remove();
    fresh.forEach(m => current.seen.add(m.id));
    const reordered = (current.messages?.at(-1)?.id || 0) > fresh[0].id;
    current.messages = [...(current.messages || []), ...fresh].sort((a, b) => a.id - b.id);
    if (reordered) container.innerHTML = messagesHtml(current.messages, current.admin);
    else container.insertAdjacentHTML('beforeend', messagesHtml(fresh, current.admin, current.lastDate));
    current.lastDate = messageDateKey(current.messages.at(-1).created_at);
    current.first ||= fresh[0].id;
    if (nearBottom || forceScroll) scrollLatest();
    else { const button = document.querySelector('[data-action="chat-latest"]'); if (button) button.hidden = false; }
  }

  function refreshConversationList(current) {
    const list = document.querySelector('#conversation-list');
    if (!list) return;
    const next = conversationView(current.items, current.id, current.admin, current.rowCache);
    if (current.listElement !== list || current.listHtml !== next.html) {
      patchConversationList(list, current.listSnapshot, next);
      current.listElement = list; current.listHtml = next.html;
    }
    current.listSnapshot = next;
    const total = document.querySelector('.inbox-count'); if (total) total.textContent = String(current.items.length);
    const badge = document.querySelector('[data-chat-unread-count]');
    if (badge) { const unread = current.items.reduce((count, item) => count + Number(item.unread_count || 0), 0); badge.textContent = unread > 99 ? '99+' : String(unread); badge.hidden = !unread; }
  }

  function setChatAccess(current, error) {
    if (chat !== current || current.page !== generation || current.owner !== ownerKey()) return;
    current.loading = false;
    const denied = [401, 403, 404].includes(error.status);
    if (denied) {
      current.denied = true; current.controller?.abort();
      current.messages = []; current.seen.clear(); current.sentMessages = [];
      chatDrafts.delete(current.id); pendingMessages.delete(current.id);
      if (current.current) current.current.writable = false;
      current.items = current.items.filter(item => item.id !== current.id);
      refreshConversationList(current);
      const textarea = document.querySelector('#chat-body');
      if (textarea) textarea.value = '';
    }
    const container = document.querySelector('#chat-messages');
    if (container && (denied || !current.seen.size)) container.innerHTML = `<div class="chat-load-error" role="status">${icon('info')}<h3>${denied ? t('这个会话当前不可访问', 'This conversation is no longer available') : t('消息暂时没有加载出来', 'Messages could not be loaded')}</h3><p>${denied ? t('访问权限可能已变化。请选择其他会话。', 'Access may have changed. Choose another conversation.') : esc(error.message || t('请检查连接后重试。', 'Check your connection and try again.'))}</p>${denied ? '' : `<button type="button" class="button secondary small" data-chat-retry>${t('重新加载', 'Try again')}</button>`}</div>`;
    document.querySelector('#chat-scroll')?.setAttribute?.('aria-busy', 'false');
    document.querySelectorAll('.chat-composer textarea,.chat-composer button').forEach(element => { element.disabled = denied || !current.seen.size || current.current?.writable === false; });
    const status = document.querySelector('#chat-connection');
    if (status) status.textContent = denied ? t('访问已失效', 'Access is no longer available') : t('连接暂时中断，正在重试。', 'Connection interrupted. Retrying.');
  }

  function bindChatControls(current) {
    const workspace = document.querySelector('.chat-workspace');
    if (!workspace?.addEventListener) return;
    const input = event => {
      if (event.target.id === 'conversation-search') { conversationSearch = event.target.value; refreshConversationList(current); }
      if (event.target.id === 'chat-body') {
        saveChatDraft(current);
        event.target.style.height = 'auto'; event.target.style.height = Math.min(event.target.scrollHeight, 160) + 'px';
      }
    };
    const click = event => {
      const filter = event.target.closest('[data-chat-filter]');
      if (filter) { conversationFilter = filter.dataset.chatFilter; workspace.querySelectorAll('[data-chat-filter]').forEach(button => button.setAttribute('aria-pressed', String(button === filter))); refreshConversationList(current); }
      const prompt = event.target.closest('[data-chat-prompt]');
      if (prompt) { const field = document.querySelector('#chat-body'); if (field && !field.disabled) { field.value = prompt.dataset.chatPrompt; field.focus(); input({ target: field }); } }
      if (event.target.closest('[data-chat-back]')) { event.preventDefault(); go(route(current.admin, 'messages', { view: 'list' })); }
      if (event.target.closest('[data-chat-retry]')) { event.preventDefault(); render(); }
    };
    const keydown = event => {
      if (event.target.id === 'chat-body' && chatSendShortcut(event) && !event.target.disabled) {
        const form = event.target.closest('form');
        const submit = form?.querySelector('[type="submit"]');
        if (submit && !submit.disabled && event.target.value.trim()) { event.preventDefault(); form.requestSubmit(submit); }
      }
    };
    workspace.addEventListener('input', input); workspace.addEventListener('click', click); workspace.addEventListener('keydown', keydown);
    chatCleanup = () => { workspace.removeEventListener('input', input); workspace.removeEventListener('click', click); workspace.removeEventListener('keydown', keydown); };
  }

  function mountChat() {
    const current = chat;
    if (!current) return;
    bindChatControls(current);
    if (!current.id || !document.querySelector('#chat-scroll')) return;
    current.mounted = true;
    const activeGeneration = generation;
    const valid = () => generation === activeGeneration && chat === current && current.owner === ownerKey() && !current.denied;
    const restorePosition = () => {
      const saved = chatDrafts.get(current.id);
      const scroll = document.querySelector('#chat-scroll');
      if (scroll && saved?.top != null && !saved.atBottom) scroll.scrollTop = saved.top;
      else scrollLatest();
    };
    const poll = async () => {
      if (!valid()) return;
      let catchingUp = false;
      try {
        if (!document.hidden) {
          // The index updates independently; new authorized messages need not wait for it.
          const index = refreshChatIndex(current).then(() => null, error => {
            if (valid() && [401, 403, 404].includes(error.status)) setChatAccess(current, error);
            return error;
          });
          for (let page = 0; page < 4 && valid() && !document.hidden; page++) {
            const initial = !current.historyReady, cursor = current.cursor;
            const messages = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages?${initial ? '' : `after=${cursor}&`}limit=100`, { fresh: true, signal: current.controller.signal, group: `chat:${current.page}` });
            if (!valid()) return;
            if (current.error) { current.error = null; const container = document.querySelector('#chat-messages'); if (container && !current.seen.size) container.innerHTML = chatWelcome(current.current, current.admin); }
            if (initial) {
              current.first = messages.first_cursor || messages.items?.[0]?.id || 0;
              current.hasOlder = !!messages.has_older;
              const history = document.querySelector('[data-action="chat-older"]'); if (history) history.hidden = !current.hasOlder;
              const textarea = document.querySelector('#chat-body'); if (textarea) textarea.value = chatDrafts.get(current.id)?.body || '';
              document.querySelector('#chat-scroll')?.setAttribute?.('aria-busy', 'false');
            }
            current.historyReady = true; current.loading = false;
            appendMessages(current, messages.items);
            current.cursor = Math.max(current.cursor, messages.next_cursor || 0);
            catchingUp = !!messages.has_more && current.cursor > cursor;
            current.hasMore = catchingUp;
            if (!catchingUp) break;
            // Bound one burst to four pages, and yield a task between pages for input/paint.
            if (page < 3) await new Promise(resolve => setTimeout(resolve, 0));
          }
          const indexError = await index;
          if (!valid()) return;
          if (indexError) throw indexError;
          const status = document.querySelector('#chat-connection');
          if (status) status.textContent = current.current?.writable === false ? t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.') : catchingUp ? t('正在同步新消息…', 'Syncing new messages…') : t('消息已同步', 'Messages are up to date');
          document.querySelectorAll('.chat-composer textarea,.chat-composer button').forEach(element => { element.disabled = current.current?.writable === false || element.getAttribute('aria-busy') === 'true'; });
          await markRead(current);
        }
      } catch (error) { catchingUp = false; if (valid()) { current.error = error; setChatAccess(current, error); } }
      finally { if (valid()) polling = setTimeout(poll, catchingUp ? 250 : 5000); }
    };
    const start = async () => {
      if (current.loading) {
        const prepared = await current.pending;
        if (!valid()) return;
        if (prepared?.error) { current.error = prepared.error; setChatAccess(current, prepared.error); }
        else {
          const result = prepared?.result || {};
          current.loading = false; current.historyReady = true;
          current.messages = [...new Map([...(result.items || []), ...(current.sentMessages || [])].map(message => [message.id, message])).values()].sort((a, b) => a.id - b.id);
          current.sentMessages = [];
          current.seen = new Set(current.messages.map(message => message.id));
          current.cursor = result.next_cursor || 0; current.first = result.first_cursor || current.messages[0]?.id || 0;
          current.hasOlder = !!result.has_older; current.hasMore = !!result.has_more; current.lastDate = messageDateKey(current.messages.at(-1)?.created_at);
          const container = document.querySelector('#chat-messages');
          if (container) container.innerHTML = current.messages.length ? messagesHtml(current.messages, current.admin) : chatWelcome(current.current, current.admin);
          const history = document.querySelector('[data-action="chat-older"]'); if (history) history.hidden = !current.hasOlder;
          document.querySelector('#chat-scroll')?.setAttribute?.('aria-busy', 'false');
          const textarea = document.querySelector('#chat-body'); if (textarea) textarea.value = chatDrafts.get(current.id)?.body || '';
          document.querySelectorAll('.chat-composer textarea,.chat-composer button').forEach(element => { element.disabled = current.current?.writable === false; });
          const status = document.querySelector('#chat-connection'); if (status) status.textContent = current.current?.writable === false ? t('此会话已归档，仅供查看历史。', 'This room is archived. Its history is read-only.') : t('消息已同步', 'Messages are up to date');
        }
      }
      if (!valid() || current.denied) return;
      restorePosition(); markRead(current).catch(() => {}); polling = setTimeout(poll, 5000);
    };
    return start();
  }

  async function sendMessage(form, data) {
    const current = chat;
    if (!current?.id || current.owner !== ownerKey()) throw new Error(t('请先选择一个会话。', 'Choose a conversation first.'));
    if (current.current?.writable === false || current.denied || current.loading) throw new Error(t('这个会话暂时不能发送消息。', 'Messages cannot be sent to this room right now.'));
    const body = String(data.get('body') || '').trim();
    if (!body) throw new Error(t('写一点内容再发送吧。', 'Write a message before sending.'));
    const previous = pendingMessages.get(current.id);
    if (!previous && pendingMessages.size >= 50) throw new Error(t('请先重试已有的待发送消息。', 'Retry an existing pending message first.'));
    const client_id = previous?.body === body ? previous.id : createMessageId();
    pendingMessages.set(current.id, { id: client_id, body });
    let message;
    try { message = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages`, { method: 'POST', body: { body, client_id } }); }
    catch (error) {
      const visible = chat?.id === current.id && chat.owner === current.owner && current.owner === ownerKey() && chat.page === generation && !chat.denied ? chat : null;
      if (visible) {
        if ([401, 403, 404].includes(error.status)) setChatAccess(visible, error);
        else { const status = document.querySelector('#chat-connection'); if (status) status.textContent = t('未发送 · 草稿已保留，可以重试', 'Not sent · Your draft is kept. Try again.'); }
      }
      throw error;
    }
    if (current.owner !== ownerKey()) return;
    if (pendingMessages.get(current.id)?.id === client_id) pendingMessages.delete(current.id);
    const saved = chatDrafts.get(current.id); if (saved?.body.trim() === body) saved.body = '';
    // A route refresh may have remounted this same room while the write completed.
    // Clear only its unchanged draft, and apply the receipt to the currently visible room.
    const visible = chat?.id === current.id && chat.owner === current.owner && chat.page === generation && !chat.denied ? chat : null;
    if (!visible) return;
    const textarea = document.querySelector('#chat-body');
    if (!textarea) return;
    if (textarea.value.trim() === body) { textarea.value = ''; if (textarea.style) textarea.style.height = 'auto'; }
    if (visible.loading) (visible.sentMessages ||= []).push(message);
    else appendMessages(visible, [message], true);
    if (visible.current) { visible.current.last_message_preview = message.body || body; visible.current.last_message_at = message.created_at; refreshConversationList(visible); }
    saveChatDraft(visible);
    // Only the incremental endpoint advances the poll cursor; concurrent replies cannot be skipped.
    api(`/api/chat/conversations/${encodeURIComponent(current.id)}/read`, { method: 'POST', body: { message_id: message.id } }).catch(() => {});
    if (chat === current) textarea.focus();
  }

  async function olderMessages() {
    const current = chat;
    const valid = () => chat === current && current.page === generation && current.owner === ownerKey() && !current.denied;
    if (!current?.first || !current.hasOlder || current.loadingOlder || !valid()) return;
    current.loadingOlder = true;
    try {
      const result = await api(`/api/chat/conversations/${encodeURIComponent(current.id)}/messages?before=${current.first}&limit=100`, { fresh: true, signal: current.controller.signal, group: `chat:${current.page}` });
      if (!valid()) return;
      const scroll = document.querySelector('#chat-scroll');
      const container = document.querySelector('#chat-messages');
      if (!scroll || !container) return;
      const oldHeight = scroll.scrollHeight, oldTop = scroll.scrollTop;
      const fresh = (result.items || []).filter(m => !current.seen.has(m.id));
      fresh.forEach(m => current.seen.add(m.id));
      current.messages = [...fresh, ...(current.messages || [])];
      const firstDate = container.querySelector?.('.chat-date');
      if (fresh.length && firstDate?.dataset.date === messageDateKey(fresh.at(-1).created_at)) firstDate.remove();
      container.insertAdjacentHTML('afterbegin', messagesHtml(fresh, current.admin));
      current.first = result.first_cursor || current.first; current.hasOlder = !!result.has_older;
      const button = document.querySelector('[data-action="chat-older"]'); if (button) button.hidden = !current.hasOlder;
      scroll.scrollTop = oldTop + scroll.scrollHeight - oldHeight;
    } catch (error) {
      // Navigating away intentionally cancels this read; it is not a failure
      // to toast over the newly selected conversation.
      if (!valid() || current.controller.signal.aborted) return;
      if ([401, 403, 404].includes(error.status)) setChatAccess(current, error);
      throw error;
    }
    finally { current.loadingOlder = false; }
  }

  function mandateFields(m = {}, admin = false) {
    const meta=currency(m);
    return `<div class="grid-2"><div class="field"><label for="mandate-industry">${t('想去的行业', 'Target industry')}</label><input class="input" id="mandate-industry" name="target_industry" maxlength="200" value="${esc(m.target_industry)}" placeholder="${esc(t('例如：教育、软件或消费品牌', 'For example: education, software, consumer brands'))}"></div><div class="field"><label for="mandate-roles">${t('想做的岗位', 'Target roles')}</label><input class="input" id="mandate-roles" name="target_roles" maxlength="500" value="${esc(m.target_roles)}" placeholder="${esc(t('例如：产品设计师、Rust 工程师', 'For example: product designer, Rust engineer'))}" required></div></div><div class="field"><label for="mandate-currency">${t('期待薪资币种', 'Expected salary currency')}</label>${currencySelect(meta.currency, 'salary_currency', 'mandate-currency')}<input type="hidden" name="original_salary_currency" value="${esc(m.salary_currency||'')}"><p class="help">${t('请按所选币种填写金额，切换币种不会自动换算。', 'Enter amounts in this currency. Changing the selection does not convert amounts.')}</p></div><div class="grid-2"><div class="field"><label for="mandate-min" data-salary-label="min">${t('期待月薪底薪 · 最低', 'Expected monthly base · minimum')}（${esc(meta.currency)}）</label><input class="input" id="mandate-min" name="salary_min" type="number" min="0" max="${amountValue(100000000,meta)}" step="${amountStep(meta)}" value="${m.salary_min_cents != null ? amountValue(m.salary_min_cents,meta) : ''}" placeholder="${esc(t('可以留空', 'Optional'))}"></div><div class="field"><label for="mandate-max" data-salary-label="max">${t('期待月薪底薪 · 最高', 'Expected monthly base · maximum')}（${esc(meta.currency)}）</label><input class="input" id="mandate-max" name="salary_max" type="number" min="0" max="${amountValue(100000000,meta)}" step="${amountStep(meta)}" value="${m.salary_max_cents != null ? amountValue(m.salary_max_cents,meta) : ''}" placeholder="${esc(t('可以留空', 'Optional'))}"></div></div><div class="grid-2"><div class="field"><label for="mandate-location">${t('地点与工作方式', 'Location and work arrangement')}</label><input class="input" id="mandate-location" name="location" maxlength="300" value="${esc(m.location)}" placeholder="${esc(t('例如：上海 / 远程 / 混合办公', 'For example: Shanghai / remote / hybrid'))}"></div><div class="field"><label for="mandate-status">${t('委托状态', 'Search status')}</label><select class="input" id="mandate-status" name="status">${['active', 'paused', 'closed'].map(s => `<option value="${s}" ${s === (admin && m.banned && m.status === 'active' ? 'paused' : m.status || 'active') ? 'selected' : ''} ${admin && s === 'active' && (m.banned || m.status !== 'active') ? 'disabled' : ''}>${mandateStatus(s)}</option>`).join('')}</select></div></div><div class="field"><label for="mandate-preferences">${t('还有什么对你很重要？', 'What else matters to you?')}</label><textarea class="input" id="mandate-preferences" name="preferences" rows="5" maxlength="5000" placeholder="${esc(t('团队、成长方向与工作节奏，以及你希望我们了解的事。', 'The team, growth, working rhythm, and anything else we should know.'))}">${esc(m.preferences)}</textarea></div>${admin ? `<div class="notice">${icon('shield')}<div>${m.allow_external ? t('本人已授权多渠道寻找。管理员不能扩大这项授权。', 'The job seeker has authorized external outreach. Administrators cannot expand this consent.') : t('本人尚未授权外部联系。请先在平台私聊中沟通。', 'External outreach is not authorized. Discuss it in the private platform conversation first.')}${m.consented_at ? `<br>${t('授权时间', 'Consent recorded')}: ${humanTime(m.consented_at)}` : ''}</div></div>` : `<label class="toggle-row mandate-consent"><input type="checkbox" name="allow_external" ${m.allow_external ? 'checked' : ''}><span><strong>${t('授权聘渡通过多渠道为我寻找机会', 'Authorize HireFerry to seek opportunities across channels')}</strong><small>${t('由平台围绕这份委托开展外部联系，并在这里记录进展。你可以随时取消授权或暂停委托。', 'HireFerry may contact relevant parties for this search and record progress here. You can withdraw consent or pause at any time.')}</small></span></label>`}`;
  }

  function outreachList(items, admin = false, userId = '') {
    outreachCache.clear();
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
    const selected=data.get('salary_currency')||currency().currency;
    const body = { target_industry: String(data.get('target_industry') || '').trim(), target_roles: String(data.get('target_roles') || '').trim(), salary_currency:selected, salary_min_cents: min ? centsInput(min,selected,{min:0,max:100000000}) : null, salary_max_cents: max ? centsInput(max,selected,{min:0,max:100000000}) : null, location: String(data.get('location') || '').trim(), preferences: String(data.get('preferences') || '').trim(), status: data.get('status') };
    if (body.salary_min_cents !== null && body.salary_max_cents !== null && body.salary_min_cents > body.salary_max_cents) throw new Error(t('最低期待薪资不能高于最高薪资。', 'Minimum salary cannot exceed maximum salary.'));
    if(!currencies().some(option=>option.currency===selected)){if(selected===data.get('original_salary_currency'))delete body.salary_currency;else throw new Error(t('请选择可用的薪资币种。','Choose an available salary currency.'));}
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
    return shell(`${heading(t('认真寻找，不止于站内。', 'The search goes beyond this site.'), t('以用户委托为起点，记录多渠道寻找与真实跟进。', 'Start with each person’s brief. Record research and real follow-ups across channels.'), '', 'SEARCH MANDATES')}<section class="panel"><form data-form="mandate-filter" class="filter-row mandate-filter"><div class="search-field">${icon('search')}<input class="input" name="q" value="${esc(query().get('q'))}" aria-label="${t('搜索委托', 'Search briefs')}" placeholder="${esc(t('姓名、行业或岗位', 'Name, industry, or role'))}"></div><select class="input" name="status" aria-label="${t('委托状态', 'Search status')}"><option value="">${t('全部状态', 'All statuses')}</option>${['active', 'paused', 'closed'].map(s => `<option value="${s}" ${query().get('status') === s ? 'selected' : ''}>${mandateStatus(s)}</option>`).join('')}</select><button class="button secondary small" type="submit">${t('查找', 'Find')}</button></form>${d.items?.length ? `<div class="mandate-list">${d.items.map(m => `<article class="mandate-card"><div class="mandate-card-top"><div><h3>${esc(m.user_name)}${m.banned ? ` <span class="status banned">${t('已封禁', 'Suspended')}</span>` : ''}</h3><p>${esc(m.target_roles)}</p></div><span class="status ${m.status === 'active' ? '' : 'closed'}">${mandateStatus(m.status)}</span></div><div class="list-meta"><span>${esc(m.target_industry || t('行业开放', 'Open industry'))}</span><span>${esc(m.location || t('地点开放', 'Open location'))}</span><span>${salaryRange(m)} / ${t('月', 'month')}</span></div><p class="mandate-summary">${esc(m.preferences || t('可以在私聊中进一步了解期待。', 'Discuss their expectations in the private conversation.'))}</p><div class="mandate-card-footer"><span class="consent-label ${m.allow_external ? 'allowed' : ''}">${icon('shield')}${m.allow_external ? t('已授权多渠道寻找', 'External outreach authorized') : t('仅平台内沟通', 'Platform conversation only')}</span><div class="row-actions"><a href="${esc(chatUrl(true, 'platform_' + m.user_id))}" data-nav class="text-button">${t('平台私聊', 'Private chat')}</a><button class="text-button" data-action="admin-mandate" data-id="${esc(m.user_id)}">${t('查看与跟进', 'Review & follow up')} →</button></div></div></article>`).join('')}</div><p class="help">${d.total ?? d.items.length} ${t('份委托', 'search briefs')}</p>${mandatePagination(d.total,d.page_size??d.limit)}` : empty(t('等待一份新的期待。', 'Waiting for a new direction.'), t('求职者主动提交的委托会显示在这里。', 'Search briefs appear here when job seekers submit them.'), '', 'target')}</section>`, 'mandates', true);
  }

  function mandatePagination(total,limit=state.config?.limits?.list_page_size||50) {
    const page = Math.max(1, Number(query().get('page')) || 1);
    const pages = Math.min(state.config?.limits?.list_max_page||10000,Math.ceil(Number(total || 0) / limit));
    if (pages < 2) return '';
    const link = n => { const p = query(); p.set('page', n); return '/admin?' + p; };
    return `<nav class="pagination" aria-label="${t('分页', 'Pagination')}">${page > 1 ? `<a href="${esc(link(page - 1))}" data-nav class="button secondary small">${t('上一页', 'Previous')}</a>` : '<span></span>'}<span>${page} / ${pages}</span>${page < pages ? `<a href="${esc(link(page + 1))}" data-nav class="button secondary small">${t('下一页', 'Next')}</a>` : '<span></span>'}</nav>`;
  }

  async function adminMandate(userId) {
    const owner = syncChatOwner();
    const view = workbenchGeneration;
    mandateCache.clear(); outreachCache.clear();
    const [d, progress] = await Promise.all([api(`/api/admin/mandates/${encodeURIComponent(userId)}`), api(`/api/admin/outreach/${encodeURIComponent(userId)}`)]);
    if (!state.admin || owner !== ownerKey() || view !== workbenchGeneration) return;
    const m = d.mandate;
    if (m) mandateCache.set(userId, m);
    if (!m) throw new Error(t('用户尚未提交委托。', 'This person has not submitted a search brief.'));
    openDialog(`<div class="eyebrow">SEARCH & FOLLOW THROUGH</div><h2 id="dialog-title">${esc(m.user_name)}</h2><p class="dialog-intro">${t('委托只对平台可见。外部联系遵循本人授权。', 'This brief is private to HireFerry. External contact follows the person’s consent.')}</p><form data-form="admin-mandate" data-id="${esc(userId)}" data-allow-external="${m.allow_external ? 'true' : 'false'}">${mandateFields(m, true)}<div class="form-actions"><button class="button" type="submit">${t('保存委托', 'Save brief')}${icon('check')}</button></div></form><hr><div class="panel-header"><h2>${t('真实寻找记录', 'Search records')}</h2><button class="button secondary small" type="button" data-action="new-outreach" data-user="${esc(userId)}" ${m.allow_external && m.status === 'active' && !m.banned ? '' : 'disabled'}>${icon('plus')}${t('记录进展', 'Record progress')}</button></div>${!m.allow_external || m.status !== 'active' || m.banned ? `<p class="help">${t('新增外部联系记录需要本人授权、委托正在寻找且账户有效。已有记录可以结束。', 'New outreach requires consent, an active search brief, and an active account. Existing records can be closed.')}</p>` : ''}${outreachList(progress.items, true, userId)}`, { wide: true });
  }

  function outreachDialog(userId, id = '') {
    syncChatOwner();
    const r = id ? outreachCache.get(id) : {};
    const mandate = mandateCache.get(userId);
    const allowed = !!(mandate?.allow_external && mandate.status === 'active' && !mandate.banned);
    if (!r) throw new Error(t('记录已过期，请重新打开委托。', 'Refresh this brief to load the record again.'));
    openDialog(`<div class="eyebrow">A REAL STEP FORWARD</div><h2 id="dialog-title">${id ? t('更新寻找进展', 'Update search progress') : t('记下这次进展', 'Record a step forward')}</h2><p class="dialog-intro">${t('仅记录实际发生的寻找或联系。内容会对这位求职者可见。', 'Record research or contact that actually happened. The job seeker can see this entry.')}</p><form data-form="outreach" data-id="${esc(id)}" data-user="${esc(userId)}"><div class="grid-2"><div class="field"><label>${t('渠道', 'Channel')}<input class="input" name="channel" value="${esc(r.channel)}" maxlength="120" placeholder="${esc(t('例如：企业官网、行业社区或直接联系', 'For example: company website, community, direct contact'))}" required></label></div><div class="field"><label>${t('当前进度', 'Current stage')}<select class="input" name="stage">${outreachStages.map(s => `<option value="${s}" ${s === (!allowed ? 'closed' : r.stage || 'identified') ? 'selected' : ''} ${!allowed && s !== 'closed' ? 'disabled' : ''}>${outreachText(s)}</option>`).join('')}</select></label></div></div><div class="field"><label>${t('机会或联系对象', 'Opportunity or contact')}<input class="input" name="opportunity" value="${esc(r.opportunity)}" maxlength="500" required></label></div><div class="field"><label>${t('来源链接，可选', 'Source link, optional')}<input class="input" name="source_url" type="url" value="${esc(r.source_url)}" maxlength="2000" placeholder="https://"></label></div><div class="field"><label>${t('跟进记录', 'Follow-up note')}<textarea class="input" name="note" rows="5" maxlength="5000">${esc(r.note)}</textarea></label></div><div class="form-actions"><button class="button" type="submit">${t('保存记录', 'Save record')}${icon('check')}</button></div></form>`, { wide: true });
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
    if (admin) return matchingWorkbench();
    const { items: matches = [] } = await api('/api/matches');
    const head = heading(t('认真了解，再相互认识。', 'A thoughtful introduction comes first.'), t('只有平台建立匹配后，双方才可以查看资料并进入独立会话。', 'Profiles and a shared room become available after HireFerry makes an introduction.'), '', 'HUMAN INTRODUCTIONS');
    return shell(`${head}${matches.length ? `<div class="user-match-grid">${matches.map(m => matchCard(m, false)).join('')}</div>` : `<section class="panel">${empty(t('值得聊的人，我们认真找。', 'We’re looking for a conversation worth having.'), t('先和平台说清期待。有合适的匹配，会在这里为你介绍。', 'Tell the team what matters. Your introductions will appear here.'), `<a href="/app?tab=messages" data-nav class="button secondary">${t('与平台沟通', 'Talk to the team')}${icon('message')}</a>`, 'link')}</section>`}`, 'matches');
  }

  const workbenchPages = new Map();
  const workbenchHistory = new Map();
  let selectedCandidate = null;
  const workbenchKeys = ['view', 'job', 'candidate', 'job_q', 'jobs_cursor', 'candidate_q', 'industry', 'roles', 'location', 'salary_min', 'salary_max', 'salary_currency', 'salary_fit', 'candidates_cursor', 'follow_q', 'status', 'follow_job', 'follow_seeker', 'follow_employer', 'matches_cursor'];
  function workbenchUrl(changes = {}) {
    const source = query();
    const params = new URLSearchParams({ tab: 'matches' });
    for (const key of workbenchKeys) if (source.get(key)) params.set(key, source.get(key));
    for (const [key, value] of Object.entries(changes)) {
      if (value === null || value === undefined || value === '') params.delete(key);
      else params.set(key, String(value));
    }
    return '/admin?' + params;
  }

  function workbenchPageSize() {
    const value=Number(state.config?.limits?.workbench_page_size);
    return Number.isInteger(value)&&value>0?Math.min(value,100):25;
  }

  async function workbenchList(kind, filters = {}, requestGeneration = workbenchGeneration) {
    syncChatOwner();
    const requestedLimit=workbenchPageSize();
    const params = new URLSearchParams({ limit: String(requestedLimit) });
    for (const [key, value] of Object.entries(filters)) if (value !== undefined && value !== null && value !== '') params.set(key, String(value));
    const signature = kind + '?' + params;
    const cursor = query().get(kind + '_cursor') || '';
    if (cursor) params.set('cursor', cursor);
    let history = workbenchHistory.get(signature);
    if (!history) history = [''];
    workbenchHistory.delete(signature); workbenchHistory.set(signature, history);
    while (workbenchHistory.size > 50) workbenchHistory.delete(workbenchHistory.keys().next().value);
    const page = await api(`/api/admin/workbench/${kind}?${params}`);
    if (requestGeneration === workbenchGeneration) workbenchPages.set(kind, { signature, cursor, history, page });
    const responseLimit=Number(page.limit);
    const limit=Number.isInteger(responseLimit)&&responseLimit>0?Math.min(responseLimit,100):requestedLimit;
    return { ...page, limit, items: (page.items || []).slice(0,limit), total: Number(page.total || 0), next_cursor: page.next_cursor || null };
  }

  function workbenchPager(kind, page) {
    const current = workbenchPages.get(kind);
    const index = current?.history.indexOf(current.cursor) ?? -1;
    return `<nav class="wb-pagination" aria-label="${t('结果分页', 'Result pages')}"><span>${t(`本页 ${page.items.length} 条 · 共 ${page.total} 条`, `${page.items.length} on this page · ${page.total} total`)}</span><div><button class="icon-button" type="button" data-action="workbench-page" data-list="${kind}" data-direction="previous" aria-label="${t('上一页', 'Previous page')}" ${index > 0 ? '' : 'disabled'}>${icon('back')}</button><button class="icon-button" type="button" data-action="workbench-page" data-list="${kind}" data-direction="next" aria-label="${t('下一页', 'Next page')}" ${page.next_cursor ? '' : 'disabled'}>${icon('arrow')}</button>${current?.cursor && (index < 0 || current.history[0] !== '') ? `<button class="text-button" type="button" data-action="workbench-page" data-list="${kind}" data-direction="first">${t('首页', 'First page')}</button>` : ''}</div></nav>`;
  }

  async function workbenchPage(kind, direction) {
    const current = workbenchPages.get(kind);
    if (!current) return;
    const index = current.history.indexOf(current.cursor);
    let cursor = '';
    if (direction === 'next') {
      cursor = current.page.next_cursor;
      if (!cursor) return;
      const prefix = index >= 0 ? current.history.slice(0, index + 1) : [current.cursor];
      current.history.splice(0, current.history.length, ...prefix, cursor);
      if (current.history.length > 100) current.history.splice(0, current.history.length - 100);
    } else if (direction === 'previous') {
      if (index <= 0) return;
      cursor = current.history[index - 1];
    } else if (direction !== 'first') return;
    return go(workbenchUrl({ [kind + '_cursor']: cursor, ...(kind === 'candidates' ? { candidate: null } : {}) }));
  }

  function optionalSalary(value,record) {
    const text = String(value ?? '').trim();
    return text ? centsInput(text,record,{min:0,max:100000000}) : null;
  }

  function updateSalaryCurrency(input) {
    const form=input.closest('form'),meta=currency(input.value||form.dataset.defaultCurrency);
    const brief=form.dataset.form==='mandate'||form.dataset.form==='admin-mandate';
    for(const bound of ['min','max']) {
      const field=form.elements['salary_'+bound],label=form.querySelector('[data-salary-label="'+bound+'"]');
      if(field){field.step=amountStep(meta);field.max=amountValue(100000000,meta);}
      if(label)label.textContent=(brief?(bound==='min'?t('期待月薪底薪 · 最低','Expected monthly base · minimum'):t('期待月薪底薪 · 最高','Expected monthly base · maximum')):(bound==='min'?t('月薪最低','Monthly min'):t('月薪最高','Monthly max')))+'（'+meta.currency+'）';
    }
  }

  async function searchWorkbench(form, data) {
    const kind = form.dataset.form;
    const text = name => String(data.get(name) || '').trim();
    if (kind === 'workbench-jobs') return go(workbenchUrl({ job_q: text('q'), jobs_cursor: null, job: null, candidate: null, candidates_cursor: null }));
    if (kind === 'workbench-candidates') {
      const usingBasePay = data.get('salary_fit') === 'on';
      const selected=text('salary_currency')||form.dataset.defaultCurrency||currency().currency;
      const min = usingBasePay ? null : optionalSalary(data.get('salary_min'),selected);
      const max = usingBasePay ? null : optionalSalary(data.get('salary_max'),selected);
      if (min !== null && max !== null && min > max) throw new Error(t('最低薪资不能大于最高薪资。', 'Minimum salary cannot exceed maximum salary.'));
      return go(workbenchUrl({ candidate_q: text('q'), industry: text('industry'), roles: text('roles'), location: text('location'), salary_min: text('salary_min'), salary_max: text('salary_max'), salary_currency: text('salary_currency'), salary_fit: data.get('salary_fit') === 'on' ? '1' : null, candidates_cursor: null, candidate: null }));
    }
    if (kind === 'workbench-followups') return go(workbenchUrl({ follow_q: text('q'), status: text('status'), matches_cursor: null }));
  }

  function workbenchTabs(view) {
    return `<nav class="wb-tabs" aria-label="${t('匹配工作台视图', 'Matching workspace views')}"><a href="${esc(workbenchUrl({ view: null }))}" data-nav ${view === 'search' ? 'aria-current="page"' : ''}>${icon('search')}${t('筛选候选人', 'Find candidates')}</a><a href="${esc(workbenchUrl({ view: 'followups' }))}" data-nav ${view === 'followups' ? 'aria-current="page"' : ''}>${icon('link')}${t('匹配跟进', 'Follow-ups')}</a></nav>`;
  }

  function workbenchJobQueue(page, selectedId) {
    return `<aside class="wb-jobs" aria-labelledby="wb-jobs-title"><div class="wb-section-header"><h2 id="wb-jobs-title">${t('招聘中的岗位', 'Hiring roles')}</h2><span>${page.total}</span></div><form data-form="workbench-jobs" class="wb-search"><label class="sr-only" for="wb-job-search">${t('搜索岗位或招聘方', 'Search roles or employers')}</label><input class="input" id="wb-job-search" name="q" maxlength="120" value="${esc(query().get('job_q') || '')}" placeholder="${esc(t('岗位 / 招聘方', 'Role / employer'))}"><button class="icon-button" type="submit" aria-label="${t('搜索岗位', 'Search roles')}">${icon('search')}</button></form><p class="wb-list-note">${t('已付款、已发布，且招聘人数大于 0。', 'Paid, published, with a positive hiring target.')}</p><div class="wb-job-list">${page.items.length ? page.items.map(j => `<a class="wb-job ${j.id === selectedId ? 'selected' : ''}" href="${esc(workbenchUrl({ job: j.id, candidate: null, candidates_cursor: null }))}" data-nav ${j.id === selectedId ? 'aria-current="true"' : ''}><strong>${esc(j.title)}</strong><span>${esc(j.employer_name)}</span><div><b>${money(j.salary_cents,j)}<small> / ${t('月', 'mo')}</small></b><span>${t(`${Number(j.headcount)} 人`, `Target: ${Number(j.headcount)}`)}</span></div><small>${t(`${Number(j.active_matches || 0)} 位正在跟进`, `${Number(j.active_matches || 0)} active introductions`)}</small></a>`).join('') : `<div class="wb-empty small"><h3>${t('没有符合条件的岗位', 'No roles found')}</h3><p>${t('调整关键词，或先确认岗位已付款并正在招聘。', 'Try another keyword, or check that a role is paid and hiring.')}</p></div>`}</div>${workbenchPager('jobs', page)}</aside>`;
  }

  function candidateFilters(job) {
    const p = query(),meta=currency(p.get('salary_currency')||job);
    const options=currencies();if(!options.some(item=>item.currency===currency(job).currency))options.push(currency(job));
    const field = (name, label, placeholder = '') => `<label>${label}<input class="input" name="${name}" maxlength="80" value="${esc(p.get(name) || '')}" placeholder="${esc(placeholder)}"></label>`;
    const advanced = [['industry', t('行业', 'Industry')], ['roles', t('方向', 'Role')], ['location', t('地点', 'Location')], ['salary_min', t('最低月薪', 'Minimum pay') + ` (${meta.currency})`], ['salary_max', t('最高月薪', 'Maximum pay') + ` (${meta.currency})`], ['salary_currency', t('币种', 'Currency')]].filter(([key]) => p.get(key));
    const filterKey = `candidate-filters:${job.id}:${new URLSearchParams(advanced.map(([key]) => [key, p.get(key)]))}`;
    const filterSummary = advanced.map(([key, label]) => `${label}: ${p.get(key)}`).join(' · ');
    return `<form data-form="workbench-candidates" data-default-currency="${esc(currency(job).currency)}" class="wb-filters"><label class="wb-keyword">${t('搜索候选人', 'Search candidates')}<input class="input" name="q" maxlength="120" value="${esc(p.get('candidate_q') || '')}" placeholder="${esc(t('姓名、简历或委托关键词', 'Name, résumé, or search-brief keywords'))}"></label><details class="wb-filter-details section-disclosure" data-disclosure="${esc(filterKey)}" ${advanced.length ? 'open' : ''}><summary><span>${t('高级筛选', 'Advanced filters')}${filterSummary ? `<span class="help"> · ${esc(filterSummary)}</span>` : `<span class="help"> · ${t('行业、方向、地点与薪资', 'Industry, role, location, and salary')}</span>`}</span></summary><div class="wb-filter-grid">${field('industry', t('目标行业', 'Target industry'))}${field('roles', t('目标方向', 'Target role'))}${field('location', t('地点', 'Location'))}<label>${t('薪资筛选币种', 'Salary filter currency')}<select class="input" name="salary_currency"><option value="">${t('全部币种；填范围时用 ', 'All currencies; bounds use ')}${esc(currency(job).currency)}</option>${options.map(option=>`<option value="${esc(option.currency)}" ${p.get('salary_currency')===option.currency?'selected':''}>${esc(option.currency)}</option>`).join('')}</select></label><div class="wb-salary-fields"><label><span data-salary-label="min">${t('月薪最低', 'Monthly min')}（${esc(meta.currency)}）</span><input class="input" name="salary_min" type="number" min="0" max="${amountValue(100000000,meta)}" step="${amountStep(meta)}" value="${esc(p.get('salary_min') || '')}"></label><label><span data-salary-label="max">${t('月薪最高', 'Monthly max')}（${esc(meta.currency)}）</span><input class="input" name="salary_max" type="number" min="0" max="${amountValue(100000000,meta)}" step="${amountStep(meta)}" value="${esc(p.get('salary_max') || '')}"></label></div></div><p class="help">${t('薪资范围只比较同一币种，不自动换算；未填上下限时不限制金额。选择「期望包含岗位底薪」后使用岗位币种与底薪。', 'Salary ranges compare the same currency without conversion. Unspecified bounds are open. The base-pay option uses the role’s currency and salary.')}</p></details><div class="wb-filter-footer"><label class="wb-check"><input type="checkbox" name="salary_fit" ${p.get('salary_fit') === '1' ? 'checked' : ''}><span>${t('期望包含岗位底薪（替代上方范围）', 'Use role base pay instead of the range above')} <b>${money(job.salary_cents,job)}</b></span></label><div><a class="text-button" data-nav href="${esc(workbenchUrl({ candidate_q: null, industry: null, roles: null, location: null, salary_min: null, salary_max: null, salary_currency: null, salary_fit: null, candidates_cursor: null, candidate: null }))}">${t('重置', 'Reset')}</a><button class="button secondary small" type="submit">${icon('search')}${t('筛选', 'Filter')}</button></div></div></form>`;
  }

  function candidateTable(page, selectedId) {
    if (!page.items.length) return `<div class="wb-empty"><span>${icon('person')}</span><h3>${t('没有符合条件的候选人', 'No candidates found')}</h3><p>${t('试试放宽筛选条件。已在这个岗位下跟进的人可在「匹配跟进」查看。', 'Try broader filters. People already introduced to this role are listed under Follow-ups.')}</p></div>${workbenchPager('candidates', page)}`;
    return `${candidateBatchControls()}<div class="wb-table-wrap" data-page-key="candidate-page:${workbenchGeneration}"><table class="wb-table wb-candidate-table"><thead><tr><th>${t('候选人 / 目标方向', 'Person / target role')}</th><th>${t('期望月薪 / 地点', 'Desired pay / location')}</th><th>${t('匹配记录', 'Introduction')}</th><th><span class="sr-only">${t('查看详情', 'View details')}</span></th></tr></thead><tbody>${page.items.map(c => `<tr data-dom-key="candidate:${esc(c.id)}" class="${c.id === selectedId ? 'selected' : ''}"><td><div class="wb-candidate-choice"><label class="wb-pick-label"><input type="checkbox" data-workbench-candidate="${esc(c.id)}" aria-describedby="wb-bulk-help" ${c.history_match ? 'disabled' : ''}><span class="sr-only">${esc(c.history_match ? t(`${c.name} 的匹配已结束，请打开详情后单独恢复`, `Open ${c.name}'s details to reopen the previous introduction`) : t(`选择 ${c.name}`, `Select ${c.name}`))}</span></label><a href="${esc(workbenchUrl({ candidate: c.id }))}" data-nav class="wb-person"><strong>${esc(c.name)}</strong><span>${esc(c.target_roles || t('未填写方向', 'Role not specified'))}</span><small>${esc(c.target_industry || t('行业不限', 'Any industry'))}</small></a></div></td><td><strong>${salaryRange(c)}</strong><span>${esc(c.location || t('地点不限', 'Any location'))}</span></td><td><span class="wb-state ${c.history_match ? 'previous' : ''}">${c.history_match ? t('曾结束匹配', 'Previously ended') : t('未匹配', 'Not introduced')}</span></td><td><a href="${esc(workbenchUrl({ candidate: c.id }))}" data-nav class="icon-button" aria-label="${esc(t(`查看 ${c.name} 的详情`, `View ${c.name}`))}">${icon('arrow')}</a></td></tr>`).join('')}</tbody></table></div>${workbenchPager('candidates', page)}`;
  }

  function candidateBatchControls() {
    const available = candidateBatch?.candidates.length || 0;
    return `<div class="wb-bulk" data-page-key="candidate-selection:${workbenchGeneration}"><div class="wb-bulk-actions"><label class="wb-bulk-all"><input type="checkbox" data-workbench-select-all aria-describedby="wb-bulk-help" ${available ? '' : 'disabled'}><span>${t('全选本页可介绍的人', 'Select eligible people on this page')}</span></label><span id="wb-bulk-count" role="status" aria-live="polite">${t('已选 0 人', '0 selected')}</span><button class="button secondary small" type="button" data-action="workbench-bulk-review" disabled>${t('核对并批量介绍', 'Review selected')}</button></div><p class="help" id="wb-bulk-help">${t('仅选择本页，翻页或筛选后会清空。已结束的匹配请逐人打开详情恢复。', 'Selection is limited to this page and clears when the page or filters change. Open each ended introduction to restore it.')}</p></div>`;
  }

  function currentCandidateBatch() {
    const batch = candidateBatch;
    if (!batch || !sameAdminView(batch.context) || query().get('job') !== batch.job.id) throw new Error(t('候选人页面已变化，请重新选择。', 'The candidate page changed. Select the people again.'));
    return batch;
  }

  function refreshCandidateSelection(batch) {
    const doc = globalThis.document;
    const counter = doc?.querySelector?.('#wb-bulk-count');
    if (counter) counter.textContent = t(`已选 ${batch.selected.size} 人`, `${batch.selected.size} selected`);
    const all = doc?.querySelector?.('[data-workbench-select-all]');
    if (all) {
      all.checked = batch.candidates.length > 0 && batch.selected.size === batch.candidates.length;
      all.indeterminate = batch.selected.size > 0 && batch.selected.size < batch.candidates.length;
      all.disabled = batch.busy || !batch.candidates.length;
    }
    for (const input of doc?.querySelectorAll?.('[data-workbench-candidate]') || []) {
      const eligible = batch.candidates.some(c => c.id === input.dataset.workbenchCandidate);
      input.checked = batch.selected.has(input.dataset.workbenchCandidate);
      input.disabled = batch.busy || !eligible;
    }
    const review = doc?.querySelector?.('[data-action="workbench-bulk-review"]');
    if (review) review.disabled = batch.busy || batch.selected.size === 0;
  }

  function selectWorkbenchCandidate(id, checked) {
    const batch = currentCandidateBatch();
    if (batch.busy) throw new Error(t('正在提交，请稍候。', 'The introduction request is in progress.'));
    if (!batch.candidates.some(c => c.id === id)) throw new Error(t('只能选择本页尚未建立过匹配的候选人。', 'Select a person on this page without a previous introduction.'));
    if (batch.selected.has(id) !== !!checked) {
      if (checked) batch.selected.add(id); else batch.selected.delete(id);
      batch.revision += 1;
    }
    refreshCandidateSelection(batch);
  }

  function selectWorkbenchPage(checked) {
    const batch = currentCandidateBatch();
    if (batch.busy) throw new Error(t('正在提交，请稍候。', 'The introduction request is in progress.'));
    batch.selected = new Set(checked ? batch.candidates.map(c => c.id) : []);
    batch.revision += 1;
    refreshCandidateSelection(batch);
  }

  function reviewCandidateBatch() {
    const batch = currentCandidateBatch();
    if (batch.busy) throw new Error(t('正在提交，请稍候。', 'The introduction request is in progress.'));
    const people = batch.candidates.filter(c => batch.selected.has(c.id));
    if (!people.length) throw new Error(t('请先选择候选人。', 'Select at least one person first.'));
    const review = { batch, revision: batch.revision, ids: people.map(c => c.id), token: ++bulkReviewSequence };
    bulkReview = review;
    openDialog(`<h2 id="dialog-title">${t(`核对 ${people.length} 人的介绍`, `Review ${people.length} introductions`)}</h2><p class="dialog-intro">${t('每位候选人各自与招聘方建立匹配。双方可查看彼此资料和介绍说明，平台私聊仍各自保密。', 'Each person gets a separate introduction to the employer. Each pair can see their profiles and this note. Private platform chats stay separate.')}</p><section class="wb-bulk-role"><span>${t('岗位 / 招聘方', 'Role / employer')}</span><h3>${esc(batch.job.title)}</h3><p>${esc(batch.job.employer_name)}</p></section><ol class="wb-bulk-review-list">${people.map(c => `<li>${esc(c.name)}</li>`).join('')}</ol><form data-form="workbench-bulk-introduce" data-bulk-review="${review.token}"><div class="field"><label for="wb-bulk-note">${t('统一介绍说明（选填）· 各匹配双方可见', 'Shared introduction note (optional) · visible to each pair')}</label><textarea class="input" id="wb-bulk-note" name="note" rows="3" maxlength="2000"></textarea></div><p class="help">${t('提交时会再次检查委托与岗位状态。若有人不符合条件，整批不会创建。', 'The server checks current briefs and the role again. If anyone is ineligible, the whole batch is rejected.')}</p><p class="wb-bulk-error" data-bulk-error role="alert" hidden></p><div class="form-actions"><button class="button secondary" type="button" data-action="close-dialog">${t('返回检查', 'Back')}</button><button class="button" type="submit">${t(`确认介绍 ${people.length} 人`, `Introduce ${people.length} people`)}${icon('arrow')}</button></div></form>`, { wide: true });
    review.dialog = currentDialog();
  }

  async function introduceCandidateBatch(form, data) {
    const review = bulkReview;
    const batch = currentCandidateBatch();
    if (!review || review.batch !== batch || review.revision !== batch.revision || String(review.token) !== form.dataset.bulkReview || form.isConnected === false || currentDialog() !== review.dialog) throw new Error(t('所选候选人或核对页面已变化，请重新核对后提交。', 'The selection or review changed. Review the selected people again.'));
    if (batch.busy) throw new Error(t('正在提交，请稍候。', 'The introduction request is in progress.'));
    const note = String(data.get('note') || '').trim();
    if (new TextEncoder().encode(note).length > 2000) throw new Error(t('介绍说明过长，请缩短后再试。', 'The introduction note is too long. Shorten it and try again.'));
    batch.busy = true;
    refreshCandidateSelection(batch);
    const errorBox = form.querySelector?.('[data-bulk-error]');
    if (errorBox) errorBox.hidden = true;
    try {
      const result = await api('/api/admin/workbench/introductions', { method: 'POST', body: { job_id: batch.job.id, seeker_ids: [...review.ids], note } });
      if (candidateBatch !== batch || bulkReview !== review || !sameAdminView(batch.context) || currentDialog() !== review.dialog || form.isConnected === false) return;
      candidateBatch = null;
      bulkReview = null;
      closeDialog();
      resetWorkbenchPages('candidates');
      resetWorkbenchPages('matches');
      await go(workbenchUrl({ candidate: null, candidates_cursor: null, matches_cursor: null }));
      notify(t(`已新建 ${Number(result.created || 0)} 个匹配，复用 ${Number(result.reused || 0)} 个已有匹配。`, `${Number(result.created || 0)} introductions created; ${Number(result.reused || 0)} existing introductions reused.`));
    } catch (error) {
      if (candidateBatch === batch && bulkReview === review && sameAdminView(batch.context) && errorBox) {
        errorBox.textContent = t('未能确认提交结果。选择和说明已保留，可重试；已有匹配不会重复创建。', 'The result could not be confirmed. Your selection and note are kept. You can retry; existing introductions will be reused.');
        errorBox.hidden = false;
      }
      throw error;
    } finally {
      batch.busy = false;
      if (candidateBatch === batch && sameAdminView(batch.context)) refreshCandidateSelection(batch);
    }
  }

  function candidatePanel(detail, job) {
    const { user, mandate: m, resume_html, match } = detail;
    const existing = match && match.status !== 'declined';
    const allowed = !user.banned && user.role === 'seeker' && m?.status === 'active';
    return `<aside class="wb-detail" data-page-key="candidate:${esc(job.id)}:${esc(user.id)}" aria-labelledby="wb-candidate-name"><div class="wb-detail-top"><a href="${esc(workbenchUrl({ candidate: null }))}" data-nav class="text-button">${icon('back')}${t('返回候选人', 'Back to candidates')}</a></div><header class="wb-person-heading">${avatar(user)}<div><h2 id="wb-candidate-name">${esc(user.name)}</h2><p>${t('求职者', 'Job seeker')}</p></div></header><a href="${esc(chatUrl(true, 'platform_' + user.id))}" data-nav class="button secondary small full">${icon('message')}${t('打开平台私聊', 'Open private platform chat')}</a>${user.public_email ? `<a class="wb-contact" href="mailto:${esc(user.public_email)}">${esc(user.public_email)}</a>` : ''}<section class="wb-brief"><h3>${t('寻找目标', 'Search brief')}</h3>${m ? `<dl><div><dt>${t('方向', 'Role')}</dt><dd>${esc(m.target_roles || '—')}</dd></div><div><dt>${t('行业', 'Industry')}</dt><dd>${esc(m.target_industry || '—')}</dd></div><div><dt>${t('月薪', 'Pay / mo')}</dt><dd>${salaryRange(m)}</dd></div><div><dt>${t('地点', 'Location')}</dt><dd>${esc(m.location || '—')}</dd></div><div><dt>${t('委托状态', 'Brief status')}</dt><dd>${mandateStatus(m.status)}</dd></div></dl>${m.preferences ? m.preferences.length > 180 ? `<details class="section-disclosure" data-disclosure="candidate-preferences:${esc(user.id)}"><summary><span>${t('补充偏好', 'Additional preferences')}<span class="help"> · ${esc(m.preferences.slice(0, 80))}…</span></span></summary><p class="wb-preferences">${esc(m.preferences)}</p></details>` : `<p class="wb-preferences">${esc(m.preferences)}</p>` : ''}<p class="help">${m.allow_external ? t('本人已授权平台外部联系。', 'External contact is authorized by this person.') : t('本人未授权平台外部联系。', 'External contact has not been authorized.')}</p>` : `<p class="help">${t('尚未提交委托。', 'No search brief has been submitted.')}</p>`}</section><details class="wb-resume section-disclosure" data-disclosure="candidate-resume:${esc(user.id)}" open><summary><h3>${t('简历', 'Résumé')}</h3></summary><div class="markdown">${resume_html || (user.resume_md ? `<p class="wb-preferences">${esc(user.resume_md)}</p>` : `<p class="help">${t('尚未填写简历。', 'No résumé has been added.')}</p>`)}</div></details><section class="wb-introduction"><h3>${existing ? t('已有匹配', 'Already introduced') : match ? t('重新介绍', 'Reintroduce') : t('介绍到这个岗位', 'Introduce to this role')}</h3><p class="wb-intro-role"><strong>${esc(job.title)}</strong><span>${esc(job.employer_name)} · ${money(job.salary_cents,job)} / ${t('月', 'mo')}</span></p>${existing ? `<p class="help">${t('这位候选人已在这个岗位下跟进，无需重复建立匹配。', 'This person already has an active introduction to this role.')}</p><button class="button secondary small" type="button" data-action="match-detail" data-id="${esc(match.id)}">${t('查看与跟进', 'View follow-up')}${icon('arrow')}</button>` : allowed ? `<form data-form="workbench-introduce" data-user="${esc(user.id)}" data-job="${esc(job.id)}"><div class="field"><label for="wb-introduction-note">${t('介绍说明 · 双方可见', 'Introduction note · visible to both sides')}</label><textarea id="wb-introduction-note" class="input" name="note" rows="4" maxlength="2000">${esc(match?.note || '')}</textarea></div><p class="help">${match ? t('重新介绍会恢复双方的资料访问及原匹配会话。平台私聊历史仍各自保密。', 'Reintroducing restores profile access and the existing shared room. Private platform histories stay separate.') : t('建立匹配后，双方可查看彼此资料并进入共同会话。平台私聊历史不会共享。', 'An introduction shares profiles and opens a shared room. Private platform histories are not shared.')}</p><button class="button full" type="submit">${match ? t('重新介绍', 'Reintroduce') : t('建立匹配', 'Make introduction')}${icon('arrow')}</button></form>` : notice(t('需要有效求职者账户与正在寻找的委托，才能建立或恢复匹配。', 'An active seeker account and active search brief are required to introduce or reintroduce.'))}</section></aside>`;
  }

  async function workbenchSearchView(requestGeneration) {
    const p = query();
    const jobs = await workbenchList('jobs', { q: p.get('job_q') }, requestGeneration);
    if (requestGeneration !== workbenchGeneration) return '';
    const jobId = p.get('job');
    let job = jobs.items.find(j => j.id === jobId);
    let content;
    if (!jobId) {
      content = `<section class="wb-candidates"><div class="wb-empty start"><span>${icon('briefcase')}</span><h2>${t('先选择一个岗位', 'Select a role first')}</h2><p>${t(`从左侧岗位队列开始，再按委托条件筛选候选人。每页最多显示 ${workbenchPageSize()} 条结果。`, `Choose a role from the queue, then filter candidates by their search briefs. Results are shown up to ${workbenchPageSize()} at a time.`)}</p></div></section>`;
    } else {
      if (!job) job = await api(`/api/admin/workbench/jobs/${encodeURIComponent(jobId)}`);
      if (requestGeneration !== workbenchGeneration) return '';
      const meta=currency(p.get('salary_currency')||job);
      const usingBasePay=p.get('salary_fit')==='1';
      const filters = { job_id: jobId, q: p.get('candidate_q'), industry: p.get('industry'), roles: p.get('roles'), location: p.get('location'), salary_min_cents: usingBasePay?job.salary_cents:optionalSalary(p.get('salary_min'),meta), salary_max_cents: usingBasePay?job.salary_cents:optionalSalary(p.get('salary_max'),meta) };
      if(usingBasePay)filters.salary_currency=currency(job).currency;
      else if(p.get('salary_currency')||filters.salary_min_cents!==null||filters.salary_max_cents!==null)filters.salary_currency=meta.currency;
      const candidates = await workbenchList('candidates', filters, requestGeneration);
      if (requestGeneration !== workbenchGeneration) return '';
      candidateBatch = { context: adminViewContext(), job, candidates: candidates.items.filter(c => !c.history_match), selected: new Set(), revision: 0, busy: false };
      const personId = p.get('candidate');
      let detail = null;
      if (personId) {
        detail = await api(`/api/admin/workbench/candidates/${encodeURIComponent(personId)}?job_id=${encodeURIComponent(jobId)}`);
        if (requestGeneration !== workbenchGeneration) return '';
        if (requestGeneration === workbenchGeneration && query().get('job') === jobId && query().get('candidate') === personId) selectedCandidate = { detail, job, generation: requestGeneration };
      }
      content = `<section class="wb-candidates"><header class="wb-selected-job"><a href="${esc(workbenchUrl({ job: null, candidate: null, candidates_cursor: null }))}" data-nav class="text-button">${icon('back')}${t('更换岗位', 'Change role')}</a><div><div><h2>${esc(job.title)}</h2><p>${esc(job.employer_name)}</p></div><strong>${money(job.salary_cents,job)}<small> / ${t('月', 'mo')}</small></strong></div><p class="wb-role-meta">${t(`计划招聘 ${Number(job.headcount)} 人`, `Hiring target: ${Number(job.headcount)}`)}<a href="${esc(workbenchUrl({ view: 'followups', follow_job: job.id, follow_q: null, status: null, matches_cursor: null }))}" data-nav>${t(`查看 ${Number(job.active_matches || 0)} 位跟进中的人`, `View ${Number(job.active_matches || 0)} active introductions`)} →</a></p>${job.jd_excerpt ? `<details class="wb-role-description section-disclosure" data-disclosure="role-summary:${esc(job.id)}"><summary>${t('查看岗位摘要', 'Role summary')}</summary><p>${esc(job.jd_excerpt)}</p><button type="button" class="text-button" data-action="admin-edit-job" data-id="${esc(job.id)}">${t('查看完整岗位', 'View full role')} →</button></details>` : ''}</header>${candidateFilters(job)}<div class="wb-results-heading"><h3>${t('候选人', 'Candidates')}<span>${candidates.total}</span></h3><p>${t('有效账户 · 正在寻找 · 尚未在此岗位跟进', 'Active accounts and search briefs · not currently introduced to this role')}</p></div>${candidateTable(candidates, personId)}</section>${detail ? candidatePanel(detail, job) : ''}`;
    }
    return `<div class="wb-workspace ${jobId ? 'has-job' : ''} ${p.get('candidate') && jobId ? 'has-detail' : ''}">${workbenchJobQueue(jobs, jobId)}${content}</div>`;
  }

  function resetWorkbenchPages(kind) {
    workbenchPages.delete(kind);
    for (const key of workbenchHistory.keys()) if (key.startsWith(kind + '?')) workbenchHistory.delete(key);
  }

  async function introduceCandidate(form, data) {
    const chosen = selectedCandidate;
    if (!state.admin || !chosen || chosen.generation !== workbenchGeneration || chosen.detail.user.id !== form.dataset.user || chosen.job.id !== form.dataset.job || query().get('job') !== chosen.job.id || query().get('candidate') !== chosen.detail.user.id) throw new Error(t('选择已更新，请重新打开候选人详情。', 'The selection changed. Open this candidate again.'));
    const { detail, job } = chosen;
    if (detail.match && detail.match.status !== 'declined') throw new Error(t('这位候选人已在该岗位跟进，请打开已有匹配。', 'This person is already introduced to this role. Open the existing follow-up.'));
    const note = String(data.get('note') || '').trim();
    if (note.length > 2000) throw new Error(t('介绍说明不能超过 2,000 字。', 'Keep the introduction note within 2,000 characters.'));
    if (detail.match) await api(`/api/admin/matches/${encodeURIComponent(detail.match.id)}`, { method: 'PATCH', body: { status: 'introduced', note } });
    else await api('/api/admin/matches', { method: 'POST', body: { seeker_id: detail.user.id, job_id: job.id, note } });
    if (!state.admin || selectedCandidate !== chosen || chosen.generation !== workbenchGeneration || query().get('job') !== job.id || query().get('candidate') !== detail.user.id) return;
    selectedCandidate = null;
    resetWorkbenchPages('candidates');
    resetWorkbenchPages('matches');
    await go(workbenchUrl({ candidate: null, candidates_cursor: null, matches_cursor: null }));
    notify(t('匹配已建立，可在「匹配跟进」继续处理。', 'Introduction saved. Continue in Follow-ups.'));
  }

  async function workbenchFollowups(requestGeneration) {
    const p = query();
    const filters = { q: p.get('follow_q'), status: p.get('status'), job_id: p.get('follow_job'), seeker_id: p.get('follow_seeker'), employer_id: p.get('follow_employer') };
    const page = await workbenchList('matches', filters, requestGeneration);
    const counts = page.stage_counts || {};
    const totalStages = statuses.reduce((sum, stage) => sum + Number(counts[stage] || 0), 0);
    const narrowed = !!(filters.job_id || filters.seeker_id || filters.employer_id);
    return `<section class="wb-followups"><div class="wb-stage-tabs" aria-label="${t('按匹配阶段筛选', 'Filter by introduction stage')}">${[['', t('全部', 'All'), totalStages], ...statuses.map(stage => [stage, matchText(stage), Number(counts[stage] || 0)])].map(([stage, label, count]) => `<a href="${esc(workbenchUrl({ status: stage, matches_cursor: null }))}" data-nav ${(!filters.status && !stage) || filters.status === stage ? 'aria-current="page"' : ''}>${label}<span>${count}</span></a>`).join('')}</div><form data-form="workbench-followups" class="wb-followup-search"><label class="sr-only" for="wb-followup-q">${t('搜索匹配', 'Search introductions')}</label><input id="wb-followup-q" class="input" name="q" maxlength="120" value="${esc(filters.q || '')}" placeholder="${esc(t('求职者、岗位或招聘方', 'Job seeker, role, or employer'))}"><input type="hidden" name="status" value="${esc(filters.status || '')}"><button type="submit" class="button secondary small">${icon('search')}${t('搜索', 'Search')}</button></form>${narrowed ? `<div class="wb-filter-notice"><span>${t('当前仅显示所选岗位或参与者的匹配。', 'Showing introductions for the selected role or participant.')}</span><a href="${esc(workbenchUrl({ follow_job: null, follow_seeker: null, follow_employer: null, matches_cursor: null }))}" data-nav class="text-button">${t('清除限定', 'Clear selection')} ×</a></div>` : ''}${page.items.length ? `<div class="wb-table-wrap"><table class="wb-table wb-followup-table"><thead><tr><th>${t('求职者', 'Job seeker')}</th><th>${t('岗位 / 招聘方', 'Role / employer')}</th><th>${t('阶段', 'Stage')}</th><th>${t('最近更新', 'Last update')}</th><th>${t('操作', 'Actions')}</th></tr></thead><tbody>${page.items.map(m => `<tr><td><strong>${esc(m.seeker_name)}</strong></td><td><strong>${esc(m.job_title)}</strong><span>${esc(m.employer_name)}</span></td><td><span class="status ${m.status === 'declined' ? 'closed' : ''}">${matchText(m.status)}</span></td><td><time>${humanTime(m.updated_at || m.created_at)}</time></td><td><div class="wb-row-actions"><button type="button" class="text-button" data-action="match-detail" data-id="${esc(m.id)}">${t('跟进', 'Follow up')} →</button><a class="text-button" href="${esc(chatUrl(true, m.conversation_id || 'match_' + m.id))}" data-nav>${icon('message')}${m.status === 'declined' ? t('历史', 'History') : t('会话', 'Room')}</a></div></td></tr>`).join('')}</tbody></table></div>` : `<div class="wb-empty"><span>${icon('link')}</span><h3>${t('没有符合条件的匹配', 'No introductions found')}</h3><p>${t('调整搜索或阶段筛选，或从「筛选候选人」建立匹配。', 'Change the search or stage filter, or make an introduction from Find candidates.')}</p></div>`}${workbenchPager('matches', page)}</section>`;
  }

  async function matchingWorkbench() {
    const requestGeneration = ++workbenchGeneration;
    selectedCandidate = null;
    candidateBatch = null;
    bulkReview = null;
    const view = query().get('view') === 'followups' ? 'followups' : 'search';
    let content;
    try { content = view === 'followups' ? await workbenchFollowups(requestGeneration) : await workbenchSearchView(requestGeneration); }
    catch (error) {
      if (requestGeneration !== workbenchGeneration) return '';
      if (error.status === 401 || error.status === 403) throw error;
      content = `<section class="panel wb-error"><h2>${t('暂时无法读取这些结果', 'These results could not be loaded')}</h2>${notice(esc(error.message), 'warning')}<div class="form-actions"><button class="button secondary small" type="button" data-action="retry">${t('重试', 'Retry')}</button><a href="${esc(route(true, 'matches'))}" data-nav class="text-button">${t('清除选择与筛选，重新开始', 'Clear selection and filters')} →</a></div></section>`;
    }
    if (requestGeneration !== workbenchGeneration) return '';
    return shell(`<div class="matching-workbench">${heading(t('匹配工作台', 'Matching workspace'), t('按岗位查找候选人，逐个查看资料并跟进匹配。', 'Search candidates for a role, review profiles, and follow up on introductions.'), '', 'MATCHING')}${workbenchTabs(view)}${content}</div>`, 'matches', true);
  }

  const adminViewContext = () => ({ generation: workbenchGeneration, pathname: location.pathname, search: location.search });
  const sameAdminView = context => state.admin && !state.bootstrap && context.generation === workbenchGeneration && context.pathname === location.pathname && context.search === location.search;
  const currentDialog = () => globalThis.document?.querySelector?.('.dialog') || null;

  async function matchDetail(id) {
    if (!state.admin) return;
    const epoch = ++matchDetailEpoch;
    const context = adminViewContext();
    const previousDialog = currentDialog();
    activeMatchDetail = null;
    const m = await api(`/api/admin/workbench/matches/${encodeURIComponent(id)}`);
    if (epoch !== matchDetailEpoch || !sameAdminView(context) || currentDialog() !== previousDialog) return;
    activeMatchDetail = { id: m.id, epoch, context };
    openDialog(`<div class="eyebrow">${t('匹配记录', 'INTRODUCTION RECORD')}</div><h2 id="dialog-title">${t('匹配跟进', 'Introduction follow-up')}</h2><div class="wb-followup-context"><section><span>${t('求职者', 'Job seeker')}</span><h3>${esc(m.seeker_name)}</h3><div><button class="text-button" type="button" data-action="admin-edit-user" data-id="${esc(m.seeker_id)}">${t('查看资料', 'View profile')}</button><a href="${esc(chatUrl(true, m.seeker_conversation_id || 'platform_' + m.seeker_id))}" data-nav class="text-button">${t('平台私聊', 'Private chat')} ↗</a></div></section><span class="wb-followup-link" aria-hidden="true">${icon('link')}</span><section><span>${t('岗位 / 招聘方', 'Role / employer')}</span><h3>${esc(m.job_title)}</h3><p>${esc(m.employer_name)}</p><div><button class="text-button" type="button" data-action="admin-edit-job" data-id="${esc(m.job_id)}">${t('查看岗位', 'View role')}</button><a href="${esc(chatUrl(true, m.employer_conversation_id || 'platform_' + m.employer_id))}" data-nav class="text-button">${t('平台私聊', 'Private chat')} ↗</a></div></section></div><div class="wb-followup-room"><span class="help">${t('最近更新', 'Last updated')} ${humanTime(m.updated_at || m.created_at)}</span><a href="${esc(chatUrl(true, m.conversation_id || 'match_' + m.id))}" data-nav class="button secondary small">${icon('message')}${m.status === 'declined' ? t('查看历史会话', 'View room history') : t('打开匹配会话', 'Open shared room')}</a></div><form data-form="match" data-id="${esc(m.id)}" data-detail-epoch="${epoch}"><div class="field"><label>${t('跟进进度', 'Progress')}<select class="input" name="status">${statuses.map(s => `<option value="${s}" ${s === m.status ? 'selected' : ''}>${matchText(s)}</option>`).join('')}</select></label></div><div class="field"><label>${t('跟进说明 · 匹配双方可见', 'Follow-up note · visible to both sides')}<textarea class="input" name="note" rows="4" maxlength="2000">${esc(m.note)}</textarea></label></div><p class="help">${m.status === 'declined' ? t('重新介绍需要有效委托与正在招聘的已付款岗位，并恢复双方对原会话的访问。', 'Reintroducing requires an active brief and a paid role that is hiring. It restores access to the existing room.') : t('结束后，双方不再访问资料和匹配会话。管理员保留只读历史。', 'Ending an introduction closes participant access to profiles and the shared room. Administrators retain read-only history.')}${t('平台私聊历史始终分开。', ' Private platform histories stay separate.')}</p><div class="form-actions"><button class="button" type="submit">${t('保存跟进', 'Save follow-up')}${icon('check')}</button></div></form>`, { wide: true });
  }

  async function saveFollowup(form, data) {
    const selected = activeMatchDetail;
    if (!selected || selected.id !== form.dataset.id || String(selected.epoch) !== form.dataset.detailEpoch || selected.epoch !== matchDetailEpoch || !sameAdminView(selected.context) || form.isConnected === false) throw new Error(t('匹配详情已变化，请重新打开后保存。', 'The introduction detail changed. Open it again before saving.'));
    const status = String(data.get('status') || '');
    if (!statuses.includes(status)) throw new Error(t('请选择有效的跟进阶段。', 'Choose a valid follow-up stage.'));
    const note = String(data.get('note') || '');
    if (note.length > 2000) throw new Error(t('跟进说明不能超过 2,000 字。', 'Keep the follow-up note within 2,000 characters.'));
    await api(`/api/admin/matches/${encodeURIComponent(selected.id)}`, { method: 'PATCH', body: { status, note } });
    if (activeMatchDetail !== selected || selected.epoch !== matchDetailEpoch || !sameAdminView(selected.context) || form.isConnected === false) return;
    activeMatchDetail = null;
    matchDetailEpoch += 1;
    closeDialog();
    await savedFollowup();
    notify(t('对接记录已保存。', 'Introduction saved.'));
  }

  async function newMatch() { return go(route(true, 'matches')); }
  async function savedFollowup() {
    resetWorkbenchPages('matches');
    resetWorkbenchPages('candidates');
    return go(workbenchUrl({ view: 'followups', matches_cursor: null, candidates_cursor: null }));
  }

  const webhookEvents = ['message.created', 'match.updated', 'job.updated', 'order.updated', 'mandate.updated', 'outreach.updated'];
  const webhookEventText = event => ({
    'message.created': t('新消息', 'New message'),
    'match.updated': t('匹配进展', 'Introduction updated'),
    'job.updated': t('岗位变化', 'Role updated'),
    'order.updated': t('订单变化', 'Order updated'),
    'mandate.updated': t('委托变化', 'Search brief updated'),
    'outreach.updated': t('寻找进展', 'Outreach updated'),
    'webhook.test': t('测试通知', 'Test notification'),
  })[event] || event;
  const deliveryText = status => ({ pending: t('等待投递', 'Queued'), processing: t('投递中', 'Sending'), delivered: t('已送达', 'Delivered'), failed: t('投递失败', 'Failed'), cancelled: t('已取消', 'Cancelled') })[status] || status;
  const deliveryErrorText = code => ({
    authorization_revoked: t('访问权限已撤销，此事件不再投递。', 'Access was revoked. This event will not be delivered.'),
    destination_blocked: t('接收地址不符合公网 HTTPS 要求，投递已阻止。', 'The destination does not meet the public HTTPS requirements. Delivery was blocked.'),
    dns_failed: t('暂时无法解析接收服务的域名。', 'The receiver’s domain could not be resolved.'),
    timeout: t('接收服务响应超时。', 'The receiver did not respond in time.'),
    network_error: t('暂时无法连接接收服务。', 'The receiver could not be reached.'),
    response_too_large: t('接收服务返回的响应过大。', 'The receiver returned an oversized response.'),
    http_error: t('接收服务返回错误，请检查 HTTP 状态和接收端日志。', 'The receiver returned an error. Check its HTTP status and receiver logs.'),
    worker_interrupted: t('投递任务曾中断，结果尚未确认。', 'The delivery task was interrupted before its result was confirmed.'),
  })[code] || t('投递未完成，请检查接收服务后重试。', 'Delivery did not complete. Check the receiver before retrying.');
  const deliveryBadge = status => `<span class="status webhook-status ${esc(status)}">${esc(deliveryText(status))}</span>`;
  const webhookPath = id => '/api/webhooks/' + encodeURIComponent(id);

  function webhookCard(endpoint, latest, failedToLoad = false) {
    return `<article class="webhook-card"><div class="webhook-card-heading"><div><h3>${esc(endpoint.name)}</h3><code class="webhook-url">${esc(endpoint.url)}</code></div><span class="status ${endpoint.enabled ? 'active' : 'closed'}">${endpoint.enabled ? t('已启用', 'Enabled') : t('已停用', 'Disabled')}</span></div><div class="webhook-event-tags">${(endpoint.events || []).map(event => `<span class="pill" title="${esc(event)}">${esc(webhookEventText(event))}</span>`).join('')}</div><div class="webhook-latest"><span>${t('最近投递', 'Latest delivery')}</span>${failedToLoad ? `<span class="muted">${t('暂时无法读取，请打开历史重试。', 'Unavailable. Open delivery history to retry.')}</span>` : latest ? `${deliveryBadge(latest.status)}<span>${esc(webhookEventText(latest.event_type))}</span><time>${esc(humanTime(latest.created_at))}</time>` : `<span class="muted">${t('还没有事件', 'No events yet')}</span>`}</div>${endpoint.dropped_events ? `<p class="webhook-warning">${t('队列达到限额，部分通知未排入投递：', 'Some notifications could not be queued because the queue limit was reached: ')}${Number(endpoint.dropped_events)}${t('。请在工作台核对最新状态。', '. Check the workspace for the latest state.')}</p>` : ''}<div class="webhook-card-actions"><button class="text-button" data-action="webhook-deliveries" data-id="${esc(endpoint.id)}">${t('投递记录', 'Delivery history')} →</button><button class="text-button" data-action="webhook-test" data-id="${esc(endpoint.id)}" ${webhookSettings.enabled && endpoint.enabled ? '' : 'disabled'}>${t('发送测试', 'Send a test')}</button><button class="text-button" data-action="webhook-edit" data-id="${esc(endpoint.id)}" ${webhookSettings.enabled ? '' : 'disabled'}>${t('编辑事件', 'Edit events')}</button><button class="text-button" data-action="webhook-toggle" data-id="${esc(endpoint.id)}" ${!webhookSettings.enabled && !endpoint.enabled ? 'disabled' : ''}>${endpoint.enabled ? t('停用', 'Disable') : t('启用', 'Enable')}</button><button class="text-button danger-text" data-action="webhook-delete" data-id="${esc(endpoint.id)}">${t('删除', 'Delete')}</button></div></article>`;
  }

  async function webhooksPage() {
    const data = await api('/api/webhooks');
    const endpoints = data.items || [];
    webhookSettings = { enabled: !!data.enabled, max_endpoints: data.limits?.max_endpoints ?? 5, max_attempts: data.limits?.max_attempts ?? 5 };
    webhookCache.clear();
    endpoints.forEach(endpoint => webhookCache.set(endpoint.id, endpoint));
    const latest = await Promise.allSettled(endpoints.map(endpoint => api(webhookPath(endpoint.id) + '/deliveries?limit=1')));
    const canCreate = webhookSettings.enabled && endpoints.length < webhookSettings.max_endpoints;
    const addButton = `<button class="button secondary small" data-action="webhook-new" ${canCreate ? '' : 'disabled'}>${icon('plus')}${t('注册端点', 'Register endpoint')}</button>`;
    return shell(`${heading(t('让进展，接入你的工作流。', 'Keep your own tools in the loop.'), t('手动注册接收地址，在消息、匹配和订单变化时收到通知。', 'Register a receiver yourself to hear about messages, introductions, and order changes.'), '', 'YOUR WEBHOOKS')}${!webhookSettings.enabled ? notice(t('联动投递暂未开放。已有端点和历史可以查看，也可以停用或删除端点。', 'Webhook delivery is currently paused. You can still review history, disable endpoints, or delete them.'), 'warning') : ''}<section class="panel"><div class="panel-header"><div><h2>${t('我的接收端点', 'Your receiving endpoints')}</h2><p>${endpoints.length} / ${webhookSettings.max_endpoints} ${t('个端点', 'endpoints')}</p></div>${addButton}</div>${endpoints.length ? `<div class="webhook-list">${endpoints.map((endpoint, index) => webhookCard(endpoint, latest[index].status === 'fulfilled' ? latest[index].value.items?.[0] : null, latest[index].status === 'rejected')).join('')}</div>` : empty(t('你决定，通知送到哪里。', 'You choose where notifications go.'), t('注册自己的 HTTPS 接收地址后，相关事件才会投递。', 'Notifications start only after you register your own HTTPS receiver.'), '', 'link')}${endpoints.length >= webhookSettings.max_endpoints ? `<p class="help">${t('已达到端点数量上限。删除不再使用的端点后可以新增。', 'You have reached the endpoint limit. Remove an unused endpoint to add another.')}</p>` : ''}</section><section class="panel webhook-guide"><div class="panel-header"><div><h2>${t('只通知必要的变化。', 'Only the change you need to know.')}</h2><p>${t('只投递你有权访问的对象，不包含简历、联系邮箱或聊天正文。', 'Only objects you may access are included. Notifications do not contain résumés, contact emails, or message bodies.')}</p></div></div><details class="section-disclosure" data-disclosure="webhook-guide:${endpoints.length ? 'configured' : 'new'}" ${endpoints.length ? '' : 'open'}><summary>${t('接入说明与验签', 'Setup and signature verification')}</summary><div class="disclosure-content"><div class="webhook-principles"><div>${icon('link')}<p><strong>${t('手动接入', 'Register manually')}</strong>${t('端点由你亲自注册。接收地址须为公网 HTTPS，使用 443 端口。', 'You register each endpoint yourself. Use a public HTTPS receiver on port 443.')}</p></div><div>${icon('key')}<p><strong>${t('验证 HMAC 签名', 'Verify the HMAC signature')}</strong>${t('创建时保存签名密钥，接收端先验签，再处理事件。密钥只显示一次。', 'Save the signing secret when you create an endpoint. Verify each request before processing it. The secret is shown once.')}</p></div><div>${icon('receipt')}<p><strong>${t('验签后，按事件去重', 'Deduplicate verified events')}</strong>${t(`请按至少一次（at-least-once）的接收方式处理通知：可能重试、重复或乱序。每次自动投递最多尝试 ${webhookSettings.max_attempts} 次，以接收端标识加已验签正文的 body.id 持久去重。`, `Handle notifications using at-least-once semantics: retries, duplicates, and out-of-order events are possible. Automatic delivery attempts are limited to ${webhookSettings.max_attempts}; persistently deduplicate using your receiver ID plus body.id from the verified payload.`)}</p></div></div><p class="help">${t('通知仅带事件 ID、对象 ID 与状态等必要元数据。需要完整内容时，通过你的已授权会话或 AI 连接读取。', 'Notifications carry only event IDs, object IDs, statuses, and necessary metadata. Retrieve full content through your authorized session or AI connection.')}</p></div></details></section>`, 'webhooks');
  }

  function webhookDialog(id = '') {
    const endpoint = id ? webhookCache.get(id) : null;
    if (id && !endpoint) throw new Error(t('端点记录已过期，请刷新页面。', 'Refresh the page to reload this endpoint.'));
    if (!webhookSettings.enabled) throw new Error(t('联动投递暂未开放。', 'Webhook delivery is currently paused.'));
    if (!id && webhookCache.size >= webhookSettings.max_endpoints) throw new Error(t('已达到端点数量上限。', 'The endpoint limit has been reached.'));
    const available = webhookEvents.filter(event => state.user?.role !== 'employer' || !['mandate.updated', 'outreach.updated'].includes(event) || endpoint?.events?.includes(event));
    const selected = endpoint?.events || ['message.created', 'match.updated'];
    openDialog(`<div class="eyebrow">CONNECT ON YOUR TERMS</div><h2 id="dialog-title">${id ? t('调整这条联动。', 'Adjust this connection.') : t('接上你的工作流。', 'Connect your own workflow.')}</h2><p class="dialog-intro">${t('只向你指定的地址投递必要的事件信息。不会发送简历和聊天正文。', 'Send necessary event metadata to the address you choose. Résumés and message bodies are never included.')}</p><form data-form="webhook" data-id="${esc(id)}"><div class="field"><label for="webhook-name">${t('端点名称', 'Endpoint name')}</label><input class="input" id="webhook-name" name="name" maxlength="100" value="${esc(endpoint?.name || '')}" placeholder="${esc(t('例如：我的自动化工作流', 'For example: My automation workflow'))}" required></div><div class="field"><label for="webhook-url">${t('HTTPS 接收地址', 'HTTPS receiver URL')}</label><input class="input" id="webhook-url" name="url" type="url" maxlength="2000" value="${esc(endpoint?.url || '')}" placeholder="https://example.com/hooks/hireferry" ${id ? 'readonly' : 'required'} autocomplete="off" spellcheck="false"><p class="help">${id ? t('更换地址需要注册新端点，并重新保存签名密钥。', 'Register a new endpoint and save its new signing secret to change the URL.') : t('仅支持公网 HTTPS / 443。请使用你自己控制的接收服务。', 'Public HTTPS on port 443 only. Use a receiver you control.')}</p></div><fieldset class="webhook-event-fields"><legend>${t('订阅事件', 'Events to receive')}</legend><div class="webhook-event-options">${available.map(event => `<label class="toggle-row"><input type="checkbox" name="events" value="${event}" ${selected.includes(event) ? 'checked' : ''}><span><strong>${esc(webhookEventText(event))}</strong><small>${event}</small></span></label>`).join('')}</div><p class="help">${t('至少选择一种。只通知你本人有权访问的对象；岗位变化包含已匹配的岗位。', 'Choose at least one. Only objects you may access are sent, including roles introduced to you.')}</p></fieldset><div class="form-actions"><button class="button secondary" type="button" data-action="close-dialog">${t('取消', 'Cancel')}</button><button class="button" type="submit">${id ? t('保存事件', 'Save events') : t('注册并生成密钥', 'Register and create secret')}${icon('arrow')}</button></div></form>`, { wide: true });
  }

  async function saveWebhook(form, data) {
    const id = form.dataset.id;
    const events = [...new Set(data.getAll('events').map(String))].filter(event => webhookEvents.includes(event));
    if (!events.length) throw new Error(t('请至少选择一种事件。', 'Choose at least one event.'));
    const body = { name: String(data.get('name') || '').trim(), events };
    if (!body.name) throw new Error(t('请填写端点名称。', 'Enter an endpoint name.'));
    if (!id) {
      let url;
      try { url = new URL(String(data.get('url') || '').trim()); } catch { throw new Error(t('请填写有效的 HTTPS 地址。', 'Enter a valid HTTPS URL.')); }
      if (url.protocol !== 'https:' || (url.port && url.port !== '443') || url.username || url.password || url.hash) throw new Error(t('地址须为 HTTPS / 443，不能包含账号密码或 # 片段。', 'Use HTTPS on port 443, without credentials or a fragment.'));
      body.url = url.href;
    }
    const result = await api(id ? webhookPath(id) : '/api/webhooks', { method: id ? 'PATCH' : 'POST', body });
    closeDialog();
    await render();
    if (id) { notify(t('联动事件已更新。', 'Webhook events updated.')); return; }
    // The secret exists only in this transient dialog. Never put it in attributes,
    // endpoint caches, localStorage, or subsequent list/history markup.
    openDialog(`<div class="eyebrow">SAVE THIS ONCE</div><h2 id="dialog-title">${t('请保存这把签名密钥。', 'Save this signing secret.')}</h2><p class="dialog-intro">${t('接收服务需要它来验证 HMAC 签名。关闭后不能再次查看；如遗失，需要重新注册端点。', 'Your receiver needs this secret to verify HMAC signatures. You cannot view it after closing; register a new endpoint if it is lost.')}</p><div class="webhook-secret"><div class="field-label" id="webhook-secret-label">${t('签名密钥 · 仅显示一次', 'Signing secret · shown once')}</div><code id="webhook-signing-secret" tabindex="0" aria-labelledby="webhook-secret-label">${esc(result.signing_secret)}</code><button class="text-button" type="button" data-action="webhook-copy-secret">${icon('copy')}${t('复制签名密钥', 'Copy signing secret')}</button></div><p class="help">${t('请保存到接收服务的私密配置中，不要放入公开仓库或聊天消息。', 'Keep it in your receiver’s private configuration, away from public repositories and chat messages.')}</p><div class="form-actions"><button type="button" class="button" data-action="close-dialog">${t('已保存，关闭', 'Saved. Close')}${icon('check')}</button></div>`);
  }

  async function copyWebhookSecret() {
    const secret = document.querySelector('#webhook-signing-secret');
    if (!secret) return;
    try {
      if (!navigator.clipboard?.writeText) throw new Error('Clipboard unavailable');
      await navigator.clipboard.writeText(secret.textContent);
      notify(t('签名密钥已复制。', 'Signing secret copied.'));
    } catch {
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(secret);
      selection?.removeAllRanges();
      selection?.addRange(range);
      secret.focus();
      notify(t('已选中密钥。请使用系统复制操作保存。', 'Secret selected. Use your device’s Copy action to save it.'));
    }
  }

  async function toggleWebhook(id) {
    const endpoint = webhookCache.get(id);
    if (!endpoint) throw new Error(t('请刷新页面后重试。', 'Refresh the page and try again.'));
    await api(webhookPath(id), { method: 'PATCH', body: { enabled: !endpoint.enabled } });
    await render();
    notify(endpoint.enabled ? t('端点已停用。', 'Endpoint disabled.') : t('端点已启用。', 'Endpoint enabled.'));
  }

  async function deleteWebhook(id) {
    await api(webhookPath(id), { method: 'DELETE' });
    await render();
    notify(t('端点已删除，签名密钥已撤销。', 'Endpoint deleted and signing secret revoked.'));
  }

  async function webhookDeliveries(id) {
    const endpoint = webhookCache.get(id);
    if (!endpoint) throw new Error(t('请刷新页面后重试。', 'Refresh the page and try again.'));
    const data = await api(webhookPath(id) + '/deliveries?limit=50');
    const canSend = webhookSettings.enabled && endpoint.enabled;
    openDialog(`<div class="eyebrow">DELIVERY HISTORY</div><h2 id="dialog-title">${esc(endpoint.name)}</h2><p class="dialog-intro">${t('最近 50 次投递。发送测试只会排队；以这里的实际结果为准。', 'The latest 50 deliveries. Sending a test only queues it; check the actual result here.')}</p><div class="webhook-history-toolbar"><code class="webhook-url">${esc(endpoint.url)}</code><button class="text-button" data-action="webhook-deliveries" data-id="${esc(id)}">${t('刷新记录', 'Refresh')}</button></div><div class="webhook-deliveries">${data.items?.length ? data.items.map(delivery => `<article class="webhook-delivery"><div class="webhook-delivery-heading"><div><h3>${esc(webhookEventText(delivery.event_type))}</h3><time>${esc(humanTime(delivery.created_at))}</time></div>${deliveryBadge(delivery.status)}</div><details class="section-disclosure" data-disclosure="webhook-delivery:${esc(id)}:${esc(delivery.id)}"><summary>${t('投递详情', 'Delivery details')}</summary><dl class="webhook-delivery-meta"><div><dt>${t('投递 ID', 'Delivery ID')}</dt><dd><code>${esc(delivery.id)}</code></dd></div><div><dt>${t('事件 ID', 'Event ID')}</dt><dd><code>${esc(delivery.event_id)}</code></dd></div><div><dt>${t('尝试次数', 'Attempts')}</dt><dd>${Number(delivery.attempts || 0)} / ${Number(delivery.max_attempts || webhookSettings.max_attempts)}${delivery.last_status_code ? ` · HTTP ${Number(delivery.last_status_code)}` : ''}</dd></div>${delivery.delivered_at ? `<div><dt>${t('送达时间', 'Delivered')}</dt><dd>${esc(humanTime(delivery.delivered_at))}</dd></div>` : delivery.next_attempt_at && ['pending', 'processing'].includes(delivery.status) ? `<div><dt>${t('下次尝试', 'Next attempt')}</dt><dd>${esc(humanTime(delivery.next_attempt_at))}</dd></div>` : ''}</dl></details>${delivery.last_error ? `<p class="webhook-warning">${esc(deliveryErrorText(delivery.last_error))}</p>` : ''}${delivery.status === 'failed' ? `<button class="text-button" data-action="webhook-retry" data-id="${esc(id)}" data-delivery="${esc(delivery.id)}" ${canSend ? '' : 'disabled'}>${t('重试这次投递', 'Retry this delivery')} →</button>` : ''}</article>`).join('') : empty(t('还没有投递记录。', 'No deliveries yet.'), t('订阅事件发生后会出现在这里。也可以发送一条测试通知。', 'Subscribed events will appear here. You can also send a test notification.'), '', 'receipt')}</div><div class="form-actions"><p class="help">${t('重试前会重新检查权限。已经撤权或归档的对象不会继续投递。', 'Access is checked again before retrying. Revoked or archived objects are not delivered.')}</p><button class="button secondary small" data-action="webhook-test" data-id="${esc(id)}" ${canSend ? '' : 'disabled'}>${t('发送测试', 'Send a test')}</button></div>`, { wide: true });
  }

  async function testWebhook(id) {
    await api(webhookPath(id) + '/test', { method: 'POST', body: {} });
    await webhookDeliveries(id);
    notify(t('测试通知已排队，请刷新查看投递结果。', 'Test notification queued. Refresh to check the result.'));
  }

  async function retryWebhook(id, deliveryId) {
    await api(webhookPath(id) + '/deliveries/' + encodeURIComponent(deliveryId) + '/retry', { method: 'POST', body: {} });
    await webhookDeliveries(id);
    notify(t('已重新排队，请刷新查看投递结果。', 'Delivery queued again. Refresh to check the result.'));
  }

  async function adminTokens() {
    const d = await api('/api/admin/tokens');
    return shell(`${heading(t('让平台 AI，协助每次连接。', 'Give your platform AI a clear role.'), t('独立的管理员授权，用于平台业务与跟进。', 'Separate administrator authorization for service operations and follow-ups.'), '', 'PLATFORM AI')}<section class="panel"><div class="panel-header"><div><h2>${t('平台业务访问令牌', 'Platform business access token')}</h2><p>${t('与求职者、招聘方的个人令牌分开管理。', 'Managed separately from job seekers’ and employers’ personal tokens.')}</p></div><span class="pill">${icon('shield')} ADMIN MCP</span></div>${notice(t('此令牌可管理用户资料、岗位、匹配、委托、寻找记录和平台消息，并查看订单统计。它代表平台发言，AI 发送的消息会明确标记。', 'This token can manage profiles, roles, introductions, search briefs, outreach records, and platform messages, and read order statistics. It speaks for HireFerry; AI messages are clearly labeled.'))}<div class="field"><label>${t('MCP 服务地址', 'MCP server URL')}</label><div class="code-box">${esc(location.origin)}/mcp</div></div><p class="help">${t('使用 HTTP 连接，并将令牌作为 Authorization: Bearer 请求头。系统配置与通行密钥管理不对 AI 开放。', 'Connect over HTTP with an Authorization: Bearer header. System configuration and passkey management are not available to AI.')}</p><form data-form="admin-token" class="token-create-form"><div class="field"><label for="admin-token-name">${t('连接名称', 'Connection name')}</label><input class="input" id="admin-token-name" name="name" maxlength="80" required placeholder="${esc(t('例如：平台招聘助手', 'For example: Platform hiring assistant'))}"></div><button class="button secondary" type="submit">${icon('plus')}${t('创建平台令牌', 'Create platform token')}</button></form><div class="token-reauth"><p class="help">${t('创建令牌需要在最近 10 分钟内完成通行密钥验证。', 'Creating a token requires a passkey check within the last 10 minutes.')}</p><button class="text-button" data-action="admin-reauth">${icon('key')}${t('重新验证通行密钥', 'Verify passkey again')}</button></div><div id="new-token"></div></section><section class="panel"><div class="panel-header"><h2>${t('已授权的平台 AI', 'Authorized platform AI')}</h2></div>${d.items?.length ? `<div class="list">${d.items.map(k => `<div class="list-row"><div><h3>${esc(k.name)}</h3><div class="list-meta"><span>${t('创建于', 'Created')} ${date(k.created_at)}</span><span>${t('到期', 'Expires')} ${date(k.expires_at)}</span><span>${t('最近使用', 'Last used')} ${date(k.last_used_at)}</span></div></div><button class="text-button" data-action="revoke-admin-token" data-id="${esc(k.id)}">${t('立即撤销', 'Revoke now')}</button></div>`).join('')}</div>` : empty(t('尚未授权平台 AI。', 'No platform AI is authorized yet.'), t('为可信的助手单独创建令牌，并妥善保管。', 'Create a separate token for a trusted assistant and keep it private.'), '', 'bot')}</section>`, 'agents', true);
  }

  return { messagesPage, mountChat, stopChat, showPendingChat, navigateChat, sendMessage, olderMessages, scrollLatest, mandatePage, saveMandate, updateSalaryCurrency, adminMandates, adminMandate, outreachDialog, saveOutreach, matchesPage, matchDetail, newMatch, savedFollowup, saveFollowup, searchWorkbench, workbenchPage, introduceCandidate, selectWorkbenchCandidate, selectWorkbenchPage, reviewCandidateBatch, introduceCandidateBatch, adminTokens, webhooksPage, webhookDialog, saveWebhook, copyWebhookSecret, toggleWebhook, deleteWebhook, webhookDeliveries, testWebhook, retryWebhook };
}

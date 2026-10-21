// HumanGrant keeps a person's sign-in separate from the assistant's access.
const RETURN_KEY = 'hireferry-human-return';
const RETURN_TTL_MS = 60 * 60 * 1000;
const REVIEW_PATH = /^\/human\/[A-Za-z0-9_-]{20,160}$/;
const LINK = /^[A-Za-z0-9_-]{20,160}$/;

export function humanReviewPath(value, origin) {
  if (typeof value !== 'string') return null;
  const input = value.trim();
  if (REVIEW_PATH.test(input)) return input;
  try {
    const url = new URL(input);
    if (url.origin !== origin || url.username || url.password || url.search || url.hash) return null;
    return REVIEW_PATH.test(url.pathname) ? url.pathname : null;
  } catch {
    return null;
  }
}

export function rememberHumanReview(storage, path, now = Date.now()) {
  if (!REVIEW_PATH.test(path)) return false;
  try {
    storage.setItem(RETURN_KEY, JSON.stringify({ path, savedAt: now }));
    return true;
  } catch {
    return false;
  }
}

export function savedHumanReview(storage, now = Date.now()) {
  try {
    const data = JSON.parse(storage.getItem(RETURN_KEY) || 'null');
    if (data && REVIEW_PATH.test(data.path) && Number.isFinite(data.savedAt)
      && data.savedAt <= now && now - data.savedAt < RETURN_TTL_MS) return data.path;
    storage.removeItem(RETURN_KEY);
  } catch { /* Storage can be disabled. The original link remains usable. */ }
  return null;
}

const PERMISSIONS = {
  'profile.read': ['查看我的资料与账户状态', 'Read my profile and account status', '读取你的登录邮箱、名称、头像、联系邮箱和简历，以及账户身份、开通与停用状态、创建和更新时间。', 'Read your sign-in email, name, avatar, contact email, and résumé. Also read your account role, activation and suspension status, and creation and update times.'],
  'profile.write': ['修改我的资料', 'Update my profile', '修改你的名称、联系邮箱、简历和头像。', 'Update your name, contact email, résumé, and avatar.'],
  'jobs.read': ['查看岗位', 'Read roles', '读取你发布的岗位和平台介绍给你的岗位。', 'Read your roles and roles introduced to you.'],
  'jobs.write': ['管理我的岗位', 'Manage my roles', '创建岗位、提交描述修改或结束招聘。付费步骤仍需单独完成。', 'Create roles, submit description changes, or close a role. Payment remains a separate step.'],
  'people.read': ['查看匹配资料', 'Read introduced profiles', '读取平台已向你介绍的求职者资料。', 'Read the profiles of job seekers introduced to you.'],
  'orders.read': ['查看我的订单', 'Read my orders', '读取订单金额和付款状态。', 'Read order amounts and payment status.'],
  'chat.read': ['读取我的会话', 'Read my conversations', '读取你有权访问的会话和消息。', 'Read conversations and messages that you can access.'],
  'chat.write': ['发送消息并标记已读', 'Send messages and mark messages as read', '在你有权访问的会话中发送消息、标记消息已读。发送的消息会标记为 AI 代发。', 'Send messages and mark messages as read in conversations you can access. Sent messages are labeled as sent by AI.'],
  'matches.read': ['查看我的匹配', 'Read my introductions', '读取平台为你建立的匹配记录。', 'Read introductions that HireFerry has made for you.'],
  'mandate.read': ['查看求职委托', 'Read my search brief', '读取你的求职要求和平台记录的寻找进展。', 'Read your job-search requirements and recorded progress.'],
};

export function createHumanGrantUI({ state, api, t, esc, icon, notice, empty, notify, go, render, session, confirmDialog }) {
  let activeReview = null;
  let grantsCsrf = null;
  let grantsRenderVersion;
  let knownGrants = new Set();
  const enabled = () => state.config?.humangrant_enabled === true;
  const storage = () => {
    try { return globalThis.sessionStorage; } catch { return null; }
  };
  const clearReturn = () => {
    try { storage()?.removeItem(RETURN_KEY); } catch { /* Optional browser storage. */ }
  };
  const returnAfterLogin = () => state.user?.role === 'unassigned' ? '/app' : savedHumanReview(storage()) || '/app';
  const mark = (large = false) => `<svg class="hg-mark${large ? ' hg-mark-large' : ''}" viewBox="0 0 64 64" fill="none" aria-hidden="true"><circle cx="25" cy="19" r="8" fill="currentColor"/><path d="M9 48v-6a16 16 0 0 1 27-11.6" stroke="currentColor" stroke-width="5.5" stroke-linecap="round"/><path class="hg-mark-check" d="m34 44 8 8 15-20" stroke="currentColor" stroke-width="5.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
  const artwork = () => `<div class="hg-art" aria-hidden="true"><img class="hg-asset-light" src="/static/humangrant/consent-light.svg" alt="" width="480" height="320"><img class="hg-asset-dark" src="/static/humangrant/consent-dark.svg" alt="" width="480" height="320"></div>`;
  const title = action => PERMISSIONS[action] ? t(PERMISSIONS[action][0], PERMISSIONS[action][1]) : action;
  const description = action => PERMISSIONS[action] ? t(PERMISSIONS[action][2], PERMISSIONS[action][3]) : t('请核对这项权限。', 'Check this permission.');
  const resource = value => value === 'self'
    ? t('仅限这个账户当前可访问的记录', 'Only records this account can currently access')
    : `${t('仅限这条记录', 'Only this record')}: ${value}`;
  const fullDate = value => {
    const time = new Date(Number(value) * 1000);
    return Number.isFinite(time.getTime()) ? time.toLocaleString(state.lang === 'en' ? 'en-GB' : 'zh-CN', { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—';
  };
  const stateName = phase => ({
    waiting_human: t('等待你确认', 'Waiting for your decision'),
    awaiting_consent: t('等待你确认', 'Waiting for your decision'),
    authenticating_human: t('等待你确认', 'Waiting for your decision'),
    succeeded: t('AI 已连接', 'AI connected'),
    awaiting_human: t('等待你确认', 'Waiting for your decision'),
    awaiting_approval: t('等待你确认', 'Waiting for your decision'),
    approved: t('已允许', 'Approved'),
    claimed: t('AI 已连接', 'AI connected'),
    active: t('AI 已连接', 'AI connected'),
    denied: t('已拒绝', 'Declined'),
    rejected: t('已拒绝', 'Declined'),
    cancelled: t('已取消', 'Canceled'),
    canceled: t('已取消', 'Canceled'),
    expired: t('已过期', 'Expired'),
    revoked: t('已撤销', 'Revoked'),
  })[phase] || phase;
  const pending = phase => ['waiting_human', 'awaiting_consent', 'authenticating_human'].includes(phase);
  const page = (content, identity = 'humangrant') => `<main id="main" class="page-width hg-page" data-page-key="${esc(identity)}">${content}</main>`;
  const permissionList = permissions => `<ul class="hg-permission-summary">${(permissions || []).map(p => `<li><strong>${esc(title(p.action))}</strong><span>${esc(resource(p.resource))}</span></li>`).join('')}</ul>`;

  function loginButton() {
    if (!enabled()) return '';
    return `<div class="hg-login-option"><a href="/humangrant" data-nav class="button hg-continue full">${mark()}<span lang="en">Continue with human beings</span>${icon('arrow')}</a><p>${t('由你确认，再让 AI 登录。', 'Let your AI sign in with your permission.')}</p></div>`;
  }

  function intro() {
    if (!enabled()) return page(`<section class="hg-card">${mark(true)}<h1>${t('HumanGrant 暂未开放', 'HumanGrant is not available')}</h1><p>${t('请使用现有登录方式。', 'Use an available sign-in method.')}</p><a href="/login" data-nav class="button">${t('返回登录', 'Back to sign-in')}</a></section>`);
    const prompt = state.lang === 'en'
      ? `Use HumanGrant to connect to ${location.origin}. First read https://github.com/LIghtJUNction/humangrant/blob/main/skills/humangrant-use/SKILL.md and ${location.origin}/api/humangrant/config. Use target_service=hireferry and client_id=hireferry-web. Request only profile.read/self and matches.read/self for up to one hour, within the service limit. Give me the human_authorization_url and wait for my approval. Keep your private key on your own device. Do not ask for my password or sign-in code.`
      : `请用 HumanGrant 连接 ${location.origin}。先阅读 https://github.com/LIghtJUNction/humangrant/blob/main/skills/humangrant-use/SKILL.md 和 ${location.origin}/api/humangrant/config。设置 target_service=hireferry、client_id=hireferry-web。只申请 profile.read/self 和 matches.read/self，有效期最多一小时，且不超过本站配置的上限。把 human_authorization_url 发给我，等待我确认后再继续。私钥保留在你自己的设备上，不要索取我的密码或登录验证码。`;
    return page(`<a href="${state.user ? '/app?tab=agents' : '/login'}" data-nav class="back-link">${icon('back')}${state.user ? t('返回 AI 连接', 'Back to AI connections') : t('返回登录', 'Back to sign-in')}</a><div class="hg-intro-grid"><section class="hg-intro"><div class="hg-brand">${mark()}<span>HumanGrant</span></div><h1>${t('由你确认。<br>AI 才能继续。', 'You decide.<br>Your AI can continue.')}</h1><p class="hg-lead">${t('让 AI 使用你选定的权限。你决定它能做什么、能用多久，也可以随时撤销。', 'Give your AI the access you choose. Set what it can do, set an expiry time, and revoke access at any time.')}</p>${artwork()}<ol class="hg-steps"><li><span>01</span><div><strong>${t('让 AI 发起申请', 'Ask your AI to request access')}</strong><p>${t('把右侧说明发给你的 AI，它会给你一个确认链接。', 'Share the instructions with your AI. It will give you a review link.')}</p></div></li><li><span>02</span><div><strong>${t('登录并选择权限', 'Sign in and choose permissions')}</strong><p>${t('查看申请者、权限和期限，再决定是否允许。', 'Check the requester, permissions, and expiry before you decide.')}</p></div></li><li><span>03</span><div><strong>${t('回到 AI 继续', 'Return to your AI')}</strong><p>${t('允许后，AI 获得独立访问凭据。你的登录凭据留在浏览器中。', 'After approval, the AI gets its own access credential. Your sign-in stays in your browser.')}</p></div></li></ol></section><div class="hg-intro-panels"><section class="hg-card"><div class="eyebrow">${t('第一步', 'START HERE')}</div><h2>${t('把这段说明发给你的 AI', 'Share this with your AI')}</h2><p>${t('先申请查看资料与匹配。更多权限可以另行申请。', 'Start with access to your profile and introductions. Request other permissions separately.')}</p><label class="field-label" for="hg-prompt">${t('接入说明', 'Connection instructions')}</label><textarea class="input hg-prompt" id="hg-prompt" readonly rows="9" spellcheck="false">${esc(prompt)}</textarea><button type="button" class="button full" data-action="hg-copy-prompt">${icon('copy')}${t('复制说明', 'Copy instructions')}</button></section><section class="hg-card hg-link-card"><h2>${t('已经收到确认链接？', 'Already have a review link?')}</h2><form data-form="hg-open-link"><label for="hg-link">${t('本站的 HumanGrant 链接', 'A HumanGrant link for this site')}</label><input id="hg-link" name="link" class="input" type="text" autocomplete="off" placeholder="${esc(location.origin)}/human/…" required maxlength="500"><button class="button secondary full" type="submit">${t('查看申请', 'Review request')}${icon('arrow')}</button></form></section></div></div>`);
  }

  async function reviewPage(link, renderVersion = state.render) {
    if (!LINK.test(link)) return page(`<section class="hg-card">${empty(t('确认链接无效', 'Invalid review link'), t('请让 AI 提供新的完整链接。', 'Ask your AI for a new, complete link.'))}</section>`);
    const review = await api(`/api/humangrant/review/${encodeURIComponent(link)}`);
    if (renderVersion !== state.render) return '';
    activeReview = { link, data: review, renderVersion };
    const isPending = pending(review.phase);
    const signedIn = !!review.human_id;
    if (!signedIn && isPending) rememberHumanReview(storage(), `/human/${link}`);
    if (signedIn && state.user?.role !== 'unassigned') clearReturn();
    const wrongAccount = signedIn && review.expected_human_id && review.expected_human_id !== review.human_id;
    const choices = review.requested_permissions || [];
    const allowed = review.grantable_permissions || [];
    const permitted = p => allowed.some(x => x.action === p.action && x.resource === p.resource);
    const canApprove = isPending && signedIn && !wrongAccount && !!review.csrf && choices.some(permitted);
    const deadline = Number(review.requested_grant_expires_at);
    const checkedAt = Number(review.checked_at);
    const options = [...new Set([900, 3600, 28800, 86400].map(n => checkedAt + n).filter(n => n <= deadline).concat(Number.isFinite(deadline) && deadline > checkedAt ? [deadline] : []))].sort((a, b) => a - b);
    const durationLabel = expiry => {
      const minutes = Math.max(1, Math.round((expiry - checkedAt) / 60));
      return minutes < 60 ? t(`${minutes} 分钟`, `${minutes} minutes`) : minutes % 60 === 0 ? t(`${minutes / 60} 小时`, `${minutes / 60} hours`) : t(`${minutes} 分钟`, `${minutes} minutes`);
    };
    let decision;
    if (!isPending) {
      const success = ['approved', 'succeeded', 'claimed', 'active'].includes(review.phase);
      decision = `<section class="hg-result" role="status"><span class="hg-result-icon">${icon(success ? 'check' : 'info')}</span><h2>${esc(stateName(review.phase))}</h2><p>${success ? t('可以回到 AI 继续。你可以在「AI 连接」中查看或撤销授权。', 'Return to your AI to continue. You can review or revoke access in AI connections.') : t('这次申请已结束。如果仍需连接，请让 AI 发起新申请。', 'This request has ended. Ask your AI for a new request if you still want to connect.')}</p><a href="${state.user ? '/app?tab=agents' : '/humangrant'}" data-nav class="button secondary">${state.user ? t('查看 AI 连接', 'View AI connections') : t('返回 HumanGrant', 'Back to HumanGrant')}</a></section>`;
    } else if (!signedIn) {
      decision = `<section class="hg-signin">${notice(state.admin ? t('请使用普通用户账户确认这次申请。管理员权限不能通过此入口授予。', 'Use a member account for this request. This page cannot grant administrator access.') : t('先登录你的聘渡账户，再选择允许的权限。打开链接不会自动授权。', 'Sign in to your HireFerry account, then choose permissions. Opening this link does not grant access.'))}<a href="/login" data-nav class="button full">${t('登录后确认', 'Sign in to review')}${icon('arrow')}</a></section>`;
    } else if (wrongAccount) {
      decision = notice(t('这次申请指定了另一个账户。当前账户不能批准，请让 AI 核对后重新申请。', 'This request is for another account. This account cannot approve it. Ask your AI to check the account and request again.'), 'warning');
    } else {
      decision = `<form data-form="hg-approve" data-link="${esc(link)}" class="hg-review-form"><fieldset class="hg-permissions"><legend>${t('选择你允许的权限', 'Choose the permissions you allow')}</legend><p class="help">${t('只勾选这次任务需要的内容。未勾选的权限不会授予。', 'Select only what this task needs. Unselected permissions will not be granted.')}</p>${choices.map((p, i) => `<label class="hg-permission${permitted(p) ? '' : ' hg-permission-unavailable'}"><input type="checkbox" name="permission" value="${i}" ${permitted(p) ? '' : 'disabled'}><span><strong>${esc(title(p.action))}</strong><span>${esc(description(p.action))}</span><small>${esc(resource(p.resource))}</small>${!permitted(p) ? `<small class="hg-unavailable">${t('当前账户无法授予这项权限', 'This account cannot grant this permission')}</small>` : ''}</span></label>`).join('')}</fieldset>${!canApprove ? notice(t('当前账户还不具备这些 AI 操作权限。你可以拒绝申请，或先到工作台查看账户条件。', 'This account is not currently eligible to grant these AI permissions. You can decline the request or check your account in the workspace.') + ` <a href="/app" data-nav>${t('查看工作台', 'Open workspace')} →</a>`, 'warning') : ''}<div class="field"><label for="hg-expiry">${t('允许多久', 'Allow access until')}</label><select class="input" name="expires_at" id="hg-expiry" ${canApprove ? '' : 'disabled'}>${options.map(expiry => `<option value="${expiry}">${esc(durationLabel(expiry))} · ${esc(fullDate(expiry))}</option>`).join('')}</select><p class="help">${t('到期后自动停止。你也可以提前撤销。', 'Access stops at expiry. You can revoke it earlier.')}</p></div><label class="hg-confirm"><input type="checkbox" name="consent" required ${canApprove ? '' : 'disabled'}><span>${t('我确认，允许这个 AI 按以上选择操作我的账户。', 'I approve this AI to use my account within the selected permissions.')}</span></label><div class="hg-decision-buttons"><button type="button" class="button secondary" data-action="hg-deny" data-link="${esc(link)}">${t('拒绝', 'Decline')}</button><button type="submit" class="button" ${canApprove && options.length ? '' : 'disabled'}>${icon('check')}${t('允许所选权限', 'Allow selected permissions')}</button></div></form>`;
    }
    return page(`<a href="/humangrant" data-nav class="back-link">${icon('back')}${t('关于 HumanGrant', 'About HumanGrant')}</a><div class="hg-review-layout"><aside class="hg-review-aside"><div class="hg-brand">${mark()}<span>HumanGrant</span></div><h1>${t('确认 AI 的访问申请', 'Review AI access')}</h1><p class="hg-lead">${t('核对这是谁的申请，以及它想做什么。只有你确认后，授权才会生效。', 'Check who is asking and what they want to do. Access starts only after your approval.')}</p><div class="hg-account"><span class="hg-detail-label">${t('当前账户', 'Current account')}</span><strong>${signedIn ? esc(review.human_name || state.user?.name || t('已登录账户', 'Signed-in account')) : t('尚未登录', 'Not signed in')}</strong>${signedIn ? `<code>${esc(review.human_id)}</code>` : ''}</div><dl class="hg-request-details"><div><dt>${t('服务', 'Service')}</dt><dd>${esc(review.target_service)}</dd></div><div><dt>${t('连接来源', 'Client')}</dt><dd>${esc(review.client_id)}</dd></div><div><dt>${t('申请到期时间', 'Request expires')}</dt><dd><time>${esc(fullDate(review.expires_at))}</time></dd></div></dl><details class="hg-identity" open><summary>${t('核对 AI 身份', 'Check AI identity')}</summary><p>${t('与 AI 提供的身份标识核对。这个标识不能证明申请者可信。', 'Compare this identifier with the one your AI provides. The identifier alone does not prove that the requester is trusted.')}</p><code>${esc(review.actor_id)}</code><span class="hg-detail-label">${t('申请编号', 'Request ID')}</span><code>${esc(review.request_id)}</code></details></aside><section class="hg-card hg-review-card"><div class="hg-card-heading"><span class="eyebrow">${t('访问确认', 'ACCESS REVIEW')}</span><span class="hg-status">${esc(stateName(review.phase))}</span></div><h2>${t('这次想做什么', 'The requested task')}</h2><blockquote class="hg-purpose">${esc(review.purpose || t('申请者未填写用途。', 'The requester gave no purpose.'))}</blockquote><p class="help hg-purpose-help">${t('以上内容由申请者填写。请按你的真实意愿决定。', 'The requester wrote this text. Decide according to your own intent.')}</p>${!signedIn || !isPending || wrongAccount ? permissionList(choices) : ''}${decision}</section></div>`, `human:${link}:${review.human_id || 'anonymous'}`);
  }

  async function grantsPanel(renderVersion = state.render) {
    if (!enabled()) return '';
    const result = await api('/api/humangrant/grants');
    if (renderVersion !== state.render) return '';
    grantsRenderVersion = renderVersion;
    grantsCsrf = result.csrf;
    knownGrants = new Set((result.grants || []).map(x => x.grant_id));
    const now = Date.now() / 1000;
    return `<section class="panel hg-grants"><div class="panel-header"><div><div class="hg-brand">${mark()}<h2>${t('HumanGrant 授权', 'HumanGrant access')}</h2></div><p>${t('每次授权有独立权限与到期时间。你可以随时撤销。', 'Each grant has its own permissions and expiry. You can revoke access at any time.')}</p></div><a href="/humangrant" data-nav class="button secondary small">${icon('plus')}${t('连接 AI', 'Connect AI')}</a></div>${result.grants?.length ? `<div class="hg-grant-list">${result.grants.map(grant => {
      const revoked = !!grant.revoked_at;
      const expired = Number(grant.expires_at) <= now;
      return `<article class="hg-grant"><div class="hg-grant-top"><div><h3>${t('AI 连接', 'AI connection')} <span>${esc(String(grant.actor_id).slice(-10))}</span></h3><p>${revoked ? t('已撤销', 'Revoked') : expired ? t('已到期', 'Expired') : `${t('有效至', 'Expires')} ${esc(fullDate(grant.expires_at))}`}</p></div>${!revoked && !expired ? `<button type="button" class="text-button" data-action="hg-revoke" data-id="${esc(grant.grant_id)}">${t('撤销授权', 'Revoke access')}</button>` : ''}</div><details><summary>${t('查看权限与身份', 'View permissions and identity')}</summary>${permissionList(grant.permissions)}<dl class="hg-request-details"><div><dt>${t('AI 身份', 'AI identity')}</dt><dd><code>${esc(grant.actor_id)}</code></dd></div><div><dt>${t('创建时间', 'Created')}</dt><dd>${esc(fullDate(grant.created_at))}</dd></div></dl></details></article>`;
    }).join('')}</div>` : `<div class="hg-no-grants">${mark(true)}<h3>${t('还没有 HumanGrant 授权', 'No HumanGrant grants yet')}</h3><p>${t('让 AI 发起申请后，你可以逐项确认权限。', 'Ask your AI to request access. You can then select its permissions.')}</p></div>`}</section>`;
  }

  async function submit(form, data) {
    if (form.dataset.form === 'hg-open-link') {
      const path = humanReviewPath(String(data.get('link') || ''), location.origin);
      if (!path) throw new Error(t('请输入本站的完整确认链接。', 'Enter a complete review link for this site.'));
      await go(path);
      return;
    }
    const current = activeReview;
    if (!current || current.renderVersion !== state.render || current.link !== form.dataset.link || !pending(current.data.phase)) throw new Error(t('请重新打开确认页。', 'Open the review page again.'));
    if (!data.has('consent')) throw new Error(t('请先确认授权。', 'Confirm your approval first.'));
    const requested = current.data.requested_permissions || [];
    const allowed = current.data.grantable_permissions || [];
    const permissions = [...new Set(data.getAll('permission'))].map(index => /^\d+$/.test(index) ? requested[Number(index)] : null).filter(Boolean);
    if (!permissions.length) throw new Error(t('请至少选择一项允许的权限。', 'Select at least one permission to allow.'));
    if (permissions.some(p => !allowed.some(x => x.action === p.action && x.resource === p.resource))) throw new Error(t('所选权限已不可用，请刷新申请。', 'A selected permission is unavailable. Refresh the request.'));
    const expiresAt = Number(data.get('expires_at'));
    if (!Number.isSafeInteger(expiresAt) || expiresAt <= Date.now() / 1000 || expiresAt > Number(current.data.requested_grant_expires_at)) throw new Error(t('所选期限无效或已过期，请刷新申请。', 'The selected expiry is invalid or has passed. Refresh the request.'));
    await api(`/api/humangrant/review/${encodeURIComponent(current.link)}/approve`, { method: 'POST', headers: { 'X-HG-CSRF': current.data.csrf }, body: { permissions, expires_at: expiresAt } });
    clearReturn();
    await render();
    notify(t('已允许所选权限。可以回到 AI 继续。', 'Selected permissions approved. Return to your AI to continue.'));
  }

  async function action(name, button) {
    if (name === 'hg-copy-prompt') {
      const field = document.querySelector('#hg-prompt');
      try {
        await navigator.clipboard.writeText(field.value);
        notify(t('说明已复制。', 'Instructions copied.'));
      } catch {
        field.focus();
        field.select();
        notify(t('请手动复制已选中的说明。', 'Copy the selected instructions manually.'));
      }
    } else if (name === 'hg-deny') {
      const current = activeReview;
      if (!current || current.renderVersion !== state.render || current.link !== button.dataset.link || !current.data.csrf) throw new Error(t('请重新打开确认页。', 'Open the review page again.'));
      await api(`/api/humangrant/review/${encodeURIComponent(current.link)}/deny`, { method: 'POST', headers: { 'X-HG-CSRF': current.data.csrf }, body: {} });
      clearReturn();
      await render();
      notify(t('已拒绝这次申请。', 'Request declined.'));
    } else if (name === 'hg-revoke') {
      const id = button.dataset.id;
      if (grantsRenderVersion !== state.render || !knownGrants.has(id) || !grantsCsrf) throw new Error(t('请刷新授权列表。', 'Refresh the access list.'));
      confirmDialog(t('撤销这次 AI 授权？', 'Revoke this AI access?'), t('这个 AI 将立即失去本次授予的权限。', 'The AI will immediately lose the permissions in this grant.'), async () => {
        await api(`/api/humangrant/grants/${encodeURIComponent(id)}`, { method: 'DELETE', headers: { 'X-HG-CSRF': grantsCsrf }, body: {} });
        await render();
        notify(t('授权已撤销。', 'Access revoked.'));
      }, { danger: true });
    }
  }

  return { enabled, loginButton, intro, reviewPage, grantsPanel, submit, action, returnAfterLogin, savedReturn: () => savedHumanReview(storage()), clearReturn };
}

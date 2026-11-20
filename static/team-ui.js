/* Team invitations use a URL fragment so the secret is not sent in page requests. */
const TOKEN = /^[A-Za-z0-9_-]{32,128}$/;
const RETURN_KEY = 'hireferry-team-invitation';
const RETURN_LIFETIME = 60 * 60 * 1000;

export function teamInvitationPath(value, origin) {
  try {
    const url = new URL(value, origin);
    if (url.origin !== origin || url.username || url.password || url.pathname !== '/team/invite' || url.search || !TOKEN.test(url.hash.slice(1))) return null;
    return url.pathname + url.hash;
  } catch { return null; }
}

export function rememberTeamInvitation(storage, path, now = Date.now()) {
  if (!/^\/team\/invite#[A-Za-z0-9_-]{32,128}$/.test(path)) return false;
  try { storage?.setItem(RETURN_KEY, JSON.stringify({ path, expires: now + RETURN_LIFETIME })); return !!storage; } catch { return false; }
}

export function savedTeamInvitation(storage, now = Date.now()) {
  try {
    const value = JSON.parse(storage?.getItem(RETURN_KEY) || 'null');
    if (value && typeof value.path === 'string' && /^\/team\/invite#[A-Za-z0-9_-]{32,128}$/.test(value.path) && Number.isFinite(value.expires) && value.expires > now && value.expires <= now + RETURN_LIFETIME) return value.path;
    storage?.removeItem(RETURN_KEY);
  } catch { /* Storage can be unavailable in private browsing. */ }
  return null;
}

export function createTeamUI({ state, api, t, esc, icon, heading, shell, notice, empty, openDialog, confirmDialog, notify, go, render, session, managerUI = null }) {
  let settings = { smtp_enabled: false };
  let members = new Map();
  let invitations = new Map();
  let review = null;
  let roleFilter = 'all';
  const storage = () => { try { return globalThis.sessionStorage; } catch { return null; } };
  const clearReturn = () => { try { storage()?.removeItem(RETURN_KEY); } catch { /* Optional browser storage. */ } };
  const fullDate = value => value ? new Date(Number(value) < 1e12 ? Number(value) * 1000 : value).toLocaleString(state.lang === 'en' ? 'en-GB' : 'zh-CN', { dateStyle: 'medium', timeStyle: 'short' }) : '—';
  const invitationStatus = value => ({ pending: t('等待加入', 'Pending'), accepted: t('已加入', 'Accepted'), revoked: t('已撤销', 'Revoked'), expired: t('已过期', 'Expired') })[value] || t('链接无效', 'Invalid link');
  const deliveryStatus = value => ({ not_sent: t('未发送邮件', 'Email not sent'), sending: t('正在发送邮件', 'Sending email'), sent: t('已交给邮件服务器', 'Accepted by mail server'), failed: t('邮件发送失败', 'Email failed') })[value] || t('未发送邮件', 'Email not sent');
  const scope = () => t('员工可处理用户资料、岗位、匹配、私聊和寻找委托，并查看订单与业务记录。团队、通行密钥和访问凭据由超级管理员管理。', 'Staff can manage profiles, jobs, introductions, private conversations, and search briefs, and view orders and business records. The super administrator manages the team, passkeys, and access credentials.');
  const page = body => `<main id="main" class="page-width team-invite-page"><section class="panel team-invite-card">${body}</section></main>`;
  const result = (title, detail) => page(`<span class="icon-badge">${icon('mail')}</span><h1>${title}</h1><p>${detail}</p><button type="button" class="button secondary" data-action="team-leave-invite">${t('返回', 'Go back')}</button>`);
  const currentInvitationPath = () => teamInvitationPath(location.href, location.origin);

  function returnAfterLogin() {
    return savedTeamInvitation(storage());
  }

  function emailChoice(id, enabled = settings.smtp_enabled) {
    return `<label class="team-email-choice" for="${id}"><input type="checkbox" id="${id}" name="send_email" ${enabled ? 'checked' : 'disabled'}><span>${t('通过本站邮件服务器发送邀请', 'Send an invitation using this site’s mail server')}</span></label>${!enabled ? `<p class="help">${t('邮件服务暂未启用。仍可生成邀请链接并复制给同事。', 'Email is not enabled. You can still create and copy an invitation link for your colleague.')}</p>` : ''}`;
  }

  const roleChoices = () => [
    ['all', t('全部', 'All')], ['human', t('人类员工', 'Human staff')],
    ...(managerUI ? [['manager', t('招聘经理', 'Managers')]] : []), ['ai', t('AI 员工', 'AI staff')],
  ];
  const showKind = kind => roleFilter === 'all' || roleFilter === kind;
  const roleTag = label => `<span class="team-role-label">${label}</span>`;

  function memberRows() {
    return [...members.values()].map(member => `<article class="team-member" data-team-kind="${member.kind === 'ai' ? 'ai' : 'human'}">
      <div class="team-person"><span class="team-kind-icon">${icon(member.kind === 'ai' ? 'bot' : 'person')}</span><div>
        <h3>${esc(member.name || member.email || member.id)}</h3><p>${esc(member.email || t('独立 AI 身份', 'Independent AI identity'))}</p>
        <div class="list-meta">${roleTag(member.kind === 'ai' ? t('AI 员工', 'AI staff') : t('人类员工', 'Human staff'))}<span>${t('加入', 'Added')} ${esc(fullDate(member.created_at))}</span><span>${t('最近使用', 'Last active')} ${esc(fullDate(member.last_used_at))}</span>${member.kind === 'ai' && member.token_expires_at ? `<span>${t('凭据到期', 'Credential expires')} ${esc(fullDate(member.token_expires_at))}</span>` : ''}</div>
      </div></div>
      <div class="team-row-actions"><span class="status ${member.active ? 'active' : 'banned'}">${member.active ? t('已启用', 'Enabled') : t('已停用', 'Disabled')}</span>${member.kind === 'ai' && member.active ? `<button type="button" class="button secondary small" data-action="team-rotate-ai" data-id="${esc(member.id)}">${t('更换凭据', 'Replace credential')}</button>` : ''}<button type="button" class="button secondary small" data-action="team-toggle-member" data-id="${esc(member.id)}">${member.active ? t('停用', 'Disable') : t('重新启用', 'Enable')}</button>${member.kind === 'ai' ? `<button type="button" class="text-button danger-text" data-action="team-delete-ai" data-id="${esc(member.id)}">${t('删除', 'Delete')}</button>` : ''}</div>
    </article>`).join('');
  }

  function invitationRows() {
    return [...invitations.values()].map(invite => `<article class="team-member" data-team-kind="human">
      <div class="team-person"><span class="team-kind-icon">${icon('mail')}</span><div><h3>${esc(invite.name || invite.email)}</h3>${invite.name ? `<p>${esc(invite.email)}</p>` : ''}
        <div class="list-meta">${roleTag(t('人类员工', 'Human staff'))}<span>${esc(deliveryStatus(invite.delivery_status))}</span><span>${t('到期', 'Expires')} ${esc(fullDate(invite.expires_at))}</span>${invite.last_sent_at ? `<span>${t('最近发送', 'Last sent')} ${esc(fullDate(invite.last_sent_at))}</span>` : ''}</div>
      </div></div>
      <div class="team-row-actions"><span class="status ${invite.status === 'accepted' ? 'active' : 'closed'}">${invitationStatus(invite.status)}</span>${['pending', 'expired'].includes(invite.status) ? `<button type="button" class="button secondary small" data-action="team-resend-invite" data-id="${esc(invite.id)}">${t('生成新邀请', 'Create a new link')}</button>` : ''}${invite.status === 'pending' ? `<button type="button" class="button secondary small" data-action="team-revoke-invite" data-id="${esc(invite.id)}">${t('撤销', 'Revoke')}</button>` : ''}</div>
    </article>`).join('');
  }

  function rosterSection(title, detail, rows, kinds, kind) {
    const visible = kinds.filter(showKind).length;
    const filtered = rows.replace(/data-team-kind="(human|manager|ai)"/g, (attribute, role) => attribute + (showKind(role) ? '' : ' hidden'));
    return `<section class="panel team-roster-panel" data-team-section="${kind}"><div class="panel-header"><div><h2>${title} <span class="pill" data-team-visible-count aria-live="polite">${visible}</span></h2><p>${detail}</p></div></div>
      <div class="team-member-list">${filtered}</div><div data-team-empty ${visible ? 'hidden' : ''}>${empty(kind === 'members' ? t('暂无符合条件的成员', 'No members in this view') : t('暂无符合条件的邀请', 'No invitations in this view'), kind === 'members' ? t('添加成员，或切换角色查看已有身份。', 'Add a member or select another role to view existing identities.') : t('创建邀请后，发送状态和有效期会显示在这里。', 'Invitation delivery and expiry will appear here.'), '', kind === 'members' ? 'person' : 'mail')}</div>
    </section>`;
  }

  async function teamPage() {
    if (!state.super_admin) throw new Error(t('只有超级管理员可以管理团队。', 'Only the super administrator can manage the team.'));
    const version = state.render;
    const [data, managers] = await Promise.all([api('/api/admin/team'), managerUI ? managerUI.load() : Promise.resolve({})]);
    if (version !== state.render) return '';
    settings = data;
    members = new Map((data.members || []).map(member => [member.id, member]));
    invitations = new Map((data.invitations || []).map(invite => [invite.id, invite]));
    const staffMembers = managers.members || [], staffInvites = managers.invitations || [];
    const memberKinds = [...members.values()].map(member => member.kind === 'ai' ? 'ai' : 'human').concat(staffMembers.map(() => 'manager'));
    const inviteKinds = [...invitations.values()].map(() => 'human').concat(staffInvites.map(() => 'manager'));
    const activeCount = [...members.values(), ...staffMembers].filter(member => member.active).length;
    const pendingCount = [...invitations.values()].filter(invite => invite.status === 'pending').length + staffInvites.filter(invite => managerUI.pending(invite)).length;
    const aiCount = [...members.values()].filter(member => member.kind === 'ai' && member.active).length;
    const actions = `<div class="team-header-actions"><button type="button" class="button secondary" data-action="admin-reauth">${icon('key')}${t('重新验证', 'Verify again')}</button><button type="button" class="button" data-action="team-add-member">${icon('plus')}${t('添加成员', 'Add a member')}</button></div>`;
    return shell(`<div class="team-workspace">${heading(t('团队', 'Team'), t('员工、招聘经理与 AI，共用一个工作台。', 'Staff, recruiting managers, and AI in one workspace.'), actions)}
      <div class="team-overview"><div>${icon('person')}<span>${t('已启用身份', 'Enabled identities')}</span><strong>${activeCount}</strong></div><div>${icon('mail')}<span>${t('等待加入', 'Pending invitations')}</span><strong>${pendingCount}</strong></div><div>${icon('bot')}<span>${t('已启用 AI', 'Enabled AI')}</span><strong>${aiCount}</strong></div></div>
      <div class="team-role-filters" role="group" aria-label="${t('按团队角色筛选', 'Filter team roles')}">${roleChoices().map(([kind, label]) => `<button type="button" class="team-filter-button" data-action="team-filter" data-kind="${kind}" aria-pressed="${roleFilter === kind}">${label}<span>${kind === 'all' ? memberKinds.length : memberKinds.filter(value => value === kind).length}</span></button>`).join('')}</div>
      ${rosterSection(t('成员', 'Members'), t('同一账户可拥有多个身份，每个身份的权限独立管理。', 'An account may have multiple identities. Manage access separately for each identity.'), memberRows() + (managerUI ? managerUI.memberRows(staffMembers) : ''), memberKinds, 'members')}
      ${rosterSection(t('邀请记录', 'Invitations'), t('查看发送状态、有效期，或撤销待接受的邀请。', 'Review delivery and expiry, or revoke a pending invitation.'), invitationRows() + (managerUI ? managerUI.invitationRows(staffInvites) : ''), inviteKinds, 'invitations')}
      <details class="team-permission-details"><summary>${icon('shield')}${t('角色权限与身份验证', 'Roles and identity verification')}</summary><div>
        <p>${scope()}</p><p>${t('招聘经理负责用户资料、岗位、匹配与沟通，不能修改财务、封禁账户或管理团队。', 'Recruiting managers handle profiles, roles, matching, and messages. They cannot change finances, suspend accounts, or manage the team.')}</p>
        <p class="help">${t('邀请或启用员工、签发 AI 凭据，需要最近 10 分钟内完成通行密钥验证。AI 凭据仅显示一次。', 'Inviting or enabling staff and issuing AI credentials require a passkey check within the last 10 minutes. AI credentials are shown once.')}</p>
      </div></details></div>`, 'team', true);
  }

  function showAddMember() {
    const choices = roleChoices().filter(([kind]) => kind !== 'all');
    const employeeForm = `<form class="team-add-form" data-form="team-invite"><h3>${t('邀请人类员工', 'Invite a human staff member')}</h3><p class="help">${t('员工可处理业务并查看财务记录。受邀者使用自己的邮箱登录。', 'Staff can manage business operations and view financial records. Invitees sign in with their own email.')}</p>
      <div class="field"><label for="team-email">${t('工作邮箱', 'Email address')}</label><input class="input" type="email" id="team-email" name="email" autocomplete="off" maxlength="254" required placeholder="colleague@example.com"></div>
      <div class="field"><label for="team-name">${t('成员名称（可选）', 'Name (optional)')}</label><input class="input" id="team-name" name="name" autocomplete="off" maxlength="80" placeholder="${esc(t('例如：招聘顾问', 'For example: Recruiting consultant'))}"></div>${emailChoice('team-send-email')}
      <p class="help">${t(`邀请在 ${Number(settings.invitation_ttl_hours) || 72} 小时后过期，只能由该邮箱接受一次。`, `The invitation expires after ${Number(settings.invitation_ttl_hours) || 72} hours and can be accepted once by this email address.`)}</p>
      <div class="form-actions"><button class="button" type="submit">${icon('mail')}${t('创建邀请', 'Create invitation')}</button></div></form>`;
    const aiForm = `<form class="team-add-form" data-form="team-ai"><h3>${t('添加 AI 员工', 'Add an AI staff member')}</h3><p class="help">${t('每个 AI 使用自己的凭据，操作记录分别保留。', 'Each AI uses its own credential and has a separate activity record.')}</p>
      <div class="field"><label for="team-ai-name">${t('AI 名称', 'AI name')}</label><input class="input" id="team-ai-name" name="name" maxlength="80" required placeholder="${esc(t('例如：候选人跟进助手', 'For example: Candidate follow-up assistant'))}"></div>
      <p class="help">${t(`凭据仅显示一次，${Number(settings.ai_token_ttl_days) || 30} 天后过期。可随时停用或更换凭据。`, `The credential is shown once and expires after ${Number(settings.ai_token_ttl_days) || 30} days. You can disable it or replace its credential at any time.`)}</p>
      <div class="field"><span class="field-label">${t('MCP 服务地址', 'MCP server URL')}</span><div class="code-box">${esc(location.origin)}/mcp</div></div>
      <div class="form-actions"><button class="button" type="submit">${icon('plus')}${t('创建 AI 员工', 'Create AI staff member')}</button></div></form>`;
    openDialog(`<div class="team-add-dialog"><h2 id="dialog-title">${t('添加团队成员', 'Add a team member')}</h2><p class="dialog-intro">${t('选择身份，再创建邀请或访问凭据。', 'Choose an identity, then create an invitation or credential.')}</p>
      <div class="team-add-choices" role="group" aria-label="${t('成员身份', 'Member identity')}">${choices.map(([kind, label]) => `<button type="button" class="team-kind-choice" data-action="team-add-kind" data-kind="${kind}" aria-pressed="${kind === 'human'}">${icon(kind === 'ai' ? 'bot' : kind === 'manager' ? 'briefcase' : 'person')}<span>${label}</span></button>`).join('')}</div>
      <div data-team-add-kind="human">${employeeForm}</div>${managerUI ? `<div data-team-add-kind="manager" hidden>${managerUI.invitationForm({ smtpEnabled: settings.smtp_enabled })}</div>` : ''}<div data-team-add-kind="ai" hidden>${aiForm}</div>
    </div>`);
  }

  function filterRoles(button) {
    const selected = button.dataset.kind;
    if (!roleChoices().some(([kind]) => kind === selected)) return;
    roleFilter = selected;
    const workspace = button.closest('.team-workspace');
    if (!workspace) return;
    for (const choice of workspace.querySelectorAll('[data-action="team-filter"]')) choice.setAttribute('aria-pressed', String(choice.dataset.kind === roleFilter));
    for (const section of workspace.querySelectorAll('[data-team-section]')) {
      let visible = 0;
      for (const row of section.querySelectorAll('[data-team-kind]')) { row.hidden = !showKind(row.dataset.teamKind); if (!row.hidden) visible += 1; }
      section.querySelector('[data-team-visible-count]').textContent = String(visible);
      section.querySelector('[data-team-empty]').hidden = visible > 0;
    }
  }

  function showInvitation(data) {
    const link = teamInvitationPath(data.invitation_url, location.origin);
    if (!link) throw new Error(t('服务器未返回有效的本站邀请链接。请刷新邀请记录。', 'The server did not return a valid invitation link for this site. Refresh the invitation list.'));
    const url = location.origin + link;
    const delivery = data.delivery_status || data.invitation?.delivery_status;
    const message = delivery === 'failed'
      ? t('邀请已创建，但邮件发送失败。可以复制链接给同事，或稍后重新生成并发送。', 'The invitation was created, but email delivery failed. Copy the link for your colleague, or generate and send another later.')
      : delivery === 'sent' ? t('邮件服务器已接收邀请邮件。你也可以复制下面的链接。', 'The mail server accepted the invitation email. You can also copy the link below.')
        : t('邀请已创建。把下面的链接发给受邀同事。', 'The invitation was created. Share this link with the invited colleague.');
    openDialog(`<h2 id="dialog-title">${t('邀请已创建', 'Invitation created')}</h2>${notice(message, delivery === 'failed' ? 'warning' : '')}<div class="field"><label for="team-invitation-secret">${t('邀请链接', 'Invitation link')}</label><textarea class="input team-secret" id="team-invitation-secret" rows="3" readonly spellcheck="false">${esc(url)}</textarea></div><button class="button" type="button" data-action="team-copy-secret" data-field="team-invitation-secret">${icon('copy')}${t('复制邀请链接', 'Copy invitation link')}</button><p class="help">${t('此链接只在本次显示。关闭后如需再次复制，请生成新邀请。仅受邀邮箱可以接受。', 'This link is shown now only. Generate a new invitation if you need another link after closing. Only the invited email can accept it.')}</p>`);
  }

  function showCredential(data) {
    if (typeof data.token !== 'string' || !data.token) throw new Error(t('未收到 AI 凭据，请刷新成员列表后重新生成。', 'No AI credential was returned. Refresh the member list and generate another.'));
    openDialog(`<h2 id="dialog-title">${t('保存 AI 员工凭据', 'Save the AI staff credential')}</h2><p class="dialog-intro">${esc(data.member?.name || '')} · ${t('有效至', 'Expires')} ${esc(fullDate(data.expires_at))}</p>${notice(t('凭据仅显示这一次。请交给对应 AI，并保存在它的私密配置中。', 'This credential is shown once. Give it to the intended AI and save it in its private configuration.'))}<div class="field"><label for="team-ai-secret">${t('访问凭据', 'Access credential')}</label><textarea class="input team-secret" id="team-ai-secret" rows="3" readonly spellcheck="false">${esc(data.token)}</textarea></div><button class="button" type="button" data-action="team-copy-secret" data-field="team-ai-secret">${icon('copy')}${t('复制凭据', 'Copy credential')}</button><p class="help">${t('连接', 'Connect to')} <code>${esc(location.origin)}/mcp</code><br>${t('在请求头中使用', 'Use the request header')} <code>Authorization: Bearer &lt;token&gt;</code></p>`);
  }

  async function publishCreated(operation, show) {
    const owner = state.csrf, version = state.render, path = location.href;
    const current = () => state.super_admin && state.csrf === owner && location.href === path;
    const response = await operation;
    // A completed request must never reopen a secret after logout or navigation.
    if (!current() || version !== state.render) return;
    await render();
    if (current()) show(response);
  }

  async function reviewPage(token, version = state.render) {
    review = null;
    if (!TOKEN.test(token)) return result(t('邀请链接不完整', 'The invitation link is incomplete'), t('请打开包含 # 后内容的完整链接，或请管理员重新邀请。', 'Open the full link, including the part after #, or ask the administrator for a new invitation.'));
    let data;
    try { data = await api('/api/team/invitations/preview', { method: 'POST', body: { token } }); }
    catch (error) { if (version !== state.render) return ''; if ([400, 404, 410].includes(error.status)) { clearReturn(); return result(t('邀请不可用', 'Invitation unavailable'), t('链接可能已失效或被撤销，请联系管理员获取新邀请。', 'The link may have expired or been revoked. Ask the administrator for a new invitation.')); } throw error; }
    if (version !== state.render) return '';
    review = { token, version, data };
    if (data.status !== 'pending') {
      clearReturn();
      return result(invitationStatus(data.status), data.status === 'accepted' ? t('这份邀请已被接受。已加入的成员可使用自己的邮箱登录总控台。', 'This invitation has already been accepted. The member can sign in to the control room with their own email.') : t('这份邀请已结束。仍需加入时，请管理员重新邀请。', 'This invitation has ended. Ask the administrator for a new one if you still need to join.'));
    }
    rememberTeamInvitation(storage(), `/team/invite#${token}`);
    let decision;
    const signedIn = !!state.user && !state.super_admin;
    if (state.super_admin) {
      decision = `${notice(t('当前是超级管理员会话。请切换为受邀邮箱账户，再接受邀请。', 'You are signed in as the super administrator. Switch to the invited email account to accept this invitation.'))}<button class="button secondary" type="button" data-action="team-switch-account">${t('切换账户', 'Switch account')}</button>`;
    } else if (!signedIn) {
      decision = `${notice(t('请先使用受邀邮箱登录。登录完成后会返回此处，由你确认加入。', 'Sign in with the invited email. You will return here to confirm joining.'))}<a class="button" href="/login" data-nav>${t('登录后加入', 'Sign in to join')}${icon('arrow')}</a>`;
    } else if (data.email_matches !== true) {
      decision = `${notice(t('当前登录邮箱与邀请不一致。请切换为受邀邮箱账户。', 'Your signed-in email does not match this invitation. Switch to the invited email account.'), 'warning')}<button class="button secondary" type="button" data-action="team-switch-account">${t('切换账户', 'Switch account')}</button>`;
    } else {
      decision = `<form data-form="team-accept" data-page-key="team-invite:${version}"><label class="team-email-choice"><input type="checkbox" name="consent" required><span>${t('我确认加入聘渡管理团队，并按上述权限协作。', 'I confirm that I want to join the HireFerry team with the permissions above.')}</span></label><button class="button" type="submit">${icon('check')}${t('接受邀请并加入', 'Accept and join')}</button></form>`;
    }
    return page(`<span class="icon-badge">${icon('person')}</span><h1>${t('加入聘渡团队', 'Join the HireFerry team')}</h1><p class="team-invite-lead">${t('管理员邀请你一起处理招聘与求职工作。', 'An administrator has invited you to help with recruiting and job searches.')}</p><dl class="team-invite-details"><div><dt>${t('受邀邮箱', 'Invited email')}</dt><dd>${esc(data.email_hint)}</dd></div>${data.name ? `<div><dt>${t('成员名称', 'Member name')}</dt><dd>${esc(data.name)}</dd></div>` : ''}<div><dt>${t('权限', 'Role')}</dt><dd>${t('员工', 'Staff')}</dd></div><div><dt>${t('邀请到期', 'Invitation expires')}</dt><dd>${esc(fullDate(data.expires_at))}</dd></div><div><dt>${t('当前账户', 'Current account')}</dt><dd>${state.super_admin ? t('超级管理员', 'Super administrator') : signedIn ? esc(state.user.email) : t('未登录', 'Not signed in')}</dd></div></dl><p class="team-permissions">${scope()}</p><div class="team-invite-decision">${decision}</div><button type="button" class="text-button" data-action="team-leave-invite">${t('暂时不加入', 'Not now')}</button>`);
  }

  async function submit(form, data) {
    const kind = form.dataset.form;
    if (kind === 'team-accept') {
      if (!review || review.version !== state.render || review.data.status !== 'pending' || review.data.email_matches !== true || currentInvitationPath() !== `/team/invite#${review.token}` || !data.get('consent') || !state.user || state.super_admin) throw new Error(t('请重新查看邀请，并确认使用受邀邮箱加入。', 'Review the invitation again and confirm joining with the invited email.'));
      const submitted = review, user = state.user.id, path = location.href;
      await api('/api/team/invitations/accept', { method: 'POST', body: { token: submitted.token } });
      if (review !== submitted || state.user?.id !== user || state.render !== submitted.version || location.href !== path) return;
      clearReturn(); review = null; await session();
      if (location.href === path) { await go('/admin?tab=messages'); notify(t('已加入团队。', 'You have joined the team.')); }
      return;
    }
    if (!state.super_admin) throw new Error(t('只有超级管理员可以管理团队。', 'Only the super administrator can manage the team.'));
    if (kind === 'team-invite') {
      await publishCreated(api('/api/admin/team/invitations', { method: 'POST', body: { email: String(data.get('email') || '').trim(), name: String(data.get('name') || '').trim(), send_email: settings.smtp_enabled && !!data.get('send_email') } }), showInvitation);
    } else if (kind === 'team-ai') {
      await publishCreated(api('/api/admin/team/ai', { method: 'POST', body: { name: String(data.get('name') || '').trim() } }), showCredential);
    } else if (kind === 'team-resend') {
      const invite = invitations.get(form.dataset.id);
      if (!invite || !['pending', 'expired'].includes(invite.status)) throw new Error(t('邀请状态已改变，请刷新。', 'The invitation changed. Refresh to continue.'));
      await publishCreated(api(`/api/admin/team/invitations/${encodeURIComponent(invite.id)}/resend`, { method: 'POST', body: { send_email: settings.smtp_enabled && !!data.get('send_email') } }), showInvitation);
    } else if (kind === 'team-ai-rotate') {
      const member = members.get(form.dataset.id);
      if (!member || member.kind !== 'ai' || !member.active) throw new Error(t('请先启用这个 AI 员工。', 'Enable this AI staff member first.'));
      await publishCreated(api(`/api/admin/team/members/${encodeURIComponent(member.id)}/token`, { method: 'POST', body: {} }), showCredential);
    }
  }

  async function action(actionName, button) {
    if (actionName === 'team-copy-secret') {
      if (!['team-invitation-secret', 'team-ai-secret'].includes(button.dataset.field)) return;
      const field = document.getElementById(button.dataset.field);
      if (!field?.value) return;
      try { await navigator.clipboard.writeText(field.value); notify(t('已复制。', 'Copied.')); }
      catch { field.focus(); field.select(); notify(t('浏览器无法自动复制，内容已选中，请手动复制。', 'The browser could not copy automatically. The text is selected; copy it manually.')); }
      return;
    }
    if (actionName === 'team-leave-invite') { clearReturn(); review = null; await go(state.admin ? '/admin?tab=messages' : state.user ? '/app' : '/login'); return; }
    if (actionName === 'team-switch-account') {
      const path = currentInvitationPath();
      if (path) rememberTeamInvitation(storage(), path);
      await api('/api/auth/logout', { method: 'POST', body: {} }); await session();
      await go('/login'); return;
    }
    if (!state.super_admin) throw new Error(t('只有超级管理员可以管理团队。', 'Only the super administrator can manage the team.'));
    if (actionName === 'team-add-member') { showAddMember(); return; }
    if (actionName === 'team-filter') { filterRoles(button); return; }
    if (actionName === 'team-add-kind') {
      const kind = button.dataset.kind;
      if (!roleChoices().some(([value]) => value === kind && value !== 'all')) return;
      const dialog = button.closest('.team-add-dialog');
      if (!dialog) return;
      for (const choice of dialog.querySelectorAll('[data-action="team-add-kind"]')) choice.setAttribute('aria-pressed', String(choice.dataset.kind === kind));
      for (const form of dialog.querySelectorAll('[data-team-add-kind]')) form.hidden = form.dataset.teamAddKind !== kind;
      return;
    }
    const id = button.dataset.id;
    if (actionName === 'team-toggle-member') {
      const member = members.get(id);
      if (!member) return;
      const enabled = !member.active;
      confirmDialog(enabled ? t('重新启用成员？', 'Enable this member?') : t('停用成员？', 'Disable this member?'), enabled
        ? t('该成员将重新获得员工业务权限。AI 需要重新生成凭据。', 'The member will regain staff access. An AI will need a new credential.')
        : t('该员工身份的业务访问将立即停用，历史记录保留；其他角色权限不受影响。', 'This staff identity loses business access immediately. History and any other roles are retained.'), async () => { await api(`/api/admin/team/members/${encodeURIComponent(id)}`, { method: 'PATCH', body: { active: enabled } }); await render(); notify(enabled ? t('成员已启用。', 'Member enabled.') : t('成员已停用。', 'Member disabled.')); }, { danger: !enabled });
    } else if (actionName === 'team-delete-ai') {
      const member = members.get(id);
      if (!member || member.kind !== 'ai') return;
      confirmDialog(t('永久删除这个 AI 员工身份？', 'Delete this AI employee identity permanently?'), t('它的访问凭据会被删除，历史业务记录保留但该身份不再出现在列表中。', 'Its credentials will be deleted. Historical business records remain, but this identity will be removed from the roster.'), async () => { await api(`/api/admin/team/members/${encodeURIComponent(id)}`, { method: 'DELETE' }); await render(); notify(t('AI 员工身份已删除。', 'AI employee identity deleted.')); }, { danger: true });
    } else if (actionName === 'team-revoke-invite') {
      const invite = invitations.get(id);
      if (!invite || invite.status !== 'pending') return;
      confirmDialog(t('撤销这份邀请？', 'Revoke this invitation?'), t('邀请链接会立即失效，不能再用它加入团队。', 'The invitation link will stop working immediately and cannot be used to join the team.'), async () => { await api(`/api/admin/team/invitations/${encodeURIComponent(id)}`, { method: 'DELETE' }); await render(); notify(t('邀请已撤销。', 'Invitation revoked.')); }, { danger: true });
    } else if (actionName === 'team-resend-invite') {
      const invite = invitations.get(id);
      if (!invite || !['pending', 'expired'].includes(invite.status)) return;
      openDialog(`<h2 id="dialog-title">${t('生成新的邀请链接', 'Generate a new invitation link')}</h2><p class="dialog-intro">${esc(invite.email)}</p>${notice(t('旧链接将立即失效，新链接会重新计时。', 'The old link will stop working immediately. The new link receives a new expiry time.'))}<form data-form="team-resend" data-id="${esc(id)}">${emailChoice('team-resend-email')}<div class="form-actions"><button type="button" class="button secondary" data-action="close-dialog">${t('取消', 'Cancel')}</button><button type="submit" class="button">${t('生成新邀请', 'Generate invitation')}</button></div></form>`);
    } else if (actionName === 'team-rotate-ai') {
      const member = members.get(id);
      if (!member || member.kind !== 'ai' || !member.active) return;
      openDialog(`<h2 id="dialog-title">${t('更换 AI 访问凭据', 'Replace the AI access credential')}</h2><p class="dialog-intro">${esc(member.name)}</p>${notice(t('旧凭据会立即失效。请保存新凭据并更新该 AI 的连接配置。', 'The old credential stops working immediately. Save the new credential and update this AI’s connection.'))}<form data-form="team-ai-rotate" data-id="${esc(id)}"><div class="form-actions"><button type="button" class="button secondary" data-action="close-dialog">${t('取消', 'Cancel')}</button><button type="submit" class="button">${t('生成新凭据', 'Generate credential')}</button></div></form>`);
    }
  }

  return { teamPage, reviewPage, returnAfterLogin, submit, action };
}

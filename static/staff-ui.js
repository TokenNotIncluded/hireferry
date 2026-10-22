// Staff UI uses the same theme tokens and authenticated request client as the
// rest of the workspace. Invitation secrets are displayed only at creation.
export function createStaffUI({
  state,api,t,esc,icon,date,heading,shell,empty,notice,notify,go,render,
  session,openDialog,confirmDialog,resetPrivateView
}) {
  const status = item => item.accepted_at
    ? t('已接受','Accepted')
    : item.revoked_at
      ? t('已撤销','Revoked')
      : item.expires_at <= Date.now()/1000
        ? t('已过期','Expired')
        : item.sent_at
          ? t('已发送','Sent')
          : t('等待接受','Pending');

  const pending = item => !item.accepted_at && !item.revoked_at && item.expires_at > Date.now() / 1000;
  const load = () => api('/api/admin/staff');

  function memberRows(items = []) {
    return items.map(member => `<article class="team-member" data-team-kind="manager">
      <div class="team-person"><span class="team-kind-icon">${icon('briefcase')}</span>
        <div><h3>${esc(member.name || member.email)}</h3><p>${esc(member.email)}</p>
          <div class="list-meta"><span class="team-role-label">${t('招聘经理', 'Recruiting manager')}</span><span>${t('加入', 'Joined')} ${esc(date(member.joined_at))}</span></div>
        </div>
      </div>
      <div class="team-row-actions"><span class="status ${member.active ? 'active' : 'banned'}">${member.active ? t('已启用', 'Enabled') : t('已停用', 'Disabled')}</span>
        <button type="button" class="button secondary small" data-action="staff-toggle" data-id="${esc(member.user_id)}" data-active="${member.active ? 'false' : 'true'}">${member.active ? t('停用', 'Disable') : t('重新启用', 'Enable')}</button>
      </div>
    </article>`).join('');
  }

  function invitationRows(items = []) {
    return items.map(invite => `<article class="team-member" data-team-kind="manager">
      <div class="team-person"><span class="team-kind-icon">${icon('mail')}</span>
        <div><h3>${esc(invite.email)}</h3><div class="list-meta"><span class="team-role-label">${t('招聘经理', 'Recruiting manager')}</span><span>${t('到期', 'Expires')} ${esc(date(invite.expires_at))}</span>${invite.sent_at ? `<span>${t('邮件已发送', 'Email sent')}</span>` : ''}</div></div>
      </div>
      <div class="team-row-actions"><span class="status ${invite.accepted_at ? 'active' : 'closed'}">${status(invite)}</span>
        ${pending(invite) ? `<button type="button" class="button secondary small" data-action="staff-revoke" data-id="${esc(invite.id)}">${t('撤销', 'Revoke')}</button>` : ''}
      </div>
    </article>`).join('');
  }

  function invitationForm({ smtpEnabled = false } = {}) {
    return `<form class="team-add-form" data-form="staff-invite">
      <h3>${t('邀请招聘经理', 'Invite a recruiting manager')}</h3>
      <p class="help">${t('经理负责候选人、岗位、匹配与沟通，不具备财务和团队管理权限。', 'Managers work with candidates, roles, matching, and messages, without finance or team management access.')}</p>
      <div class="field"><label for="staff-invite-email">${t('工作邮箱', 'Email address')}</label><input id="staff-invite-email" class="input" name="email" type="email" autocomplete="off" maxlength="254" placeholder="colleague@example.com" required></div>
      <label class="team-email-choice" for="staff-send-email"><input type="checkbox" id="staff-send-email" name="send_email" ${smtpEnabled ? 'checked' : 'disabled'}><span>${t('通过本站邮件服务器发送邀请', 'Send an invitation using this site’s mail server')}</span></label>
      ${smtpEnabled ? '' : `<p class="help">${t('邮件服务未启用，可创建并复制邀请链接。', 'Email is not enabled. You can create and copy an invitation link.')}</p>`}
      <p class="help">${t('邀请七天内有效，只能由受邀邮箱接受一次。', 'The invitation expires after seven days and can be accepted once by the invited email.')}</p>
      <div class="form-actions"><button class="button" type="submit">${icon('mail')}${t('创建邀请', 'Create invitation')}</button></div>
    </form>`;
  }

  // Old callers keep the same entry point while the roster lives in Team.
  async function adminPage() { await go('/admin?tab=team'); return ''; }

  async function invitePage(token) {
    const item=await api('/api/staff/invitations/'+encodeURIComponent(token));
    const account=state.user;
    const correct=account && account.email?.toLowerCase()===item.email.toLowerCase();
    const intro=`${heading(t('加入聘渡团队','Join the HireFerry team'),t('团队经理邀请','Manager invitation'),'','WELCOME ABOARD')}
      <section class="panel staff-invite-card">
        <span class="staff-invite-icon">${icon('mail')}</span>
        <h2>${t('你收到了团队邀请','You have a team invitation')}</h2>
        <p>${t('受邀邮箱','Invited email')} <strong>${esc(item.email)}</strong></p>
        <p>${t('角色：团队经理','Role: Team manager')} · ${t('有效期至','Expires')} ${date(item.expires_at)}</p>
        ${!account?`<a data-nav href="/login?return_to=${encodeURIComponent('/staff/invite/'+token)}" class="button">${t('使用受邀邮箱登录','Sign in with invited email')}${icon('arrow')}</a>`
        :correct?`<button data-action="staff-accept" data-token="${esc(token)}" class="button">${t('确认加入团队','Accept invitation')}${icon('check')}</button>`
        :`${notice(t('当前登录邮箱与邀请不一致。请切换至受邀邮箱。','Your signed-in email does not match. Switch to the invited email.'),'warning')}
          <button data-action="staff-switch-account" data-token="${esc(token)}" class="button secondary">${t('切换账户','Switch account')}</button>`}
        <p class="help">${t('接受邀请后才能进入团队工作台。链接不可转发。','Access starts only after accepting. Do not forward the link.')}</p>
      </section>`;
    return `<main id="main" class="page-width staff-invite-landing">${intro}</main>`;
  }

  async function submit(form,data) {
    if(form.dataset.form!=='staff-invite')return;
    if(!state.super_admin)throw new Error(t('只有超级管理员可以管理团队。','Only the super administrator can manage the team.'));
    const owner=state.csrf, version=state.render, path=location.href;
    const current=()=>state.super_admin&&state.csrf===owner&&location.href===path;
    const result=await api('/api/admin/staff/invitations',{
      method:'POST',
      body:{email:String(data.get('email')||'').trim(),role:'manager',
        send_email:data.get('send_email')==='on'}
    });
    if(!current()||version!==state.render)return;
    let link;
    try { link=new URL(result.invite_url,location.origin); } catch { /* Report the invalid response below. */ }
    if(!link||link.origin!==location.origin||link.username||link.password||link.search||link.hash||!/^\/staff\/invite\/[A-Za-z0-9_-]{43}$/.test(link.pathname))throw new Error(t('服务器未返回有效的本站邀请链接。请刷新邀请记录。','The server did not return a valid invitation link for this site. Refresh the invitation list.'));
    await render();
    if(!current())return;
    openDialog(`<h2 id="dialog-title">${result.sent?t('邀请邮件已发送','Invitation email sent'):t('邀请链接已创建','Invitation link created')}</h2>
      <p class="dialog-intro">${t('此链接只显示一次，仅受邀邮箱可以接受。','This link is shown once. Only the invited email can accept it.')}</p>
      <div class="field"><label for="team-invitation-secret">${t('邀请链接','Invitation link')}</label><textarea id="team-invitation-secret" class="input team-secret" rows="3" readonly spellcheck="false">${esc(link.href)}</textarea></div>
      <button class="button" type="button" data-action="team-copy-secret" data-field="team-invitation-secret">${icon('copy')}${t('复制邀请链接','Copy invitation link')}</button>`);
    notify(result.sent?t('邀请邮件已发送。','Invitation email sent.'):t('邀请已创建，请复制链接。','Invitation created. Copy the link.'));
  }

  async function action(name,button) {
    if(['staff-revoke','staff-toggle'].includes(name)&&!state.super_admin)throw new Error(t('只有超级管理员可以管理团队。','Only the super administrator can manage the team.'));
    if(name==='staff-copy') {
      await navigator.clipboard.writeText(button.dataset.copy);
      notify(t('邀请链接已复制。','Invitation link copied.'));
    } else if(name==='staff-accept') {
      await api('/api/staff/invitations/'+encodeURIComponent(button.dataset.token)+'/accept',{method:'POST',body:{}});
      await session();
      await go('/admin?tab=matches');
      notify(t('已加入团队，可以开始工作。','You joined the team.'));
    } else if(name==='staff-switch-account') {
      const target='/staff/invite/'+button.dataset.token;
      await api('/api/auth/logout',{method:'POST',body:{}});
      resetPrivateView();
      await go('/login?return_to='+encodeURIComponent(target));
    } else if(name==='staff-revoke') {
      confirmDialog(t('撤销这份邀请？','Revoke this invitation?'),
        t('邀请链接将立即失效。','This invitation link will immediately stop working.'),
        async()=>{await api('/api/admin/staff/invitations/'+encodeURIComponent(button.dataset.id),{method:'DELETE'});await render();notify(t('邀请已撤销。','Invitation revoked.'));},{danger:true});
    } else if(name==='staff-toggle') {
      const enable=button.dataset.active==='true';
      confirmDialog(enable?t('重新启用这位经理？','Enable this manager?'):t('停用这位经理？','Disable this manager?'),
        enable?t('恢复经理权限。','Restore manager permissions.'):t('招聘经理权限将立即停用；其他角色权限与个人账户保留。','Recruiting manager access ends immediately. Other roles and the personal account remain available.'),
        async()=>{await api('/api/admin/staff/members/'+encodeURIComponent(button.dataset.id),{method:'PATCH',body:{active:enable}});await render();notify(enable?t('成员已启用。','Manager enabled.'):t('成员已停用。','Manager disabled.'));},{danger:!enable});
    }
  }
  return {adminPage,invitePage,submit,action,load,memberRows,invitationRows,invitationForm,pending};
}

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

  async function adminPage() {
    const data=await api('/api/admin/staff');
    const active=(data.members||[]).filter(m=>m.active).length;
    const members=(data.members||[]).map(m=>`<div class="staff-row">
      <span class="staff-avatar">${icon('person')}</span>
      <div class="staff-row-details"><strong>${esc(m.name||m.email)}</strong>
        <span>${esc(m.email)} · ${t('经理','Manager')}</span>
        <small>${m.active?t('在职','Active'):t('已停用','Disabled')} · ${t('加入','Joined')} ${date(m.joined_at)}</small>
      </div>
      <button class="button secondary small" data-action="staff-toggle"
        data-id="${esc(m.user_id)}" data-active="${m.active?'false':'true'}">
        ${m.active?t('停用','Disable'):t('重新启用','Enable')}
      </button>
    </div>`).join('');
    const invites=(data.invitations||[]).map(v=>`<div class="staff-row">
      <span class="staff-avatar">${icon('mail')}</span>
      <div class="staff-row-details"><strong>${esc(v.email)}</strong>
        <span>${t('经理邀请','Manager invitation')} · ${status(v)}</span>
        <small>${t('有效期至','Expires')} ${date(v.expires_at)}</small>
      </div>
      ${!v.accepted_at&&!v.revoked_at&&v.expires_at>Date.now()/1000
        ?`<button class="button secondary small" data-action="staff-revoke" data-id="${esc(v.id)}">${t('撤销','Revoke')}</button>`
        :''}
    </div>`).join('');
    const body=`${heading(t('团队员工','Team members'),t('人类成员使用自己的邮箱账户工作，不共享超级管理员的通行密钥。','Human teammates sign in with their own email accounts. Administrator passkeys are never shared.'),'','OUR TEAM')}
    <section class="panel staff-panel">
      <div class="panel-header"><h2>${t('邀请新经理','Invite a manager')}</h2>
        <p>${t('受邀者需要使用收件邮箱登录并确认加入；邀请七天内有效，只能接受一次。','Invitees must sign in with the invited email and accept within seven days. Each link works once.')}</p>
      </div>
      <form class="staff-invite-form" data-form="staff-invite">
        <div class="field"><label for="staff-invite-email">${t('员工邮箱','Employee email')}</label>
          <input id="staff-invite-email" class="input" name="email" type="email" autocomplete="off" maxlength="254" placeholder="employee@example.com" required></div>
        <div class="field"><label for="staff-role">${t('角色','Role')}</label>
          <select id="staff-role" class="input" name="role"><option value="manager">${t('团队经理 · 招聘业务','Manager · recruiting operations')}</option></select></div>
        <label class="staff-mail-choice"><input type="checkbox" name="send_email" checked>
          <span>${t('通过已配置的 SMTP 服务器发送邀请邮件','Send invitation using the configured SMTP server')}</span></label>
        <div class="form-actions"><button class="button" type="submit">${icon('mail')}${t('创建邀请','Create invitation')}</button></div>
      </form>
      ${notice(t('若邮件服务器未配置，可取消勾选，只生成一次性邀请链接。经理可处理招聘匹配与沟通，不能修改财务、封禁、通行密钥或员工权限。','If SMTP is unavailable, uncheck sending to create a one-time link. Managers can handle matching and messages, but cannot modify finance, suspensions, passkeys or staff permissions.'))}
    </section>
    <section class="panel staff-panel"><div class="panel-header"><h2>${t('团队成员','Members')} · ${active}</h2></div>
      <div class="staff-roster">${members||empty(t('还没有员工','No employees yet'),t('先创建一封团队邀请。','Start with an invitation.'),'','person')}</div>
    </section>
    <section class="panel staff-panel"><div class="panel-header"><h2>${t('邀请记录','Invitation history')}</h2></div>
      <div class="staff-roster">${invites||empty(t('没有邀请记录','No invitations'),t('邀请会在过期后按保留策略清理。','Old invitations are automatically pruned.'),'','mail')}</div>
    </section>`;
    return shell(body,'staff',true);
  }

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
    const result=await api('/api/admin/staff/invitations',{
      method:'POST',
      body:{email:String(data.get('email')||'').trim(),role:'manager',
        send_email:data.get('send_email')==='on'}
    });
    await render();
    openDialog(`<div class="eyebrow">TEAM INVITATION</div>
      <h2 id="dialog-title">${result.sent?t('邀请邮件已发送','Invitation email sent'):t('邀请链接已创建','Invitation link created')}</h2>
      <p class="dialog-intro">${t('此链接只会显示一次，请妥善处理。','This link is displayed once. Handle it securely.')}</p>
      <div class="staff-link-box"><code>${esc(result.invite_url)}</code>
        <button class="button secondary small" data-action="staff-copy" data-copy="${esc(result.invite_url)}">${icon('copy')}${t('复制链接','Copy link')}</button></div>
      <div class="form-actions"><button class="button" data-action="close-dialog">${t('完成','Done')}</button></div>`);
    notify(result.sent?t('邀请邮件已发送。','Invitation email sent.'):t('邀请已创建，请复制链接。','Invitation created. Copy the link.'));
  }

  async function action(name,button) {
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
        enable?t('恢复经理权限。','Restore manager permissions.'):t('将立即停止其团队业务访问，个人账户仍可正常登录。','Team access ends immediately. Their personal account remains available.'),
        async()=>{await api('/api/admin/staff/members/'+encodeURIComponent(button.dataset.id),{method:'PATCH',body:{active:enable}});await render();notify(enable?t('成员已启用。','Manager enabled.'):t('成员已停用。','Manager disabled.'));},{danger:!enable});
    }
  }
  return {adminPage,invitePage,submit,action};
}

// Promotional entitlements use server quotes. The browser never decides a discount.
export function normalizePromoCode(value) {
  return String(value || '').trim().toUpperCase();
}

export function createPromoUI({state,api,t,esc,icon,money,date,currency,currencySelect,centsInput,heading,shell,empty,notice,openDialog,closeDialog,notify,go,render,session,confirmDialog,createMessageId,refreshJobPrice}) {
  const codes = new Map();
  const quotes = new WeakMap();
  const submissions = new WeakMap();
  const previewRequests = new WeakMap();
  const applicationStatus = status => ({pending:t('待审核','Awaiting review'),approved:t('已通过','Approved'),rejected:t('未通过','Declined')})[status] || status;
  const integer = (value, maximum, label) => {
    const text = String(value ?? '').trim();
    if (!/^\d+$/.test(text) || !Number.isSafeInteger(Number(text)) || Number(text) > maximum) throw new Error(t(`${label}须为 0 至 ${maximum} 的整数。`, `${label} must be an integer from 0 to ${maximum}.`));
    return Number(text);
  };
  const query = () => new URLSearchParams(globalThis.location?.search || '');
  const currentPage = () => Math.max(1, Number(query().get('page')) || 1);
  const codeState = code => !code.enabled ? t('已停用','Disabled') : code.expires_at && code.expires_at <= Date.now()/1000 ? t('已到期','Expired') : !code.activation_remaining&&!code.publish_remaining ? t('暂无剩余次数','No remaining uses') : t('可使用','Available');
  const usage = code => `<dl class="promo-usage"><div><dt>${t('免费激活剩余','Free activations left')}</dt><dd>${Number(code.activation_remaining) || 0}<small> / ${Number(code.free_activation_limit) || 0}</small></dd></div><div><dt>${t('发布优惠剩余','Discounted publications left')}</dt><dd>${Number(code.publish_remaining) || 0}<small> / ${Number(code.publish_limit) || 0}</small></dd></div><div><dt>${t('发布费用减免','Publication fee discount')}</dt><dd>${Number(code.discount_percent) || 0}%<small>${Number(code.discount_percent)===100?t('免费发布','Free publication'):''}</small></dd></div></dl>${Number(code.publish_reserved)>0?`<p class="help">${t(`${code.publish_reserved} 次已用于待付订单；取消或到期后释放。`,`${code.publish_reserved} use(s) are reserved for pending orders and released on cancellation or expiry.`)}</p>`:''}`;

  function pagination(result, section, admin=false) {
    const page=Number(result.page)||currentPage(),limit=Number(result.limit)||50,total=Number(result.total)||0;
    if (total <= limit && page===1) return '';
    const link=next=>{const params=query();params.set('tab','promos');params.set('view',section);params.set('page',String(next));return `${admin?'/admin':'/app'}?${params}`;};
    return `<nav class="pagination" aria-label="${t('优惠码分页','Promotions pagination')}">${page>1?`<a class="button secondary small" href="${esc(link(page-1))}" data-nav>${t('上一页','Previous')}</a>`:''}<span>${page} / ${Math.max(page,Math.ceil(total/limit))}</span>${page*limit<total?`<a class="button secondary small" href="${esc(link(page+1))}" data-nav>${t('下一页','Next')}</a>`:''}</nav>`;
  }

  function codeCard(code, admin=false) {
    codes.set(code.id,code);
    return `<article class="promo-code-card"><div class="promo-row"><div><h3>${esc(code.label||t('招聘优惠码','Hiring promo code'))}</h3><p class="promo-code"><code>${esc(code.code)}</code><button type="button" class="text-button" data-action="promo-copy" data-id="${esc(code.id)}" aria-label="${t('复制优惠码','Copy promo code')}">${icon('copy')}</button></p></div><span class="promo-state">${codeState(code)}</span></div>${usage(code)}<div class="promo-row"><p class="help">${code.expires_at?t('有效期至','Expires')+' '+date(code.expires_at):t('不设到期时间','No expiry')}</p>${admin?`<button type="button" class="text-button" data-action="promo-edit" data-id="${esc(code.id)}">${t('调整优惠','Edit promotion')}${icon('edit')}</button>`:!state.user?.employer_paid?`<button type="button" class="text-button" data-action="promo-use" data-id="${esc(code.id)}">${t('用于免费激活','Use for free activation')}${icon('arrow')}</button>`:`<a href="/app/jobs/new" data-nav class="text-button">${t('去发布岗位','Post a role')}${icon('arrow')}</a>`}</div></article>`;
  }

  function activationForm() {
    return `<section class="promo-section"><h2>${t('用优惠码免费激活','Activate free with a code')}</h2><p class="help">${t('先查看优惠，再确认激活。未选择身份的账户会同时设为招聘方。','Check the code, then confirm activation. An account without a role becomes an employer account.')}</p><form data-form="promo-activation"><div class="grid-2"><div class="field"><label for="activation-promo-code">${t('优惠码','Promo code')}</label><input class="input code" id="activation-promo-code" name="code" autocomplete="off" autocapitalize="characters" spellcheck="false" maxlength="64" required></div><div class="field"><label for="promo-activation-currency">${t('订单记录币种','Order currency')}</label>${currencySelect(currency().currency,'currency','promo-activation-currency')}</div></div><div class="promo-quote" data-promo-quote role="status" aria-live="polite"></div><div class="form-actions"><button type="button" class="button secondary" data-action="promo-activation-preview">${t('查看优惠','Check code')}</button><button type="submit" class="button" disabled>${t('确认免费激活','Confirm free activation')}${icon('arrow')}</button></div></form></section>`;
  }

  async function memberPage() {
    if (state.user?.role==='seeker') return shell(empty(t('求职服务永久免费','Job seeking is always free'),t('招聘优惠码用于招聘账号激活和岗位发布。','Hiring promo codes are for employer activation and role publication.')),'promos');
    const result=await api(`/api/promo-applications?page=${currentPage()}`);
    codes.clear();
    const applications=result.items||[],pending=applications.some(item=>item.status==='pending');
    const rows=applications.map(item=>`<article class="promo-application"><div class="promo-row"><h3>${applicationStatus(item.status)}</h3><span class="help">${date(item.created_at)}</span></div><details class="promo-disclosure"><summary>${t('申请与审核说明','Application and review notes')}</summary><p class="promo-prose">${esc(item.reason)}</p>${item.review_note?`<p class="help promo-prose">${esc(item.review_note)}</p>`:''}</details>${item.promo_code?codeCard(item.promo_code):''}</article>`).join('');
    return shell(`<div class="promo-page">${heading(t('招聘优惠','Hiring promotions'),t('申请优惠，或使用已有优惠码。激活与发布次数分别计算。','Apply for a promotion or use a code you already have. Activation and publication allowances are separate.'))}${!state.user?.employer_paid?activationForm():''}<section class="promo-section"><div class="promo-row"><h2>${t('我的申请','My applications')}</h2><button type="button" class="text-button" data-action="promo-refresh">${t('刷新状态','Refresh status')}</button></div>${rows||empty(t('还没有申请','No applications yet'),t('说明你的招聘计划，提交后在这里查看结果。','Tell us about your hiring plan. The review result appears here.'))}${pagination(result,'applications')}<details class="promo-disclosure" ${!applications.length?'open':''}><summary>${t('提交优惠申请','Apply for a promotion')}</summary>${pending?notice(t('已有申请等待审核，结果会显示在这里。','An application is awaiting review. Its result will appear here.')):`<form data-form="promo-application"><div class="field"><label for="promo-reason">${t('申请说明','Tell us about your request')}</label><textarea class="input" id="promo-reason" name="reason" rows="4" maxlength="2000" required placeholder="${esc(t('介绍招聘计划，以及需要哪些支持。','Describe your hiring plan and the support you need.'))}"></textarea></div><button type="submit" class="button">${t('提交申请','Submit application')}${icon('arrow')}</button></form>`}</details></section></div>`,'promos');
  }

  function expiryValue(timestamp) {
    if(!timestamp)return '';
    const time=new Date(timestamp*1000),pad=n=>String(n).padStart(2,'0');
    return `${time.getFullYear()}-${pad(time.getMonth()+1)}-${pad(time.getDate())}T${pad(time.getHours())}:${pad(time.getMinutes())}`;
  }

  function settingsFields(code={}) {
    const edit=!!code.id,activation=code.free_activation_limit??1;
    return `${!edit?`<details class="promo-disclosure"><summary>${t('自定义码与名称（可选）','Custom code and label (optional)')}</summary><div class="grid-2"><div class="field"><label for="promo-custom-code">${t('优惠码','Promo code')}</label><input class="input code" id="promo-custom-code" name="code" maxlength="64" autocomplete="off" autocapitalize="characters" placeholder="${esc(t('留空自动生成','Leave empty to generate'))}"></div><div class="field"><label for="promo-label">${t('名称','Label')}</label><input class="input" id="promo-label" name="label" maxlength="120"></div></div></details>`:`<div class="field"><label for="promo-label">${t('名称','Label')}</label><input class="input" id="promo-label" name="label" maxlength="120" value="${esc(code.label)}"></div>`}<div class="promo-allowance"><label class="promo-check"><input type="checkbox" name="include_activation" ${activation>0?'checked':''}><span>${t('附带免费激活次数','Include free activations')}</span></label><div class="field"><label for="promo-activation-limit">${t('免费激活总次数','Total free activations')}</label><input class="input" id="promo-activation-limit" type="number" name="free_activation_limit" min="0" max="1000000" step="1" value="${activation}" required ${activation>0?'':'disabled'}><p class="help">${t('默认 1 次；关闭后仍可使用发布优惠。','Defaults to 1. Publication discounts remain available when disabled.')}</p></div></div><div class="grid-2"><div class="field"><label for="promo-publish-limit">${t('发布优惠总次数','Total discounted publications')}</label><input class="input" id="promo-publish-limit" type="number" name="publish_limit" min="0" max="1000000" step="1" value="${code.publish_limit??1}" required><p class="help">${t('0 表示不提供发布优惠。','0 means no publication allowance.')}</p></div><div class="field"><label for="promo-discount">${t('发布费减免百分比','Publication fee discount (%)')}</label><input class="input" id="promo-discount" type="number" name="discount_percent" min="0" max="100" step="1" value="${code.discount_percent??100}" required><p class="help">${t('100% 表示免费发布；50% 表示减免一半。','100% means free publication; 50% means half off.')}</p></div></div><details class="promo-disclosure"><summary>${t('有效期与启用状态','Expiry and availability')}</summary><div class="field"><label for="promo-expiry">${t('有效期至（当前设备时间）','Expiry (this device’s local time)')}</label><input class="input" id="promo-expiry" type="datetime-local" name="expires_at" value="${expiryValue(code.expires_at)}"><p class="help">${t('留空表示不设期限。','Leave empty for no expiry.')}</p></div><label class="promo-check"><input type="checkbox" name="enabled" ${code.enabled===false?'':'checked'}><span>${t('启用此优惠码','Enable this code')}</span></label></details>${edit?`<p class="help">${t('总次数包含已使用和待付订单预占的次数，不能小于这两项之和。已完成的优惠不受修改影响。','Totals include used and reserved allowances and cannot be lower than their sum. Completed discounts remain unchanged.')}</p>`:''}`;
  }

  function settings(data, editing=false, previous=null) {
    const result={label:String(data.get('label')||'').trim(),free_activation_limit:data.has('include_activation')?integer(data.get('free_activation_limit'),1000000,t('激活次数','Activation allowance')):0,publish_limit:integer(data.get('publish_limit'),1000000,t('发布次数','Publication allowance')),discount_percent:integer(data.get('discount_percent'),100,t('减免比例','Discount')),enabled:data.has('enabled')};
    if(!editing&&String(data.get('code')||'').trim())result.code=normalizePromoCode(data.get('code'));
    if(editing&&previous?.expires_at&&data.get('expires_at')===expiryValue(previous.expires_at)) {
      // Omitting an unchanged expiry also allows correcting an already expired code.
    }else if(data.get('expires_at')) {
      const timestamp=new Date(String(data.get('expires_at'))).getTime();
      if(!Number.isFinite(timestamp)||timestamp<=Date.now())throw new Error(t('有效期须晚于当前时间。','Expiry must be in the future.'));
      result.expires_at=Math.floor(timestamp/1000);
    }else if(editing)result.clear_expiry=true;
    return result;
  }

  async function adminPage() {
    const view=query().get('view')==='codes'?'codes':'applications',status=query().get('status')||'pending';
    const params=new URLSearchParams({page:String(currentPage())});
    if(view==='applications'&&status!=='all')params.set('status',status);
    const result=await api(`/api/admin/promo-${view==='codes'?'codes':'applications'}?${params}`);
    codes.clear();
    const items=result.items||[];
    const content=view==='codes'?items.map(code=>codeCard(code,true)).join(''):items.map(item=>`<article class="promo-application"><div class="promo-row"><div><h3>${esc(item.user_name||item.user_id)}</h3><p class="help">${date(item.created_at)} · ${applicationStatus(item.status)}</p></div>${item.status==='pending'?`<button type="button" class="button secondary small" data-action="promo-review" data-id="${esc(item.id)}">${t('审核申请','Review request')}${icon('arrow')}</button>`:''}</div><p class="promo-prose">${esc(item.reason)}</p>${item.review_note?`<p class="help promo-prose">${esc(item.review_note)}</p>`:''}${item.promo_code?codeCard(item.promo_code,true):''}</article>`).join('');
    return shell(`<div class="promo-page">${heading(t('优惠码与申请','Promo codes and requests'),t('审批招聘优惠，分别设置激活次数、发布次数与减免比例。','Review requests and set activation allowance, publication allowance, and discounts.'),`<button type="button" class="button" data-action="promo-create">${icon('plus')}${t('创建优惠码','Create code')}</button>`)}<nav class="promo-tabs" aria-label="${t('优惠码管理','Promotion management')}"><a href="/admin?tab=promos&view=applications" data-nav ${view==='applications'?'aria-current="page"':''}>${t('申请审核','Applications')}</a><a href="/admin?tab=promos&view=codes" data-nav ${view==='codes'?'aria-current="page"':''}>${t('优惠码','Codes')}</a></nav>${view==='applications'?`<form data-form="promo-filter" class="promo-filter"><label for="promo-status">${t('申请状态','Status')}</label><select class="input" id="promo-status" name="status">${['pending','approved','rejected','all'].map(value=>`<option value="${value}" ${value===status?'selected':''}>${value==='all'?t('全部','All'):applicationStatus(value)}</option>`).join('')}</select><button type="submit" class="button secondary small">${t('查看','Apply')}</button></form>`:''}<section class="promo-section">${content||empty(t('暂无记录','No records'),view==='codes'?t('可以直接创建，或在申请通过时生成优惠码。','Create a code directly, or issue one when approving a request.'):t('符合条件的申请会显示在这里。','Applications matching this filter appear here.'))}${pagination(result,view,true)}</section></div>`,'promos',true);
  }

  function jobFields() {
    return `<details class="promo-disclosure promo-job"><summary>${t('使用优惠码','Use a promo code')}</summary><div class="field"><label for="job-promo-code">${t('优惠码','Promo code')}</label><div class="promo-code-entry"><input class="input code" id="job-promo-code" name="promo_code" maxlength="64" autocomplete="off" autocapitalize="characters" spellcheck="false"><button type="button" class="button secondary" data-action="promo-job-preview">${t('查看优惠','Check code')}</button></div><p class="help">${t('100% 减免可免费发布。优惠码次数以确认结果为准。','A 100% discount makes publication free. Availability is checked again when you confirm.')}</p></div><div class="promo-quote" data-promo-quote role="status" aria-live="polite"></div><a href="/app?tab=promos" data-nav class="text-button">${t('申请招聘优惠','Apply for a hiring promotion')}${icon('arrow')}</a></details>`;
  }

  function quoteInput(form, purpose) {
    const selected=form.elements.currency?.value||currency().currency;
    const body={code:normalizePromoCode(form.elements[purpose==='publish'?'promo_code':'code']?.value),purpose,currency:selected};
    if(!body.code)throw new Error(t('请填写优惠码。','Enter a promo code.'));
    if(purpose==='publish')body.salary_cents=centsInput(form.elements.salary.value,selected);
    return body;
  }

  function matchingQuote(form, purpose) {
    const saved=quotes.get(form);
    if(!saved||saved.owner!==state.user?.id)return null;
    try { return saved.key===JSON.stringify(quoteInput(form,purpose))?saved.result:null; }catch{return null;}
  }

  async function preview(form, purpose) {
    const body=quoteInput(form,purpose),key=JSON.stringify(body),owner=state.user?.id;
    const request=Symbol();previewRequests.set(form,request);
    quotes.delete(form);
    if(purpose==='publish')refreshJobPrice(form);else form.querySelector('[type="submit"]').disabled=true;
    const output=form.querySelector('[data-promo-quote]');output.classList?.remove('promo-error');output.textContent=t('正在核对优惠…','Checking the promotion…');
    try {
      const result=await api('/api/promo-codes/preview',{method:'POST',body});
      if(form.isConnected===false||state.user?.id!==owner||previewRequests.get(form)!==request||JSON.stringify(quoteInput(form,purpose))!==key)return;
      quotes.set(form,{key,owner,result});
      output.innerHTML=`<p><strong>${result.amount_cents===0?t('本次免费','Free this time'):money(result.amount_cents,result)}</strong>${purpose==='publish'?` · ${t('减免','Discount')} ${Number(result.discount_percent)}%`:''}</p><p class="help">${purpose==='publish'?t(`发布优惠剩余 ${result.promo_code.publish_remaining} 次。`,`Publication allowance remaining: ${result.promo_code.publish_remaining}.`):t(`免费激活剩余 ${result.promo_code.activation_remaining} 次。`,`Free activations remaining: ${result.promo_code.activation_remaining}.`)}</p>`;
      if(purpose==='publish')refreshJobPrice(form);else form.querySelector('[type="submit"]').disabled=false;
    }catch(error){if(form.isConnected===false||state.user?.id!==owner||previewRequests.get(form)!==request)return;output.textContent=error.message;output.classList?.add('promo-error');throw error;}
  }

  function refreshJob(form) {
    if(!form.elements.promo_code)return;
    const code=normalizePromoCode(form.elements.promo_code.value),quote=matchingQuote(form,'publish');
    const output=form.querySelector('[data-promo-quote]'),submit=form.querySelector('[data-job-submit]');
    if(!code){quotes.delete(form);output.textContent='';return;}
    if(!quote){submit.disabled=true;output.textContent=t('点击「查看优惠」，确认本次费用。','Select “Check code” to confirm this publication fee.');return;}
    form.querySelector('#job-cost').textContent=money(quote.amount_cents,quote);
    const free=quote.amount_cents===0;
    form.querySelector('[data-payment-options]').hidden=free;
    const note=form.querySelector('[data-job-payment-note]');
    if(note)note.textContent=free?t('本次免费，确认后立即发布。','This publication is free and takes effect on confirmation.'):t('按优惠后的金额付款，确认收款后发布岗位。','Pay the discounted amount. The role is published after payment is confirmed.');
    if(free){submit.disabled=form.dataset.exhausted==='true';submit.innerHTML=t('确认免费发布','Confirm free publication')+icon('arrow');}
  }

  function prepareJob(form, body) {
    const code=normalizePromoCode(form.elements.promo_code?.value);
    if(!code)return body;
    const quote=matchingQuote(form,'publish');
    if(!quote)throw new Error(t('请重新查看优惠，再确认发布。','Check the promotion again before confirming publication.'));
    const payload={...body,promo_code:code,expected_amount_cents:quote.amount_cents};
    if(quote.amount_cents===0)payload.provider='';
    // Keep one key for this editor until a successful response navigates away.
    // If a response was lost, editing then retrying must not create another role.
    const request_key=submissions.get(form)||createMessageId();
    submissions.set(form,request_key);
    return {...payload,request_key};
  }

  function changed(input) {
    const form=input.closest('form');
    if(!form)return;
    if(form.dataset.form==='job'&&['promo_code','currency','salary'].includes(input.name)){quotes.delete(form);previewRequests.delete(form);}
    if(input.name==='include_activation') {
      const allowance=form.elements.free_activation_limit;allowance.disabled=!input.checked;
      if(input.checked&&Number(allowance.value)===0)allowance.value='1';
    }
    if(form.dataset.form==='promo-activation'&&['code','currency'].includes(input.name)) {
      quotes.delete(form);previewRequests.delete(form);form.querySelector('[type="submit"]').disabled=true;
      form.querySelector('[data-promo-quote]').textContent='';
    }
  }

  async function submit(form,data) {
    const kind=form.dataset.form;
    if(kind==='promo-application') {
      await api('/api/promo-applications',{method:'POST',body:{reason:String(data.get('reason')||'').trim()}});
      await render();notify(t('申请已提交，可在这里查看审核结果。','Application submitted. Check the review result here.'));
    }else if(kind==='promo-activation') {
      if(!matchingQuote(form,'activation'))throw new Error(t('请先查看优惠，再确认激活。','Check the code before confirming activation.'));
      const body=quoteInput(form,'activation');delete body.purpose;
      const result=await api('/api/promo-codes/redeem-activation',{method:'POST',body});
      state.user=result.user;closeDialog();await session();await go('/app?tab=jobs');notify(t('招聘账号已免费激活。','Your employer account is activated free of charge.'));
    }else if(kind==='promo-filter') {
      await go('/admin?'+new URLSearchParams({tab:'promos',view:'applications',status:String(data.get('status')||'pending')}));
    }else if(kind==='promo-code') {
      const editing=!!form.dataset.id;
      await api(`/api/admin/promo-codes${editing?'/'+encodeURIComponent(form.dataset.id):''}`,{method:editing?'PATCH':'POST',body:settings(data,editing,codes.get(form.dataset.id))});
      closeDialog();await go('/admin?tab=promos&view=codes');notify(t('优惠码已保存。','Promo code saved.'));
    }else if(kind==='promo-review') {
      const decision=data.get('decision'),body={decision,note:String(data.get('note')||'').trim()};
      if(decision==='approved')body.settings=settings(data);
      await api(`/api/admin/promo-applications/${encodeURIComponent(form.dataset.id)}/review`,{method:'POST',body});
      closeDialog();await render();notify(decision==='approved'?t('申请已通过，优惠码已发放。','Application approved and code issued.'):t('审核结果已保存。','Review saved.'));
    }
  }

  async function action(action,button) {
    if(action==='promo-job-preview'||action==='promo-activation-preview')await preview(button.closest('form'),action==='promo-job-preview'?'publish':'activation');
    else if(action==='promo-refresh')await render();
    else if(action==='promo-copy') {const code=codes.get(button.dataset.id);if(code){await navigator.clipboard.writeText(code.code);notify(t('优惠码已复制。','Promo code copied.'));}}
    else if(action==='promo-use') {const code=codes.get(button.dataset.id),input=document.querySelector('#activation-promo-code');if(code&&input){input.value=code.code;input.focus();await preview(input.closest('form'),'activation');}}
    else if(action==='promo-create'||action==='promo-edit') {
      const code=action==='promo-edit'?codes.get(button.dataset.id):null;
      if(action==='promo-edit'&&!code)throw new Error(t('请刷新列表后重试。','Refresh the list and try again.'));
      openDialog(`<h2 id="dialog-title">${code?t('调整优惠码','Edit promo code'):t('创建优惠码','Create promo code')}</h2>${code?`<p class="promo-code"><code>${esc(code.code)}</code></p>`:''}<form class="promo-form" data-form="promo-code" data-id="${esc(code?.id||'')}">${settingsFields(code||{})}<div class="form-actions"><button type="submit" class="button">${t('保存优惠码','Save code')}${icon('check')}</button></div></form>`,{wide:true});
      if(!code)globalThis.document?.querySelector('[data-form="promo-code"] [name="include_activation"]')?.focus();
    }else if(action==='promo-review') {
      openDialog(`<h2 id="dialog-title">${t('审核优惠申请','Review promotion request')}</h2><form class="promo-form" data-form="promo-review" data-id="${esc(button.dataset.id)}"><div class="field"><label for="promo-decision">${t('审核结果','Decision')}</label><select class="input" id="promo-decision" name="decision"><option value="approved">${t('通过并发放优惠码','Approve and issue code')}</option><option value="rejected">${t('不通过','Decline')}</option></select></div><div data-promo-approval-settings>${settingsFields()}</div><div class="field"><label for="promo-review-note">${t('审核说明（申请人可见）','Review note (visible to applicant)')}</label><textarea class="input" id="promo-review-note" name="note" rows="3" maxlength="1000"></textarea></div><div class="form-actions"><button type="submit" class="button">${t('保存审核结果','Save decision')}${icon('check')}</button></div></form>`,{wide:true});
    }else if(action==='promo-cancel-order') {
      confirmDialog(t('取消这笔待付优惠订单？','Cancel this pending promotional order?'),t('取消后释放预占的优惠次数。此订单不能继续支付。','Cancellation releases the reserved promotion allowance. This order can no longer be paid.'),async()=>{await api(`/api/orders/${encodeURIComponent(button.dataset.id)}/cancel`,{method:'POST',body:{}});await render();notify(t('订单已取消，优惠次数已释放。','Order cancelled. The allowance is available again.'));});
    }
  }

  return {memberPage,adminPage,jobFields,refreshJob,prepareJob,changed,submit,action,settings,preview,matchingQuote};
}

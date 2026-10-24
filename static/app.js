import { renderCliGuide } from './cli-guide.js';
import { renderDownloads, mountDownloads } from './downloads.js';
import { mountCliGuide, copyCliCommand } from './cli-motion.js';
import { createPublicConfigCache, samePublicConfig } from './browser-cache.js';
import { animateContentRefresh } from './content-refresh.js';
import { createServiceUI, createMessageId } from './service-ui.js';
import { renderLegal } from './legal-content.js';
import { takeCliReturn } from './cli-auth-return.js';
import { mountPage, pageIdentity, pagePlaceholder } from './navigation.js';
import { createDisclosureUI } from './disclosure.js';
import { effectiveOrderStatus, activationRecovery, createBillingUI } from './billing-ui.js';
import { currencyMeta, currencyOptions, minorToDecimal, parseMinor, minorStep, formatMinor, calculateFee } from './money.js';
import { createHumanGrantUI } from './humangrant-ui.js';
import { createPromoUI } from './promo-ui.js';
import { createStaffUI } from './staff-ui.js';
import { createRequestClient, createSessionReader, navigationReads, RequestTimeoutError } from './requests.js';
import { createTeamUI } from './team-ui.js';
import { createControlsUI } from './controls.js';
import { createFinanceUI } from './finance-ui.js';
import { createPeopleJobsUI } from './people-jobs-ui.js';
import { syncPageMetadata } from './seo.js';
import { createRelationTools } from './relation-webmcp.js';
import { createWebMCPOwner } from './webmcp-lifecycle.js';

let humanGrantModule;
function humanGrant() { return humanGrantModule ||= createHumanGrantUI({state,api,t,esc,icon,notice,empty,notify,go,render,session,confirmDialog}); }
let teamModule;
function team() { return teamModule ||= createTeamUI({state,api,t,esc,icon,heading,shell,notice,empty,openDialog,confirmDialog,notify,go,render,session,managerUI:staffUI()}); }
let staffModule;
function staffUI() { return staffModule ||= createStaffUI({state,api,t,esc,icon,date,heading,shell,empty,notice,notify,go,render,session,openDialog,confirmDialog,resetPrivateView}); }
let promoModule;
function promos() { return promoModule ||= createPromoUI({state,api,t,esc,icon,money,date,currency,currencySelect,centsInput,heading,shell,empty,notice,openDialog,closeDialog,notify,go,render,session,confirmDialog,createMessageId,refreshJobPrice}); }

const root = document.querySelector('#root');
const dialogs = document.querySelector('#dialog-root');
const disclosureUI = createDisclosureUI({root, doc:document, win:window});
const controlsUI = createControlsUI({doc:document,win:window});
let financeModule, peopleJobsModule, financeCleanup;
function finance() { return financeModule ||= createFinanceUI({state,api,t,esc,money,currency,currencies,qs,shell,icon,orderTable,pagination,notice,go,kindText,statusText}); }
function peopleJobs() { return peopleJobsModule ||= createPeopleJobsUI({state,api,t,esc,icon,avatar,money,date,shell,badge,matchText,go}); }
function resetAdminViews() { financeCleanup?.();financeCleanup=null;financeModule?.destroy();peopleJobsModule?.destroy();controlsUI.close(); }
function savedLanguage() {try{return localStorage.getItem('hireferry-language');}catch{return null;}}
const state = { lang: savedLanguage() || 'zh', config:null, configCached:false, cliGuide:null, user:null, admin:false, owner:false, super_admin:false, staffRole:null, staff:null, bootstrap:false, csrf:null, sculpture:null, homeStory:null, particleTitles:null, brandMotion:null, motionPaused:false, render:0, eligibility:null, chat:null, chatTimer:null };
const t = (zh,en) => state.lang === 'en' ? en : zh;
const esc = value => String(value ?? '').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const currency = record => currencyMeta(state.config,record);
const currencies = () => currencyOptions(state.config);
const money = (minor,record) => formatMinor(minor ?? 0,currency(record),state.lang==='en'?'en-US':'zh-CN');
const amountValue = (minor,record) => minorToDecimal(minor,currency(record).currency_decimals);
const amountStep = record => minorStep(currency(record).currency_decimals);
const date = value => value ? new Date(typeof value==='number' && value<1e12?value*1000:value).toLocaleDateString(state.lang==='en'?'en-GB':'zh-CN',{year:'numeric',month:'short',day:'numeric'}) : '—';
const icons = {
  github:'<path fill="currentColor" stroke="none" d="M12 .9a11.1 11.1 0 0 0-3.51 21.63c.55.1.76-.24.76-.53v-2.07c-3.09.67-3.74-1.31-3.74-1.31-.51-1.28-1.24-1.62-1.24-1.62-1.01-.69.08-.68.08-.68 1.12.08 1.71 1.15 1.71 1.15.99 1.7 2.6 1.21 3.23.93.1-.72.39-1.21.7-1.49-2.47-.28-5.06-1.24-5.06-5.49 0-1.21.43-2.2 1.14-2.97-.11-.28-.49-1.41.11-2.94 0 0 .93-.3 3.05 1.14a10.62 10.62 0 0 1 5.55 0c2.12-1.44 3.05-1.14 3.05-1.14.6 1.53.22 2.66.11 2.94.71.77 1.14 1.76 1.14 2.97 0 4.26-2.6 5.21-5.08 5.49.4.35.75 1.02.75 2.06V22c0 .29.2.64.76.53A11.1 11.1 0 0 0 12 .9Z"/>',
  star:'<path d="m12 3 2.8 5.7 6.3.9-4.55 4.43 1.08 6.27L12 17.34l-5.63 2.96 1.08-6.27L2.9 9.6l6.3-.9Z"/>',
  more:'<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  palette:'<path d="M12 3a9 9 0 1 0 0 18h1.4a2.6 2.6 0 0 0 2.2-4c-.6-.9 0-2 1.1-2H18a3 3 0 0 0 3-3 9 9 0 0 0-9-9Z"/><circle cx="7.5" cy="10" r=".75"/><circle cx="10" cy="6.5" r=".75"/><circle cx="14.5" cy="6.5" r=".75"/><circle cx="17" cy="10" r=".75"/>',
  sun:'<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5"/>',
  moon:'<path d="M20.8 13A9 9 0 0 1 11 3.2 9 9 0 1 0 20.8 13Z"/>',
  monitor:'<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/>',
  panel:'<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>',
  message:'<path d="M21 11.5a8.5 8.5 0 0 1-8.5 8.5H4l-3 3V11.5A8.5 8.5 0 0 1 9.5 3h3A8.5 8.5 0 0 1 21 11.5Z"/><path d="M6 9h10M6 13h7"/>',target:'<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1"/>',send:'<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>',board:'<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M9 3v18M15 3v18M6 7v5m6-5v8m6-8v3"/>',
  arrow:'<path d="M4 12h16M13 5l7 7-7 7"/>',back:'<path d="M20 12H4m7-7-7 7 7 7"/>',check:'<path d="m5 12 4 4L19 6"/>',close:'<path d="m6 6 12 12M6 18 18 6"/>',
  person:'<circle cx="12" cy="8" r="4"/><path d="M4 21v-2a8 8 0 0 1 16 0v2"/>',briefcase:'<rect x="3" y="7" width="18" height="14" rx="2"/><path d="M8 7V3h8v4M3 12l9 3 9-3M12 12v5"/>',
  shield:'<path d="m12 3 8 3v6c0 5-8 9-8 9S4 17 4 12V6z"/><path d="m8 12 3 3 5-6"/>',mail:'<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 6 9 7 9-7"/>',
  file:'<path d="M14 3H5v18h14V8zM14 3v5h5M8 12h8M8 16h6"/>',plus:'<path d="M12 5v14M5 12h14"/>',search:'<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/>',
  key:'<circle cx="8" cy="8" r="5"/><path d="m12 12 9 9m-4-4 3-3m-6 0 3-3"/>',chart:'<path d="M3 3v18h18M7 16v-4m5 4V7m5 9v-7"/>',receipt:'<path d="M5 3v18l3-2 4 2 4-2 3 2V3l-3 2-4-2-4 2zM9 9h6M9 13h6"/>',
  bot:'<rect x="4" y="6" width="16" height="14" rx="4"/><path d="M12 3v3M8 11v2m8-2v2M9 16h6M1 11v4m22-4v4"/>',logout:'<path d="M9 4H4v16h5m6-4 4-4-4-4m-6 4h10"/>',
  eye:'<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12"/><circle cx="12" cy="12" r="3"/>',upload:'<path d="M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6"/>',info:'<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v1"/>',
  pause:'<path d="M8 5v14M16 5v14"/>',play:'<path d="m8 5 11 7-11 7z"/>',copy:'<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',leaf:'<path d="M20 3C6 2 1 11 7 17s15 1 13-14ZM4 21 16 9"/>',
  globe:'<circle cx="12" cy="12" r="9"/><ellipse cx="12" cy="12" rx="4" ry="9"/><path d="M3 12h18"/>',clock:'<circle cx="12" cy="12" r="9"/><path d="M12 6v6l4 3"/>',link:'<path d="m9 15 6-6M7 17l-1 1a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0m2 8a4 4 0 0 0 5 0l5-5a4 4 0 0 0-5-5l-1 1"/>',
  trash:'<path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7"/>',edit:'<path d="m14 5 5 5M4 20l5-1L21 7l-5-5L4 14z"/>',wallet:'<path d="M21 8H4a2 2 0 0 1 0-4h15v4M3 6v13a2 2 0 0 0 2 2h16V8M21 12h-6v5h6"/>',
};
const icon = (name,cls='') => `<svg class="${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name]||icons.info}</svg>`;
const boat = '<svg class="brand-mark" viewBox="0 0 40 40" fill="currentColor" aria-hidden="true"><path d="M5 24h30l-8 10H13zM18 4v17H4zM22 8l12 13H22z"/></svg>';
const brand = (footer=false) => `<a href="/" data-nav class="brand" ${footer?'':'data-brand'} aria-label="${t('聘渡首页','HireFerry home')}">${boat}<span>聘渡</span>${footer?'':'<span class="brand-en">HireFerry</span>'}</a>`;
const avatar = (user,cls='') => user?.avatar && /^\/api\/avatars\/[a-zA-Z0-9-]+$/.test(user.avatar) ? `<img class="avatar ${cls}" src="${esc(user.avatar)}" alt="${esc(user.name||t('头像','Avatar'))}">` : `<span class="avatar ${cls}" aria-hidden="true">${esc((user?.name||'H').slice(0,1).toUpperCase())}</span>`;
const statusText = value => ({published:t('正在招聘','Published'),draft:t('等待支付','Awaiting payment'),closed:t('已关闭','Closed'),deleted:t('已删除','Deleted'),pending:t('待支付','Pending'),paid:t('已支付','Paid'),paid_needs_review:t('待人工核对','Needs review'),expired:t('已过期','Expired'),failed:t('失败','Failed'),canceled:t('已取消','Canceled'),cancelled:t('已取消','Cancelled'),partially_refunded:t('部分退款','Partially refunded'),refunded:t('已全额退款','Fully refunded'),banned:t('已封禁','Banned'),active:t('正常','Active')})[value]||value;
const badge = value => `<span class="status ${esc(value)}">${esc(statusText(value))}</span>`;
const jobCache = new Map();
const jobHeadcount = job => Number(job.headcount ?? 1);
const headcountText = job => t(`计划招聘 ${jobHeadcount(job).toLocaleString('zh-CN')} 人`,`Hiring target: ${jobHeadcount(job).toLocaleString('en-US')}`);
function jobBadge(job) {
  if(job.status==='published'&&jobHeadcount(job)===0)return `<span class="status closed">${t('暂不招聘','Hiring paused')}</span>`;
  if(job.status==='deleted')return `<span class="status closed">${Number(job.delete_count)===1?t('回收站','In trash'):t('已永久删除','Permanently deleted')}</span>`;
  return badge(job.status);
}
function headcountInput(value) {
  const text=String(value??'').trim();
  const maximum=state.config?.limits?.max_job_headcount ?? 1000000;
  if(!/^\d+$/.test(text)||!Number.isSafeInteger(Number(text))||Number(text)>maximum)throw new Error(t(`招聘人数须为 0 至 ${maximum.toLocaleString('zh-CN')} 的整数。`,`Hiring target must be an integer from 0 to ${maximum.toLocaleString('en-US')}.`));
  return Number(text);
}
function newJobInput(data) {
  const selected=data.get('currency')||currency().currency;
  if(!currencies().some(option=>option.currency===selected))throw new Error(t('请选择可用的薪资币种。','Choose an available salary currency.'));
  return {title:data.get('title'),jd_md:data.get('jd_md'),salary_cents:centsInput(data.get('salary'),selected),currency:selected,provider:data.get('provider')||'',headcount:headcountInput(data.get('headcount')??1)};
}
function headcountControl(job,admin=false) {
  if(!(admin?['published']:['draft','published']).includes(job.status)||(!admin&&!state.user?.employer_paid))return '';
  const fieldId=`headcount-${admin?'admin-':''}${job.id}`;
  return `<details class="section-disclosure job-headcount-disclosure" data-disclosure="headcount-${esc(job.id)}"><summary>${t('调整招聘人数','Change hiring target')}</summary><div class="disclosure-content"><form class="job-headcount-form" data-form="job-headcount" data-id="${esc(job.id)}"${admin?' data-admin="true"':''}><label for="${esc(fieldId)}">${t('招聘人数','Hiring target')} <span>${t('免费调整','Free to change')}</span></label><div class="headcount-input-row"><input id="${esc(fieldId)}" class="input" name="headcount" type="number" min="0" max="${state.config?.limits?.max_job_headcount??1000000}" step="1" inputmode="numeric" value="${jobHeadcount(job)}" required aria-describedby="${esc(fieldId)}-help"><button class="button secondary small" type="submit">${t('保存人数','Save target')}</button></div><p class="help" id="${esc(fieldId)}-help">${job.status==='draft'?t('人数大于 0 且发布付款确认后开始匹配。','A positive target and confirmed publication payment are required to start matching.'):t('填 0 暂停；改回正数恢复。','Set 0 to pause; a positive number resumes hiring.')}</p></form></div></details>`;
}
async function saveJobHeadcount(id,value,admin=false) {
  return api(admin?`/api/admin/jobs/${encodeURIComponent(id)}`:`/api/jobs/${encodeURIComponent(id)}/headcount`,{method:'PATCH',body:{headcount:headcountInput(value)}});
}
function jobDeletionPrompt(job,forcePermanent=false,admin=false) {
  const permanent=admin||forcePermanent||Number(job.delete_count)>=1;
  if(!permanent)return {permanent:false,title:t('把这个岗位移入回收站？','Move this role to the trash?'),description:t('首次删除免费，可在回收站恢复一次或直接永久删除。删除期间双方无法访问匹配资料与会话，平台私聊不受影响。不占描述修改次数，也不会自动退款。','The first deletion is free. You can restore the role once or permanently delete it from the trash. Introduced parties lose access to the role and shared room while it is deleted; private platform chats stay separate. This uses no description update and does not trigger a refund.')};
  const reason=!admin&&!forcePermanent?t('这个岗位已用过一次回收机会。','This role has already used its one recovery opportunity.'):admin?t('管理员执行的是永久删除。','Administrator deletion is permanent.'):'';
  return {permanent:true,title:t('永久删除这个岗位？','Permanently delete this role?'),description:reason+t('本次将清除岗位标题和正文，不能恢复；必要的订单与安全关联记录保留。操作免费，不占描述修改次数，也不会自动退款。',' Its title and description will be erased and cannot be restored. Necessary order and security records remain. The operation is free, uses no description update, and does not trigger a refund.')};
}
async function changeJobLifecycle(job,operation,admin=false) {
  if(!['delete','restore','permanent'].includes(operation)||!Number.isSafeInteger(job.version)||job.version<0)throw new Error(t('岗位记录已变化，请刷新后重试。','Refresh the role before trying again.'));
  const base=`/api/${admin?'admin/':''}jobs/${encodeURIComponent(job.id)}`;
  const path=operation==='restore'?base+'/restore':operation==='permanent'&&!admin?base+'/permanent':base;
  return api(path,{method:operation==='restore'?'POST':'DELETE',body:{expected_version:job.version}});
}
async function jobLifecycleDialog(id,operation='delete',admin=false) {
  const job=jobCache.get(id)||await api(`/api/${admin?'admin/':''}jobs/${encodeURIComponent(id)}`).then(result=>result.job||result);
  const restoring=operation==='restore';
  const prompt=restoring?{title:t('恢复这个岗位？','Restore this role?'),description:t('免费恢复一次。仍符合权限条件的原有匹配可恢复访问。恢复后再次删除将永久清除岗位；开通或发布付款权益失效的岗位只会恢复为已关闭，不会重新招聘。平台私聊始终保持独立。','Restore this role once, free of charge. Existing introductions regain access if they still meet the access rules. Deleting again will be permanent. A role without valid activation or publication payment rights returns only as closed, without resuming hiring. Private platform chats always remain separate.')}:jobDeletionPrompt(job,operation==='permanent',admin);
  confirmDialog(prompt.title,prompt.description,async()=>{await changeJobLifecycle(job,operation,admin);await render();notify(restoring?t('岗位已恢复。再次删除将永久清除。','Role restored. Its next deletion will be permanent.'):prompt.permanent?t('岗位已永久删除。','Role permanently deleted.'):t('岗位已移入回收站，可恢复一次。','Role moved to the trash. It can be restored once.'));},{danger:!restoring&&prompt.permanent});
}
const kindText = kind => ({onboarding:t('招聘方开通','Employer activation'),publish:t('发布招聘','Job publication'),edit:t('修改岗位描述','Job description update')})[kind]||kind;
const roleText = role => ({seeker:t('求职者','Job seeker'),employer:t('招聘方','Employer'),unassigned:t('待选择身份','Choose your role')})[role]||role;
const qs = () => new URLSearchParams(location.search);
const contactEmail = () => state.config?.support_email?.trim() || state.config?.contact_email?.trim() || 'support@lmm.best';
const contactMailto = subject => `mailto:${encodeURIComponent(contactEmail())}${subject?'?subject='+encodeURIComponent(subject):''}`;
const billingReturnKey = 'hireferry-billing-return';
const staffReturnKey = 'hireferry-staff-return';
const jobReturnKey = 'hireferry-job-return';
const profileReturnKey = 'hireferry-profile-return';
const roleIntentKey = 'hireferry-role-intent';
const roleReturnKey = 'hireferry-role-return';
const referralCodeKey = 'hireferry:referral_code';
// Authentication creates a deterministic placeholder so URLs and WebAuthn
// records stay stable before a member chooses a public name. It is not the
// name the member claimed, so onboarding must still offer the claim step.
const isGeneratedHandle = value => /^user_[0-9a-f]{16,27}$/.test(String(value || ''));
function billingReturnPath(orderId='') {return '/billing/return'+(orderId?'?'+new URLSearchParams({order_id:orderId}):'');}
function safeBillingReturn(value) {
  if(!value)return '';
  try {const url=new URL(value,location.origin);if(url.origin!==location.origin||url.pathname!=='/billing/return')return '';const id=url.searchParams.get('order_id')||'';if(id&&!/^[A-Za-z0-9_-]{1,128}$/.test(id))return '';return billingReturnPath(id);}catch{return '';}
}
function safeJobReturn(value) {try{const url=new URL(value,location.origin);const template=url.searchParams.get('template')||'';if(url.origin!==location.origin||url.pathname!=='/app/jobs/new'||(template&&template!=='ai_uiux'))return '';return template?`/app/jobs/new?template=${template}`:'/app/jobs/new';}catch{return '';}}
function safeProfileReturn(value) {try{const url=new URL(value,location.origin);return url.origin===location.origin&&url.pathname==='/app'&&url.searchParams.get('tab')==='profile'?'/app?tab=profile':'';}catch{return '';}}
function validRole(value) {return value==='employer'||value==='seeker'?value:'';}
function currentRoleIntent() {try{return validRole(qs().get('role')||sessionStorage.getItem(roleIntentKey)||'');}catch{return validRole(qs().get('role')||'');}}
function rememberRoleIntent() {const role=validRole(qs().get('role'));if(role)try{sessionStorage.setItem(roleIntentKey,role);}catch{}return role;}
function clearRoleIntent() {try{sessionStorage.removeItem(roleIntentKey);}catch{}}
function rememberRoleReturn(value) {if(!value)return;try{sessionStorage.setItem(roleReturnKey,value);}catch{}}
function peekRoleReturn() {try{return safeJobReturn(sessionStorage.getItem(roleReturnKey))||safeProfileReturn(sessionStorage.getItem(roleReturnKey));}catch{return '';}}
function consumeRoleReturn() {const value=peekRoleReturn();try{sessionStorage.removeItem(roleReturnKey);}catch{}return value;}
function rememberBillingReturn() {rememberRoleIntent();const target=safeBillingReturn(qs().get('return_to'));if(target)try{sessionStorage.setItem(billingReturnKey,target);}catch{}const invite=safeStaffReturn(qs().get('return_to'));if(invite)try{sessionStorage.setItem(staffReturnKey,invite);}catch{}const job=safeJobReturn(qs().get('return_to'));if(job)try{sessionStorage.setItem(jobReturnKey,job);}catch{}const profile=safeProfileReturn(qs().get('return_to'));if(profile)try{sessionStorage.setItem(profileReturnKey,profile);}catch{}}
function safeStaffReturn(value) {try{const url=new URL(value,location.origin);return url.origin===location.origin&&/^\/staff\/invite\/[A-Za-z0-9_-]{43}$/.test(url.pathname)?url.pathname:'';}catch{return '';}}
function afterSignIn() {
  let saved='',staffSaved='',jobSaved='',profileSaved='';
  rememberRoleIntent();
  try{saved=sessionStorage.getItem(billingReturnKey)||'';sessionStorage.removeItem(billingReturnKey);staffSaved=sessionStorage.getItem(staffReturnKey)||'';sessionStorage.removeItem(staffReturnKey);jobSaved=sessionStorage.getItem(jobReturnKey)||'';sessionStorage.removeItem(jobReturnKey);profileSaved=sessionStorage.getItem(profileReturnKey)||'';sessionStorage.removeItem(profileReturnKey);}catch{}
  const review=state.config?.humangrant_enabled?humanGrant().returnAfterLogin():'/app';
  const direct=safeJobReturn(qs().get('return_to'))||safeProfileReturn(qs().get('return_to'))||safeJobReturn(jobSaved)||safeProfileReturn(profileSaved);
  if(direct&&currentRoleIntent())rememberRoleReturn(direct);
  return direct||safeStaffReturn(qs().get('return_to'))||team().returnAfterLogin()||safeStaffReturn(staffSaved)||safeBillingReturn(qs().get('return_to'))||safeBillingReturn(saved)||(review!=='/app'?review:state.staffRole&&!state.staff&&!state.owner?'/admin?tab=matches':state.admin?'/admin?tab=messages':'/app');
}
const pricing = () => state.config?.pricing || {};
function feeSettings(kind,record,terms=null) {
  const rules=terms||pricing(),option=currencies().find(item=>item.currency===currency(record).currency);
  return {mode:rules[`${kind}_mode`]||'salary_percent',bps:rules[`${kind}_bps`]||0,discount_percent:terms?rules[`${kind}_discount_percent`]||0:option?.[`${kind}_discount_percent`]??rules[`${kind}_discount_percent`]??0,fixed_minor:terms?rules[`${kind}_fixed_minor`]||0:option?.[`${kind}_fixed_minor`]??rules[`${kind}_fixed_minor`]??0};
}
function feeRule(kind,record,terms=null) {
  const settings=feeSettings(kind,record,terms);
  { const original = settings.mode==='free'?0:settings.mode==='fixed'?settings.fixed_minor:settings.bps; const effective = settings.mode==='free'||settings.discount_percent===100?0:settings.mode==='fixed'?Math.round(settings.fixed_minor*(100-settings.discount_percent)/100):settings.bps*(100-settings.discount_percent)/100; const label = settings.mode==='free'?t('免费','Free'):settings.mode==='fixed'?t(`每次 ${money(settings.fixed_minor,record)}`,`${money(settings.fixed_minor,record)} per operation`):t(`${percent(settings.bps)} × 月薪底薪`,`${percent(settings.bps)} × base monthly salary`); const discounted = settings.mode==='free'||settings.discount_percent===100?t('免费','Free'):settings.mode==='fixed'?t(`每次 ${money(effective,record)}`,`${money(effective,record)} per operation`):t(`${percent(effective)} × 月薪底薪`,`${percent(effective)} × base monthly salary`); return discountLabel(label,discounted); }
}
function currencySelect(selected=currency().currency,name='currency',id='currency',locked=false) {
  const options=currencies();
  if(!options.some(option=>option.currency===selected))options.push(currency(selected));
  return locked?`<input type="hidden" name="${esc(name)}" value="${esc(selected)}"><input class="input" id="${esc(id)}" value="${esc(selected)}" readonly aria-readonly="true">`:`<select class="input" id="${esc(id)}" name="${esc(name)}">${options.map(option=>`<option value="${esc(option.currency)}" ${option.currency===selected?'selected':''}>${esc(option.currency)}</option>`).join('')}</select>`;
}
const providerText = provider => provider==='free'?t('免费完成','Completed free'):provider==='stripe'?'Stripe':provider==='waffo'?'Waffo Pancake':String(provider||'—');
const orderBadge = order => order.provider==='free'?`<span class="status paid">${t('免费完成','Completed free')}</span>`:badge(order.status);
const percent = bps => (Number(bps)/100).toLocaleString(state.lang==='en'?'en-US':'zh-CN')+'%';
const discountLabel = (original, discounted) => String(original) === String(discounted) ? discounted : `<s>${original}</s> <strong>${discounted}</strong>`;
const discountText = (original, discounted) => String(original) === String(discounted) ? discounted : `${original} → ${discounted}`;
function pagination(total,limit=state.config?.limits?.list_page_size||50) {const page=Math.max(1,Number(qs().get('page'))||1),pages=Math.max(1,Math.min(state.config?.limits?.list_max_page||10000,Math.ceil(Number(total||0)/limit)));if(pages<2)return '';const url=n=>{const p=qs();p.set('page',n);return location.pathname+'?'+p;};return `<nav class="pagination" aria-label="${t('分页','Pagination')}"><a class="button secondary small" ${page>1?`href="${esc(url(page-1))}" data-nav`:'aria-disabled="true"'}>${icon('back')}${t('上一页','Previous')}</a><span>${page} / ${pages}</span><a class="button secondary small" ${page<pages?`href="${esc(url(page+1))}" data-nav`:'aria-disabled="true"'}>${t('下一页','Next')}${icon('arrow')}</a></nav>`;}
function listParams() {const p=new URLSearchParams();for(const k of ['q','page'])if(qs().get(k))p.set(k,qs().get(k));return p.size?'?'+p:'';}
let toastTimer, lastFocus, trapListener, logoClicks=0, logoTimeout;

function requestErrorText(message) {
  return ({
    'Sign in with a passkey again before granting team access': t('请先点击“重新验证通行密钥”，完成验证后再继续团队操作。', 'Choose “Verify passkey again” before continuing this team action.'),
    'Employee access has been disabled': t('员工访问权限已停用，请联系超级管理员。', 'Your staff access is disabled. Contact the super administrator.'),
    'The team has reached its active member limit': t('已达到团队成员数量上限，请先停用不再需要的成员。', 'The team member limit has been reached. Disable a member who no longer needs access.'),
    'An invitation is already pending for this email; resend or revoke it first': t('这个邮箱已有待接受邀请，请在邀请记录中生成新链接，或先撤销原邀请。', 'This email already has a pending invitation. Generate a new link from its invitation record, or revoke it first.'),
    'This account is already an active employee or is suspended': t('该账户已是员工，或当前处于封禁状态。请核对成员和账户状态。', 'This account is already a staff member or is suspended. Check its member and account status.'),
    'This invitation has expired, was revoked or was already accepted': t('邀请已过期、已撤销或已被接受，请刷新查看。', 'This invitation has expired, been revoked, or already been accepted. Refresh to check.'),
    'Sign in with the email address that received this invitation': t('请使用收到此邀请的邮箱登录。', 'Sign in with the email address that received this invitation.'),
    'Restore the suspended account before enabling employee access': t('请先解除该账户的封禁，再启用员工权限。', 'Restore the suspended account before enabling staff access.'),
    'SMTP is not configured; create a copyable invitation link instead': t('邮件服务未配置，请取消发送邮件并生成可复制的邀请链接。', 'Email is not configured. Turn off email delivery to create a copyable invitation link.'),
    'Use a promotion code of 6–64 letters, numbers, hyphens or underscores': t('优惠码须为 6–64 位英文字母、数字、短横线或下划线。','Use 6–64 letters, numbers, hyphens or underscores for the promo code.'),
    'Promotion code not found': t('未找到这个优惠码。','Promo code not found.'),
    'Promotion code is unavailable': t('优惠码不可用，请检查后重试。','The promo code is unavailable. Check it and try again.'),
    'Promotion code is disabled or expired': t('优惠码已停用或到期，请使用其他优惠码。','The promo code is disabled or expired. Use another code.'),
    'This promotion code has no free activations remaining': t('此优惠码没有剩余免费激活次数。','This code has no free activations remaining.'),
    'This promotion code has no publication uses remaining': t('此优惠码没有剩余发布优惠次数。待付订单可能已占用次数。','This code has no publication uses remaining. Pending orders may be reserving its allowance.'),
    'Promotion code already exists': t('此优惠码已存在，请换一个。','This promo code already exists. Choose another one.'),
    'Promotion limits must be 0–1000000 and the discount must be 0–100 percent': t('使用次数须为 0–1000000，减免比例须为 0–100%。','Allowances must be 0–1000000 and the discount 0–100%.'),
    'Promotion expiry must be a future Unix timestamp': t('优惠码有效期须晚于当前时间。','Promo expiry must be in the future.'),
    'Limits cannot be lower than consumed and reserved uses': t('总次数不能小于已使用与待付订单预占次数之和。','Totals cannot be lower than used and reserved allowances.'),
    'An active employer or unassigned account is required': t('需要正常的招聘账号，或尚未选择身份的账号。','An active employer account or an account without a role is required.'),
    'Tell us what you are hiring for': t('请填写招聘计划与申请说明。','Describe your hiring plan and request.'),
    'You already have an application awaiting review': t('已有申请等待审核，请在招聘优惠页查看结果。','An application is already awaiting review. Check Hiring promotions for its result.'),
    'This application has already been reviewed': t('此申请已经审核，请刷新后查看结果。','This application has already been reviewed. Refresh to see the result.'),
    'This request_key was already used for different publication details': t('这次提交已有不同内容的订单记录，请先查看「订单」和「我的招聘」，避免重复发布。','This submission already has an order with different details. Check Orders and My roles before creating another role.'),
    'The price changed; preview the promotion again before publishing': t('本次费用已变化，请重新点击「查看优惠」确认金额后再发布。','The price changed. Select “Check code” again to review the current amount before publishing.'),
    'An activation payment needs review before using a promotion code': t('已有开通付款需要人工核对，请先联系平台处理。','An existing activation payment needs review. Contact HireFerry before using the code.'),
    'Activate your employer account first': t('请先激活招聘账号，再使用发布优惠。','Activate your employer account before using a publication discount.'),
    'This order cannot be cancelled': t('此订单当前不能取消，请刷新查看状态。','This order cannot currently be cancelled. Refresh to check its status.'),
    'This payment provider is not enabled for the order currency': t('此支付方式不支持订单币种，请选择兼容的支付方式或联系平台。','This payment method is not available for the order currency. Choose a compatible method or contact HireFerry.'),
    'Choose a supported salary currency': t('请选择支持的薪资币种。','Choose a supported salary currency.'),
    'Headcount exceeds the configured maximum': t('招聘人数超过站点上限，请按输入框提示调整。','The hiring target exceeds the site limit. Use the range shown beside the field.'),
    'Invalid base monthly salary': t('底薪金额超出允许范围，请检查币种与金额。','The base salary is outside the allowed range. Check its currency and amount.'),
    'Invalid base monthly salary or fee rate': t('底薪或收费规则无法计算，请检查输入或联系平台。','The base salary or fee rule could not be calculated. Check the input or contact HireFerry.'),
    'Invalid fee': t('当前金额无法按收费规则生成有效费用，请检查底薪或联系平台。','This salary does not produce a valid fee under the configured rule. Check the base salary or contact HireFerry.'),
    'Job not found or no longer available for matching': t('这个岗位已不在招聘队列中，请重新选择。', 'This role is no longer in the hiring queue. Choose another role.'),
    'This job is no longer available for new matches; choose another job': t('这个岗位暂不能建立新匹配，请选择其他岗位。已有匹配可在跟进列表查看。', 'This role cannot receive new introductions. Choose another role. Existing introductions remain in Follow-ups.'),
    'Job not found': t('未找到这个岗位，请重新选择。', 'Role not found. Choose another role.'),
    'Candidate not found': t('未找到这位候选人，请刷新结果。', 'Candidate not found. Refresh the results.'),
    'Mandate not found': t('未找到这份委托，请刷新候选人资料。', 'Search brief not found. Refresh the candidate profile.'),
    'Match not found': t('未找到这次匹配，请刷新跟进列表。', 'Introduction not found. Refresh Follow-ups.'),
    'This match already exists': t('这组双方已有匹配，请刷新并打开已有跟进记录。', 'This introduction already exists. Refresh and open its follow-up record.'),
    'Invalid match status': t('匹配阶段无效，请重新选择。', 'Invalid introduction stage. Choose a valid stage.'),
    'Search text must be at most 120 characters without control characters': t('搜索关键词最多 120 字，不能包含控制字符。', 'Search text must be at most 120 characters and contain no control characters.'),
    'Search text must be at most 80 characters without control characters': t('行业、方向或地点最多 80 字，不能包含控制字符。', 'Industry, role, and location filters must be at most 80 characters and contain no control characters.'),
    'Use at most eight search terms': t('一次最多输入 8 个以空格分隔的关键词。', 'Use no more than eight space-separated search terms.'),
    'Page size must be between 1 and 100': t('每页条数须在 1 至 100 之间。', 'Page size must be between 1 and 100.'),
    'Salary range must be ordered and between 0 and 100000000 cents': t('请检查所选币种的薪资范围，最低金额不能高于最高金额。', 'Check the salary bounds in the selected currency. The minimum cannot exceed the maximum.'),
    'Invalid cursor or changed search filters; restart from the first page': t('分页位置已失效或筛选条件已改变，请从第一页重新查询。', 'The page position is invalid or filters have changed. Restart from the first page.'),
    'Headcount must be between 0 and 1000000': t('招聘人数超出站点允许范围，请检查输入。', 'The hiring target exceeds the site’s allowed range. Check the input.'),
    'Headcount can only change for a draft or published job': t('只有草稿或已发布岗位可以调整人数；已关闭或删除的岗位不能用人数调整重新招聘。', 'Only draft or published roles can change their hiring target. A closed or deleted role cannot resume hiring this way.'),
    'Closed jobs cannot resume hiring': t('已关闭的岗位不能重新开始招聘。', 'Closed roles cannot resume hiring.'),
    'Expected job version must be nonnegative': t('岗位版本无效，请刷新后重试。', 'The role version is invalid. Refresh and try again.'),
    'The job changed; refresh before deleting or restoring it': t('岗位已发生变化，请刷新后重新确认删除或恢复。', 'The role has changed. Refresh and confirm deletion or restoration again.'),
    'This job cannot be restored': t('这个岗位无法恢复。仅首次删除后、仍在回收站的岗位可恢复一次；永久删除不能恢复。', 'This role cannot be restored. Only a role in the trash after its first deletion can be restored once; permanent deletion cannot be undone.'),
    'The job has no restorable state': t('岗位没有可恢复的状态，请联系平台核对。', 'The role has no recoverable state. Contact HireFerry for a review.'),
    'The job has no restorable headcount': t('岗位缺少可恢复的人数记录，请联系平台核对。', 'The role has no recoverable hiring target. Contact HireFerry for a review.'),
    'An active employer account is required': t('此操作需要未被封禁的招聘方账户。', 'This action requires an employer account that is not suspended.'),
    'This job belongs to another account': t('你只能管理自己账户的岗位。', 'You can only manage roles belonging to your account.'),
    'Only paid jobs can be changed': t('管理员只能修改已付款的岗位；草稿由招聘方管理。', 'Administrators can only modify paid roles. Drafts are managed by their employer.'),
    'Resolve the pending payment before editing this job': t('请先处理此岗位的待付订单，再修改描述或状态。招聘人数仍可独立免费调整。', 'Resolve the role’s pending payment before changing its description or status. The hiring target can still be changed separately for free.'),
    'This activation order expired. Create a new activation order from Orders.': t('这笔开通订单已过期，可在订单页重新发起开通。','This activation order expired. Restart activation from Orders.'),
    'An activation payment is already recorded. Please contact support to check it before paying again.': t('已有开通款项入账，请先联系平台核对，无需再次付款。','An activation payment is recorded. Contact support to review it before paying again.'),
    'An activation order already exists in another currency or provider. Continue it from Orders or contact support.': t('已有一笔待支付的开通订单。请在订单页继续付款，或等待其到期后重新选择。','An activation order is already awaiting payment. Continue it from Orders, or choose again after it expires.'),
    'This payment can no longer be applied; refresh the order before continuing': t('这笔订单已无法继续付款，请刷新查看最新状态。','This order can no longer be paid. Refresh to check its current status.'),
    'This order is not pending': t('这笔订单已不处于待付款状态，请刷新订单查看。', 'This order is no longer awaiting payment. Refresh to check its current status.'),
    'A deleted job cannot be checked out': t('已删除岗位的订单暂不能付款。请先在回收站恢复岗位，或联系平台核对。', 'A deleted role cannot proceed to checkout. Restore it from the trash first, or contact HireFerry for a review.'),
    'Match requires an active seeker mandate and a paid, published job with positive hiring headcount and active accounts': t('建立匹配需要正在寻找的求职委托、已付款发布且招聘人数大于 0 的岗位，以及双方正常的账户。', 'An introduction requires an active search brief, a paid published role with a hiring target above zero, and active accounts.'),
    'Webhook endpoint limit reached': t('已达到联动端点数量上限。', 'The webhook endpoint limit has been reached.'),
    'Webhook delivery queue is full': t('投递队列暂时已满，请稍后再试。', 'The delivery queue is full. Please try again later.'),
    'Enable this webhook before testing': t('请先启用这个端点，再发送测试通知。', 'Enable this endpoint before sending a test.'),
    'Only failed deliveries can be retried': t('只有失败的投递可以手动重试。', 'Only failed deliveries can be retried manually.'),
    'This event is no longer authorized for delivery': t('这个事件的访问权限已撤销，不再允许投递。', 'Access to this event was revoked. It can no longer be delivered.'),
    'Wait one minute between webhook tests': t('请等待一分钟后再发送测试。', 'Wait one minute before sending another test.'),
    'Wait one minute between manual retries': t('请等待一分钟后再手动重试。', 'Wait one minute before retrying manually again.'),
    'Outbound webhooks are disabled by the administrator': t('联动投递暂未开放，可以查看历史、停用或删除端点。', 'Webhook delivery is paused. You can review history, disable endpoints, or delete them.'),
    'Choose one or more supported webhook events': t('请至少选择一种支持的联动事件。', 'Choose at least one supported webhook event.'),
    'Use a webhook name between 1 and 100 characters': t('端点名称须为 1 至 100 个字符。', 'Use an endpoint name between 1 and 100 characters.'),
    'Use a public HTTPS URL on port 443 without credentials or a fragment': t('请填写公网 HTTPS / 443 地址，不能包含账号密码或 # 片段。', 'Use a public HTTPS URL on port 443, without credentials or a fragment.'),
  })[message] || message;
}

const requests = createRequestClient(async (path,{method='GET',body,...options}={}) => {
  const headers = {...options.headers};
  if(body!==undefined && !(body instanceof FormData)) { headers['Content-Type']='application/json'; body=JSON.stringify(body); }
  if(method!=='GET' && state.csrf) headers['X-CSRF-Token']=state.csrf;
  const response=await fetch(path,{method,body,credentials:'same-origin',cache:'no-store',...options,headers});
  const result=await response.json().catch(()=>({}));
  if(!response.ok) { const e=new Error(requestErrorText(result.error||result.message||`${t('请求失败','Request failed')} (${response.status})`)); e.status=response.status;throw e; }
  return result;
});
const sessionReader = createSessionReader(() => api('/api/session',{fresh:true}));
// A public page may paint before its identity read is applied. Explicit resets
// still provide a known guest state so expiry can show sign-in immediately.
let sessionKnown=false;
const configCache = createPublicConfigCache(() => api('/api/config',{fresh:true}));
const cachedConfig = configCache.peek();
if(cachedConfig){applyConfig(cachedConfig);state.configCached=true;}
const publicPages = new Set(['/','/cli','/downloads','/contact','/refunds','/for-employers']);
const isPublicDirectoryPage = path => path==='/discover'||/^\/discover\/(?:jobs|people)\/[^/]+$/.test(path);
const isPublicProfilePage = path => /^\/u\/[A-Za-z0-9_-]+$/.test(path);
const isPublicPagePath = path => publicPages.has(path)||isPublicDirectoryPage(path)||isPublicProfilePage(path);
function applyConfig(config) {
  state.config=config;state.configCached=false;
  if(!savedLanguage())state.lang=config.default_language==='en'?'en':'zh';
  document.documentElement.lang=state.lang==='en'?'en':'zh-CN';
  updatePageMetadata();
}
function updatePageMetadata() {
  syncPageMetadata({doc:document,path:location.pathname,lang:state.lang,config:state.config,origin:location.origin});
}
function publicBody(path) {
  if(path==='/')return home();
  if(path==='/for-employers')return employerLandingPage();
  if(path==='/cli')return renderCliGuide({config:state.config,origin:location.origin,lang:state.lang,esc,icon});
  if(path==='/downloads')return renderDownloads({lang:state.lang,icon});
  if(path==='/contact')return contactPage();
  if(path==='/refunds')return refundsPage();
}
async function refreshPublicPage(version,{refreshConfig=true}={}) {
  if(!publicPages.has(location.pathname)||version!==state.render)return;
  const path=location.pathname,before=identityOf(state),language=state.lang,displayedConfig=state.config;
  const isCurrent=()=>version===state.render&&location.pathname===path;
  sessionKnown=false;
  root.dataset.configState='checking';
  try {
    // Settle both reads before applying identity, which cancels private requests.
    // Either successful result remains useful when the other endpoint is unavailable.
    const [configuration,identity]=await Promise.allSettled([refreshConfig?configCache.refresh():Promise.resolve({value:state.config,changed:false}),sessionReader.read()]);
    if(!isCurrent())return;
    if(identity.status==='fulfilled')await session(version);
    if(!isCurrent())return;
    if(configuration.status==='fulfilled')applyConfig(configuration.value.value);
    root.dataset.configState=configuration.status==='fulfilled'?'fresh':'stale';
    if(samePublicConfig(displayedConfig,state.config)&&before===identityOf(state)&&language===state.lang)return;
    const html=header()+publicBody(path)+footer();
    await animateContentRefresh(root,html,()=>{
      state.cliGuide?.dispose();state.cliGuide=null;
      state.downloads?.dispose();state.downloads=null;
      if(path==='/'){state.homeStory?.dispose();state.homeStory=null;state.particleTitles?.dispose();state.particleTitles=null;state.sculpture?.dispose();state.sculpture=null;}
      mountPage(root,html,pageIdentity(path,qs().get('tab')));
      disclosureUI.sync({scope:identityOf(state)});syncCurrencyFields();mountBrand();
      if(path==='/cli')state.cliGuide=mountCliGuide(root);
      if(path==='/downloads')state.downloads=mountDownloads(root,{lang:state.lang,icon});
      if(path==='/')void mountHome();
    },{isCurrent});
  }catch{if(isCurrent())root.dataset.configState='stale';}
}
const identityOf = value => `${value.super_admin?'owner':value.staff?`staff:${value.staff.id}`:(value.staffRole||value.staff_role)==='manager'?`manager:${value.user?.id||''}`:value.user?.id||'guest'}:${value.csrf||''}`;
const authMutation = path => /^\/api\/(?:auth\/|role$|team\/invitations\/accept$|staff\/invitations\/[^/]+\/accept$|admin\/(?:auth\/bootstrap|passkeys\/.*\/finish))/.test(path);
function invalidateIdentity() {
  invalidateWebMCP();resetAdminViews();sessionKnown=false;sessionReader.invalidate();requests.invalidate({abort:true});configCache.invalidate();jobCache.clear();state.eligibility=null;
}
function announceIdentityChange() {try{localStorage.setItem('hireferry-session-change',`${Date.now()}:${Math.random()}`);}catch{}}
async function api(path,{method='GET',body,fresh=false,cache,...options}={}) {
  method=method.toUpperCase();
  const requestIdentity=identityOf(state);
  const write=method!=='GET',identityChange=write&&authMutation(path);
  const changesProfile=write&&(path==='/api/profile'||path.startsWith('/api/profile/'));
  // Graph queries and presentation state never mutate people, jobs or introductions.
  // Like read receipts, they must not invalidate unrelated navigation reads on every interaction.
  const changesData=write&&!/^\/api\/chat\/(?:platform$|conversations\/[^/]+\/read$)/.test(path)&&!path.startsWith('/api/admin/relations/')&&path!=='/api/markdown';
  if(identityChange)invalidateIdentity();else if(changesData){requests.invalidate();state.eligibility=null;}
  if(changesProfile)sessionReader.invalidate();
  try {
    const group=method==='GET'&&root.dataset?.navigationState==='loading'&&!options.prefetch&&!options.headers&&!['/api/session','/api/config'].includes(path)?`navigation:${state.render}`:undefined;
    const result=await requests.request(path,{method,body,group,...options,fresh:fresh||cache===false});
    if(identityChange){invalidateIdentity();announceIdentityChange();}
    else if(changesData)requests.invalidate();
    if(changesProfile)sessionReader.invalidate();
    return result;
  } catch(error) {
    if([401,403].includes(error.status)&&identityOf(state)===requestIdentity) {
      if(error.status===401&&(state.user||state.admin)) {
        const administrator=state.admin;
        resetPrivateView();
        // A background poll can discover expiry too. Remove private DOM in this same task.
        if(location.pathname.startsWith('/app'))void go('/login');
        else if(location.pathname==='/admin'&&administrator)void go('/admin');
        else void render();
      }else{requests.invalidate();sessionReader.invalidate();state.eligibility=null;invalidateWebMCP();}
    }
    if(error instanceof RequestTimeoutError)error.message=error.uncertain?t('请求超时，操作结果尚未确认。请刷新查看，再决定是否重试。','The request timed out and its result is unknown. Refresh to check before retrying.'):t('服务器响应超时。请检查网络后重试。','The server took too long to respond. Check your connection and try again.');
    throw error;
  }
}
async function session(renderVersion) {
  const s=await sessionReader.read({force:renderVersion===undefined});
  if(renderVersion===undefined||renderVersion===state.render){
    const elevated={...s,admin:!!s.admin||s.staff_role==='manager',owner:s.super_admin===true,super_admin:s.super_admin===true,staffRole:s.staff_role||null,staff:s.staff||null};
    if(webMCPIdentity(state)!==webMCPIdentity(elevated)){invalidateWebMCP();requests.invalidate({abort:true});jobCache.clear();state.eligibility=null;}
    Object.assign(state,{user:s.user,admin:elevated.admin,owner:elevated.owner,super_admin:elevated.super_admin,staffRole:elevated.staffRole,staff:elevated.staff,bootstrap:s.bootstrap,csrf:s.csrf});
    sessionKnown=true;registerWebMCP({refreshEligibility:true});
  }
  return s;
}
function resumeCliLogin(authenticated=false) {try{const path=takeCliReturn(sessionStorage,location.origin,state,authenticated);if(path){location.assign(path);return true;}}catch{/* Browser storage is optional. */}return false;}
function notify(message,error=false) {clearTimeout(toastTimer);document.querySelector('#toast-root').innerHTML=`<div class="toast ${error?'error':''}" role="status">${icon(error?'info':'check')}<span>${esc(message)}</span></div>`;toastTimer=setTimeout(()=>document.querySelector('#toast-root').replaceChildren(),5500);}
async function copyText(value) {
  try { await navigator.clipboard.writeText(value); return true; }
  catch {
    const area=document.createElement('textarea');area.value=value;area.setAttribute('readonly','');area.style.position='fixed';area.style.opacity='0';document.body.append(area);area.select();let copied=false;try{copied=document.execCommand('copy');}catch{/* Clipboard fallback is optional. */}area.remove();if(!copied)throw new Error(t('设备不支持复制，请手动选择文本。','Copy is unavailable on this device. Select the text manually.'));return true;
  }
}
function setBusy(el,busy) {if(!el)return;el.disabled=busy;el.setAttribute('aria-busy',String(busy));}
function go(path) {const next=new URL(path,location.href);if(next.origin!==location.origin)return;const changedPage=pageIdentity(location.pathname,qs().get('tab'))!==pageIdentity(next.pathname,next.searchParams.get('tab'));closeDialog();history.pushState({},'',next.pathname+next.search+next.hash);if(changedPage)window.scrollTo({top:0,behavior:'instant'});return render({navigation:true});}
function themeMenu() {return `<details class="theme-menu"><summary aria-label="${t('切换主题','Change theme')}" title="${t('主题','Theme')}">${icon('palette')}</summary><div class="theme-panel"><div class="palette-options" role="group" aria-label="${t('配色','Color palette')}">${[['neutral','纸墨','Paper & ink'],['blue','海蓝','Ocean blue'],['violet','紫罗兰','Violet'],['rose','玫瑰','Rose'],['amber','琥珀','Amber'],['teal','青碧','Teal']].map(([value,zh,en])=>`<button type="button" class="palette-swatch" data-palette-choice="${value}" aria-label="${t(zh,en)}" title="${t(zh,en)}" aria-pressed="false"><span class="swatch-color" aria-hidden="true">${icon('check')}</span></button>`).join('')}</div><div class="mode-options" role="group" aria-label="${t('外观','Appearance')}">${[['light','sun','浅色','Light'],['dark','moon','深色','Dark'],['system','monitor','跟随系统','System']].map(([value,glyph,zh,en])=>`<button type="button" data-theme-choice="${value}" aria-label="${t(zh,en)}" title="${t(zh,en)}" aria-pressed="false">${icon(glyph)}</button>`).join('')}</div></div></details>`;}
const githubStars = new Map();
const githubStarRequests = new Map();
function sourceRepository() {
  try {
    const url=new URL(state.config?.source_url||'https://github.com/TokenNotIncluded/hireferry');
    return url.protocol==='https:'&&url.hostname==='github.com'&&/^\/[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+\/?$/.test(url.pathname)?url.pathname.replace(/^\/|\/$/g,'').replace(/\.git$/,''):null;
  } catch { return null; }
}
function openSourceBadge() {
  const repo=sourceRepository();
  return `<a class="open-source-badge" href="${esc(state.config?.source_url||'https://github.com/TokenNotIncluded/hireferry')}" target="_blank" rel="noopener noreferrer" title="${t('查看源代码 · AGPL-3.0-or-later','View source · AGPL-3.0-or-later')}"${repo?` data-source-repository="${esc(repo)}"`:''}>${icon(repo?'github':'file')}<span>${t('自由开源','Free & open source')}</span>${repo?`<span class="open-source-stars">${icon('star')}<span data-github-stars>Star</span></span>`:''}</a>`;
}
async function mountGithubStars() {
  const badge=root.querySelector('[data-source-repository]');if(!badge)return;
  const repo=badge.dataset.sourceRepository,key=`hireferry:github-stars:${repo}`;
  const valid=value=>Number.isSafeInteger(value?.count)&&value.count>=0&&Number.isFinite(value?.at)&&value.at>0;
  const paint=value=>{
    for(const link of root.querySelectorAll('[data-source-repository]')) {
      if(link.dataset.sourceRepository!==repo)continue;
      const count=link.querySelector('[data-github-stars]');
      count.textContent=value.count.toLocaleString(state.lang==='en'?'en-US':'zh-CN');
      count.setAttribute('aria-label',`${value.count.toLocaleString('en-US')} GitHub Stars`);
    }
  };
  let cached=githubStars.get(repo);
  if(!cached)try{const value=JSON.parse(localStorage.getItem(key));if(valid(value))githubStars.set(repo,cached=value);}catch{/* Storage is optional. */}
  if(cached)paint(cached);
  if(cached&&Date.now()-cached.at>=0&&Date.now()-cached.at<3600000)return;
  if(!githubStarRequests.has(repo))githubStarRequests.set(repo,(async()=>{
    const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),5000);
    try {
      const response=await fetch(`https://api.github.com/repos/${repo}`,{credentials:'omit',referrerPolicy:'no-referrer',signal:controller.signal,headers:{Accept:'application/vnd.github+json'}});
      if(!response.ok)return null;
      const data=await response.json(),value={count:data.stargazers_count,at:Date.now()};
      if(!valid(value))return null;
      githubStars.set(repo,value);
      try{localStorage.setItem(key,JSON.stringify(value));}catch{/* Storage is optional. */}
      return value;
    } catch { return null; } finally {clearTimeout(timer);}
  })());
  const value=await githubStarRequests.get(repo);if(value)paint(value);
}
function header() {
  const workspace=(location.pathname==='/admin'&&state.admin)||(location.pathname.startsWith('/app')&&state.user);
  const navigation = `<a href="/discover" data-nav>${t('发现机会','Discover')}</a><a href="/#how" data-scroll="how">${t('服务流程','How it works')}</a><a href="/#pricing" data-scroll="pricing">${t('服务与定价','Pricing')}</a><a href="/downloads" data-nav>${t('下载','Downloads')}</a>`;
  if(workspace)return `<header class="site-header workspace-header" data-dom-key="site-header">${brand()}<div class="workspace-location"><span class="workspace-location-divider" aria-hidden="true">/</span>${state.admin?t('管理工作台','Administration'):t('我的工作台','Workspace')}</div><div class="nav-right"><a href="/contact" data-nav class="workspace-help">${icon('info')}<span>${t('帮助','Help')}</span></a>${themeMenu()}<button class="language" type="button" data-action="language" aria-label="${t('Switch to English','切换为中文')}">${t('EN','中文')}</button></div></header>${state.config?.development?`<div class="environment-banner workspace-environment" data-dom-key="environment">${t('测试环境 · 以下为测试资料','Test environment · Test data only')}</div>`:''}`;
  return `<header class="site-header page-width" data-dom-key="site-header">${brand()}<nav class="nav" aria-label="${t('主导航','Primary navigation')}">${navigation}</nav><div class="nav-right">${themeMenu()}<button class="language" type="button" data-action="language" aria-label="${t('Switch to English','切换为中文')}">${t('EN','中文')}</button><a class="button small" href="${state.admin?'/admin':state.user?'/app':'/login'}" data-nav>${state.admin?t('总控台','Control room'):state.user?t('我的工作台','My workspace'):t('登录 / 注册','Get started')} ${icon('arrow')}</a></div></header><nav class="mobile-site-nav page-width" data-dom-key="mobile-nav" aria-label="${t('了解聘渡','Explore HireFerry')}">${navigation}</nav>${state.config?.development?`<div class="environment-banner" data-dom-key="environment">${t('演示 / 开发环境 · 请勿提交真实个人资料','Demo / development environment · Please use test data')}</div>`:''}`;
}
function footer() {return `<footer class="footer page-width" data-dom-key="footer"><div class="footer-left">${brand(true)}<span>© ${new Date().getFullYear()} HireFerry</span></div><nav class="footer-links" aria-label="${t('服务说明与联系','Service information and contact')}"><a href="/downloads" data-nav>${t('下载','Downloads')}</a><a href="/cli" data-nav>${t('CLI 与技能','CLI & skills')}</a><a href="/privacy" data-nav>${t('隐私','Privacy')}</a><a href="/terms" data-nav>${t('服务条款','Terms')}</a><a href="/refunds" data-nav>${t('退款政策','Refunds')}</a><a href="/contact" data-nav>${t('联系','Contact')}</a>${state.config?.support_email?`<a href="${esc(contactMailto())}" aria-label="${t('客服邮箱','Support email')}">${esc(contactEmail())}</a>`:''}<a href="${esc(state.config?.source_url||'https://github.com/TokenNotIncluded/hireferry')}" target="_blank" rel="noopener noreferrer">${t('自由开源 · AGPL-3.0','Free & open source · AGPL-3.0')}</a></nav></footer>`;}
function heading(title,subtitle='',action='') {return `<div class="page-heading"><div><h1>${title}</h1>${subtitle?`<p>${subtitle}</p>`:''}</div>${action}</div>`;}
function empty(title,description,action='',ico='file') {return `<div class="empty-state"><span class="empty-icon">${icon(ico)}</span><h3>${title}</h3><p>${description}</p>${action}</div>`;}
function notice(text,style='') {return `<div class="notice ${style}">${icon('info')}<div>${text}</div></div>`;}
function dialogFocusables() {
  return [...dialogs.querySelectorAll('a[href],button,input:not([type="hidden"]),textarea,select,summary,[tabindex]')]
    .filter(node => node.tabIndex >= 0 && !node.matches(':disabled') && !node.closest('[hidden],[inert]') && node.getClientRects().length && window.getComputedStyle(node).visibility === 'visible')
    .sort((a,b) => (a.tabIndex || Infinity) - (b.tabIndex || Infinity));
}
function openDialog(content,{close=true,wide=false}={}) {
  closeDialog();lastFocus=document.activeElement;
  dialogs.innerHTML=`<div class="dialog-backdrop"><section class="dialog ${wide?'wide':''}" role="dialog" aria-modal="true" aria-labelledby="dialog-title">${close?`<button class="icon-button dialog-close" data-action="close-dialog" aria-label="${t('关闭','Close')}">${icon('close')}</button>`:''}${content}</section></div>`;
  controlsUI.sync(dialogs);document.body.style.overflow='hidden';dialogFocusables()[0]?.focus();
  trapListener=e=>{
    if(e.key==='Escape'&&close)closeDialog();
    if(e.key!=='Tab')return;
    const focusables=dialogFocusables();
    if(!focusables.length)return;
    const start=focusables[0],end=focusables.at(-1);
    if(e.shiftKey&&document.activeElement===start){e.preventDefault();end.focus();}
    else if(!e.shiftKey&&document.activeElement===end){e.preventDefault();start.focus();}
  };
  document.addEventListener('keydown',trapListener);
}
function closeDialog() {const hadDialog=!!dialogs.querySelector('[role="dialog"]'),opener=lastFocus;lastFocus=null;controlsUI.clear(dialogs);dialogs.replaceChildren();document.body.style.overflow='';if(trapListener)document.removeEventListener('keydown',trapListener);trapListener=null;if(hadDialog&&opener?.isConnected)opener.focus?.();}
function confirmDialog(title,description,action,{danger=false}={}) {openDialog(`<h2 id="dialog-title">${title}</h2><p class="dialog-intro">${description}</p><div class="form-actions"><button class="button secondary" data-action="close-dialog">${t('取消','Cancel')}</button><button class="button ${danger?'danger':''}" id="confirm-action">${t('确认','Confirm')}</button></div>`);document.querySelector('#confirm-action').addEventListener('click',async e=>{setBusy(e.currentTarget,true);try{await action();closeDialog();}catch(error){notify(error.message,true);setBusy(e.currentTarget,false);}});}

function pricingSection() {
  const meta=currency(),publish=feeSettings('publish'),example=parseMinor('6000',meta.currency_decimals);
  const price=feeRule('publish',meta);
  const publishFree=publish.mode==='free'||publish.discount_percent===100;
  const activationFree=Number(pricing().onboarding_minor??pricing().onboarding_cents)===0;
  const activationLabel=activationFree?t('免费','Free'):discountLabel(money(pricing().onboarding_original_minor??pricing().onboarding_cents),money(pricing().onboarding_minor??pricing().onboarding_cents));
  const activationText=activationFree?t('早期阶段免费开通，可发布多个岗位。','Free early access includes multiple roles.'):t('开通后可发布多个岗位。','Publish multiple roles with one account.');
  const publishText=publishFree?t('早期阶段免费发布，确认后立即开始匹配。','Free early access publication starts matching immediately.'):t('支付后开始匹配。','Matching starts after payment.');
  const suffix=publish.mode==='salary_percent'?t('岗位月薪底薪','base monthly salary'):t('每次发布','per publication');
  let quote='—';try{quote=money(calculateFee(example,publish),meta);}catch{}
  return `<section class="section pricing-section" id="pricing"><div class="section-heading"><div><h2>${t('服务费用','Pricing')}</h2></div><p>${t('起步阶段求职和招聘都完全免费。','Early access is completely free for job seekers and employers.')}</p></div><div class="pricing-grid"><article class="pricing-card seeker"><div class="pricing-kicker">${t('零门槛入口 · 早期阶段','Zero-cost entry · Early access')}</div><h3>${t('给求职者','For job seekers')}</h3><div class="price">${t('免费','Free')}</div><p>${t('提交简历。让 AI 初筛与匹配。','Share your résumé. Let AI screen and match.')}</p><ul class="pricing-points"><li>${t('写清目标，减少盲目投递','Set a clear target and stop applying blindly')}</li><li>${t('连接 AI，持续筛选机会','Connect AI and keep screening opportunities')}</li></ul><a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button secondary">${t('注册独一无二的名字','Register a unique name')}${icon('arrow')}</a></article><article class="pricing-card"><div class="pricing-details"><div><h3>${t('招聘方开通','Employer activation')}</h3><div class="price"><span data-refresh-key="pricing-activation">${activationLabel}</span><small>/ ${t('一次','once')}</small></div><p>${activationText}</p></div><div><h3>${t('每个招聘广告','Each job advertisement')}</h3><div class="price"><span data-refresh-key="pricing-publication">${price}</span><small>/ ${suffix}</small></div><p>${publishText}</p></div></div><div class="price-calculator"><label for="salary-example">${t('例如底薪','Example salary')} · ${esc(meta.currency)}<input id="salary-example" type="number" min="${amountStep(meta)}" max="${amountValue(1000000000,meta)}" step="${amountStep(meta)}" value="6000" inputmode="decimal" aria-label="${esc(t('岗位月薪底薪','Base monthly salary')+' · '+meta.currency)}"></label><span>${t('发布费','Posting fee')} <strong class="price-result" id="salary-example-result">${quote}</strong></span></div><div class="pricing-note" data-refresh-key="pricing-note">${t(`按固定底薪计费。发布后锁定标题、币种和底薪。<br>描述修改每次 ${feeRule('edit')}。最多 ${pricing().max_edits} 次。<br>招聘人数可免费调整。填 0 暂停新匹配。`,`Fees use fixed base pay. Publication locks the title, currency and salary.<br>Description updates: ${feeRule('edit')}. Up to ${pricing().max_edits} updates.<br>Change hiring targets for free. Set 0 to pause new introductions.`)}</div>${currencies().length>1?`<details class="currency-fees"><summary>${t('查看各币种费用','Fees by currency')}</summary>${currencyFeeTable()}<p class="help">${t('开通时选择付款币种。岗位费用按创建时选择的币种结算。','Choose a currency at checkout. Role fees use the currency selected for that role.')}</p></details>`:''}</article></div></section>`;
}
function currencyFeeTable() {
  return `<div class="table-wrap"><table class="data-table"><thead><tr><th>${t('币种','Currency')}</th><th>${t('发布费','Publication')}</th><th>${t('每次描述修改','Each description update')}</th></tr></thead><tbody>${currencies().map(meta=>`<tr><th scope="row">${esc(meta.currency)}</th><td data-refresh-key="fee-publish-${esc(meta.currency)}">${feeRule('publish',meta)}</td><td data-refresh-key="fee-edit-${esc(meta.currency)}">${feeRule('edit',meta)}</td></tr>`).join('')}</tbody></table></div>`;
}

function publicDiscoveryPromo() {
  return `<section class="public-discovery-promo" aria-labelledby="public-discovery-promo-title"><div><div class="pricing-kicker">${t('公开目录 · MCP 就绪','OPEN DIRECTORY · MCP READY')}</div><h2 id="public-discovery-promo-title">${t('先浏览公开机会，再决定注册。','Browse public opportunities before you register.')}</h2><p>${t('岗位和公开主页无需登录即可查看。找到方向后，再免费注册独一无二的名字，持续接收 AI 匹配。','Browse public roles and profiles without an account. When you find your direction, register a unique name for free and let AI keep matching.')}</p></div><div class="public-discovery-promo-actions"><a href="/discover" data-nav class="button secondary">${t('浏览公开目录','Browse public directory')}${icon('arrow')}</a><a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button">${t('免费创建个人主页','Create a free profile')}${icon('arrow')}</a><a href="/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew" data-nav class="button ghost">${t('免费发布岗位','Post a free role')}${icon('arrow')}</a></div></section>`;
}

function home() {
  const activationFree=Number(pricing().onboarding_minor??pricing().onboarding_cents)===0;
  const publishFree=feeSettings('publish').mode==='free'||feeSettings('publish').discount_percent===100;
  const stories = [
    {
      title:t('投了几百份简历<br>却石沉大海？<br><em>让 AI 持续帮你找</em>','Hundreds of résumés<br>and no reply?<br><em>Let AI keep looking</em>'),
      description:t('只需填写目标、地点和薪资期待。<br>连接 AI 后，7×24 持续筛选岗位、匹配机会，不用再盲目海投。','Set your target role, location and salary expectations.<br>Connect AI to keep screening roles and matching opportunities around the clock.')
    },
    {
      title:t('先聊需求<br>再做匹配<br><em>合适再认识</em>','Share your needs<br>Find the fit<br><em>Meet your match</em>'),
      description:t('先把目标说清楚。<br>AI 按岗位要求和项目经历做匹配，合适后再共享资料。','Start with a clear brief.<br>AI matches role requirements with project experience before profiles are shared.')
    },
    {
      title:t('起步阶段免费<br>求职与招聘<br><em>先找到合适的人</em>','Free early access<br>Find work and hire<br><em>Meet the right people</em>'),
      description:activationFree&&publishFree?t('招聘账户和岗位发布都免费。<br>先注册独一无二的名字，再开始找人或找工作。','Employer activation and job publication are free.<br>Claim a unique username, then find work or hire.') : t(`招聘账户 ${discountLabel(money(pricing().onboarding_original_minor??pricing().onboarding_cents),money(pricing().onboarding_minor??pricing().onboarding_cents))} 一次开通。<br>岗位发布按当前规则计费。`,`Activate an employer account for ${money(pricing().onboarding_cents)}.<br>Pay per job at the published rate.`)
    }
  ];
  return `<main id="main" class="home-main" itemscope itemtype="https://schema.org/WebSite">
    <meta itemprop="name" content="HireFerry"><meta itemprop="alternateName" content="聘渡"><link itemprop="url" href="${esc(state.config?.public_origin||location.origin)}/">
    <section class="home-story" aria-label="${t('认识聘渡','Discover HireFerry')}">
      <div class="hero page-width">
        <div class="hero-copy">
          <div class="hero-slides">${stories.map((story,i)=>`<article class="hero-slide" data-story-slide="${i}" aria-hidden="${i!==0}"><h1>${story.title}</h1><p class="hero-description" data-refresh-key="home-story-${i}">${story.description}</p></article>`).join('')}</div>
          <div class="hero-actions"><a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button">${t('免费开始找工作','Start finding work free')}${icon('arrow')}</a><a href="/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew" data-nav class="button secondary">${t('免费创建岗位','Create a free role')}${icon('arrow')}</a></div>
          ${openSourceBadge()}
        </div>
        <div class="sculpture-panel">
          <canvas id="sculpture" aria-label="${t('渡船、桥与罗盘粒子雕塑','Particle sculptures of a ferry, bridge and compass')}"></canvas>
          <div class="fallback" hidden>HireFerry</div>
          <button type="button" class="sculpture-pause icon-button" data-action="pause" aria-label="${t('暂停动画','Pause animation')}" aria-pressed="false">${icon('pause')}</button>
        </div>
      </div>
    </section>
    <div class="home-flow page-width">
      <section class="section" id="how">
        <div class="section-heading"><h2>${t('不用再盲目海投','Stop applying blindly')}</h2><p>${t('填写目标，连接 AI，让寻找持续运行。','Set a target, connect AI, and keep the search moving.')}</p></div>
        <div class="steps">
          <article class="step"><h3>${t('填写你的目标','Set your target')}</h3><p>${t('写清岗位方向、地点、薪资和工作方式。<br>简历与项目经历一起提交。','Set your role, location, salary and work style.<br>Share your résumé and project experience.')}</p></article>
          <article class="step"><h3>${t('AI 持续筛选','AI keeps screening')}</h3><p>${t('连接后 7×24 持续理解岗位、筛选机会并匹配。<br>不再重复改简历和盲目投递。','Once connected, AI keeps understanding roles, screening opportunities and matching.<br>Stop rewriting and applying blindly.')}</p></article>
          <article class="step"><h3>${t('开始交流','Start a conversation')}</h3><p>${t('匹配后共享资料。<br>在专属会话中沟通下一步。','Share profiles after a match.<br>Discuss next steps in your shared room.')}</p></article>
        </div>
      </section>
      ${pricingSection()}
      ${publicDiscoveryPromo()}
      <section class="home-bottom"><h2>${t('从这里开始','Start here')}</h2><a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button">${t('注册独一无二的名字','Register a unique name')}${icon('arrow')}</a></section>
    </div>
  </main>`;
}

let brandMotionModule;
function mountBrand() {
  if (state.brandMotion || !window.IntersectionObserver) return;
  brandMotionModule ||= import('./brand-motion.js');
  brandMotionModule.then(({mountBrandMotion}) => {
    if (!state.brandMotion) {
      state.brandMotion = mountBrandMotion();
      state.brandMotion.pause(state.motionPaused);
    }
  }).catch(() => { brandMotionModule = null; });
}

let homeModules;
async function mountHome() {
  void mountGithubStars();
  const canvas=document.querySelector('#sculpture');if(!canvas)return;
  const version=state.render,mountVersion=state.homeMount=(state.homeMount||0)+1;
  try{
    homeModules ||= Promise.all([import('./runtime/timeline.js'),import('./runtime/motion.js'),import('./scenes.js'),import('./home-story.js'),import('./particle-type.js')]).catch(error=>{homeModules=null;throw error;});
    const [{createTimeline},{mountSculpture},{makeScenes},{mountHomeStory},{mountParticleTitles}]=await homeModules;
    if(version!==state.render||mountVersion!==state.homeMount||!canvas.isConnected)return;
    const shouldPause=state.motionPaused || navigator.connection?.saveData===true;
    state.particleTitles=mountParticleTitles(document.querySelector('.home-story'));
    state.sculpture=mountSculpture(canvas,createTimeline(makeScenes(),{hold:8,morph:2}),{
      onState: status => {
        const paused=status==='paused';
        if(status==='paused'||status==='playing')state.motionPaused=paused;
        state.particleTitles?.pause(state.motionPaused);
        state.brandMotion?.pause(state.motionPaused);
        const button=document.querySelector('[data-action=pause]');
        if(button){
          button.innerHTML=icon(paused?'play':'pause');
          button.setAttribute('aria-label',paused?t('继续动画','Resume animation'):t('暂停动画','Pause animation'));
          button.setAttribute('aria-pressed',String(paused));
          button.hidden=status==='reduced'||status==='unsupported';
        }
        if(status==='unsupported')document.querySelector('.fallback').hidden=false;
      }
    });
    state.sculpture?.pause(shouldPause);
    state.homeStory=mountHomeStory(document.querySelector('.home-story'),{
      onPosition:position=>{state.sculpture?.seek(position);state.particleTitles?.seek(position);}
    });
  }catch{if(version===state.render&&mountVersion===state.homeMount){const fallback=document.querySelector('.fallback');if(fallback)fallback.hidden=false;}}
}

function sidebar(tab,admin=false) {
  const entries=admin?[
    ...(state.owner||state.staff?[['overview','chart',t('总览','Overview')]]:[]),
    ['messages','message',t('消息','Messages')],['matches','link',t('人才与岗位','People & roles')],['mandates','target',t('寻找委托','Search briefs')],
    ...(state.owner||state.staff?[['orders','receipt',t('订单与流水','Orders')]]:[]),
    ...(state.owner?[['promos','receipt',t('优惠码','Promotions')],['team','person',t('团队','Team')],['agents','bot',t('平台 AI','Platform AI')],['keys','key',t('通行密钥','Passkeys')]]:[])
  ]:[
    ['messages','message',t('消息','Messages')],
    ...(state.user?.role==='seeker'?[['mandate','target',t('寻找委托','My search')]]:[]),
    ...(state.user?.role==='employer'?[['jobs','briefcase',t('我的招聘','My roles')]]:[]),
    ...(state.user?.role!=='seeker'?[['promos','receipt',t('招聘优惠','Hiring promotions')]]:[]),
    ['profile',state.user?.role==='employer'?'person':'file',state.user?.role==='employer'?t('我的资料','My profile'):t('我的简历','My résumé')],['matches','link',t('我的匹配','Introductions')],['orders','receipt',t('订单','Orders')],['agents','bot',t('AI 连接','AI connections')],['webhooks','link',t('联动','Webhooks')]
  ];
  const item=([key,ico,label])=>`<a href="${admin?'/admin':'/app'}?tab=${key}" data-nav title="${esc(label)}" aria-label="${esc(label)}" class="${tab===key?'active':''}"${tab===key?' aria-current="page"':''}>${icon(ico)}<span>${label}</span></a>`;
  const primary=entries.slice(0,4),secondary=entries.slice(4);
  const cliConnection=admin&&!state.owner?'':`<a href="/oauth/connections" title="${t('命令行连接','Command-line connections')}" aria-label="${t('命令行连接','Command-line connections')}">${icon('key')}<span>${t('命令行连接','Command-line connections')}</span></a>`;
  const logout=`<button type="button" data-action="logout">${icon('logout')}<span>${t('退出登录','Sign out')}</span></button>`;
  return `<aside class="sidebar workspace-sidebar" data-dom-key="sidebar"><div class="workspace-sidebar-top"><div class="workspace-nav-label">${t('工作空间','WORKSPACE')}</div><button type="button" class="icon-button workspace-panel-toggle" data-workspace-panel="navigation" aria-controls="workspace-navigation" aria-expanded="true" aria-label="${t('收起导航栏','Collapse navigation')}" title="${t('收起导航栏','Collapse navigation')}" data-label-collapse="${t('收起导航栏','Collapse navigation')}" data-label-expand="${t('展开导航栏','Expand navigation')}">${icon('panel')}</button></div><nav id="workspace-navigation" class="sidebar-nav" aria-label="${t('工作台导航','Workspace navigation')}">${primary.map(item).join('')}<div class="workspace-secondary"><div class="workspace-nav-label">${t('管理与连接','MANAGE & CONNECT')}</div>${secondary.map(item).join('')}${cliConnection}</div><details class="workspace-more"><summary aria-label="${t('更多页面','More pages')}">${icon('board')}<span>${t('更多','More')}</span></summary><div class="workspace-more-menu">${secondary.map(item).join('')}${cliConnection}<a href="/contact" data-nav>${icon('info')}<span>${t('帮助与联系','Help & contact')}</span></a>${logout}</div></details></nav><div class="workspace-sidebar-foot"><a href="/contact" data-nav class="workspace-support" aria-label="${t('联系聘渡','Contact HireFerry')}" title="${t('联系聘渡','Contact HireFerry')}">${icon('message')}<span>${t('需要帮忙？','Need a hand?')}<small>${t('联系聘渡','Contact HireFerry')}</small></span>${icon('arrow')}</a><div class="sidebar-user">${admin?`<span class="avatar">${icon('shield')}</span>`:avatar(state.user)}<div class="sidebar-account"><div class="sidebar-name">${admin?esc(state.staff?.name||state.user?.name||t('聘渡管理团队','HireFerry team')):esc(state.user?.name||t('我的账户','My account'))}</div><div class="role-label">${admin?(state.owner?t('超级管理员','Super administrator'):state.staff?t('团队员工','Staff'):t('团队经理','Team manager')):roleText(state.user?.role)}</div></div><button class="icon-button" type="button" data-action="logout" aria-label="${t('退出登录','Sign out')}">${icon('logout')}</button></div><div class="workspace-policy-links"><a href="/privacy" data-nav>${t('隐私','Privacy')}</a><a href="/terms" data-nav>${t('条款','Terms')}</a></div></div></aside>`;
}
function shell(body,tab,admin=false) {const activation=!admin&&state.user?.role==='employer'&&!state.user.employer_paid&&tab!=='orders'&&tab!=='messages'?`<div class="workspace-activation"><span>${t('招聘方账户尚未开通','Employer account not active')}</span><a href="/app?tab=orders" data-nav class="text-button">${t('查看开通与付款','Activation & payments')}${icon('arrow')}</a></div>`:'';return `<main id="main" class="page-width app-layout workspace-layout" data-workspace-page="${esc(tab)}">${sidebar(tab,admin)}<section class="app-content" tabindex="-1">${activation}${body}</section></main>`;}
function paymentProviders(record) {
  const code=currency(record).currency,configured=state.config?.payment_currencies;
  return (state.config?.providers||[]).filter(provider=>(!configured||(Array.isArray(configured[provider])&&configured[provider].includes(code)))&&(provider!=='waffo'||!state.config?.waffo_regions||waffoRegions(record).length>0));
}
const paymentMethodText = method => ({card:t('银行卡','Card'),applepay:'Apple Pay',googlepay:'Google Pay',wechat:t('微信支付','WeChat Pay'),alipay:t('支付宝','Alipay')})[method]||method;
const regionText = region => ({global:t('全球','Global'),china:t('中国','China')})[region] || region;
function waffoRegions(record) {
  const code=currency(record).currency;
  return Object.entries(state.config?.waffo_regions||{}).filter(([,settings])=>settings.currencies.includes(code));
}
function paymentModeNotice(provider='waffo') {
  return state.config?.payment_modes?.[provider]==='test'?`<p class="payment-test-notice" role="status">${icon('info')}<span>${t('测试模式 · 不会真实扣款','Test mode · No real charges')}</span></p>`:'';
}
function gatewayInputs(record,{hideRegions=false}={}) {
  const providers=paymentProviders(record),code=currency(record).currency;
  const regions=waffoRegions(record);
  return providers.length?`<div class="gateway-choice">${providers.map((name,i)=>`<label class="gateway-option"><input type="radio" name="provider" value="${esc(name)}" ${i===0?'checked':''}><span>${esc(providerText(name))}<small>${name==='stripe'?t('安全在线结账','Secure online checkout'):t('一次性支付','One-time payment')}</small></span></label>`).join('')}</div>${providers.includes('waffo')?`${!hideRegions&&regions.length?`<div class="field"><label for="payment-region">${t('Waffo Pancake 区域','Waffo Pancake region')}</label><select class="input" id="payment-region" name="payment_region">${regions.map(([name])=>`<option value="${esc(name)}">${esc(regionText(name))}</option>`).join('')}</select></div>`:''}${paymentModeNotice()}`:''}`:notice(t(`当前没有支持 ${code} 的在线支付方式。请更换币种或联系支持邮箱。`,`No online payment method is available for ${code}. Choose another currency or contact support.`),'warning');
}

function acquisitionSource() {
  const params=new URLSearchParams(location.search), parts=[params.get('utm_source'),params.get('utm_medium'),params.get('utm_campaign'),params.get('utm_content'),params.get('ref')].filter(Boolean).map(value=>value.trim()).filter(Boolean);
  const current=parts.join(':').slice(0,120);
  try {
    if(current) sessionStorage.setItem('hireferry:acquisition_source',current);
    return current||sessionStorage.getItem('hireferry:acquisition_source')||'';
  } catch { return current; }
}
function referralCode() {
  const value = qs().get('ref') || '';
  const current = /^hf_[A-Za-z0-9_-]{1,61}$/u.test(value) ? value.toLowerCase() : '';
  try {
    if (current) sessionStorage.setItem(referralCodeKey, current);
    return current || sessionStorage.getItem(referralCodeKey) || '';
  } catch { return current; }
}
function login() {
  const source=acquisitionSource(),referral=referralCode(),intent=currentRoleIntent();
  const googleParams=new URLSearchParams();
  if(source)googleParams.set('source',source);
  if(referral)googleParams.set('referral_code',referral);
  const googleHref=state.config?.google_enabled?`/api/auth/google/start${googleParams.toString()?`?${googleParams}`:''}`:'#';
  const intentContext=intent==='employer'?`<p class="auth-context">${t('招聘方 · 免费发布一个岗位。登录后继续创建岗位。','Employer · post one role free. Sign in to continue to the editor.')}</p>`:intent==='seeker'?`<p class="auth-context">${t('求职者 · 创建独一无二的名字和公开主页。','Job seeker · create a unique name and public homepage.')}</p>`:'';
  return `<main id="main" class="page-width auth-wrap"><section class="auth-intro"><h1>${t('从认识你<br>开始匹配','Start with you<br>Find the fit')}</h1><p>${t('用邮箱登录。<br>开始求职或招聘。','Sign in with your email.<br>Find work or find people.')}</p></section><section class="auth-card"><h2>${t('欢迎来到聘渡','Welcome to HireFerry')}</h2><p class="subtitle">${t('登录或创建账户','Sign in or create an account')}</p>${intentContext}<a href="${googleHref}" class="button secondary full" data-google-sign-in ${!state.config?.google_enabled?'aria-disabled="true" data-action="google-unavailable"':''}><span class="google-mark">G</span>${t('使用 Google 继续','Continue with Google')}</a><button type="button" class="button secondary full" data-action="user-passkey-login">${icon('key')}${t('使用 Passkey 登录','Sign in with a passkey')}</button><p class="help">${t('使用此设备、同步的密码管理器或安全密钥中的 Passkey。','Use a passkey from this device, a synced password manager, or a security key.')}</p>${state.config?.humangrant_enabled?humanGrant().loginButton():''}<div class="separator">${t('或使用邮箱','or use your email')}</div><form data-form="email-start"><div class="field"><label for="login-email">${t('邮箱地址','Email address')}</label><input class="input" id="login-email" name="email" type="email" autocomplete="email" placeholder="you@example.com" required maxlength="254"></div>${referral?`<p class="notice">${icon('person')}<span>${t('你正通过成员邀请链接注册。','You are joining through a member invitation link.')}</span></p>`:''}<button type="submit" class="button full" ${state.config?.email_enabled===false?'disabled':''}>${t('发送登录验证码','Send a sign-in code')}${icon('arrow')}</button></form>${state.config?.email_enabled===false?`<p class="help">${t('邮箱登录暂未开放。','Email sign-in is not available yet.')}</p>`:''}<div class="auth-legal">${t('继续即表示你已阅读并同意','By continuing, you agree to our')} <a href="/terms" data-nav>${t('服务条款','Terms')}</a> ${t('及','and')} <a href="/privacy" data-nav>${t('隐私说明','Privacy notice')}</a>。</div></section></main>`;
}
function emailCode(email,devCode) {const card=document.querySelector('.auth-card');card.innerHTML=`<h2>${t('查收你的邮箱','Check your inbox')}</h2><p class="subtitle">${t('验证码已发送至','We sent a sign-in code to')}<br><span class="auth-email">${esc(email)}</span></p><form data-form="email-verify"><input type="hidden" name="email" value="${esc(email)}"><div class="field"><label for="email-code">${t('登录验证码','Sign-in code')}</label><input class="input code-input" id="email-code" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]+" maxlength="12" placeholder="· · · · · ·" required><p class="help">${t('验证码短期有效，且只能使用一次。','This code expires soon and can only be used once.')}</p></div>${devCode?notice(`${t('仅开发测试环境','Development environment only')}: <strong>${esc(devCode)}</strong>`,'warning'):''}<button class="button full" type="submit">${t('确认并继续','Verify and continue')}${icon('arrow')}</button></form><div class="auth-legal"><button class="text-button" data-action="restart-login">${t('换一个邮箱','Use a different email')}</button></div>`;document.querySelector('#email-code').focus();}
function chooseRole() {
  const intent=currentRoleIntent();
  if(intent==='employer') {openDialog(`<div class="eyebrow">FOR RECRUITERS</div><h2 id="dialog-title">${t('继续发布免费岗位','Continue to your free role')}</h2><p class="dialog-intro">${t('你刚才选择了招聘方。完成一次免费开通后，直接进入岗位编辑器。','You chose the employer path. Complete the free activation once, then go straight to the role editor.')}</p><button class="role-card role-card-primary" data-action="role-employer">${icon('briefcase')}<strong>${t('开通招聘方账户','Activate employer account')}</strong><span>${t('早期阶段免费，不公开邮箱、订单或私聊。','Free early access. Email, orders and private chats stay private.')}</span><b>${Number(pricing().onboarding_minor??pricing().onboarding_cents)===0?t('免费开通','Free early access'):discountLabel(money(pricing().onboarding_original_minor??pricing().onboarding_cents),money(pricing().onboarding_minor??pricing().onboarding_cents))+' '+t('一次性开通','one-time activation')} →</b></button>`,{close:false});return;}
  if(intent==='seeker') {openDialog(`<div class="eyebrow">FOR JOB SEEKERS</div><h2 id="dialog-title">${t('继续创建你的公开主页','Continue to your public homepage')}</h2><p class="dialog-intro">${t('你刚才选择了求职者路径。注册和创建主页免费，是否公开由你自己决定。','You chose the job seeker path. Registration and profile creation are free; publishing is always your choice.')}</p><button class="role-card role-card-primary" data-action="role-seeker">${icon('person')}<strong>${t('创建求职者资料','Create job seeker profile')}</strong><span>${t('先保存资料，再决定是否打开公开主页。','Save your profile first, then choose whether to publish it.')}</span><b>${t('永久免费','Always free')} →</b></button>`,{close:false});return;}
  openDialog(`<div class="eyebrow">YOUR NEXT CHAPTER</div><h2 id="dialog-title">${t('这次，你想寻找什么？','What brings you here?')}</h2><p class="dialog-intro">${t('选择你的身份，我们会为你准备合适的开始。','Choose your role. We’ll prepare the right start for you.')}</p><div class="role-options"><button class="role-card" data-action="role-seeker">${icon('person')}<strong>${t('我在找工作','I’m finding work')}</strong><span>${t('告诉平台期待，我们帮你持续找。','Share your expectations. We help with the search.')}</span><b>${t('永久免费','Always free')} →</b></button><button class="role-card" data-action="role-employer">${icon('briefcase')}<strong>${t('我想招人','I’m hiring')}</strong><span>${t('发布岗位，我们帮你认真寻找。','Share your role. We help find the right person.')}</span><b>${Number(pricing().onboarding_minor??pricing().onboarding_cents)===0?t('免费开通','Free early access'):discountLabel(money(pricing().onboarding_original_minor??pricing().onboarding_cents),money(pricing().onboarding_minor??pricing().onboarding_cents))+' '+t('一次性开通','one-time activation')} →</b></button></div><p class="promo-entry"><a class="text-button" href="/app?tab=promos" data-nav>${t('使用优惠码或申请免费激活','Use a code or apply for free activation')}${icon('arrow')}</a></p><p class="help" style="margin-top:20px">${t('选择后，身份将与这个账户关联。招聘方完成开通后即可发布岗位，费用会在确认前列明。','Your choice is linked to this account. Employers can post after activation; any fee is shown before confirmation.')}</p>`,{close:false});
}
function activationOptions(region) {
  const options=pricing().activation_currency_options||[{...currency(),onboarding_minor:pricing().onboarding_cents}];
  const settings=state.config?.waffo_regions?.[region];
  return settings?settings.currencies.map(code=>options.find(option=>option.currency===code)).filter(Boolean):options;
}
function activationCurrencyOptions(options,selected) {
  return options.map(option=>`<option value="${esc(option.currency)}" ${option.currency===selected?'selected':''}>${esc(option.currency)} · ${discountText(money(option.onboarding_original_minor??option.onboarding_minor,option),money(option.onboarding_minor,option))}</option>`).join('');
}
function employerActivation(retry=false) {
  const regions=Object.keys(state.config?.waffo_regions||{}).filter(region=>activationOptions(region).length);
  const region=regions.find(region=>activationOptions(region).some(option=>option.currency===currency().currency))||regions[0];
  const options=activationOptions(region),selected=options.find(option=>option.currency===currency().currency)||options[0];
  const free=Number(selected?.onboarding_minor)===0;
  openDialog(`<div class="eyebrow">FOR EMPLOYERS</div><h2 id="dialog-title">${retry?t('重新发起开通','Restart activation'):t('开通招聘方账户','Activate your employer account')}</h2><p class="dialog-intro">${free?t('早期阶段免费开通，之后可以发布多个岗位。','Free early access includes multiple roles.'):t('一次性开通招聘方账户，之后可以发布多个岗位。每个岗位的发布费单独计算。','Activate your employer account once. Then publish multiple roles, each with its own posting fee.')}</p><p class="promo-entry"><a class="text-button" href="/app?tab=promos" data-nav>${t('使用优惠码或申请免费激活','Use a code or apply for free activation')}${icon('arrow')}</a></p><form data-form="activate-employer" class="activation-checkout">${!free&&regions.length?`<fieldset class="payment-region-choice"><legend>${t('Waffo Pancake 区域','Waffo Pancake region')}</legend><div class="payment-region-options">${regions.map(name=>`<label><input type="radio" name="payment_region" value="${esc(name)}" ${name===region?'checked':''}><span>${icon(name==='global'?'globe':'wallet')}<strong>${esc(regionText(name))}</strong><small>${esc(state.config.waffo_regions[name].payment_methods.map(paymentMethodText).join(' · '))}</small></span></label>`).join('')}</div></fieldset>`:''}<div class="field"><label for="activation-currency">${t('付款币种','Payment currency')}</label><select class="input" name="currency" id="activation-currency">${activationCurrencyOptions(options,selected?.currency)}</select></div><div class="cost-summary activation-total"><div class="cost-row total"><span>${t('招聘方开通 · 一次性','Employer activation · one-time')}</span><strong data-activation-total>${money(selected?.onboarding_minor,selected)}</strong></div></div><div data-activation-gateway>${free?'':gatewayInputs(selected,{hideRegions:true})}</div><p class="cost-rule" data-activation-rule>${free?t('开通免费，确认后即可使用。','Activation is free and takes effect on confirmation.'):t('付款确认后自动开通。不自动续费，无月费。','Activates after payment confirmation. No renewals or monthly fees.')}</p><div class="form-actions"><button type="button" class="button secondary" data-action="close-dialog">${t('稍后再说','Maybe later')}</button><button type="submit" class="button" ${free||paymentProviders(selected).length?'':'disabled'}><span data-activation-submit>${free?t('确认开通','Activate account'):t('前往支付','Continue to payment')}</span>${icon('arrow')}</button></div></form>`);
}
function refreshActivationPayment(form) {
  const select=form.elements.currency,options=activationOptions(form.elements.payment_region?.value);
  const selected=options.find(option=>option.currency===select.value)||options[0];
  select.innerHTML=activationCurrencyOptions(options,selected?.currency);
  const free=Number(selected?.onboarding_minor)===0;
  form.querySelector('[data-activation-total]').textContent=money(selected?.onboarding_minor,selected);
  form.querySelector('[data-activation-gateway]').innerHTML=free?'':gatewayInputs(selected,{hideRegions:true});
  form.querySelector('[data-activation-rule]').textContent=free?t('开通免费，确认后即可使用。','Activation is free and takes effect on confirmation.'):t('付款确认后自动开通。不自动续费，无月费。','Activates after payment confirmation. No renewals or monthly fees.');
  form.querySelector('[type="submit"]').disabled=!selected||(!free&&!paymentProviders(selected).length);
  form.querySelector('[data-activation-submit]').textContent=free?t('确认开通','Activate account'):t('前往支付','Continue to payment');
}

async function publicProfilePage(handle) {
  const data = await api(`/api/public-profiles/${encodeURIComponent(handle)}`);
  const p = data.profile;
  const links = Object.entries(p.ai_profile_links || {}).map(([name,url]) => `<a class="button secondary small" href="${esc(url)}" rel="me noopener" target="_blank">${esc(name)}</a>`).join('');
  const claimHref = `/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile&ref=profile_${encodeURIComponent(p.handle)}`;
  return `<main id="main" class="page-width public-profile"><section class="panel profile-hero">${avatar(p,'large')}<p class="eyebrow">@${esc(p.handle)}</p><h1>${esc(p.name || p.handle)}</h1>${p.public_email ? `<a class="contact-email" href="mailto:${encodeURIComponent(p.public_email)}">${esc(p.public_email)}</a>` : ''}${links ? `<div class="profile-links">${links}</div>` : ''}<div class="profile-claim"><p>${t('也想拥有自己的独立 AI 身份？注册一个独一无二的名字，创建你的个人主页。','Want your own independent AI identity? Register a unique name and create your personal homepage.')}</p><a class="button" href="${claimHref}" data-nav>${t('注册我的名字','Register my name')}${icon('arrow')}</a></div></section><section class="panel markdown"><h2>${t('个人介绍','About')}</h2>${p.resume_html || `<p class="help">${t('这个人还没有填写公开介绍。','This person has not added a public introduction yet.')}</p>`}</section></main>`;
}
function employerLandingPage() {
  const contact=contactMailto('HireFerry 招聘合作');
  return `<main id="main" class="page-width employer-landing"><section class="employer-hero"><div class="employer-hero-copy"><p class="eyebrow">FOR RECRUITERS · FREE EARLY ACCESS</p><h1>${t('把岗位交给 AI，少收一堆不匹配简历。','Let AI sort the signal from the noise.')}</h1><p>${t('早期阶段招聘方免费发布岗位。写清业务、赛道和真正要解决的问题，求职者和 AI 都能直接理解你的需求。先发布一个岗位试试，不需要月费。','Early access job publishing is free. Describe the business, niche and real problem to solve so candidates and AI can understand the role. Start with one role; there is no monthly fee.')}</p><div class="employer-hero-actions"><a href="/login?role=employer&return_to=%2Fapp%2Fjobs%2Fnew" data-nav class="button">${t('免费发布一个岗位','Post one role free')}${icon('arrow')}</a><a href="/discover?utm_source=employer_landing&utm_campaign=employer_cta&utm_content=hero" data-nav class="button secondary">${t('先看公开目录','View public directory')}</a></div><ul class="employer-proof-chips" aria-label="${t('招聘方早期权益','Recruiter early-access benefits')}"><li>${t('免费早期发布','Free early access')}</li><li>${t('无需月费','No monthly fee')}</li><li>${t('仅公开主动发布内容','Only your explicit public content')}</li></ul></div><aside class="employer-hero-card"><p class="pricing-kicker">${t('招聘方现在得到什么','WHAT YOU GET')}</p><h2>${t('免费、公开、可被 AI 读取','Free, public and AI-readable')}</h2><ul><li>${t('岗位发布和早期账户开通免费','Free early-access activation and role publishing')}</li><li>${t('只公开你明确发布的岗位内容','Only the role content you explicitly publish is public')}</li><li>${t('公开 MCP 让 AI 直接检索岗位','Public MCP lets AI search the role directly')}</li></ul><a class="text-button" href="${esc(contact)}">${t('需要协助导入岗位？联系团队','Need help importing a role? Contact us')} ${icon('arrow')}</a></aside></section><section class="employer-benefit-grid"><article><span class="employer-step">01</span><h2>${t('写清真实问题','Describe the real problem')}</h2><p>${t('不要只填职位名称。补充业务方向、目标用户、关键职责和经验边界，减少无效申请。','Go beyond a title. Add the business, users, responsibilities and experience boundaries to reduce noise.')}</p></article><article><span class="employer-step">02</span><h2>${t('公开一个岗位','Publish one role')}</h2><p>${t('岗位可以先公开一个版本，随时调整招聘人数；早期阶段发布费用为 0。','Start with one public version and adjust the hiring target later. Early-access publication is free.')}</p></article><article><span class="employer-step">03</span><h2>${t('让 AI 帮你匹配','Let AI find the fit')}</h2><p>${t('求职者可以直接浏览，AI 也可以通过免费 MCP 读取明确公开的岗位。','Candidates can browse directly, and AI can read explicitly public roles through the free MCP.')}</p></article></section><section class="employer-proof"><div><p class="eyebrow">${t('清晰的边界','CLEAR BOUNDARIES')}</p><h2>${t('公开岗位，不公开私聊。','Public roles, private conversations.')}</h2></div><p>${t('公开目录只包含你主动发布且仍在招聘中的岗位。账户邮箱、订单、匹配历史和私聊不会因为公开岗位而开放。','The public directory includes only roles you deliberately publish while hiring. Account email, orders, match history and private conversations stay private.')}</p></section></main>`;
}
function publicSearchSummary(value) {return String(value||'').replace(/<[^>]+>/g,' ').replace(/\s+/g,' ').trim().slice(0,240);}
function publicDirectoryQuery(tab) {const p=new URLSearchParams();p.set('tab',tab);if(qs().get('q'))p.set('q',qs().get('q'));if(qs().get('page'))p.set('page',qs().get('page'));return p;}
function publicMcpEndpoint() {return `${location.origin}/mcp/public`;}
function publicMcpExample() {return `curl ${publicMcpEndpoint()} \\
  -H 'Content-Type: application/json' \\
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"hireferry_public_search_jobs","arguments":{"q":"Rust backend","page":1}}}'`;}
function publicMcpCard() {
  const endpoint=publicMcpEndpoint(),example=publicMcpExample();
  const docs=`${String(state.config?.source_url||'https://github.com/TokenNotIncluded/hireferry').replace(/\/$/,'')}/blob/main/docs/mcp-public.md`;
  const tools=['hireferry_public_get_site_config','hireferry_public_search_jobs','hireferry_public_get_job','hireferry_public_search_people','hireferry_public_get_person'];
  return `<aside class="public-directory-note public-mcp-card" aria-labelledby="public-mcp-title"><span>${icon('bot')}</span><div><strong id="public-mcp-title">${t('给 AI 的入口','For AI agents')}</strong><p>${t('无需登录 · 只读 · 只返回本人明确公开的岗位和简历。这个地址是 POST-only 接口，不能直接当网页打开。','No login · read-only · only explicitly public roles and résumés. This is a POST-only endpoint, not a web page.')}</p><div class="public-mcp-endpoint"><code>${esc(endpoint)}</code><button type="button" class="button secondary small" data-action="copy-public-mcp" data-copy="${esc(endpoint)}">${icon('copy')}${t('复制地址','Copy endpoint')}</button></div><div class="public-mcp-actions"><button type="button" class="button secondary small" data-action="copy-public-mcp-example" data-copy="${esc(example)}">${icon('copy')}${t('复制请求示例','Copy request')}</button><a class="text-button" href="${esc(docs)}" target="_blank" rel="noopener noreferrer">${t('查看工具说明','View tool guide')} ${icon('arrow')}</a></div><details class="public-mcp-tools"><summary>${t('查看 5 个公开工具','View 5 public tools')}</summary><ul>${tools.map(name=>`<li><code>${name}</code></li>`).join('')}</ul></details><details class="public-mcp-example"><summary>${t('查看 curl 示例','View curl example')}</summary><pre><code>${esc(example)}</code></pre></details></div></aside>`;
}
function publicDirectoryCta(placement='directory') {
  return `<section class="public-directory-cta" aria-labelledby="public-directory-cta-title"><div><p class="eyebrow">${t('下一步 · 早期阶段免费','NEXT STEP · FREE EARLY ACCESS')}</p><h2 id="public-directory-cta-title">${t('找到方向后，把自己也放进公开目录。','Once you find your direction, put yourself on the public directory.')}</h2><p>${t('求职者可以注册独一无二的名字并公开个人主页；招聘方可以免费发布岗位。AI 会读取你明确公开的内容。','Job seekers can claim a unique name and publish a homepage. Employers can post roles for free. AI reads only what you explicitly make public.')}</p></div><div class="public-directory-cta-actions"><a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button">${t('创建我的公开主页','Create my public profile')}${icon('arrow')}</a><a href="/for-employers?utm_source=directory&utm_campaign=employer_cta&utm_content=${encodeURIComponent(placement)}" data-nav class="button secondary">${t('发布免费岗位','Post a free role')}${icon('arrow')}</a></div></section>`;
}
function publicDirectorySkeleton(path=location.pathname) {
  const detail=path!=='/discover';
  return `<main id="main" class="page-width public-directory public-directory-loading" aria-busy="true"><div class="sr-only" role="status" aria-live="polite">${t('正在加载公开目录','Loading public directory')}</div><header class="public-directory-hero"><div><p class="eyebrow">OPEN TALENT GRAPH · MCP READY</p><span class="skeleton-line skeleton-title" aria-hidden="true"></span><span class="skeleton-line" aria-hidden="true"></span><span class="skeleton-line short" aria-hidden="true"></span></div></header>${detail?'<section class="public-directory-panel public-directory-skeleton-detail"><span class="skeleton-line skeleton-title" aria-hidden="true"></span><span class="skeleton-line" aria-hidden="true"></span><span class="skeleton-line" aria-hidden="true"></span><span class="skeleton-block" aria-hidden="true"></span></section>':'<nav class="public-directory-tabs" aria-label="'+t('公开目录','Public directory')+'"><span class="skeleton-pill" aria-hidden="true"></span><span class="skeleton-pill" aria-hidden="true"></span></nav><section class="public-directory-panel"><span class="skeleton-input" aria-hidden="true"></span><div class="public-directory-skeleton-list"><span class="skeleton-block" aria-hidden="true"></span><span class="skeleton-block" aria-hidden="true"></span></div></section>'}</main>`;
}
async function publicDirectoryPage() {
  const tab=qs().get('tab')==='people'?'people':'jobs',query=publicDirectoryQuery(tab),q=query.get('q')||'';
  const result=await api(`/api/public/${tab}${q||query.get('page')?`?${new URLSearchParams([...query].filter(([key])=>key!=='tab'))}`:''}`);
  const items=result.items||[];
  const tabs=`<nav class="public-directory-tabs" aria-label="${t('公开目录','Public directory')}"><a class="${tab==='jobs'?'active':''}" href="/discover?tab=jobs" data-nav ${tab==='jobs'?'aria-current="page"':''}>${icon('briefcase')}${t('公开岗位','Public roles')}</a><a class="${tab==='people'?'active':''}" href="/discover?tab=people" data-nav ${tab==='people'?'aria-current="page"':''}>${icon('person')}${t('公开简历','Public résumés')}</a></nav>`;
  const cards=tab==='jobs'?items.map(job=>`<article class="public-directory-card" itemscope itemtype="https://schema.org/JobPosting"><div class="public-directory-card-main"><p class="eyebrow">${t('公开招聘','PUBLIC ROLE')} · <span itemprop="datePosted">${date(job.created_at)}</span></p><h2 itemprop="title">${esc(job.title)}</h2><p class="public-directory-meta"><span itemprop="hiringOrganization">${esc(job.employer?.name||t('公开招聘方','Public employer'))}</span><span>${money(job.salary_cents,job)} / ${t('月，底薪','month, base')}</span><span>${t('招聘','Hiring')} ${Number(job.headcount||0).toLocaleString(state.lang==='en'?'en-US':'zh-CN')} ${t('人','people')}</span></p><p class="public-directory-summary" itemprop="description">${esc(publicSearchSummary(job.jd_html||job.jd_md))}</p></div><a class="button secondary small" href="/discover/jobs/${encodeURIComponent(job.id)}" data-nav>${t('查看岗位','View role')}${icon('arrow')}</a></article>`).join(''):items.map(person=>`<article class="public-directory-card" itemscope itemtype="https://schema.org/Person"><div class="public-directory-card-main"><p class="eyebrow">${t('公开简历','PUBLIC RÉSUMÉ')} · @${esc(person.handle)}</p><h2 itemprop="name">${esc(person.name||person.handle)}</h2><p class="public-directory-meta"><span>${t('公开 AI 链接','Public AI links')} ${Object.keys(person.ai_profile_links||{}).length}</span><span>${t('更新于','Updated')} ${date(person.updated_at||person.created_at)}</span></p><p class="public-directory-summary" itemprop="description">${esc(publicSearchSummary(person.resume_html||person.resume_md))}</p></div><a class="button secondary small" href="/discover/people/${encodeURIComponent(person.handle)}" data-nav>${t('查看简历','View résumé')}${icon('arrow')}</a></article>`).join('');
  const searchLabel=tab==='jobs'?t('搜索公开岗位','Search public roles'):t('搜索公开简历','Search public résumés');
  const searchForm=`<form data-form="public-directory-search" class="public-directory-search"><label class="sr-only" for="public-directory-q">${searchLabel}</label><input id="public-directory-q" class="input" name="q" type="search" value="${esc(q)}" maxlength="100" placeholder="${esc(tab==='jobs'?t('搜索岗位、技能或雇主','Search roles, skills or employers'):t('搜索姓名、技能或经历','Search names, skills or experience'))}" autocomplete="off"><button class="button" type="submit">${icon('search')}${t('搜索','Search')}</button></form>`;
  const count=t(`共 ${Number(result.total||0).toLocaleString('zh-CN')} 条公开记录`,` ${Number(result.total||0).toLocaleString('en-US')} public records`);
  const emptyAction=q?`<a href="/discover?tab=${tab}" data-nav class="button secondary small">${t('清除搜索','Clear search')}${icon('arrow')}</a>`:tab==='people'?`<a href="/login?role=seeker&return_to=%2Fapp%3Ftab%3Dprofile" data-nav class="button secondary small">${t('创建我的公开主页','Create my public homepage')}${icon('arrow')}</a>`:`<a href="/for-employers?utm_source=directory&utm_campaign=employer_cta&utm_content=empty_state" data-nav class="button secondary small">${t('发布免费岗位','Post a free role')}${icon('arrow')}</a>`;
  const emptyIcon=q?'search':tab==='people'?'person':'briefcase';
  const emptyTitle=q
    ? t('没有找到匹配的公开记录','No public records match your search.')
    : tab==='jobs'
      ? t('暂时没有公开岗位','No public roles yet')
      : t('暂时没有公开简历','No public résumés yet');
  const emptyDescription=q
    ? t('试试更短的关键词，或清除搜索条件。','Try a shorter keyword or clear the search.')
    : tab==='jobs'
      ? t('招聘方发布并保持招聘中的岗位会出现在这里。','Published roles that are still hiring appear here.')
      : t('只有求职者主动打开公开主页后，个人资料才会出现在这里。未公开的简历不会被目录或公共 MCP 读取。','Only seekers who deliberately publish a homepage appear here. Private résumés are not read by the directory or public MCP.');
  const listContent=cards||empty(emptyTitle,emptyDescription,emptyAction,emptyIcon);
  return `<main id="main" class="page-width public-directory"><header class="public-directory-hero"><div><p class="eyebrow">OPEN TALENT GRAPH · MCP READY</p><h1>${t('公开机会，直接被发现。','Public opportunities, ready to be found.')}</h1><p>${t('只展示明确公开的岗位和简历。AI 可以直接读取，人与人也可以直接浏览。','Only explicitly public roles and résumés appear here. AI can read the same structured directory that people browse.')}</p></div><div class="public-directory-actions"><span class="pill">${t('免费公开 MCP','Free public MCP')}</span><code>/mcp/public</code></div></header>${tabs}<section class="public-directory-panel"><div class="public-directory-toolbar">${searchForm}<span class="results-count" role="status" aria-live="polite">${count}</span></div><div class="public-directory-list">${listContent}</div>${pagination(result.total||0,result.limit)}</section>${publicDirectoryCta('directory')} ${publicMcpCard()}</main>`;
}
async function publicJobPage(id) {const job=await api(`/api/public/jobs/${encodeURIComponent(id)}`);syncPageMetadata({doc:document,path:location.pathname,lang:state.lang,config:state.config,origin:location.origin,publicRecord:job});const link=`${location.origin}/discover/jobs/${encodeURIComponent(job.id)}`;return `<main id="main" class="page-width public-directory-detail"><a class="back-link" href="/discover?tab=jobs" data-nav>${icon('back')}${t('返回公开岗位','Back to public roles')}</a><div class="public-directory-detail-layout"><article class="public-directory-detail-card" itemscope itemtype="https://schema.org/JobPosting"><p class="eyebrow">${t('公开岗位 · 正在招聘','PUBLIC ROLE · HIRING')}</p><h1 itemprop="title">${esc(job.title)}</h1><p class="public-directory-meta"><span>${esc(job.employer?.name||t('公开招聘方','Public employer'))}</span><span>${money(job.salary_cents,job)} / ${t('月，底薪','month, base')}</span><span>${t('招聘','Hiring')} ${Number(job.headcount||0).toLocaleString(state.lang==='en'?'en-US':'zh-CN')} ${t('人','people')}</span></p><button type="button" class="button secondary small public-copy-link" data-action="copy-public-link" data-copy="${esc(link)}">${icon('copy')}${t('复制岗位链接','Copy role link')}</button><article class="markdown" itemprop="description">${job.jd_html||`<p class="help">${t('招聘方尚未补充岗位详情。','The recruiter has not added more role details yet.')}</p>`}</article></article><aside class="public-detail-aside">${publicDirectoryCta('detail_sidebar')}${publicMcpCard()}</aside></div></main>`;}
async function publicPersonPage(handle) {const person=await api(`/api/public/people/${encodeURIComponent(handle)}`);syncPageMetadata({doc:document,path:location.pathname,lang:state.lang,config:state.config,origin:location.origin,publicRecord:person});const link=`${location.origin}/discover/people/${encodeURIComponent(person.handle)}`;const links=Object.entries(person.ai_profile_links||{}).map(([name,url])=>`<a class="button secondary small" href="${esc(url)}" target="_blank" rel="me noopener noreferrer" aria-label="${esc(t(`在新标签页打开 ${name} 主页`,`Open ${name} profile in a new tab`))}">${esc(name)}</a>`).join('');return `<main id="main" class="page-width public-directory-detail"><a class="back-link" href="/discover?tab=people" data-nav>${icon('back')}${t('返回公开简历','Back to public résumés')}</a><div class="public-directory-detail-layout"><article class="public-directory-detail-card" itemscope itemtype="https://schema.org/Person">${avatar(person,'large')}<p class="eyebrow">${t('公开简历','PUBLIC RÉSUMÉ')} · @${esc(person.handle)}</p><h1 itemprop="name">${esc(person.name||person.handle)}</h1>${person.public_email?`<p class="contact-email"><strong>${t('公开联系邮箱','Public contact email')}</strong><a href="mailto:${encodeURIComponent(person.public_email)}">${esc(person.public_email)}</a></p>`:''}${links?`<div class="profile-links">${links}</div>`:''}<button type="button" class="button secondary small public-copy-link" data-action="copy-public-link" data-copy="${esc(link)}">${icon('copy')}${t('复制公开主页','Copy public homepage')}</button><article class="markdown" itemprop="description">${person.resume_html||''}</article></article><aside class="public-detail-aside">${publicDirectoryCta('detail_sidebar')}${publicMcpCard()}</aside></div></main>`;}
async function profilePage() {
  const [u,keyData]=await Promise.all([api('/api/profile'),api('/api/auth/passkeys')]),mail=u.email_notifications;
  const links=u.ai_profile_links||{};
  const passkeys=keyData.passkeys||[];
  const publicUrl=u.handle?`${location.origin}/u/${u.handle}`:'';
  const profileSetup=isGeneratedHandle(u.handle)?`<section class="panel profile-setup-callout" aria-labelledby="profile-setup-title"><p class="eyebrow">NEXT 3 STEPS · FREE</p><h2 id="profile-setup-title">${t('先把名字占下来，再决定公开。','Claim your name first, then decide whether to publish.')}</h2><p>${t('三步完成：注册唯一名字，写清你做过的项目，再打开公开主页。默认不会公开邮箱、私聊或匹配记录。','Three steps: claim a unique name, describe your projects, then choose whether to publish. Email, private chats and match history stay private by default.')}</p><ol><li>${t('注册唯一名字','Claim a unique name')}</li><li>${t('补充经历和 AI 主页链接','Add experience and AI profile links')}</li><li>${t('自己决定是否公开','Choose whether to publish')}</li></ol></section>`:'';
  const handleHelp=isGeneratedHandle(u.handle)?t('当前是系统临时名。改成你想抢注的名字并保存，之后这个名字就是你的独立 AI 身份。','This is a temporary system name. Replace it with the name you want to claim, then save it as your independent AI identity.'):t('名字只能使用小写字母、数字、短横线或下划线，且必须独一无二。','Use lowercase letters, numbers, hyphens or underscores. The name must be unique.');
  const profileExtras=`<section class="panel profile-public-settings"><h2>${t('个人主页','Personal homepage')}</h2><p class="help">${t('注册一个独一无二的用户名，决定是否公开你的个人主页。','Register a unique username and choose whether to publish your personal homepage.')}</p><div class="grid-2"><label class="field">${t('用户名','Username')}<input class="input" name="handle" value="${esc(u.handle||'')}" minlength="3" maxlength="32" pattern="[a-z0-9_-]+" autocomplete="username" required><span class="help">${handleHelp}</span></label><label class="check-label"><input type="checkbox" name="public_profile" ${u.public_profile?'checked':''}> ${t('公开个人主页','Publish my homepage')}</label></div>${publicUrl?`<div class="profile-share"><p class="help">${t('主页地址','Homepage')}: <a href="${esc(publicUrl)}" target="_blank" rel="noopener">${esc(publicUrl)}</a></p><button type="button" class="button secondary small" data-action="copy-profile-link" data-copy="${esc(publicUrl)}">${icon('copy')}${t('复制主页链接','Copy homepage link')}</button></div>`:''}<div class="grid-2">${['chatgpt','grok','claude','gemini','perplexity'].map(name=>`<label class="field">${esc(name)} profile URL<input class="input" name="ai_${name}" type="url" value="${esc(links[name]||'')}" placeholder="https://..."></label>`).join('')}</div></section>`;
  const referral=u.referral||{};
  const referralPanel=referral.url?`<section class="panel referral-panel"><div class="panel-header"><div><h2>${t('邀请同行','Invite people you trust')}</h2><p>${t('把这个链接分享给正在找工作或准备招人的朋友。注册免费，平台会用 AI 协助匹配。','Share this link with people looking for work or hiring. Sign-up is free, and HireFerry uses AI to help with matching.')}</p></div><span class="pill">${t(`已邀请 ${Number(referral.invited_count||0)} 人`,`Invited ${Number(referral.invited_count||0)}`)}</span></div><div class="profile-share"><code>${esc(referral.url)}</code><div class="profile-share-actions"><button type="button" class="button small" data-action="share-referral-link" data-copy="${esc(referral.url)}">${icon('arrow')}${t('直接分享','Share')}</button><button type="button" class="button secondary small" data-action="copy-referral-link" data-copy="${esc(referral.url)}">${icon('copy')}${t('复制链接','Copy link')}</button></div></div><p class="help">${t('邀请只记录新账户的来源，不会自动发送消息，也不会公开被邀请人的邮箱。','Invites only record the source of a new account. No messages are sent automatically, and invitees’ emails stay private.')}</p></section>`:'';
  const emailSettings=`<section class="panel"><h2>${t('邮件通知','Email notifications')}</h2><p class="help">${mail.enabled?t('发送至登录邮箱，可随时关闭。','Sent to your sign-in email. You can turn them off anytime.'):t('平台尚未启用邮件通知。','Email notifications are not enabled yet.')}</p><form data-form="email-preferences">${[['messages',t('新消息','New messages')],['matches',t('匹配动态','Introduction updates')],['orders',t('订单动态','Order updates')],['platform',t('平台服务通知','Platform service notices')]].map(([key,label])=>`<label class="check-label"><input type="checkbox" name="${key}" ${mail.preferences[key]?'checked':''}> ${label}</label>`).join('')}<div class="form-actions"><button type="submit" class="button secondary">${t('保存通知设置','Save notification settings')}</button></div></form></section>`;
  const passkeyPanel=`<section class="panel profile-passkeys"><div class="panel-header"><div><h2>${t('登录 Passkey','Sign-in passkeys')}</h2><p>${t('用设备指纹、面容、密码管理器或安全密钥快速登录。私密部分只保存在你的设备中。','Sign in with a fingerprint, face, password manager, or security key. The private part stays on your device.')}</p></div><span class="pill">WebAuthn</span></div>${passkeys.length?`<div class="list">${passkeys.map(key=>`<div class="list-row"><div><h3>${esc(key.name)}</h3><div class="list-meta"><span>${t('绑定于','Added')} ${date(key.created_at)}</span><span>${t('最近使用','Last used')} ${date(key.last_used_at)}</span></div></div><button type="button" class="text-button danger-text" data-action="delete-user-passkey" data-id="${esc(key.id)}">${t('删除','Remove')}</button></div>`).join('')}</div>`:empty(t('还没有绑定 Passkey。','No passkeys yet.'),t('绑定后，登录页可以直接使用设备验证。','After you add one, the login page can use device verification.'),'','key')}<hr><form data-form="user-passkey-register"><div class="grid-2"><div class="field"><label for="user-passkey-name">${t('Passkey 名称','Passkey name')}</label><input class="input" id="user-passkey-name" name="name" maxlength="80" placeholder="${esc(t('例如：我的手机','For example: My phone'))}" required></div><div class="field" style="align-self:end"><button class="button" type="submit">${icon('plus')}${t('绑定 Passkey','Add a passkey')}</button></div></div></form><p class="help">${t('绑定和删除需要刚刚完成登录或重新登录。删除后仍可用邮箱或 Google 登录。','Adding or removing a passkey requires a recent sign-in. Email or Google sign-in remains available after removal.')}</p></section>`;
  return shell(`${profileSetup}${profileExtras}${referralPanel}${heading(u.role==='seeker'?t('我的简历','My résumé'):t('团队资料','Team profile'),t('资料仅平台与已匹配的对方可见，不会公开展示。','Your profile is visible only to HireFerry and an introduced counterpart.'),'',u.role==='seeker'?'YOUR RÉSUMÉ':'YOUR PROFILE')}<form data-form="profile"><details class="section-disclosure profile-section" data-disclosure="profile-basics" open><summary>${t('基本资料与联系方式','Profile & contact details')}</summary><div class="disclosure-content"><div class="avatar-editor">${avatar(u,'large')}<div><div class="field-label">${t('你的头像','Your avatar')}</div><div class="avatar-actions"><label class="upload-label">${icon('upload')}${t('上传图片','Upload image')}<input type="file" id="avatar-upload" accept="image/png,image/jpeg,image/webp" hidden></label><button type="button" class="text-button" data-action="svg-avatar">${t('使用 SVG 文本','Use SVG text')}</button></div><p class="help">${t('PNG、JPG 或 WebP。图片会移除附带信息。','PNG, JPG or WebP. Image metadata is removed.')}</p></div></div><div class="grid-2"><div class="field"><label for="profile-name">${t('显示名称','Display name')}</label><input class="input" id="profile-name" name="name" value="${esc(u.name)}" maxlength="100" required></div><div class="field"><label for="profile-email">${t('联系邮箱','Contact email')}</label><input class="input" id="profile-email" type="email" name="public_email" value="${esc(u.public_email)}" placeholder="hello@example.com" maxlength="254"><p class="help">${t('仅平台和已匹配的对方可以看到。','Only HireFerry and an introduced counterpart can see this address.')}</p></div></div>${notice(`${t('登录邮箱','Sign-in email')}: ${esc(u.email)}<br>${t('仅用于登录，不向匹配方展示。无需提供手机号。','Used only for sign-in and never shared with a match. No phone number required.')}`)}${u.role!=='seeker'?'<input type="hidden" name="resume_md" value="">':''}</div></details>${u.role==='seeker'?`<details class="section-disclosure profile-section" data-disclosure="profile-resume" open><summary>${t('经历与技能','Experience & skills')}</summary><div class="disclosure-content"><div class="tabs" role="tablist"><button type="button" data-action="resume-edit" role="tab" aria-selected="true">${t('编辑','Write')}</button><button type="button" data-action="resume-preview" role="tab" aria-selected="false">${t('预览','Preview')}</button></div><label class="field-label" for="resume-md">${t('简历内容','Résumé content')}</label><textarea id="resume-md" name="resume_md" class="input code resume-editor" maxlength="60000" placeholder="${esc(t('# 介绍一下自己\n\n## 我擅长的事\n- ...\n\n## 工作 / 项目经历\n- ...\n\n## 我期待的下一站\n...','# A little about me\n\n## What I do well\n- ...\n\n## Work / projects\n- ...\n\n## What comes next\n...'))}">${esc(u.resume_md)}</textarea><div id="resume-preview" class="markdown" hidden></div><p class="help resume-privacy">${icon('shield')}${t('匹配前，只有平台可以查看这份简历。','Before an introduction, only HireFerry can view this résumé.')}</p></div></details>`:''}<div class="form-actions"><p class="help">${t('不需要填写电话号码、身份证或家庭地址。','No phone number, identity document, or home address required.')}</p><button class="button" type="submit">${t('保存资料','Save profile')}${icon('check')}</button></div></form>${passkeyPanel}${emailSettings}`,'profile');
}
async function myJobs() {
  const trash=qs().get('view')==='trash';
  const r=await api((trash?'/api/my/jobs/trash':'/api/jobs/mine')+listParams());
  const jobs=r.items||[];
  jobCache.clear();jobs.forEach(job=>jobCache.set(job.id,job));
  const actions=trash?`<a href="/app?tab=jobs" data-nav class="button secondary">${icon('back')}${t('返回我的岗位','Back to my roles')}</a>`:`<div class="jobs-page-actions"><a href="/app?tab=jobs&view=trash" data-nav class="button secondary">${icon('trash')}${t('回收站','Trash')}</a><a href="/app/jobs/new" data-nav class="button">${icon('plus')}${t('发布新岗位','Post a new role')}</a></div>`;
  const head=heading(trash?t('岗位回收站','Deleted roles'):t('我的招聘','My roles'),trash?t('首次删除的岗位可以免费恢复一次，或直接永久删除。恢复后再次删除将永久清除。','A role’s first deletion can be restored once for free, or made permanent. Deleting it again after restoration is permanent.'):t('管理岗位与招聘人数，由平台寻找并介绍合适的人。','Manage your roles and hiring targets. HireFerry finds and introduces people.'),actions,trash?'YOUR RECENTLY DELETED ROLES':'YOUR OPPORTUNITIES');
  const rows=jobs.map(job=>`<article class="list-row job-row"><div class="job-row-main"><h3>${trash?esc(job.title):`<a href="/jobs/${encodeURIComponent(job.id)}" data-nav>${esc(job.title)}</a>`}</h3><div class="list-meta">${jobBadge(job)}<span class="job-salary">${money(job.salary_cents,job)} <small>/ ${t('月，底薪','month, base')}</small></span><span>${headcountText(job)}</span><span>${t('描述已修改','Description updates')} ${job.edits_used}</span></div><div class="row-actions job-row-links">${trash?`<button class="text-button" data-action="restore-job" data-id="${esc(job.id)}">${t('免费恢复','Restore for free')}</button><button class="text-button danger-text" data-action="permanently-delete-job" data-id="${esc(job.id)}">${t('永久删除','Delete permanently')}</button>`:`${job.status==='published'?`<a class="text-button" href="/app/jobs/${encodeURIComponent(job.id)}/edit" data-nav>${t('修改描述','Edit description')}</a>`:job.status==='draft'?`<a class="text-button" href="/app?tab=orders" data-nav>${t('查看待付订单','View pending order')} →</a>`:''}<button class="text-button ${Number(job.delete_count)>=1?'danger-text':''}" data-action="delete-job" data-id="${esc(job.id)}">${Number(job.delete_count)>=1?t('永久删除','Delete permanently'):t('删除岗位','Delete role')}</button>`}</div></div>${trash?'':headcountControl(job)}</article>`).join('');
  return shell(`${head}${!trash&&!state.user.employer_paid?notice(`${t('招聘方账户尚未开通。请先开通账户，即可发布招聘。','Your employer account is not active. Activate it to publish roles.')} <button class="text-button" data-action="role-employer">${t('开通账户','Activate account')} →</button> <a class="text-button" href="/app?tab=promos" data-nav>${t('使用优惠码 / 申请优惠','Use a code / Apply for a promotion')}</a>`,'warning'):''}${!trash?`<details class="section-disclosure" data-disclosure="job-headcount-guide"><summary>${t('人数、暂停与恢复说明','Hiring targets, pausing & resuming')}</summary><div class="disclosure-content">${notice(t('人数可免费、无限次调整，不占描述修改次数。已发布岗位填 0 暂停新匹配，改回正数恢复；已有匹配和会话保留。人数由你手动维护。','Change the hiring target free of charge, as often as needed, without using a description update. For a published role, set 0 to pause new introductions; a positive number resumes them. Existing introductions and chats remain. You maintain the count yourself.'))}</div></details>`:''}<section class="panel"><div class="panel-header"><h2>${trash?t('岗位回收站','Role trash'):t('我的岗位','My roles')} <span class="muted">${jobs.length.toString().padStart(2,'0')}</span></h2><span class="new-label">${trash?t('可恢复一次','ONE RESTORATION'):t('底薪透明','BASE SALARY, UPFRONT')}</span></div>${jobs.length?`<div class="list">${rows}</div>${pagination(r.total??jobs.length,r.limit)}`:empty(trash?t('回收站是空的。','The trash is empty.'):t('第一个岗位，从这里开始。','Your first role starts here.'),trash?t('首次删除的岗位会显示在这里，永久删除的岗位无法恢复。','Roles appear here after their first deletion. Permanently deleted roles cannot be restored.'):t('写清工作内容、固定底薪与招聘人数，我们一起寻找合适的人。','Describe the work, fixed base salary, and hiring target. We’ll help find the right person.'),trash?'':`<a href="/app/jobs/new" data-nav class="button secondary">${icon('plus')}${t('创建岗位','Create a role')}</a>`,trash?'trash':'briefcase')}${trash?`<p class="help job-trash-note">${t('删除与恢复均不收费，也不自动退款。删除期间匹配资料与共同会话暂停访问，平台私聊保持独立；永久删除清除岗位标题与正文，保留必要订单和安全关联记录。','Deletion and restoration are free and do not trigger refunds. Deleted roles and shared rooms are inaccessible while in the trash; private platform chats stay separate. Permanent deletion erases the title and description while retaining necessary order and security records.')}</p>`:''}</section>`,'jobs');
}
async function jobEditor(id) {
  let job=null,terms=null;
  if(id){const result=await api(`/api/jobs/${encodeURIComponent(id)}`);job=result.job;terms=result.terms;}
  if(!state.user.employer_paid)return shell(`${heading(t('先开通，再出发。','Activate your account to begin.'))}<section class="panel">${empty(t('招聘方账户尚未开通','Your employer account is not active'),Number(pricing().onboarding_minor??pricing().onboarding_cents)===0?t('早期阶段免费开通，之后可以发布多个岗位。','Free early access includes multiple roles.'):t(`开通费用为 ${money(pricing().onboarding_cents)}，开通后可以发布多个岗位。`,`Activation costs ${money(pricing().onboarding_cents)}. Once activated, you can post multiple roles.`),`<div class="promo-entry"><button class="button" data-action="role-employer">${t('开通账户','Activate account')}</button><a class="text-button" href="/app?tab=promos" data-nav>${t('使用优惠码 / 申请优惠','Use a code / Apply for a promotion')}</a></div>`,'briefcase')}</section>`,'jobs');
  const template=qs().get('template');
  const preset=(!id&&template==='ai_uiux')?{title:t('AI 产品 UI/UX 设计师','AI Product UI/UX Designer'),jd:t(`## 你会做什么\n\n- 负责面向消费者的 AI 产品体验与界面设计\n- 与产品、工程、GTM 团队协作，把想法推进到上线\n- 建立并迭代设计规范、原型和可用性验证\n\n## 我们希望你\n\n- 有 Consumer AI 产品设计经验\n- 熟悉 AI-native 工作方式\n- 能清楚说明作品集中的取舍、验证和结果\n\n## 工作方式与地点\n\n请补充办公地点、远程/混合方式和时区。\n\n## 申请方式\n\n请填写作品集链接和你希望招聘方了解的项目背景。`,`## What you will do\n\n- Own product experience and interface design for a consumer AI product\n- Work with product, engineering, and GTM teams from idea to launch\n- Build and iterate design systems, prototypes, and usability checks\n\n## What you bring\n\n- Consumer AI product design experience\n- Comfort with AI-native workflows\n- Ability to explain decisions, validation, and outcomes in your portfolio\n\n## Working arrangement\n\nAdd location, remote/hybrid expectations, and time zone.\n\n## How to apply\n\nShare a portfolio link and the project context you want the hiring team to understand.`)}:null;
  const locked=!!job,meta=currency(job),kind=locked?'edit':'publish',settings=feeSettings(kind,meta,terms),free=settings.mode==='free';
  const maximum=Number((terms||pricing()).max_edits),exhausted=locked&&job.edits_used>=maximum;
  const salary=job?.salary_cents??parseMinor('6000',meta.currency_decimals);
  let quote='—';try{quote=money(calculateFee(salary,settings),meta);}catch{}
  return shell(`<a href="/app?tab=jobs" data-nav class="back-link">${icon('back')}${t('返回我的招聘','Back to my roles')}</a>${heading(locked?t('让工作描述更清楚。','Make the role a little clearer.'):t('给好的人，一个好机会。','Give the right person a great opportunity.'),locked?t(`标题、币种与底薪已锁定。修改规则：${feeRule('edit',meta,terms)}，最多 ${maximum} 次。`,`The title, currency, and base salary are locked. Update rule: ${feeRule('edit',meta,terms)}. Up to ${maximum} updates.`):t('填写固定底薪，不含奖金、补贴、提成或预期收入。','Enter fixed base salary only, without bonuses, allowances, commission, or projected income.'))}${exhausted?notice(t('这个岗位已用完全部修改机会。','This role has used all available updates.'),'warning'):''}
  <form data-form="job" data-id="${esc(job?.id||'')}" data-fee-mode="${settings.mode}" data-fee-bps="${settings.bps}" data-fee-discount="${settings.discount_percent}" data-fee-fixed="${settings.fixed_minor}" data-exhausted="${exhausted}"><section class="panel">${preset?notice(t('已载入 AI 产品 UI/UX 岗位模板。请补充地点、薪资和申请方式后发布。','An AI product UI/UX role template is loaded. Add location, salary, and application details before publishing.')):''}<div class="field"><label for="job-title">${t('岗位名称','Job title')}</label><input class="input" id="job-title" name="title" maxlength="120" value="${esc(job?.title??preset?.title??'')}" placeholder="${esc(t('例如：Rust 后端工程师','For example: Rust backend engineer'))}" ${locked?'readonly':''} required></div><div class="grid-2"><div class="field"><label for="job-currency">${t('薪资与岗位费用币种','Salary and role-fee currency')}</label>${currencySelect(meta.currency,'currency','job-currency',locked)}<p class="help">${t('金额按所选币种填写，发布后锁定，不自动换算。','Enter amounts in the selected currency. Publication locks it; changing a selection does not convert amounts.')}</p></div><div class="field"><label for="job-salary" id="job-salary-label">${t('每月固定底薪','Fixed monthly base salary')} · ${esc(meta.currency)}</label><input class="input" id="job-salary" name="salary" type="number" min="${amountStep(meta)}" max="${amountValue(1000000000,meta)}" step="${amountStep(meta)}" inputmode="decimal" value="${amountValue(salary,meta)}" ${locked?'readonly':''} required><p class="help">${t('只填写实际保证支付的底薪。','Enter only the base salary you guarantee.')}</p></div></div>
  ${locked?`<div class="notice job-headcount-note">${icon('person')}<div>${headcountText(job)} · <a href="/app?tab=jobs" data-nav class="help-link">${t('招聘人数在我的岗位中免费调整，不占描述修改次数。','Change the hiring target for free in My roles, without using a description update.')}</a></div></div>`:`<div class="field job-headcount-create"><label for="job-headcount">${t('招聘人数','Hiring target')}</label><input class="input" id="job-headcount" name="headcount" type="number" min="0" max="${state.config?.limits?.max_job_headcount??1000000}" step="1" inputmode="numeric" value="1" required aria-describedby="job-headcount-help"><p class="help" id="job-headcount-help">${t('人数不影响发布费，可免费无限次调整。填 0 暂不招聘，改回正数即可恢复新匹配。','The target does not change the publication fee and can be changed for free as often as needed. Set 0 to pause new introductions and a positive number to resume.')}</p></div>`}
  <div class="field"><label for="job-jd">${t('工作描述 · Markdown','Job description · Markdown')}</label><textarea class="input code resume-editor" id="job-jd" name="jd_md" maxlength="60000" placeholder="${esc(t('## 你会做什么\n\n## 我们希望你\n\n## 工作方式与地点\n\n## 其他说明','## What you will do\n\n## What you bring\n\n## How and where we work\n\n## Anything else'))}" required>${esc(job?.jd_md??preset?.jd??'')}</textarea><p class="help">${t('明确工作内容与要求，说明地点和工作方式。请勿索取身份证、手机号等无关信息。','Describe the work, requirements, location, and working arrangement. Avoid requesting unrelated personal information.')}</p></div>
  ${locked?'':promos().jobFields()}
  <div class="notice">${icon('info')}<div ${locked?'':'data-job-payment-note'}>${locked?`${t('剩余修改次数','Updates remaining')}: ${Math.max(0,maximum-job.edits_used)} / ${maximum}. `:''}${free?t('本次免费，确认后立即生效。','This operation is free and takes effect on confirmation.'):t('内容在支付确认后生效。修改订单未支付时，现有描述保持展示。','Changes take effect after payment is confirmed. The current description stays visible while an update is unpaid.')}</div></div><div class="cost-summary"><div class="cost-row"><span>${t('计费规则','Fee calculation')}</span><span id="job-fee-rule">${feeRule(kind,meta,terms)}</span></div><div class="cost-row total"><span>${locked?t('本次修改费用','Update fee'):t('本次发布费用','Publication fee')}</span><strong id="job-cost">${quote}</strong></div></div><p id="job-price-error" class="help" role="status" hidden></p><div data-payment-options data-currency="${esc(meta.currency)}" ${free?'hidden':''}>${free?'':gatewayInputs(meta)}</div><div class="form-actions"><a class="button secondary" href="/app?tab=jobs" data-nav>${t('返回','Back')}</a><button class="button" type="submit" data-job-submit ${exhausted||(!free&&!paymentProviders(meta).length)?'disabled':''}>${free?(locked?t('确认保存修改','Confirm update'):t('确认发布岗位','Confirm publication')):t('确认并前往支付','Confirm and continue to payment')}${icon('arrow')}</button></div></section></form>`,'jobs');
}
function refreshJobPrice(form) {
  if(!form)return;
  const selected=form.elements.currency?.value||currency().currency,meta=currency(selected),locked=!!form.dataset.id;
  const settings=locked?{mode:form.dataset.feeMode,bps:Number(form.dataset.feeBps),discount_percent:Number(form.dataset.feeDiscount),fixed_minor:Number(form.dataset.feeFixed)}:feeSettings('publish',meta);
  const paymentOptions=form.querySelector('[data-payment-options]');
  if(!locked&&paymentOptions.dataset.currency!==meta.currency){const previous=form.elements.provider?.value;paymentOptions.innerHTML=settings.mode==='free'?'':gatewayInputs(meta);paymentOptions.dataset.currency=meta.currency;if(previous&&paymentProviders(meta).includes(previous)&&form.elements.provider)form.elements.provider.value=previous;}
  const input=form.elements.salary;
  input.step=amountStep(meta);input.min=amountStep(meta);input.max=amountValue(1000000000,meta);
  form.querySelector('#job-salary-label').textContent=t('每月固定底薪','Fixed monthly base salary')+' · '+meta.currency;
  if(!locked)form.querySelector('#job-fee-rule').textContent=feeRule('publish',meta);
  let valid=true,error='';
  try{form.querySelector('#job-cost').textContent=money(calculateFee(centsInput(input.value,meta),settings),meta);}catch(cause){valid=false;error=cause.message==='Fee below one minor unit'?t(`当前比例计算的费用不足 ${money(1,meta)}，请检查底薪金额。`,`The calculated fee is below ${money(1,meta)}. Check the base salary.`):cause.message;form.querySelector('#job-cost').textContent='—';}
  const message=form.querySelector('#job-price-error');message.textContent=error;message.hidden=valid;
  const submit=form.querySelector('[data-job-submit]');
  submit.disabled=!valid||form.dataset.exhausted==='true'||(settings.mode!=='free'&&!paymentProviders(meta).length);
  if(!locked){const note=form.querySelector('[data-job-payment-note]');if(note)note.textContent=settings.mode==='free'?t('本次免费，确认后立即发布。','This publication is free and takes effect on confirmation.'):t('付款确认后发布岗位。','The role is published after payment is confirmed.');paymentOptions.hidden=settings.mode==='free';submit.innerHTML=(settings.mode==='free'?t('确认发布岗位','Confirm publication'):t('确认并前往支付','Confirm and continue to payment'))+icon('arrow');if(form.elements.promo_code)promos().refreshJob(form);}
}

function syncCurrencyFields(container=root) {
  const example=container.querySelector('#salary-example'),result=container.querySelector('#salary-example-result');
  if(example&&result){try{result.textContent=money(calculateFee(centsInput(example.value),feeSettings('publish')));}catch{result.textContent='—';}}
  const jobForm=container.querySelector('form[data-form="job"]');
  if(jobForm)refreshJobPrice(jobForm);
  for(const select of container.querySelectorAll('select[name="salary_currency"]'))service.updateSalaryCurrency(select);
}

function orderRefundDetails(order) {const refunded=Number(order.refunded_cents||0);return refunded>0?`<span class="order-refund">${t('已退','Refunded')} ${money(refunded,order)}</span><span class="order-net">${t('当前净额','Net amount')} ${money(Number(order.amount_cents)-refunded,order)}</span>`:'';}

async function ordersPage() {
  const r=await api('/api/orders');
  if(state.user?.role==='employer'&&!state.user.employer_paid&&(r.items||[]).some(order=>order.kind==='onboarding'&&order.status==='paid'))await session();
  const billing=createBillingUI({state,t,esc,icon,money,date,kindText,providerText,badge,orderRefundDetails,empty,contactMailto});
  return shell(`${heading(t('订单与付款','Orders & payments'),t('查看付款状态，继续开通或查找历史订单。','Check payments, complete activation, and find previous orders.'),`<button type="button" class="text-button" data-action="billing-refresh">${icon('clock')}${t('刷新状态','Refresh status')}</button>`,'YOUR ORDERS')}${billing.ordersContent(r.items||[])}`,'orders');
}
async function agentsPage() {
  const [r,eligibility,humanGrants]=await Promise.all([api('/api/tokens'),api('/api/ai/eligibility'),state.config?.humangrant_enabled?humanGrant().grantsPanel():'']);
  const allowed=!!eligibility.eligible;
  return shell(`${heading(t('AI 连接','AI connections'),t('连接助手来沟通和跟进匹配。','Connect an assistant for conversations and follow-ups.'),'','AI CONNECTIONS')}<section class="cli-connect-promo"><div><h2>${t('命令行工具','Command-line tools')}</h2><p>${t('安装 hfy 后登录。AI 助手可通过技能使用。','Install hfy and sign in. Add the skill for your AI assistant.')}</p></div><a href="/cli" data-nav class="button secondary small">${t('CLI 与技能安装','Install CLI & skill')}${icon('arrow')}</a></section>${humanGrants}<section class="panel"><div class="panel-header"><div><h2>${t('连接你的 AI 助手','Connect your AI assistant')}</h2><p>${t('通过 MCP 连接，或使用支持 WebMCP 的浏览器。','Connect through MCP, or use a browser that supports WebMCP.')}</p></div><span class="pill">MCP + WebMCP</span></div>${state.user.role==='employer'?notice(t(`招聘方需要至少一个符合发布时 AI 薪资门槛的已发布岗位，且发布权益有效、未关闭或删除。当前新岗位门槛：${(eligibility.currency_thresholds||currencies()).map(option=>money(option.employer_ai_min_salary_minor??eligibility.min_salary_cents,option)).join(' / ')}。原岗位保留发布时门槛；人数为 0 不影响资格。`,`Employers need an entitled, published role that meets its recorded AI salary threshold and is neither closed nor deleted. Current thresholds for new roles: ${(eligibility.currency_thresholds||currencies()).map(option=>money(option.employer_ai_min_salary_minor??eligibility.min_salary_cents,option)).join(' / ')}. Existing roles retain their publication threshold. A target of 0 does not remove eligibility.`),allowed?'':'warning'):notice(t('求职者免费使用 AI 连接。AI 只能访问你的资料、委托、会话及平台建立的匹配。','AI connections are free for job seekers. AI can access only your profile, search brief, conversations, and platform introductions.'))}${!allowed?`<div class="ai-locked"><span>${icon('key')}</span><h3>${t('完成条件后，AI 连接会开放。','AI access opens when the condition is met.')}</h3><p>${t('你仍然可以正常使用工作台，与平台沟通。','You can continue using the workspace and talking to HireFerry.')}</p><a class="button secondary" href="/app?tab=jobs" data-nav>${t('查看我的岗位','View my roles')}${icon('arrow')}</a></div>`:`<div class="field"><label>${t('MCP 服务地址','MCP server URL')}</label><div class="code-box">${esc(location.origin)}/mcp</div></div><p class="help">${t('连接方式：HTTP。使用下面创建的令牌，作为 Authorization: Bearer 请求头。令牌仅可操作你的账户。','Connection: HTTP. Use a token below as an Authorization: Bearer header. Tokens can only act on your account.')}</p><div class="notice" style="margin-top:20px">${icon('shield')}<div>${t('AI 发送的消息会标记为 AI 代发。平台私聊与匹配会话保持独立，收费操作仍需支付。','Messages sent by AI are labeled. Private and shared rooms remain separate, and paid actions still require payment.')}</div></div><form data-form="token" class="token-create-form"><div class="field"><label for="token-name">${t('连接名称','Connection name')}</label><input class="input" id="token-name" name="name" maxlength="80" placeholder="${esc(t('例如：我的 AI 助手','For example: My AI assistant'))}" required></div><button class="button secondary" type="submit">${icon('plus')}${t('创建访问令牌','Create access token')}</button></form><div id="new-token"></div>`}</section><section class="panel"><div class="panel-header"><h2>${t('已授权的连接','Authorized connections')}</h2></div>${r.items?.length?`<div class="list">${r.items.map(k=>`<div class="list-row"><div><h3>${esc(k.name)}</h3><div class="list-meta"><span>${t('创建于','Created')} ${date(k.created_at)}</span><span>${t('到期','Expires')} ${date(k.expires_at)}</span>${!allowed?`<span class="status closed">${t('当前不可用','Currently unavailable')}</span>`:''}</div></div><button class="text-button" data-action="revoke-token" data-id="${esc(k.id)}">${t('撤销','Revoke')}</button></div>`).join('')}</div>`:empty(t('你还没有连接 AI。','No AI connections yet.'),allowed?t('准备好时，创建一个令牌即可开始。','Create a token when you’re ready. Keep it private.'):t('满足条件后，可以创建个人 AI 连接。','Create a personal AI connection when eligible.'),'','bot')}</section>`,'agents');
}
async function jobsPage() { await go(state.user?'/app?tab=matches':'/login');return ''; }
async function jobDetail(id) {
  const {job,employer,jd_html}=await api(`/api/jobs/${encodeURIComponent(id)}`);
  const own=job.owner_id===state.user?.id;
  return `<main id="main" class="page-width public-layout"><section class="panel"><a class="back-link" href="/app?tab=${own?'jobs':'matches'}" data-nav>${icon('back')}${own?t('我的招聘','My roles'):t('我的匹配','My introductions')}</a><div class="job-heading"><div><h1>${esc(job.title)}</h1><div class="list-meta">${jobBadge(job)}<span>${headcountText(job)}</span><span>${t('发布于','Posted')} ${date(job.created_at)}</span></div></div><div class="job-salary">${money(job.salary_cents,job)} <small>/ ${t('月，底薪','month, base')}</small></div></div>${job.status==='published'&&jobHeadcount(job)===0?notice(t('这个岗位暂不接收新的匹配，已有匹配会话仍可继续。','This role is paused for new introductions. Existing introduction rooms remain available.')):''}<article class="markdown">${jd_html||''}</article></section><aside class="public-sidebar panel">${avatar(employer,'large')}<h3 style="margin-top:17px">${esc(employer?.name||t('招聘方','Employer'))}</h3><span class="pill">${icon('shield')}${t('仅匹配方可见','Shared with introduced parties')}</span>${employer?.public_email?`<a href="mailto:${encodeURIComponent(employer.public_email)}" class="contact-email">${esc(employer.public_email)}</a>`:`<p class="help" style="margin:16px 0">${t('尚未填写联系邮箱。','No contact email has been added.')}</p>`}<a class="button" href="/app?tab=matches" data-nav>${t('回到匹配会话','Back to introductions')}${icon('message')}</a><p class="help" style="margin-top:18px">${t('这份岗位由平台介绍给你。沟通与后续安排，可以在匹配会话中继续。','HireFerry has introduced this role. Continue the conversation and next steps in your introduction room.')}</p></aside></main>`;
}
async function peoplePage() { await go(state.user?'/app?tab=matches':'/login');return ''; }
async function personDetail(id) {
  const r=await api(`/api/people/${encodeURIComponent(id)}`);const u=r.user;
  return `<main id="main" class="page-width public-layout"><section class="panel"><a class="back-link" href="/app?tab=matches" data-nav>${icon('back')}${t('我的匹配','My introductions')}</a><div class="job-heading"><div class="sidebar-user" style="margin:0">${avatar(u,'large')}<div><h1>${esc(u.name)}</h1><p class="help">${t('平台介绍的求职者','Introduced by HireFerry')}</p></div></div></div><article class="markdown">${r.resume_html||''}</article></section><aside class="public-sidebar panel"><h3>${t('从一次交流开始。','Start a conversation.')}</h3><span class="pill">${icon('shield')}${t('仅匹配方可见','Shared with introduced parties')}</span>${u.public_email?`<a class="contact-email" href="mailto:${encodeURIComponent(u.public_email)}">${esc(u.public_email)}</a>`:`<p class="help">${t('尚未填写联系邮箱。','No contact email has been added.')}</p>`}<a class="button" href="/app?tab=matches" data-nav>${t('回到匹配会话','Back to introductions')}${icon('message')}</a><p class="help" style="margin-top:16px">${t('这份资料仅用于此次匹配。请尊重对方的隐私与选择。','This profile is shared for your introduction. Respect this person’s privacy and choices.')}</p></aside></main>`;
}
async function legal(kind) {
  const language=state.lang;
  const documentContent=await api(`/api/legal/${encodeURIComponent(kind)}?lang=${encodeURIComponent(language)}`);
  return renderLegal(documentContent,language);
}

function refundsPage() {
  return `<main id="main" class="page-width"><article class="prose"><div class="eyebrow">PAYMENTS, WITH CARE</div><h1 style="margin-top:18px">${t('退款问题，我们认真核对。','Refunds deserve a careful review.')}</h1><p>${t('这份说明适用于聘渡的招聘方开通、岗位发布和岗位描述修改订单。求职者使用服务免费。','This policy covers HireFerry employer activation, job publication, and job description update orders. The service is free for job seekers.')}</p><h2>${t('如何提出退款问题','How to ask about a refund')}</h2><p>${t('如有误付、重复付款、付款后服务未生效或其他订单异常，请发送邮件至支持邮箱，并提供订单号与简要情况说明。订单号可以在工作台的「订单」中找到。平台会根据订单记录和实际服务情况进行人工核对。','For a mistaken payment, duplicate payment, a service that did not activate after payment, or another order issue, email support with your order number and a brief description. Find the order number under Orders in your workspace. HireFerry reviews the order record and the service actually provided.')}</p><p><a href="${esc(contactMailto(t('HireFerry 订单与退款咨询','HireFerry order and refund inquiry')))}" class="contact-email-link" data-refresh-key="contact-email">${esc(contactEmail())}</a></p><h2>${t('退款如何处理','How refunds are handled')}</h2><p>${t('退款经人工核对后，由平台通过支付服务商的商户后台处理。网站不直接发起退款。我们会在沟通中说明核对结果和后续处理方式；请保留订单号，以便继续跟进。','After review, HireFerry handles refunds through the payment provider’s merchant dashboard. The website does not initiate refunds directly. We explain the review result and next steps in our correspondence. Keep the order number for follow-up.')}</p><h2>${t('如何确认退款状态','How to confirm refund status')}</h2><p>${t('收到并验证 Waffo 成功退款通知后，订单会显示累计已退款金额、当前净额及「部分退款」或「已全额退款」状态。若原订单仍需人工核对，部分退款后可能继续显示「待人工核对」。失败或尚未确认的退款不会显示为成功，也不会扣减已记录的净收款。请在订单页面刷新核对。Stripe 退款请联系支持核对；本站暂不自动同步 Stripe 退款状态和净额。','After a successful Waffo refund notification is received and verified, the order shows the cumulative refunded amount, current net amount, and Partially refunded or Fully refunded status. An order that already needs manual review may retain Needs review after a partial refund. Failed or unconfirmed refunds are not shown as successful and do not reduce recorded net receipts. Refresh the order page to check. For Stripe refunds, contact support to verify the outcome; this website does not yet automatically update Stripe refund status or net amounts.')}</p><h2>${t('只提供必要的信息','Share only what is needed')}</h2><p>${t('退款咨询不需要提供银行卡卡号、手机号、验证码、通行密钥或 AI 访问令牌。请提供订单号与问题说明；支付信息由对应支付服务商处理。','A refund inquiry does not require a card number, phone number, sign-in code, passkey, or AI access token. Send the order number and a description of the issue. Payment information is handled by the relevant provider.')}</p><div class="policy-actions"><a href="/app?tab=orders" data-nav class="button secondary">${t('查看我的订单','View my orders')}${icon('receipt')}</a><a href="/contact" data-nav class="text-button">${t('联系与支持','Contact and support')} →</a></div></article></main>`;
}

function contactPage() {
  let origin=location.origin;
  try {const configured=new URL(state.config?.public_origin||origin);if(['https:','http:'].includes(configured.protocol))origin=configured.origin;}catch{}
  return `<main id="main" class="page-width"><article class="prose contact-page"><h1 style="margin-top:18px">${t('联系我们','Contact us')}</h1><p>${t('咨询求职、招聘、账户或订单问题。','Help with your search, hiring, account or order.')}</p><section class="panel contact-card"><span class="contact-icon">${icon('mail')}</span><div><h2>${t('支持邮箱','Support email')}</h2><a href="${esc(contactMailto())}" class="contact-email-link" data-refresh-key="contact-email">${esc(contactEmail())}</a><p class="help">${t('订单与退款咨询请附上订单号和简要说明。','For orders and refunds, include your order number and a brief description.')}</p></div></section><h2>${t('在工作台沟通','Talk in your workspace')}</h2><p>${t('在私聊中讨论需求。在匹配会话中联系对方。','Discuss your needs privately. Contact your match in a shared room.')}</p><a href="${state.user?'/app?tab=messages':'/login'}" data-nav class="button">${t('与平台沟通','Talk to HireFerry')}${icon('message')}</a><h2>${t('关于付款与隐私','Payments and privacy')}</h2><p>${t('付款咨询请提供订单号。资料更正或删除可通过邮箱申请。','Include your order number for payment queries. Email us to correct or delete your profile.')}</p><div class="policy-links"><a href="/refunds" data-nav class="help-link">${t('退款政策','Refund policy')}</a><a href="/privacy" data-nav class="help-link">${t('隐私说明','Privacy notice')}</a><a href="/terms" data-nav class="help-link">${t('服务条款','Terms')}</a></div><p class="contact-origin" data-refresh-key="contact-origin">${t('聘渡官网','HireFerry website')} · <a href="${esc(origin)}" class="help-link">${esc(origin)}</a></p></article></main>`;
}

async function render({navigation=false}={}) {
  const navigationStarted=performance.now();
  const fast=sessionReader.fresh()?service.navigateChat?.(new URL(location.href),{onlyIfChanged:!navigation}):null;
  if(fast){
    requests.cancel(`navigation:${state.render}`);
    const version=++state.render;
    root.dataset.navigationState='loading';root.dataset.navigationSequence=String(version);delete root.dataset.navigationReadyMs;delete root.dataset.navigationShellMs;
    const record=stage=>requestAnimationFrame(()=>{if(version!==state.render)return;root.dataset[stage==='shell'?'navigationShellMs':'navigationReadyMs']=(performance.now()-navigationStarted).toFixed(1);if(stage!=='shell')root.dataset.navigationState=stage;});
    record('shell');await fast;record('ready');return;
  }
  requests.cancel(`navigation:${state.render}`);
  const version=++state.render;resetAdminViews();service.stopChat();state.cliGuide?.dispose();state.cliGuide=null;state.homeStory?.dispose();state.homeStory=null;state.particleTitles?.dispose();state.particleTitles=null;document.body.classList.toggle('home-page',location.pathname==='/');state.sculpture?.dispose();state.sculpture=null;document.documentElement.lang=state.lang==='en'?'en':'zh-CN';updatePageMetadata();
  state.downloads?.dispose();state.downloads=null;
  const path=location.pathname,tab=qs().get('tab');
  const publicPage=isPublicPagePath(path);
  const pageKey=pageIdentity(path,tab);
  const placeholderTab=path.startsWith('/human/')?'human-review':path.startsWith('/app/jobs/')?'editor':tab||(path==='/admin'?'overview':'messages');
  const pendingBody=()=>{if(isPublicDirectoryPage(path))return publicDirectorySkeleton(path);const body=pagePlaceholder(placeholderTab,t('正在加载内容','Loading content'));if(path==='/admin'&&state.admin)return shell(body,tab||'overview',true);if(path.startsWith('/app')&&state.user)return shell(body,tab||'messages');return `<main id="main" class="page-width initial-placeholder">${body}</main>`;};
  root.dataset.navigationState='loading';root.dataset.navigationSequence=String(version);delete root.dataset.navigationReadyMs;delete root.dataset.navigationShellMs;
  const recordPaint=stage=>requestAnimationFrame(()=>{if(version!==state.render)return;root.dataset[stage==='shell'?'navigationShellMs':'navigationReadyMs']=(performance.now()-navigationStarted).toFixed(1);if(stage!=='shell')root.dataset.navigationState=stage;});
  const showPending=(force=false)=>{if(!force&&root.dataset.pageKey===pageKey){root.classList.add('page-pending');(root.querySelector('.app-content')||root.querySelector('main'))?.setAttribute('aria-busy','true');if((path==='/app'&&(!tab||tab==='messages'))||(path==='/admin'&&tab==='messages'))service.showPendingChat(qs().get('conversation'));}else {controlsUI.clear(root);mountPage(root,header()+pendingBody()+footer(),pageKey,{busy:true});disclosureUI.sync({scope:identityOf(state)});}};
  showPending();
  recordPaint('shell');
  try {
    let loadedConfig=false;
    if(!state.config||(!publicPage&&state.configCached)) {
      // Read in parallel, then apply identity: its private-request cancellation must not abort config.
      // A public page needs no account data before its first paint.
      const [result]=await Promise.all([configCache.refresh(),publicPage?Promise.resolve():sessionReader.read()]);
      if(version!==state.render)return;
      if(!publicPage)await session(version);
      if(version!==state.render)return;
      applyConfig(result.value);loadedConfig=true;showPending(true);
    }
    if(resumeCliLogin())return;
    let body;
    if(path==='/')body=home();
    else if(path==='/discover')body=await publicDirectoryPage();
    else if(/^\/discover\/jobs\/[A-Za-z0-9_-]+$/.test(path))body=await publicJobPage(decodeURIComponent(path.split('/')[3]));
    else if(/^\/discover\/people\/[A-Za-z0-9_-]+$/.test(path))body=await publicPersonPage(decodeURIComponent(path.split('/')[3]));
    else if(/^\/u\/[A-Za-z0-9_-]+$/.test(path))body=await publicProfilePage(decodeURIComponent(path.split('/')[2]));
    else if(path==='/for-employers')body=employerLandingPage();
    else if(path==='/cli')body=renderCliGuide({config:state.config,origin:location.origin,lang:state.lang,esc,icon});
    else if(path==='/downloads')body=renderDownloads({lang:state.lang,icon});
    else if(path==='/humangrant')body=humanGrant().intro();
    else if(/^\/human\/[A-Za-z0-9_-]+$/.test(path)){await session(version);if(version!==state.render)return;body=await humanGrant().reviewPage(path.split('/')[2],version);}
    else if(path==='/team/invite'){await session(version);if(version!==state.render)return;body=await team().reviewPage(location.hash.slice(1),version);}
    else if(path==='/login'){if(!sessionKnown)await session(version);if(version!==state.render||resumeCliLogin())return;if(state.user||state.admin){go(afterSignIn());return;}body=login();}
    else if(/^\/staff\/invite\/[A-Za-z0-9_-]{43}$/.test(path)){await session(version);if(version!==state.render)return;body=await staffUI().invitePage(path.split('/')[3]);}
    else if(path==='/jobs'||path==='/people'){go(`/discover?tab=${path==='/people'?'people':'jobs'}`);return;}
    else if(/^\/jobs\/[^/]+$/.test(path))body=await jobDetail(decodeURIComponent(path.split('/')[2]));
    else if(/^\/people\/[^/]+$/.test(path))body=await personDetail(decodeURIComponent(path.split('/')[2]));
    else if(path==='/privacy'||path==='/terms')body=await legal(path.slice(1));
    else if(path==='/refunds')body=refundsPage();
    else if(path==='/contact')body=contactPage();
    else if(path==='/app'||path.startsWith('/app/')) {
      await session(version);if(version!==state.render)return;if(!state.user){const returnTo=path==='/app/jobs/new'?`${path}${location.search}`:'';go(returnTo?`/login?return_to=${encodeURIComponent(returnTo)}`:'/login');return;}
      if(path==='/app'&&!tab&&!qs().has('order')){const returnTo=afterSignIn();if(returnTo!=='/app'){go(returnTo);return;}}
      if(path==='/app/jobs/new')body=await jobEditor();
      else if(/^\/app\/jobs\/[^/]+\/edit$/.test(path))body=await jobEditor(decodeURIComponent(path.split('/')[3]));
      else if(qs().has('order')&&!tab)body=await paymentReturn();
      else if(tab==='messages')body=await service.messagesPage();
      else if(tab==='mandate')body=await service.mandatePage();
      else if(tab==='matches')body=await matchesPage();
      else if(tab==='orders')body=await ordersPage();
      else if(tab==='promos')body=await promos().memberPage();
      else if(tab==='agents')body=await agentsPage();
      else if(tab==='webhooks')body=await service.webhooksPage();
      else if(tab==='profile')body=await profilePage();
      else if(tab==='jobs'&&state.user.role==='employer')body=await myJobs();
      else body=await service.messagesPage();
    }
    else if(path==='/admin'){await session(version);if(version!==state.render)return;body=await adminPage(tab||'overview');}
    else if(path==='/billing/return'||path==='/payment/return'||path==='/payment/success'||path==='/checkout/return'){await session(version);if(version!==state.render)return;body=await paymentReturn();}
    else body=`<main id="main" class="page-width page-error"><h1 style="margin-top:20px">${t('页面未找到','Page not found')}</h1><p>${t('请检查地址。','Check the address.')}</p><a href="/" data-nav class="button">${t('回到首页','Back to home')}${icon('arrow')}</a></main>`;
    if(version!==state.render||resumeCliLogin())return;controlsUI.clear(root);mountPage(root,header()+body+footer(),pageKey);disclosureUI.sync({scope:identityOf(state)});syncCurrencyFields();controlsUI.sync(root);if(path==='/admin'){financeCleanup=financeModule?.bind(root);peopleJobsModule?.mount(root);}recordPaint('ready');mountBrand();if(path==='/')void mountHome();
    if(path==='/cli')state.cliGuide=mountCliGuide(root);
    if(path==='/downloads')state.downloads=mountDownloads(root,{lang:state.lang,icon});
    if(publicPage)void refreshPublicPage(version,{refreshConfig:!loadedConfig});
    if((path==='/app'&&(!tab||tab==='messages')&&!qs().has('order'))||(path==='/admin'&&tab==='messages'))service.mountChat();
    if(path.startsWith('/app')&&tab!=='promos'&&state.user?.role==='unassigned'&&!state.admin)chooseRole();
    if(location.hash){document.getElementById(location.hash.slice(1))?.scrollIntoView();}
    registerWebMCP();
  } catch(error) {if(version!==state.render)return;const publicDetail=isPublicDirectoryPage(path)&&path!=='/discover';const back=path.startsWith('/discover/people/')?'/discover?tab=people':'/discover?tab=jobs';const message=publicDetail?`<section class="route-error public-route-error" role="alert"><p class="eyebrow">${t('公开内容','PUBLIC CONTENT')}</p><h1>${t('这个公开内容暂时无法查看','This public content is not available')}</h1><p>${t('它可能已经停止公开，或者链接已经失效。','It may have stopped being public or the link may have expired.')}</p><div class="form-actions"><a class="button secondary" href="${back}" data-nav>${t('返回公开目录','Back to public directory')}${icon('back')}</a><button class="button" data-action="retry">${t('重试','Try again')}${icon('arrow')}</button></div></section>`:`<section class="route-error" role="alert"><h1>${t('内容暂时无法加载','Content could not be loaded')}</h1><p>${esc(error.message)}</p><button class="button" data-action="retry">${t('重试','Try again')}${icon('arrow')}</button></section>`;const body=path==='/admin'&&state.admin?shell(message,tab||'overview',true):path.startsWith('/app')&&state.user?shell(message,tab||'messages'):`<main id="main" class="page-width">${message}</main>`;mountPage(root,header()+body+footer(),pageKey);disclosureUI.sync({scope:identityOf(state)});recordPaint('error');}
}

async function adminPage(tab) {
  if(state.bootstrap&&!state.admin)return `<main id="main" class="page-width admin-auth"><div class="admin-auth-top"><span class="icon-badge">${icon('key')}</span><h1>${t('绑定第一把通行密钥','Add your first passkey')}</h1><p>${t('完成后，管理员登录只接受通行密钥。','After this step, administrator sign-in uses passkeys only.')}</p></div><section class="auth-card"><h2>${t('安全地开始。','A secure beginning.')}</h2><p class="subtitle">${t('使用设备指纹、面容或实体安全密钥。一次性密码不会再用于登录。','Use your device’s fingerprint, face recognition, or security key. The one-time password cannot be used to sign in again.')}</p><form data-form="passkey-register"><div class="field"><label for="passkey-name">${t('密钥名称','Passkey name')}</label><input class="input" id="passkey-name" name="name" value="${t('我的主设备','My primary device')}" maxlength="80" required></div><button class="button full" type="submit">${icon('key')}${t('绑定通行密钥','Create a passkey')}</button></form><p class="help">${t('请在可信设备上操作。首次绑定完成后可以再添加备用密钥。','Use a trusted device. You can add backup passkeys after this first one.')}</p></section></main>`;
  if(!state.admin){const s=await api('/api/admin/auth/status');return `<main id="main" class="page-width admin-auth"><div class="admin-auth-top"><span class="icon-badge">${icon('shield')}</span><h1>${t('聘渡总控台','HireFerry control room')}</h1><p>${t('每次相遇背后，都有人认真负责。','Real people, behind every connection.')}</p></div><section class="auth-card"><h2>${s.initialized?t('欢迎回来。','Welcome back.'):t('第一次，安全启程。','A secure first step.')}</h2><p class="subtitle">${s.initialized?t('使用已绑定的通行密钥登录。','Sign in with a registered passkey.'):t('输入在服务器上生成的一次性密码。','Enter the one-time password generated on the server.')}</p>${s.initialized?`<button class="button full" data-action="admin-passkey-login">${icon('key')}${t('使用通行密钥登录','Sign in with a passkey')}</button><p class="help">${t('支持已绑定的指纹、面容与实体安全密钥。','Supports registered device biometrics and security keys.')}</p><div class="separator">${t('团队员工','Team staff')}</div><a href="/login" data-nav class="button secondary full">${icon('mail')}${t('使用受邀邮箱登录','Sign in with your invited email')}</a><p class="help">${t('请先接受管理员发给你的邀请，再使用自己的账户登录。','Accept the administrator’s invitation first, then sign in with your own account.')}</p>`:`<form data-form="admin-bootstrap"><div class="field"><label for="bootstrap-password">${t('一次性密码','One-time password')}</label><input class="input" type="password" id="bootstrap-password" name="password" autocomplete="one-time-code" required></div><button class="button full" type="submit">${t('验证并绑定通行密钥','Verify and add a passkey')}${icon('arrow')}</button></form><p class="help">${t('管理员需要先在 Linux 服务器中运行初始化命令。密码短期有效，仅用于首次绑定。','An administrator must run the initialization command on the Linux server first. The password expires soon and is only for the first passkey setup.')}</p>`}</section></main>`;}
  const businessTabs=['messages','matches','mandates','users','jobs'];
  if(!state.owner&&!businessTabs.includes(tab)&&!(state.staff&&['overview','orders'].includes(tab)))return shell(`${heading(t('此页面由超级管理员管理','This page is managed by the super administrator'),t('你可以继续在消息、匹配及其他业务页面协作。','Continue working in messages, introductions, and other business pages.'))}<a class="button" href="/admin?tab=messages" data-nav>${t('打开消息','Open messages')}${icon('arrow')}</a>`,'messages',true);
  if(tab==='team'||tab==='staff')return team().teamPage();
  if(tab==='promos')return promos().adminPage();
  if(tab==='messages')return service.messagesPage(true);if(tab==='mandates')return service.adminMandates();if(tab==='agents')return service.adminTokens();if(tab==='users'||tab==='jobs')return peopleJobs().page();if(tab==='orders')return adminOrders();if(tab==='keys')return adminKeys();if(tab==='matches')return ['search','followups'].includes(qs().get('view'))||qs().has('job')||qs().has('candidate')?matchesPage(true):peopleJobs().page();return adminOverview();
}
async function adminOverview() { return finance().overviewPage(); }
function orderTable(items) {return items.length?`<div class="table-wrap"><table class="data-table"><thead><tr><th>${t('订单','Order')}</th><th>${t('类型','Type')}</th><th>${t('金额','Amount')}</th><th>${t('状态','Status')}</th><th>${t('支付方式','Provider')}</th><th>${t('时间','Created')}</th></tr></thead><tbody>${items.map(o=>`<tr><td><span class="order-number" title="${esc(o.id)}">${esc(o.id.slice(0,12))}</span></td><td>${kindText(o.kind)}</td><td class="money">${money(o.amount_cents,o)}${orderRefundDetails(o)}</td><td>${orderBadge(o)}</td><td>${esc(providerText(o.provider))} ${o.live_mode===false&&o.provider!=='free'?`<span class="test-badge">${t('测试','Test')}</span>`:''}</td><td>${date(o.created_at)}</td></tr>`).join('')}</tbody></table></div>`:empty(t('还没有订单。','No orders yet.'),t('实际交易发生后，记录会出现在这里。','Actual transactions will appear here.'),'','receipt');}
async function editUser(id) {const u=await api(`/api/admin/users/${encodeURIComponent(id)}`);if(!u)throw new Error(t('未找到这个用户，请刷新后重试。','This person was not found. Refresh and try again.'));if(!state.owner&&!state.staff){openDialog(`<h2 id="dialog-title">${esc(u.name||u.email)}</h2><p class="dialog-intro">${esc(u.email)} · ${roleText(u.role)}</p><p>${esc(u.public_email||'')}</p>${u.role==='seeker'?`<div class="field"><label>${t('简历 · 只读','Résumé · read only')}<textarea class="input code" rows="10" readonly>${esc(u.resume_md||'')}</textarea></label></div>`:''}`,{wide:true});return;}openDialog(`<div class="eyebrow">MANAGE PROFILE</div><h2 id="dialog-title">${esc(u.name||u.email)}</h2><p class="dialog-intro">${esc(u.email)} · ${roleText(u.role)}</p><form data-form="admin-user" data-id="${esc(u.id)}"><div class="grid-2"><div class="field"><label>${t('显示名称','Display name')}<input class="input" name="name" value="${esc(u.name)}" maxlength="100" required></label></div><div class="field"><label>${t('联系邮箱','Contact email')}<input class="input" name="public_email" type="email" value="${esc(u.public_email)}" maxlength="254"></label></div></div>${u.role==='seeker'?`<div class="field"><label>${t('简历 · Markdown','Résumé · Markdown')}<textarea class="input code" name="resume_md" maxlength="60000" rows="10">${esc(u.resume_md)}</textarea></label></div>`:''}<p class="help">${t('封禁和恢复访问请在用户列表中单独操作。保存资料不会改变账户状态。','Manage suspension separately from the user list. Saving this profile does not change account access.')}</p><div class="form-actions">${u.role==='seeker'?`<button type="button" class="button danger small" data-action="delete-resume" data-id="${esc(u.id)}">${t('删除简历','Delete résumé')}</button>`:''}<button type="submit" class="button">${t('保存修改','Save changes')}${icon('check')}</button></div></form>`,{wide:true});}
async function userAccessDialog(id) {
  const version=state.render;
  const {user,access}=await api(`/api/admin/users/${encodeURIComponent(id)}/access`);
  if(version!==state.render)return;
  const banned=!access.banned;
  const consequence=banned?t('封禁后立即退出此账户的所有登录，并撤销 AI 令牌。该账户不能继续登录、访问匹配资料或发送消息；通知投递也会停止。','Suspension ends all sign-ins and revokes AI tokens. The account cannot sign in, view introduced profiles or send messages. Event deliveries also stop.'):t('恢复后需要重新登录、重新创建 AI 令牌。旧会话和已撤销的令牌不会恢复。','The person must sign in again and create new AI tokens. Old sessions and revoked tokens will stay invalid.');
  openDialog(`<div class="eyebrow">${t('账户访问','ACCOUNT ACCESS')}</div><h2 id="dialog-title">${banned?t('封禁这个账户？','Suspend this account?'):t('恢复这个账户的访问？','Restore account access?')}</h2><p class="dialog-intro">${esc(user.name||user.email)} · ${esc(user.email)}</p>${notice(consequence,banned?'warning':'')}<form data-form="admin-user-access" data-id="${esc(id)}" data-banned="${banned}"><div class="field"><label for="access-reason">${t('操作原因（会记录到审计日志）','Reason (recorded in the audit log)')}</label><textarea class="input" id="access-reason" name="reason" maxlength="500" rows="3" required></textarea></div>${access.reason?`<p class="help">${t('上次操作原因','Previous reason')}：${esc(access.reason)}${access.changed_at?' · '+date(access.changed_at):''}</p>`:''}<div class="form-actions"><button type="button" class="button secondary" data-action="close-dialog">${t('取消','Cancel')}</button><button type="submit" class="button ${banned?'danger':''}">${banned?t('确认封禁','Suspend account'):t('恢复访问','Restore access')}</button></div></form>`);
}

async function editAdminJob(id) {
  const j=await api(`/api/admin/jobs/${encodeURIComponent(id)}`);
  if(!j)throw new Error(t('未找到岗位。','Role not found.'));
  jobCache.clear();jobCache.set(j.id,j);
  const intro=`<div class="eyebrow">MANAGE OPPORTUNITY</div><h2 id="dialog-title">${esc(j.title||t('已永久删除的岗位','Permanently deleted role'))}</h2><p class="dialog-intro">${money(j.salary_cents,j)} / ${t('月底薪，标题与底薪不能修改。','month base. The title and salary cannot be changed.')}</p><div class="list-meta job-admin-meta">${jobBadge(j)}<span>${headcountText(j)}</span></div>`;
  if(!state.owner&&!state.staff){openDialog(`${intro}<div class="field"><label>${t('岗位描述 · 只读','Job description · read only')}<textarea class="input code" rows="10" readonly>${esc(j.jd_md||'')}</textarea></label></div>`,{wide:true});return;}
  if(j.status==='deleted') {
    openDialog(`${intro}${notice(Number(j.delete_count)===1?t('此岗位在回收站，可免费恢复一次；管理员也可以将其永久删除。','This role is in the trash. It can be restored once for free, or permanently deleted by an administrator.'):t('岗位标题与正文已永久清除，不能恢复。必要的订单与安全关联记录保留。','The title and description have been permanently erased and cannot be restored. Necessary order and security records remain.'))}${Number(j.delete_count)===1?`<div class="form-actions"><button type="button" class="button danger small" data-action="admin-delete-job" data-id="${esc(id)}">${t('永久删除','Delete permanently')}</button><button type="button" class="button" data-action="admin-restore-job" data-id="${esc(id)}">${t('免费恢复一次','Restore once for free')}${icon('back')}</button></div>`:''}`,{wide:true});
    return;
  }
  if(j.status==='draft') {
    openDialog(`${intro}${notice(t('这是尚未完成发布付款的草稿。内容与人数由招聘方管理，管理员可以永久删除。','This draft has not completed publication payment. Its employer manages the content and hiring target; administrators can permanently delete it.'))}<div class="field"><label>${t('岗位描述 · 只读','Job description · read-only')}<textarea class="input code" rows="10" readonly>${esc(j.jd_md)}</textarea></label></div><div class="form-actions"><button type="button" class="button danger small" data-action="admin-delete-job" data-id="${esc(id)}">${t('永久删除岗位','Delete role permanently')}</button></div>`,{wide:true});
    return;
  }
  openDialog(`${intro}${headcountControl(j,true)}${['draft','published'].includes(j.status)?`<p class="help job-admin-headcount-help">${t('人数可免费无限次修改，不占描述修改次数。0 只暂停新匹配，原有匹配与会话保留。','Change the target free of charge as often as needed, without using a description update. Zero pauses new introductions; existing introductions and chats remain.')}</p><hr>`:''}<form data-form="admin-job" data-id="${esc(id)}"><div class="field"><label>${t('岗位描述 · Markdown','Job description · Markdown')}<textarea name="jd_md" class="input code" rows="12" maxlength="60000">${esc(j.jd_md)}</textarea></label></div><div class="field"><label>${t('岗位状态','Role status')}<select name="status" class="input"><option value="${esc(j.status)}">${j.status==='published'?t('已发布','Published'):statusText(j.status)}</option>${j.status==='published'||j.status==='draft'?`<option value="closed">${t('永久关闭招聘','Close hiring permanently')}</option>`:''}</select></label><p class="help">${t('只想暂停新匹配，请把人数改为 0。关闭后不能通过增加人数重新招聘。','To pause new introductions, set the target to 0. A closed role cannot resume hiring by increasing its target.')}</p></div><div class="form-actions"><button type="button" class="button danger small" data-action="admin-delete-job" data-id="${esc(id)}">${t('永久删除岗位','Delete role permanently')}</button><button type="submit" class="button">${t('保存内容与状态','Save content and status')}${icon('check')}</button></div></form>`,{wide:true});
}
async function adminOrders() { return finance().ordersPage(); }
async function adminKeys() {const d=await api('/api/admin/passkeys');return shell(`${heading(t('守好每一把钥匙。','Keep the keys in good hands.'),t('管理员只通过通行密钥登录。建议绑定至少两把。','Administrators sign in with passkeys only. Keep at least two registered.'),'','PASSKEYS')}<section class="panel"><div class="panel-header"><div><h2>${t('已绑定的通行密钥','Registered passkeys')}</h2><p>${t('添加或删除密钥需要近期身份验证。','Adding or removing passkeys requires recent authentication.')}</p></div><button class="button secondary small" data-action="admin-reauth">${icon('shield')}${t('重新验证','Verify again')}</button></div><div class="list">${(d.passkeys||[]).map(k=>`<div class="list-row"><div><h3>${esc(k.name)}</h3><div class="list-meta"><span>${t('绑定于','Added')} ${date(k.created_at)}</span><span>${t('最近使用','Last used')} ${date(k.last_used_at)}</span></div></div><button class="text-button" data-action="delete-passkey" data-id="${esc(k.id)}" ${(d.passkeys||[]).length<2?'disabled':''}>${t('移除','Remove')}</button></div>`).join('')}</div><hr><form data-form="passkey-register"><div class="grid-2"><div class="field"><label for="passkey-name">${t('新密钥名称','New passkey name')}</label><input class="input" id="passkey-name" name="name" maxlength="80" placeholder="${esc(t('例如：备用安全密钥','For example: Backup security key'))}" required></div><div class="field" style="align-self:end"><button class="button" type="submit">${icon('plus')}${t('绑定另一把密钥','Add another passkey')}</button></div></div></form><p class="help">${t('不能删除最后一把通行密钥。密钥私密部分保存在你的设备中，本站仅保存公钥。','The last passkey cannot be removed. Its private part stays on your device; this site stores the public key.')}</p></section>`,'keys',true);}
const matchText=s=>({introduced:t('已介绍','Introduced'),interview:t('面试中','Interviewing'),hired:t('已入职','Hired'),declined:t('暂不合适','Not a fit'),pending:t('待对接','Pending')})[s]||s;
async function matchesPage(admin=false) { return service.matchesPage(admin); }
async function newMatch() { return service.newMatch(); }
async function editMatch(id) { return service.matchDetail(id); }
function billingLoginPrompt(orderId='') {
  const loginUrl='/login?'+new URLSearchParams({return_to:billingReturnPath(orderId)});
  return `<main id="main" class="page-width"><section class="billing-return"><span class="billing-icon">${icon('shield')}</span><div class="eyebrow">YOUR PAYMENT, PRIVATELY</div><h1>${t('先登录，再查看订单。','Sign in to check your order.')}</h1><p class="billing-intro">${t('订单只对所属账户开放。请使用付款时的账户登录，之后查看支付服务商已确认的状态。','Orders are private to their account. Sign in with the account used for payment to check the provider-confirmed status.')}</p><div class="billing-actions"><a href="${esc(loginUrl)}" data-nav class="button">${t('登录并查看订单','Sign in and view order')}${icon('arrow')}</a><a href="/contact" data-nav class="text-button">${t('需要帮助','Need help?')}</a></div></section></main>`;
}

async function paymentReturn() {
  // Query parameters identify an order only. Payment and refund state always
  // come from the account-authorized read endpoint, never the return URL.
  const requested=qs().get('order_id')||qs().get('order')||'';
  const orderId=/^[A-Za-z0-9_-]{1,128}$/.test(requested)?requested:'';
  if(!state.user)return billingLoginPrompt(orderId);
  const actions=`<div class="billing-actions"><button class="button secondary" data-action="billing-refresh">${t('刷新订单状态','Refresh order status')}${icon('clock')}</button><a href="/app?tab=orders" data-nav class="button">${t('我的订单','My orders')}${icon('arrow')}</a></div>`;
  if(!orderId)return `<main id="main" class="page-width"><section class="billing-return"><span class="billing-icon">${icon('receipt')}</span><div class="eyebrow">YOUR PAYMENT RECORD</div><h1>${t('到订单里，确认这次付款。','Check this payment in your orders.')}</h1><p class="billing-intro">${t('返回链接中没有可用的订单号。你可以在我的订单中查看当前状态。返回页面本身不代表付款成功。','This return link has no usable order number. Check the current status in My orders. Returning from checkout does not confirm payment.')}</p>${actions}</section></main>`;
  let order;
  try {order=await api('/api/orders/'+encodeURIComponent(orderId));}
  catch(error) {
    if(error.status===401)return billingLoginPrompt(orderId);
    return `<main id="main" class="page-width"><section class="billing-return"><span class="billing-icon">${icon('receipt')}</span><div class="eyebrow">YOUR PAYMENT RECORD</div><h1>${t('暂时无法查看这笔订单。','We couldn’t load this order yet.')}</h1><p class="billing-intro">${error.status===403||error.status===404?t('请确认你正在使用付款时的账户，也可以到我的订单中核对。','Check that you are using the account that made the payment, or look in My orders.'):t('订单状态暂时无法读取。请稍后刷新，或向支持邮箱提供订单号进行核对。','The order status could not be loaded. Refresh shortly, or send the order number to support for review.')}</p><p class="billing-order-id">${esc(orderId)}</p>${actions}<p class="billing-support"><a href="${esc(contactMailto())}" class="help-link">${esc(contactEmail())}</a></p></section></main>`;
  }
  order={...order,status:effectiveOrderStatus(order)};
  const canRestart=order.kind==='onboarding'&&!state.user.employer_paid&&['expired','failed','canceled','cancelled','refunded'].includes(order.status);
  const recoveryAction=canRestart?`<button type="button" class="button" data-action="role-employer">${t('重新发起开通','Restart activation')}${icon('arrow')}</button>`:'';
  const title=order.provider==='free'?t('已免费完成。','Completed free of charge.'):({expired:t('这笔订单已过期。','This order has expired.'),pending:t('等待支付确认。','Waiting for payment confirmation.'),paid:t('付款已确认。','Payment confirmed.'),paid_needs_review:t('款项已收，等待人工核对。','Payment received. A review is needed.'),partially_refunded:t('这笔订单已部分退款。','This order is partially refunded.'),refunded:t('这笔订单已全额退款。','This order is fully refunded.'),failed:t('这次付款尚未完成。','This payment was not completed.'),canceled:t('这笔订单已取消。','This order was cancelled.'),cancelled:t('这笔订单已取消。','This order was cancelled.')})[order.status]||t('查看这笔订单的当前状态。','Check the current order status.');
  const explanation=order.provider==='free'?t('本次操作没有收取费用，对应内容已生效。','No fee was charged for this operation. Its changes have taken effect.'):({pending:t('支付服务商的确认尚未到达。请稍后刷新，暂时不要仅因这个页面再次付款。','The provider’s confirmation has not arrived yet. Refresh shortly; do not pay again solely because of this page.'),paid:t('本站已收到并验证支付确认。你可以在工作台继续查看对应服务。','The site has received and verified payment confirmation. Continue in your workspace to view the corresponding service.'),paid_needs_review:t('付款已确认，但对应服务未能自动应用。请联系支持邮箱，并提供下方订单号。','Payment is confirmed, but the corresponding service could not be applied automatically. Contact support with the order number below.'),partially_refunded:t('已确认的退款金额与当前净额显示如下。如仍有问题，请保留订单号联系支持邮箱。','The confirmed refunded amount and current net amount are shown below. Keep the order number when contacting support.'),refunded:t('本站已收到并验证全额退款确认。订单记录会保留，便于后续查询。','The site has received and verified confirmation of a full refund. The order record remains available for reference.'),failed:t('请在我的订单中查看详情；如你已经付款，请先联系支持邮箱核对。','Check My orders for details. If you have already paid, contact support for a review first.'),canceled:t('请以这笔订单的记录为准。如有误付或异常，可向支持邮箱提供订单号。','Use this order record as the reference. Contact support with its number about mistaken payments or other issues.'),cancelled:t('这笔订单不再接受付款。如已经付款，请提供订单号联系支持邮箱核对。','This order no longer accepts payment. If you already paid, contact support with the order number for a review.')})[order.status]||t('显示内容来自当前账户的订单记录。','This information comes from the current account’s order record.');
  return `<main id="main" class="page-width"><section class="billing-return"><span class="billing-icon">${icon(order.status==='paid'?'check':'receipt')}</span><div class="eyebrow">YOUR PAYMENT RECORD</div><h1>${title}</h1><p class="billing-intro">${explanation}</p><article class="panel billing-order"><div class="panel-header"><h2>${kindText(order.kind)}</h2>${order.provider==='free'?`<span class="status paid">${t('免费完成','Completed free')}</span>`:badge(order.status)}</div><dl><div><dt>${t('订单号','Order number')}</dt><dd class="billing-order-id">${esc(order.id)}</dd></div><div><dt>${t('订单金额','Order amount')}</dt><dd>${money(order.amount_cents,order)}</dd></div>${Number(order.refunded_cents)>0?`<div><dt>${t('已确认退款','Confirmed refunds')}</dt><dd>${money(order.refunded_cents,order)}</dd></div><div><dt>${t('当前净额','Net amount')}</dt><dd>${money(Number(order.amount_cents)-Number(order.refunded_cents),order)}</dd></div>`:''}<div><dt>${t('支付服务商','Payment provider')}</dt><dd>${esc(providerText(order.provider))}${order.live_mode===false&&order.provider!=='free'?` <span class="test-badge">${t('测试订单','Test order')}</span>`:''}</dd></div><div><dt>${t('创建时间','Created')}</dt><dd>${date(order.created_at)}</dd></div></dl></article><p class="help billing-source">${order.provider==='free'?t('这里显示当前账户的免费操作记录。','This is the current account’s record of a free operation.'):t('返回页面本身不代表付款成功。这里显示本站已验证的订单记录。','Returning from checkout does not confirm payment. This page shows the site’s verified order record.')}</p>${recoveryAction?`<div class="billing-actions">${recoveryAction}</div>`:''}${actions}<div class="billing-support"><a href="/refunds" data-nav class="help-link">${t('退款政策','Refund policy')}</a><a href="/contact" data-nav class="help-link">${t('联系支持','Contact support')}</a></div></section></main>`;
}

function decodeBuffer(value) {const s=value.replace(/-/g,'+').replace(/_/g,'/');return Uint8Array.from(atob(s.padEnd(Math.ceil(s.length/4)*4,'=')),c=>c.charCodeAt(0));}
function encodeBuffer(buffer) {return btoa(String.fromCharCode(...new Uint8Array(buffer))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');}
function credentialJson(c) {if(typeof c.toJSON==='function')return c.toJSON();const r=c.response;return {id:c.id,rawId:encodeBuffer(c.rawId),type:c.type,authenticatorAttachment:c.authenticatorAttachment,clientExtensionResults:c.getClientExtensionResults(),response:{clientDataJSON:encodeBuffer(r.clientDataJSON),...(r.attestationObject?{attestationObject:encodeBuffer(r.attestationObject),transports:r.getTransports?.()||[]}:{authenticatorData:encodeBuffer(r.authenticatorData),signature:encodeBuffer(r.signature),userHandle:r.userHandle?encodeBuffer(r.userHandle):null})}};}
async function passkey(register=false,name='',scope='admin') {const member=scope==='member',returnPath=member?afterSignIn():(state.admin?location.pathname+location.search:'/admin');if(!window.PublicKeyCredential)throw new Error(t('此浏览器不支持通行密钥，请使用支持 WebAuthn 的浏览器。','This browser does not support passkeys. Use a browser with WebAuthn support.'));const prefix=(member?'/api/auth/passkeys':'/api/admin/passkeys')+'/'+(register?'register':'login');const start=await api(prefix+'/start',{method:'POST',body:register?{name}:{}});const publicKey=start.options.publicKey;publicKey.challenge=decodeBuffer(publicKey.challenge);if(publicKey.user)publicKey.user.id=decodeBuffer(publicKey.user.id);for(const field of ['allowCredentials','excludeCredentials'])if(publicKey[field])publicKey[field]=publicKey[field].map(c=>({...c,id:decodeBuffer(c.id)}));let credential;try{credential=await navigator.credentials[register?'create':'get']({publicKey});}catch(error){if(error.name==='NotAllowedError')throw new Error(t('验证已取消或超时。可以再次尝试。','Authentication was canceled or timed out. You can try again.'));throw error;}if(!credential)throw new Error(t('没有收到通行密钥。','No passkey response was received.'));await api(prefix+'/finish',{method:'POST',body:{credential:credentialJson(credential)}});await session();if(!resumeCliLogin(true))await go(register?(member?'/app?tab=profile':'/admin?tab=keys'):returnPath);}
async function payOrder(order,paymentRegion) {
  if(order.kind==='onboarding' && ['expired','failed','canceled','cancelled','refunded'].includes(effectiveOrderStatus(order))){
    await session();
    if(state.user?.employer_paid){await go('/app?tab=orders');return;}
    const result=await api('/api/orders'),recovery=activationRecovery(state.user,result.items||[]);
    if(recovery.kind==='pending')order=recovery.order;
    else if(['start','retry'].includes(recovery.kind)){await go('/app?tab=orders');employerActivation(true);return;}
    else {await go('/app?tab=orders');return;}
  }
  if(effectiveOrderStatus(order)!=='pending') {await go('/app?tab=orders');notify(order.status==='paid'?t('这笔订单已完成。','This order is complete.'):t('订单状态已更新，请查看最新记录。','The order status has changed. Check its current record.'));return;}
  try {order=await api(`/api/orders/${encodeURIComponent(order.id)}/checkout`,{method:'POST',body:paymentRegion&&order.provider==='waffo'?{payment_region:paymentRegion}:{}});}
  catch(error){
    if(order.kind==='onboarding'&&error.status===409){await go('/app?tab=orders');notify(error.message,true);return;}
    throw error;
  }
  if(!order.checkout_url)throw new Error(t('尚未获得支付链接。可在订单页继续重试。','A checkout link is not available yet. You can retry from Orders.'));
  const url=new URL(order.checkout_url);
  if(url.protocol!=='https:')throw new Error(t('支付链接不安全，已停止跳转。','The checkout URL is not secure. Navigation was stopped.'));
  location.assign(url.href);
}

function centsInput(value,record,options={}) {
  const meta=currency(record),bounds={min:1,max:1000000000,...options};
  try{return parseMinor(value,meta.currency_decimals,bounds);}catch{
    throw new Error(t(`请填写 ${meta.currency} 金额，最多 ${meta.currency_decimals} 位小数，范围为 ${amountValue(bounds.min,meta)} 至 ${amountValue(bounds.max,meta)}。`,`Enter a ${meta.currency} amount with at most ${meta.currency_decimals} decimal places, from ${amountValue(bounds.min,meta)} to ${amountValue(bounds.max,meta)}.`));
  }
}

document.addEventListener('submit',async event=>{
  const form=event.target.closest('[data-form]');if(!form)return;event.preventDefault();const submit=event.submitter||form.querySelector('[type=submit]'),data=new FormData(form),kind=form.dataset.form;setBusy(submit,true);
  try {
    if(kind.startsWith('hg-'))await humanGrant().submit(form,data);
    else if(kind==='email-preferences'){await api('/api/notifications/email',{method:'PUT',body:Object.fromEntries(['messages','matches','orders','platform'].map(key=>[key,data.has(key)]))});notify(t('通知设置已保存。','Notification settings saved.'));}
    else if(kind==='admin-email'){const result=await api('/api/admin/notifications/email',{method:'POST',body:{user_id:form.dataset.userId,subject:data.get('subject'),body:data.get('body'),request_key:form.dataset.requestKey}});closeDialog();notify(t('邮件已排队，可在通知记录中核对发送状态。','Email queued. Check the delivery records for its status.'));}
    else if(kind.startsWith('team-'))await team().submit(form,data);
    else if(kind.startsWith('staff-'))await staffUI().submit(form,data);
    else if(kind.startsWith('promo-'))await promos().submit(form,data);
    else if(kind==='public-directory-search'){const p=new URLSearchParams({tab:qs().get('tab')==='people'?'people':'jobs'});const q=String(data.get('q')||'').trim();if(q)p.set('q',q);go('/discover?'+p);}
    else if(kind==='email-start'){rememberBillingReturn();const email=String(data.get('email')).trim();const source=acquisitionSource(),referral=referralCode();const r=await api('/api/auth/email/start',{method:'POST',body:{email,...(source?{source}: {}),...(referral?{referral_code:referral}: {})}});emailCode(email,r.dev_code);}
    else if(kind==='email-verify'){await api('/api/auth/email/verify',{method:'POST',body:{email:data.get('email'),code:data.get('code')}});await session();if(!resumeCliLogin(true))go(afterSignIn());}
    else if(kind==='profile'){const links={};for(const input of document.querySelectorAll('[name^="ai_"]')){const value=String(input.value||'').trim();if(value)links[input.name.slice(3)]=value;}state.user=await api('/api/profile',{method:'PUT',body:{name:data.get('name'),handle:document.querySelector('[name="handle"]')?.value||state.user.handle,public_email:data.get('public_email'),resume_md:data.get('resume_md')||'',public_profile:!!document.querySelector('[name="public_profile"]')?.checked,ai_profile_links:links,visible:false}});notify(t('资料和个人主页设置已保存。','Profile and homepage settings saved.'));}
    else if(kind==='svg-avatar'){state.user=await api('/api/profile/avatar',{method:'PUT',body:{svg:data.get('svg')}});closeDialog();await render();notify(t('头像已更新。','Avatar updated.'));}
    else if(kind==='activate-employer'){const r=await api('/api/role',{method:'POST',body:{role:'employer',provider:data.get('provider')||'',currency:data.get('currency')}});state.user=r.user;closeDialog();if(r.order){await go('/app?tab=orders');await payOrder(r.order,data.get('payment_region'));}else go(consumeRoleReturn()||'/app');}
    else if(kind==='job'){const id=form.dataset.id,body=id?{jd_md:data.get('jd_md'),provider:data.get('provider')||''}:promos().prepareJob(form,newJobInput(data));const r=await api(id?`/api/jobs/${encodeURIComponent(id)}/edit`:'/api/jobs',{method:'POST',body});if(r.order&&r.order.status!=='paid'){await go('/app?tab=orders');await payOrder(r.order,data.get('payment_region'));}else{await go('/app?tab=jobs');notify(id?t('岗位描述已更新。','Role description updated.'):t('岗位已发布。','Role published.'));}}
    else if(kind==='job-headcount'){const count=headcountInput(data.get('headcount'));await saveJobHeadcount(form.dataset.id,count,form.dataset.admin==='true');if(form.dataset.admin==='true')closeDialog();await render();notify(count===0?t('人数已保存为 0，暂停新匹配；已有会话保留。','Target saved as 0. New introductions are paused; existing chats remain.'):t('招聘人数已免费更新。','Hiring target updated for free.'));}
    else if(kind==='token'||kind==='admin-token'){const r=await api(kind==='admin-token'?'/api/admin/tokens':'/api/tokens',{method:'POST',body:{name:data.get('name')}});await render();document.querySelector('#new-token').innerHTML=`<div class="token-secret"><strong class="field-label">${t('请现在保存，令牌只显示一次。','Save it now. This token is shown only once.')}</strong><code>${esc(r.token)}</code><button class="text-button" type="button" data-action="copy-token" data-copy="${esc(r.token)}">${icon('copy')}${t('复制令牌','Copy token')}</button><p class="help">${t('请勿分享或放入公开仓库。','Do not share it or put it in a public repository.')}</p></div>`;form.reset();notify(t('访问令牌已创建。','Access token created.'));}
    else if(kind==='chat')await service.sendMessage(form,data);
    else if(kind==='mandate')await service.saveMandate(form,data);
    else if(kind==='admin-mandate')await service.saveMandate(form,data,true);
    else if(kind==='outreach')await service.saveOutreach(form,data);
    else if(kind==='webhook')await service.saveWebhook(form,data);
    else if(kind==='mandate-filter'){const p=new URLSearchParams({tab:'mandates'});for(const[k,v]of data)if(v)p.set(k,v);go('/admin?'+p);}
    else if(kind==='admin-bootstrap'){await api('/api/admin/auth/bootstrap',{method:'POST',body:{password:data.get('password')}});await session();await render();}
    else if(kind==='passkey-register'){await passkey(true,String(data.get('name')));}
    else if(kind==='user-passkey-register'){await passkey(true,String(data.get('name')),'member');}
    else if(kind==='people-jobs-filter')await peopleJobs().submit(form,data);
    else if(kind==='admin-user'){const body={name:data.get('name'),public_email:data.get('public_email')};if(form.querySelector('[name=resume_md]')){body.resume_md=data.get('resume_md');body.visible=false;}await api(`/api/admin/users/${encodeURIComponent(form.dataset.id)}`,{method:'PATCH',body});closeDialog();await render();notify(t('用户资料已更新。','Profile updated.'));}
    else if(kind==='admin-user-access'){const reason=String(data.get('reason')||'').trim();if(!reason||new TextEncoder().encode(reason).length>500)throw new Error(t('请填写操作原因，最多 500 字节。','Enter a reason of at most 500 bytes.'));const banned=form.dataset.banned==='true';await api(`/api/admin/users/${encodeURIComponent(form.dataset.id)}/access`,{method:'POST',body:{banned,reason}});closeDialog();await render();notify(banned?t('账户已封禁，旧登录和 AI 令牌已撤销。','Account suspended. Old sign-ins and AI tokens are revoked.'):t('访问已恢复，请让用户重新登录。','Access restored. The person must sign in again.'));}
    else if(kind==='admin-job'){const body={jd_md:data.get('jd_md')};if(['closed','published'].includes(data.get('status')))body.status=data.get('status');await api(`/api/admin/jobs/${encodeURIComponent(form.dataset.id)}`,{method:'PATCH',body});closeDialog();await render();notify(t('岗位已更新。','Role updated.'));}
    else if(['workbench-jobs','workbench-candidates','workbench-followups'].includes(kind))await service.searchWorkbench(form,data);
    else if(kind==='workbench-introduce')await service.introduceCandidate(form,data);
    else if(kind==='workbench-bulk-introduce')await service.introduceCandidateBatch(form,data);
    else if(kind==='match')await service.saveFollowup(form,data);
  }catch(error){notify(error.message,true);}finally{setBusy(submit,false);}
});
let navigationIntentTimer;
function prefetchNavigation(link) {
  if(!link||state.admin||!sessionReader.fresh()||navigator.connection?.saveData)return;
  const url=new URL(link.getAttribute('href'),location.href);
  if(url.origin!==location.origin)return;
  for(const path of navigationReads(url,state.user))void api(path,{prefetch:true}).catch(()=>{});
}
document.addEventListener('pointerover',event=>{
  const link=event.target.closest('[data-nav]');if(!link||link.contains(event.relatedTarget))return;
  clearTimeout(navigationIntentTimer);navigationIntentTimer=setTimeout(()=>prefetchNavigation(link),70);
});
document.addEventListener('pointerout',event=>{const link=event.target.closest('[data-nav]');if(link&&!link.contains(event.relatedTarget))clearTimeout(navigationIntentTimer);});
document.addEventListener('focusin',event=>prefetchNavigation(event.target.closest('[data-nav]')));
document.addEventListener('pointerdown',event=>{if(event.button===0)prefetchNavigation(event.target.closest('[data-nav]'));});
document.addEventListener('click',async event=>{
  const brandLink=event.target.closest('[data-brand]');if(brandLink){logoClicks++;clearTimeout(logoTimeout);logoTimeout=setTimeout(()=>logoClicks=0,1800);if(logoClicks>=5){logoClicks=0;event.preventDefault();go('/admin');return;}if(location.pathname==='/'){event.preventDefault();return;}}
  const scroll=event.target.closest('[data-scroll]');if(scroll){event.preventDefault();if(location.pathname!=='/')go('/#'+scroll.dataset.scroll);else document.getElementById(scroll.dataset.scroll)?.scrollIntoView({behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});return;}
  if(event.target.closest('[data-google-sign-in]')&&state.config?.google_enabled){rememberBillingReturn();team().returnAfterLogin();}
  const nav=event.target.closest('[data-nav]');if(nav&&!event.metaKey&&!event.ctrlKey&&!event.shiftKey&&event.button===0){event.preventDefault();go(nav.getAttribute('href'));return;}
  const button=event.target.closest('[data-action]');if(!button||button.disabled)return;event.preventDefault();const action=button.dataset.action,id=button.dataset.id;
  try{
    if(action.startsWith('hg-'))await humanGrant().action(action,button);
    else if(action.startsWith('team-')){setBusy(button,true);await team().action(action,button);}
    else if(action.startsWith('staff-'))await staffUI().action(action,button);
    else if(action.startsWith('promo-')){setBusy(button,true);await promos().action(action,button);}
    else if(action==='language'){state.lang=state.lang==='en'?'zh':'en';localStorage.setItem('hireferry-language',state.lang);closeDialog();await render();}
    else if(action==='close-dialog')closeDialog();
    else if(action==='pause')state.sculpture?.pause();
    else if(action==='retry'||action==='billing-refresh'){setBusy(button,true);if(action==='billing-refresh')await session();await render();}
    else if(action==='google-unavailable')notify(t('Google 登录暂未开放，请使用邮箱。','Google sign-in is not available yet. Please use email.'),true);
    else if(action==='restart-login')await render();
    else if(action==='logout'){disclosureUI.reset();await api('/api/auth/logout',{method:'POST',body:{}});Object.assign(state,{user:null,admin:false,owner:false,super_admin:false,staffRole:null,staff:null,bootstrap:false,csrf:null});go('/');}
    else if(action==='role-seeker'){setBusy(button,true);const r=await api('/api/role',{method:'POST',body:{role:'seeker'}});state.user=r.user;clearRoleIntent();closeDialog();go(consumeRoleReturn()||'/app');}
    else if(action==='role-employer'){
      setBusy(button,true);
      if(state.user?.role==='unassigned'){const r=await api('/api/role',{method:'POST',body:{role:'employer'}});state.user=r.user;clearRoleIntent();closeDialog();await render();}
      else await session();
      clearRoleIntent();
      if(state.user?.employer_paid)await go(consumeRoleReturn()||'/app?tab=jobs');
      else {const orders=await api('/api/orders');const recovery=activationRecovery(state.user,orders.items||[]);
        if(recovery.kind==='pending')await payOrder(recovery.order);
        else if(recovery.kind==='review'){await go('/app?tab=orders');notify(t('已有开通款项，请先联系平台核对。','An activation payment is recorded. Contact support for a review.'),true);}
        else if(['start','retry'].includes(recovery.kind))employerActivation(recovery.kind==='retry');
      }
    }
    else if(action==='svg-avatar')openDialog(`<div class="eyebrow">YOUR AVATAR</div><h2 id="dialog-title">${t('用一段 SVG 表达自己。','An avatar, in SVG.')}</h2><p class="dialog-intro">${t('支持静态 SVG。脚本、外部资源和动画会被拒绝。','Static SVG only. Scripts, external resources, and animation are rejected.')}</p><form data-form="svg-avatar"><div class="field"><label for="avatar-svg">${t('SVG 文本','SVG source')}</label><textarea class="input code" name="svg" id="avatar-svg" rows="8" required maxlength="50000" placeholder="&lt;svg xmlns=&quot;http://www.w3.org/2000/svg&quot; viewBox=&quot;0 0 64 64&quot;&gt;…&lt;/svg&gt;"></textarea></div><div class="form-actions"><button class="button" type="submit">${t('保存头像','Save avatar')}${icon('check')}</button></div></form>`);
    else if(action==='resume-preview'){const r=await api('/api/markdown',{method:'POST',body:{text:document.querySelector('#resume-md').value}});document.querySelector('#resume-preview').innerHTML=r.html;document.querySelector('#resume-preview').hidden=false;document.querySelector('#resume-md').hidden=true;document.querySelectorAll('.tabs [role=tab]').forEach(b=>b.setAttribute('aria-selected',String(b===button)));}
    else if(action==='resume-edit'){document.querySelector('#resume-md').hidden=false;document.querySelector('#resume-preview').hidden=true;document.querySelectorAll('.tabs [role=tab]').forEach(b=>b.setAttribute('aria-selected',String(b===button)));}
    else if(action==='delete-job')await jobLifecycleDialog(id);
    else if(action==='restore-job')await jobLifecycleDialog(id,'restore');
    else if(action==='permanently-delete-job')await jobLifecycleDialog(id,'permanent');
    else if(action==='admin-restore-job')await jobLifecycleDialog(id,'restore',true);
    else if(action==='close-job')confirmDialog(t('结束这个岗位的招聘？','Close this role?'),t('关闭后不能恢复招聘。仅想暂停，请将招聘人数设为 0。原有订单记录保留。','Closing cannot be undone. To pause temporarily, set the hiring target to 0. Order records remain.'),async()=>{await api(`/api/jobs/${encodeURIComponent(id)}/close`,{method:'POST',body:{}});await render();notify(t('岗位已关闭。','Role closed.'));});
    else if(action==='pay-order'){setBusy(button,true);await payOrder(await api(`/api/orders/${encodeURIComponent(id)}`));}
    else if(action==='copy-command'){await copyCliCommand(button);notify(t('命令已复制。','Command copied.'));}
    else if(action==='copy-public-mcp'){await copyText(button.dataset.copy);notify(t('公开 MCP 地址已复制。','Public MCP endpoint copied.'));}
    else if(action==='copy-public-mcp-example'){await copyText(button.dataset.copy);notify(t('请求示例已复制。','Request example copied.'));}
    else if(action==='copy-public-link'){await copyText(button.dataset.copy);notify(t('公开链接已复制。','Public link copied.'));}
    else if(action==='copy-token'){await copyText(button.dataset.copy);notify(t('已复制。请妥善保管。','Copied. Keep it private.'));}
    else if(action==='copy-profile-link'){await copyText(button.dataset.copy);notify(t('主页链接已复制，可以分享给别人。','Homepage link copied. You can share it now.'));}
    else if(action==='copy-referral-link'){await copyText(button.dataset.copy);notify(t('邀请链接已复制，可以发给朋友。','Invite link copied.'));}
    else if(action==='share-referral-link'){
      const url=button.dataset.copy;
      if(typeof navigator.share==='function'){
        try{await navigator.share({title:t('加入聘渡','Join HireFerry'),text:t('免费注册，让 AI 帮你找工作或匹配人才。','Sign up free and let AI help you find work or people.'),url});notify(t('分享面板已打开。','Share sheet opened.'));}
        catch(error){if(error?.name!=='AbortError'){await copyText(url);notify(t('已复制邀请链接，可以发给朋友。','Invite link copied. You can send it to a friend.'));}}
      }else{await copyText(url);notify(t('设备不支持直接分享，已复制邀请链接。','Direct sharing is unavailable, so the invite link was copied.'));}
    }
    else if(action==='revoke-token')confirmDialog(t('撤销这个 AI 连接？','Revoke this AI connection?'),t('使用这个令牌的 AI 将立即失去访问权限。','The AI using this token will immediately lose access.'),async()=>{await api(`/api/tokens/${encodeURIComponent(id)}`,{method:'DELETE'});await render();notify(t('连接已撤销。','Connection revoked.'));});
    else if(action==='user-passkey-login'){setBusy(button,true);await passkey(false,'','member');}
    else if(action==='admin-passkey-login'||action==='admin-reauth'){setBusy(button,true);await passkey();}
    else if(action==='delete-passkey')confirmDialog(t('移除这把通行密钥？','Remove this passkey?'),t('移除后，这把密钥无法再登录总控台。必须保留至少一把。','This passkey will no longer sign in. At least one passkey must remain.'),async()=>{await api(`/api/admin/passkeys/${encodeURIComponent(id)}`,{method:'DELETE'});await render();notify(t('通行密钥已移除。','Passkey removed.'));},{danger:true});
    else if(action==='delete-user-passkey')confirmDialog(t('删除这把 Passkey？','Remove this passkey?'),t('删除后它不能再登录这个账户，但你仍可使用邮箱或 Google 登录。','It will no longer sign in to this account. Email or Google sign-in remains available.'),async()=>{await api(`/api/auth/passkeys/${encodeURIComponent(id)}`,{method:'DELETE'});await render();notify(t('Passkey 已删除。','Passkey removed.'));},{danger:true});
    else if(action==='admin-email-user'){const [user,history]=await Promise.all([api(`/api/admin/users/${encodeURIComponent(id)}`),api(`/api/admin/notifications/email?user_id=${encodeURIComponent(id)}`)]);openDialog(`<h2 id="dialog-title">${t('发送服务通知','Send service notice')}</h2><p class="dialog-intro">${esc(user.name||id)} · ${t('发送至该用户登录邮箱，遵守其通知设置。','Sent to this user’s sign-in email, respecting their preferences.')}</p><form data-form="admin-email" data-user-id="${esc(id)}" data-request-key="${createMessageId()}"><div class="field"><label for="mail-subject">${t('主题','Subject')}</label><input id="mail-subject" name="subject" class="input" maxlength="160" required></div><div class="field"><label for="mail-body">${t('内容','Message')}</label><textarea id="mail-body" name="body" class="input" rows="5" maxlength="5000" required></textarea></div><div class="form-actions"><button type="submit" class="button">${t('确认并发送','Confirm & send')}</button></div></form><details class="section-disclosure"><summary>${t('最近发送记录','Recent delivery records')}</summary><div class="disclosure-content">${history.items.length?history.items.slice(0,10).map(item=>`<p>${esc(item.subject)} · ${esc(({pending:t('排队中','Queued'),processing:t('发送中','Sending'),accepted:t('SMTP 已接受','SMTP accepted'),failed:t('发送失败','Failed'),cancelled:t('已取消','Cancelled')})[item.status]||item.status)} · ${date(item.created_at)}</p>`).join(''):`<p>${t('暂无记录','No records yet')}</p>`}</div></details>`);}
    else if(action==='admin-edit-user')await editUser(id);
    else if(action==='admin-user-access')await userAccessDialog(id);
    else if(action==='delete-resume')confirmDialog(t('删除这份简历？','Delete this résumé?'),t('简历内容将被清空，并停止共享。此操作不能撤销。','The résumé will be erased and no longer shared. This cannot be undone.'),async()=>{await api(`/api/admin/users/${encodeURIComponent(id)}/resume`,{method:'DELETE'});await render();notify(t('简历已删除。','Résumé deleted.'));},{danger:true});
    else if(action==='admin-edit-job')await editAdminJob(id);
    else if(action==='admin-delete-job')await jobLifecycleDialog(id,'permanent',true);
    else if(action==='new-match')await newMatch();
    else if(action==='workbench-page'){setBusy(button,true);await service.workbenchPage(button.dataset.list,button.dataset.direction);}
    else if(action==='workbench-bulk-review')service.reviewCandidateBatch();
    else if(action==='update-match'||action==='match-detail')await editMatch(id);
    else if(action==='chat-older'){setBusy(button,true);await service.olderMessages();}
    else if(action==='chat-latest')service.scrollLatest();
    else if(action==='admin-mandate')await service.adminMandate(id);
    else if(action==='new-outreach')service.outreachDialog(button.dataset.user);
    else if(action==='edit-outreach')service.outreachDialog(button.dataset.user,id);
    else if(action==='webhook-new')service.webhookDialog();
    else if(action==='webhook-edit')service.webhookDialog(id);
    else if(action==='webhook-copy-secret')await service.copyWebhookSecret();
    else if(action==='webhook-deliveries'){setBusy(button,true);await service.webhookDeliveries(id);}
    else if(action==='webhook-toggle'){setBusy(button,true);await service.toggleWebhook(id);}
    else if(action==='webhook-test'){setBusy(button,true);await service.testWebhook(id);}
    else if(action==='webhook-retry'){setBusy(button,true);await service.retryWebhook(id,button.dataset.delivery);}
    else if(action==='webhook-delete')confirmDialog(t('删除这个联动端点？','Delete this webhook endpoint?'),t('签名密钥将撤销，未完成的投递停止。需要恢复时请重新注册。','Its signing secret will be revoked and pending deliveries stopped. Register a new endpoint if you need to connect again.'),async()=>{await service.deleteWebhook(id);},{danger:true});
    else if(action==='revoke-admin-token')confirmDialog(t('撤销这个平台 AI 授权？','Revoke this platform AI authorization?'),t('它将立即失去平台业务访问权限。','It will immediately lose access to platform operations.'),async()=>{await api(`/api/admin/tokens/${encodeURIComponent(id)}`,{method:'DELETE'});await render();notify(t('平台授权已撤销。','Platform authorization revoked.'));},{danger:true});
  }catch(error){notify(error.message,true);}finally{setBusy(button,false);}
});
document.addEventListener('input',event=>{
  if(event.target.id==='salary-example'){try{document.querySelector('#salary-example-result').textContent=money(calculateFee(centsInput(event.target.value),feeSettings('publish')));}catch{document.querySelector('#salary-example-result').textContent='—';}}
  if(['job-salary','job-promo-code'].includes(event.target.id)){promos().changed(event.target);refreshJobPrice(event.target.closest('form'));}
  if(event.target.closest('[data-form="promo-activation"]'))promos().changed(event.target);
});
document.addEventListener('change',event=>{
  const input=event.target;if(input.disabled)return;
  try{
    if(input.closest('[data-form^="promo-"]')){promos().changed(input);if(input.name==='decision'){const settings=input.closest('form').querySelector('[data-promo-approval-settings]');settings.hidden=input.value==='rejected';for(const control of settings.querySelectorAll('input,select,textarea'))control.disabled=settings.hidden;if(!settings.hidden)settings.querySelector('[name="free_activation_limit"]').disabled=!settings.querySelector('[name="include_activation"]').checked;}return;}
    if(input.closest('[data-form="activate-employer"]')&&['currency','payment_region'].includes(input.name)){refreshActivationPayment(input.closest('form'));return;}
    if(input.id==='job-currency'){promos().changed(input);refreshJobPrice(input.closest('form'));return;}
    if(input.name==='salary_currency'){service.updateSalaryCurrency(input);return;}
    if(input.dataset.workbenchCandidate!==undefined)service.selectWorkbenchCandidate(input.dataset.workbenchCandidate,input.checked);
    else if(input.matches('[data-workbench-select-all]'))service.selectWorkbenchPage(input.checked);
  }catch(error){input.checked=false;notify(error.message,true);}
});
document.addEventListener('change',async event=>{if(event.target.id!=='avatar-upload')return;const file=event.target.files[0];if(!file)return;if(file.size>2*1024*1024){notify(t('图片不能超过 2 MB。','The image must be 2 MB or smaller.'),true);return;}const data=new FormData();data.append('file',file);try{state.user=await api('/api/profile/avatar',{method:'POST',body:data});document.querySelector('.avatar-editor .avatar').outerHTML=avatar(state.user,'large');notify(t('头像已更新。','Avatar updated.'));}catch(error){notify(error.message,true);}event.target.value='';});
window.addEventListener('popstate',()=>{closeDialog();render();});
window.addEventListener('hashchange',()=>{if(location.pathname==='/team/invite'){closeDialog();void render();}});
window.addEventListener('pagehide',event=>{invalidateWebMCP();resetAdminViews();service.stopChat();if(dialogs.querySelector('#webhook-signing-secret, #team-invitation-secret, #team-ai-secret'))closeDialog();if(!event.persisted){state.homeStory?.dispose();state.particleTitles?.dispose();state.sculpture?.dispose();state.brandMotion?.dispose();}});
function resetPrivateView() {
  disclosureUI.reset();
  invalidateIdentity();sessionKnown=true;Object.assign(state,{user:null,admin:false,owner:false,super_admin:false,staffRole:null,staff:null,bootstrap:false,csrf:null});service.stopChat();closeDialog();root.dataset.pageKey='';
}
async function refreshVisibleSession() {
  if(document.hidden||!state.config)return;
  const before=identityOf(state);
  try{await session();if(identityOf(state)!==before){service.stopChat();closeDialog();root.dataset.pageKey='';await render();}}
  catch(error){sessionReader.invalidate();requests.invalidate();if(error.status===401&&(state.user||state.admin)){resetPrivateView();void render();}}
}
window.addEventListener('storage',event=>{if(event.key==='hireferry-session-change'){resetPrivateView();void render();}});
document.addEventListener('visibilitychange',()=>{if(document.hidden){clearTimeout(navigationIntentTimer);requests.invalidate();}else if(publicPages.has(location.pathname))void refreshPublicPage(state.render);else if(isPublicPagePath(location.pathname))void render();else void refreshVisibleSession();});
window.addEventListener('online',()=>{if(publicPages.has(location.pathname))void refreshPublicPage(state.render);else if(isPublicPagePath(location.pathname))void render();});
window.addEventListener('pageshow',event=>{if(event.persisted){resetPrivateView();void render();}});

const service=createServiceUI({state,api,t,esc,icon,avatar,money,currency,currencies,currencySelect,amountValue,amountStep,date,heading,shell,empty,notice,badge,matchText,openDialog,closeDialog,notify,go,render,centsInput});

const webMCPOwner=createWebMCPOwner();
const publicWebMCPOwner=createWebMCPOwner();
let webMCPEligibilityRead=null,webMCPEpoch=0,relationWebMCPOwner;
const webMCPIdentity=(value=state)=>JSON.stringify([identityOf(value),!!value.admin,!!value.owner,value.staffRole||value.staff_role||null,value.user?.role||null,!!value.user?.banned]);
function clearWebMCPCatalog() {webMCPEpoch++;void webMCPOwner.clear();void publicWebMCPOwner.clear();void relationWebMCPOwner?.clear();}
function invalidateWebMCP() {webMCPEligibilityRead=null;clearWebMCPCatalog();}
function registerWebMCP({refreshEligibility=false}={}) {
  const context=document.modelContext||navigator.modelContext;
  const publicPage=location.pathname==='/discover'||/^\/discover\/(?:jobs|people)\/[^/]+$/.test(location.pathname);
  if(!context?.registerTool){invalidateWebMCP();return;}
  if(publicPage){const epoch=webMCPEpoch,scope=`public:${location.pathname}`;void publicWebMCPOwner.sync({scope,tools:publicWebMCPTools(()=>webMCPEpoch===epoch&&(location.pathname==='/discover'||location.pathname.startsWith('/discover/'))),isCurrent:()=>webMCPEpoch===epoch&&(location.pathname==='/discover'||location.pathname.startsWith('/discover/'))});}else void publicWebMCPOwner.clear();
  // Human managers do not inherit the owner's browser-AI tool catalog.
  if(state.staffRole&&!state.owner&&!state.staff||(!state.admin&&!state.user)){void webMCPOwner.clear();return;}
  const scope=webMCPIdentity();
  if(!state.admin&&(!state.eligibility||refreshEligibility)){
    if(webMCPEligibilityRead?.scope!==scope){
      const request={scope};webMCPEligibilityRead=request;
      void api('/api/ai/eligibility',{fresh:true}).then(eligibility=>{
        if(webMCPEligibilityRead!==request||webMCPIdentity()!==scope)return;
        state.eligibility=eligibility;registerWebMCP();
      }).catch(()=>{if(webMCPEligibilityRead===request&&webMCPIdentity()===scope)invalidateWebMCP();})
        .finally(()=>{if(webMCPEligibilityRead===request)webMCPEligibilityRead=null;});
    }
  }
  if(!state.admin&&!state.eligibility?.eligible){if(state.eligibility)clearWebMCPCatalog();else void webMCPOwner.clear();return;}
  const epoch=webMCPEpoch;
  const isCurrent=()=>webMCPEpoch===epoch&&webMCPIdentity()===scope&&(state.admin||state.eligibility?.eligible!==false);
  void webMCPOwner.sync({scope,tools:webMCPTools(()=>isCurrent()&&webMCPOwner.isCurrent(scope)),isCurrent});
  if(state.admin&&relationPageActive()&&root.querySelector('.people-jobs-workspace')){
    relationWebMCPOwner ||= createRelationTools({api:(path,options={})=>api(path,{...options,headers:{...options.headers,'X-HireFerry-AI':'webmcp'}})});
    void relationWebMCPOwner.sync({scope,isCurrent:()=>isCurrent()&&state.admin&&relationPageActive()});
  }else void relationWebMCPOwner?.clear();
}
function relationPageActive() {
  if(location.pathname!=='/admin')return false;
  const query=qs(),tab=query.get('tab');
  return ['users','jobs'].includes(tab)||(tab==='matches'&&!['search','followups'].includes(query.get('view'))&&!query.has('job')&&!query.has('candidate'));
}
function publicWebMCPTools(isCurrent) {
  const object=(properties={},required=[])=>({type:'object',properties,required,additionalProperties:false});
  const str={type:'string'};
  const publicApi=(path)=>{if(!isCurrent())throw new Error('Public directory changed. Reopen it.');return api(path,{fresh:true});};
  const tool=(name,description,inputSchema,execute)=>({name,description,inputSchema,annotations:{readOnlyHint:true,destructiveHint:false,consequentialHint:false},execute:async input=>{try{const value=await execute(input||{});return{content:[{type:'text',text:JSON.stringify(value)}]};}catch(error){return{content:[{type:'text',text:error.message}],isError:true};}}});
  const query=(path,input={})=>{const p=new URLSearchParams();for(const[k,v]of Object.entries(input))if(v!==undefined&&v!==null&&v!=='')p.set(k,String(v));return p.size?`${path}?${p}`:path;};
  return [
    tool('hireferry_public_get_site_config',t('读取公开站点设置和免费公开 MCP 入口。','Read public site settings and the free public MCP endpoint.'),object(),()=>publicApi('/api/config')),
    tool('hireferry_public_search_jobs',t('搜索公开、正在招聘的岗位。','Search published public roles that are still hiring.'),object({q:str,page:{type:'integer',minimum:1}}),x=>publicApi(query('/api/public/jobs',x))),
    tool('hireferry_public_get_job',t('读取一个公开岗位。','Read one public role.'),object({id:str},['id']),x=>publicApi(`/api/public/jobs/${encodeURIComponent(x.id)}`)),
    tool('hireferry_public_search_people',t('搜索明确公开的求职者简历。','Search seeker résumés explicitly marked public.'),object({q:str,page:{type:'integer',minimum:1}}),x=>publicApi(query('/api/public/people',x))),
    tool('hireferry_public_get_person',t('读取一个公开简历。','Read one public résumé.'),object({handle:{type:'string',pattern:'^[a-z0-9_-]{3,32}$'}},['handle']),x=>publicApi(`/api/public/people/${encodeURIComponent(x.handle)}`)),
  ];
}
function webMCPTools(isCurrent) {
  const tools=[];
  const requireCurrent=()=>{if(!isCurrent())throw new Error('Session changed. Reopen the current workspace.');};
  const object=(properties={},required=[])=>({type:'object',properties,required,additionalProperties:false});
  const str={type:'string'},integer={type:'integer'},bool={type:'boolean'};
  const idSchema=object({id:str},['id']);
  const versionedIdSchema=object({id:str,expected_version:{type:'integer',minimum:0}},['id','expected_version']);
  const headcountSchema={type:'integer',minimum:0,maximum:state.config?.limits?.max_job_headcount??1000000};
  const currencySchema={type:'string',enum:currencies().map(option=>option.currency)};
  const aiApi=(path,options={})=>{requireCurrent();return api(path,{...options,headers:{...options.headers,'X-HireFerry-AI':'webmcp'}});};
  const queryPath=(path,values={})=>{const p=new URLSearchParams();for(const[k,v]of Object.entries(values))if(v!==undefined&&v!==null&&v!=='')p.set(k,String(v));return path+(p.size?'?'+p:'');};
  const tool=(name,description,inputSchema,execute,readOnly=false,destructive=false)=>{
    tools.push({
      name,description,inputSchema,annotations:{readOnlyHint:readOnly,destructiveHint:destructive,consequentialHint:destructive},
      execute:async input=>{
        try{
          requireCurrent();
          const eligibility=await api('/api/ai/eligibility',{fresh:true});
          requireCurrent();
          if(!eligibility.eligible){state.eligibility=eligibility;invalidateWebMCP();throw new Error(t('当前暂不具备 AI 访问资格。','AI access is not currently available.'));}
          const result=await execute(input||{});
          requireCurrent();
          return{content:[{type:'text',text:JSON.stringify(result)}]};
        }catch(error){return{content:[{type:'text',text:error.message}],isError:true};}
      },
    });
  };
  tool('hireferry_get_site_config','Read current site pricing, allowed currencies and their minor-unit precision, AI salary thresholds, and operational limits. Existing roles retain their original published terms.',object(),()=>api('/api/config'),true);
  const chatToolPrefix=state.admin?'hireferry_admin_':'hireferry_';
  tool(chatToolPrefix+'list_conversations','List accessible private platform conversations and separate introduction rooms. Private platform history is never copied into a match room.',object(),()=>aiApi('/api/chat/conversations'),true);
  tool(chatToolPrefix+'read_messages','Read a permitted conversation. Omit after for the latest 100 messages; use after for new messages or before for older history. Administrators can read archived rooms, but writable=false rooms cannot receive messages.',object({id:str,after:integer,before:integer,limit:{type:'integer',minimum:1,maximum:100}},['id']),x=>aiApi(queryPath(`/api/chat/conversations/${encodeURIComponent(x.id)}/messages`,{after:x.after,before:x.before,limit:x.limit})),true);
  tool(chatToolPrefix+'send_message','Send a message in the chosen permitted conversation. Only send when authorized by the human. Messages are marked as AI-authored. A platform private conversation and an introduction room have different participants; verify the intended recipient first.',object({conversation_id:str,body:{type:'string',minLength:1,maxLength:10000},client_id:str,format:{enum:['plain','markdown','openui']}},['conversation_id','body']),x=>aiApi(`/api/chat/conversations/${encodeURIComponent(x.conversation_id)}/messages`,{method:'POST',body:{body:x.body,client_id:x.client_id||createMessageId(),...(x.format?{format:x.format}:{})}}));
  tool(chatToolPrefix+'get_chat_updates','Read new messages across permitted conversations with an incremental numeric cursor. Poll no faster than every 5 seconds.',object({after:integer,limit:{type:'integer',minimum:1,maximum:100}}),x=>aiApi(queryPath('/api/chat/updates',x)),true);
  tool('hireferry_get_chat_capabilities','Read chat formats, streaming transports, welcome copy, limits and the OpenUI library schema URL.',object(),()=>api('/api/chat/capabilities'),true);
  tool(chatToolPrefix+'get_conversation','Read current participants and write access for one permitted conversation.',object({conversation_id:str},['conversation_id']),x=>aiApi(`/api/chat/conversations/${encodeURIComponent(x.conversation_id)}`),true);
  tool(chatToolPrefix+'get_chat_events','Read compact message events after a saved event cursor. These IDs differ from message IDs. On resync_required reload messages and use event_cursor. Prefer the SSE endpoint from get_chat_capabilities for subscriptions.',object({conversation_id:str,after:{type:'integer',minimum:0},limit:{type:'integer',minimum:1,maximum:100}},['after']),x=>aiApi(queryPath('/api/chat/events',x)),true);
  tool(chatToolPrefix+'begin_message','Begin one AI-authored message with its first nonempty text chunk. Reuse client_id and the same first chunk on retries. Requires user authorization to send in this room.',object({conversation_id:str,client_id:{type:'string',minLength:1,maxLength:80},body:{type:'string',minLength:1,maxLength:10000},format:{enum:['plain','markdown','openui']}},['conversation_id','client_id','body']),x=>{const{conversation_id,...body}=x;return aiApi(`/api/chat/conversations/${encodeURIComponent(conversation_id)}/streams`,{method:'POST',body});});
  tool(chatToolPrefix+'append_message','Append the next chunk of your active message. Sequence starts at 1; repeat the same sequence and delta for safe retries. Returns a compact acknowledgement.',object({conversation_id:str,message_id:{type:'integer',minimum:1},sequence:{type:'integer',minimum:1,maximum:4096},delta:{type:'string',minLength:1,maxLength:4000}},['conversation_id','message_id','sequence','delta']),x=>aiApi(`/api/chat/conversations/${encodeURIComponent(x.conversation_id)}/streams/${x.message_id}/append`,{method:'POST',body:{sequence:x.sequence,delta:x.delta}}));
  for(const action of ['finish','cancel'])tool(chatToolPrefix+action+'_message',`${action==='finish'?'Complete':'Stop'} your active message using its next sequence. Stopping retains the partial text and labels it as stopped.`,object({conversation_id:str,message_id:{type:'integer',minimum:1},sequence:{type:'integer',minimum:1,maximum:4097}},['conversation_id','message_id','sequence']),x=>aiApi(`/api/chat/conversations/${encodeURIComponent(x.conversation_id)}/streams/${x.message_id}/${action}`,{method:'POST',body:{sequence:x.sequence}}));
  tool(chatToolPrefix+'mark_read','Mark messages in a permitted conversation as read.',object({conversation_id:str,message_id:integer},['conversation_id','message_id']),x=>aiApi(`/api/chat/conversations/${encodeURIComponent(x.conversation_id)}/read`,{method:'POST',body:{message_id:x.message_id}}));
  if(state.admin){
    tool('hireferry_admin_send_email_notification','Queue one authorized service email to a platform user. Uses the platform SMTP server and stored login email, and respects notification preferences. No arbitrary email addresses. Reuse request_key for identical retries. Queue acceptance is not inbox delivery.',object({user_id:str,subject:{type:'string',minLength:1,maxLength:160},body:{type:'string',minLength:1,maxLength:5000},request_key:{type:'string',minLength:8,maxLength:128,pattern:'^[A-Za-z0-9_-]+$'}},['user_id','subject','body','request_key']),x=>aiApi('/api/admin/notifications/email',{method:'POST',body:x}));
    tool('hireferry_admin_list_email_notifications','Read recent email delivery records. accepted means SMTP accepted, not inbox delivery or reading.',object({user_id:str}),x=>aiApi(queryPath('/api/admin/notifications/email',x)),true);
    const listSchema=object({q:str,role:{type:'string',enum:['seeker','employer','unassigned']},page:{type:'integer',minimum:1}});
    tool('hireferry_admin_list_users','List platform users for service operations. Never disclose private data to an unrelated party.',listSchema,x=>aiApi(queryPath('/api/admin/users',x)),true);
    tool('hireferry_admin_get_user','Read a user profile as the platform administrator.',idSchema,x=>aiApi(`/api/admin/users/${encodeURIComponent(x.id)}`),true);
    tool('hireferry_admin_update_user','Update profile fields or suspend a user. Profiles are not public; visibility follows active introductions. Use only for authorized platform operations.',object({id:str,name:str,public_email:str,resume_md:str,banned:bool},['id']),x=>{const{id,...body}=x;return aiApi(`/api/admin/users/${encodeURIComponent(id)}`,{method:'PATCH',body});});
    tool('hireferry_admin_delete_resume','Erase a user résumé. This is destructive and requires explicit human authorization.',idSchema,x=>aiApi(`/api/admin/users/${encodeURIComponent(x.id)}/resume`,{method:'DELETE'}));
    tool('hireferry_admin_list_jobs','List private job records for platform matching, with optional keyword and status filters.',object({q:str,status:str,page:{type:'integer',minimum:1}}),x=>aiApi(queryPath('/api/admin/jobs',x)),true);
    tool('hireferry_admin_get_job','Read one private job record as the administrator.',idSchema,x=>aiApi(`/api/admin/jobs/${encodeURIComponent(x.id)}`),true);
    tool('hireferry_admin_update_job','Manage a job description, close it permanently, or change its hiring target for free. Headcount-only updates create no order and consume no description update. Zero pauses new introductions and keeps existing rooms. Closed or deleted jobs cannot resume hiring through this tool. Title and original base salary remain immutable.',object({id:str,jd_md:str,status:{type:'string',enum:['published','closed']},headcount:headcountSchema},['id']),x=>{const{id,...body}=x;if(body.headcount!==undefined)body.headcount=headcountInput(body.headcount);return aiApi(`/api/admin/jobs/${encodeURIComponent(id)}`,{method:'PATCH',body});});
    tool('hireferry_admin_delete_job','Permanently delete a job as administrator, erasing its title and description. This cannot be restored. Requires explicit human authorization and the current job version. Free, with no refund or description update used; necessary transaction and security records remain.',versionedIdSchema,x=>aiApi(`/api/admin/jobs/${encodeURIComponent(x.id)}`,{method:'DELETE',body:{expected_version:x.expected_version}}),false,true);
    tool('hireferry_admin_restore_job','Restore a role from its first deletion once, free of charge, after human authorization. Use its current version. Existing introductions regain access subject to current access rules. Restoring without valid payment rights returns a closed role, not renewed hiring. A later deletion is permanent.',versionedIdSchema,x=>aiApi(`/api/admin/jobs/${encodeURIComponent(x.id)}/restore`,{method:'POST',body:{expected_version:x.expected_version}}));
    const workbenchPageSchema={q:{type:'string',maxLength:120},cursor:str,limit:{type:'integer',minimum:1,maximum:100}};
    tool('hireferry_admin_search_jobs','Search the server-paged queue of paid, published roles with a positive hiring target. Pass next_cursor to fetch the next page. No candidate profiles are loaded by this search.',object({...workbenchPageSchema,employer_id:str}),x=>aiApi(queryPath('/api/admin/workbench/jobs',x)),true);
    tool('hireferry_admin_search_candidates','Search active seeker briefs for one selected hiring role, excluding its current active introductions. Salary amounts are integer minor units of salary_currency, defaulting to the selected job’s currency. Only equal currencies are compared, without conversion. Explicit currency alone filters that currency; job_id alone does not filter salary. A declined history_match must be reopened through update_introduction, not duplicated.',object({...workbenchPageSchema,job_id:str,industry:{type:'string',maxLength:80},roles:{type:'string',maxLength:80},location:{type:'string',maxLength:80},salary_currency:{type:'string',pattern:'^[A-Z]{3}$'},salary_min_cents:{type:'integer',minimum:0,maximum:100000000},salary_max_cents:{type:'integer',minimum:0,maximum:100000000}},['job_id']),x=>aiApi(queryPath('/api/admin/workbench/candidates',x)),true);
    tool('hireferry_admin_search_matches','Search introductions by stage, role, participant, or keyword. Results are server-paged by latest update. Restart from the first page after changes; stage_counts ignore the status filter and preserve other filters.',object({...workbenchPageSchema,status:{type:'string',enum:['introduced','interview','hired','declined']},job_id:str,seeker_id:str,employer_id:str}),x=>aiApi(queryPath('/api/admin/workbench/matches',x)),true);
    tool('hireferry_admin_list_introductions',`Read the latest ${state.config?.limits?.recent_list_limit||200} introductions, including concluded records. Use admin_search_matches for server-paged follow-up. The note is visible to both matched parties.`,object(),()=>aiApi('/api/admin/matches'),true);
    tool('hireferry_admin_create_introduction','Introduce a seeker with an active mandate and a paid published job with a hiring target above zero. This opens their profiles to each other and creates a separate shared room. The note is an introduction for BOTH parties, never an internal note.',object({seeker_id:str,job_id:str,note:str},['seeker_id','job_id','note']),x=>aiApi('/api/admin/matches',{method:'POST',body:x}));
    tool('hireferry_admin_introduce_candidates','Introduce 1–100 reviewed candidates to one hiring role in one atomic request. Opens profiles and a separate shared room for each pair. Notes are visible to each pair. Repeated requests reuse active introductions without changing their notes or progress. Closed introductions must be reopened individually. Any ineligible candidate or role rejects the whole batch.',object({job_id:{type:'string',minLength:1,maxLength:80},seeker_ids:{type:'array',items:{type:'string',minLength:1,maxLength:80},minItems:1,maxItems:100,uniqueItems:true},note:{type:'string',maxLength:2000}},['job_id','seeker_ids']),x=>aiApi('/api/admin/workbench/introductions',{method:'POST',body:{job_id:x.job_id,seeker_ids:x.seeker_ids,note:x.note||''}}));
    tool('hireferry_admin_update_introduction','Update an introduction stage and its note visible to both parties. Declining closes ordinary participant access; administrators retain read-only history. Reopening a declined introduction requires a positive hiring target. Existing active introductions can progress while the target is zero.',object({id:str,status:{type:'string',enum:['introduced','interview','hired','declined']},note:str},['id','status','note']),x=>{const{id,...body}=x;return aiApi(`/api/admin/matches/${encodeURIComponent(id)}`,{method:'PATCH',body});});
    tool('hireferry_admin_list_mandates','List submitted private search briefs. Never invent consent or external outreach.',object({q:str,status:{type:'string',enum:['active','paused','closed']},page:{type:'integer',minimum:1}}),x=>aiApi(queryPath('/api/admin/mandates',x)),true);
    tool('hireferry_admin_get_mandate','Read a submitted search brief and its external outreach consent.',object({user_id:str},['user_id']),x=>aiApi(`/api/admin/mandates/${encodeURIComponent(x.user_id)}`),true);
    tool('hireferry_admin_update_mandate','Update an existing search brief or pause it. Administrators cannot create consent or expand external outreach authorization. Salary values use integer minor units; omitted salary_currency preserves the recorded currency.',object({user_id:str,target_industry:str,target_roles:str,salary_currency:currencySchema,salary_min_cents:{type:['integer','null'],minimum:0,maximum:100000000},salary_max_cents:{type:['integer','null'],minimum:0,maximum:100000000},location:str,preferences:str,status:{type:'string',enum:['active','paused','closed']}},['user_id']),async x=>{const{user_id,...changes}=x;const{mandate}=await aiApi(`/api/admin/mandates/${encodeURIComponent(user_id)}`);if(!mandate)throw new Error('Only the seeker can submit a new mandate.');const body=Object.fromEntries(['target_industry','target_roles','salary_min_cents','salary_max_cents','location','preferences','status'].map(k=>[k,changes[k]!==undefined?changes[k]:mandate[k]]));body.allow_external=mandate.allow_external;if(changes.salary_currency!==undefined)body.salary_currency=changes.salary_currency;return aiApi(`/api/admin/mandates/${encodeURIComponent(user_id)}`,{method:'PATCH',body});});
    tool('hireferry_admin_get_outreach','Read recorded research and external-contact progress for one search brief.',object({user_id:str},['user_id']),x=>aiApi(`/api/admin/outreach/${encodeURIComponent(x.user_id)}`),true);
    const outreachSchema={channel:str,opportunity:str,source_url:str,stage:{type:'string',enum:['identified','contacted','replied','interview','offer','closed']},note:str};
    tool('hireferry_admin_record_outreach','Record research or contact that actually occurred, subject to the job seeker’s consent. This endpoint only records progress; it does not search external services or send external messages. The job seeker can see the record.',object({user_id:str,...outreachSchema},['user_id','channel','opportunity','stage','note']),x=>{const{user_id,...body}=x;return aiApi(`/api/admin/outreach/${encodeURIComponent(user_id)}`,{method:'POST',body});});
    tool('hireferry_admin_update_outreach','Update an existing factual research or contact record. Do not fabricate progress.',object({id:str,...outreachSchema},['id','channel','opportunity','stage','note']),x=>{const{id,...body}=x;return aiApi(`/api/admin/outreach/records/${encodeURIComponent(id)}`,{method:'PATCH',body});});
    tool('hireferry_admin_list_orders','Read orders, including test/live flags and payment status. This does not change payments.',object({from:str,to:str,status:str,page:{type:'integer',minimum:1}}),x=>aiApi(queryPath('/api/admin/orders',x)),true);
    tool('hireferry_admin_overview','Read revenue statistics for one currency and separate currency_totals. Never add or compare amounts across currencies. Dates are UTC; explicitly distinguish live revenue from test transactions.',object({from:str,to:str,currency:{type:'string',pattern:'^[A-Z]{3}$'},mode:{type:'string',enum:['live','test']}}),x=>aiApi(queryPath('/api/admin/overview',x)),true);
    tool('hireferry_admin_audit','Read the platform business audit log.',object(),()=>aiApi('/api/admin/audit'),true);
    return tools;
  }
  tool('hireferry_get_profile','Read your own private profile.',object(),()=>aiApi('/api/profile'),true);
  tool('hireferry_update_profile','Update your own profile and contact email. Profiles are shared only with HireFerry and active introduced parties. Sign-in email is private.',object({name:str,public_email:str,resume_md:str},['name','public_email','resume_md']),x=>aiApi('/api/profile',{method:'PUT',body:{...x,visible:false}}));
  tool('hireferry_set_svg_avatar','Set a static SVG avatar on your own profile. Unsafe SVG is rejected.',object({svg:str},['svg']),x=>aiApi('/api/profile/avatar',{method:'PUT',body:x}));
  tool('hireferry_set_image_avatar','Upload your own PNG, JPEG, or WebP avatar as base64 bytes, maximum 2 MiB.',object({base64:str,mime_type:{type:'string',enum:['image/png','image/jpeg','image/webp']}},['base64','mime_type']),x=>{if(x.base64.length>2800000)throw new Error('Image is too large');const bytes=Uint8Array.from(atob(x.base64),c=>c.charCodeAt(0));const form=new FormData();form.append('file',new Blob([bytes],{type:x.mime_type}),'avatar');return aiApi('/api/profile/avatar',{method:'POST',body:form});});
  tool('hireferry_list_introductions','Read only your active platform introductions. Public discovery is separate: use the unauthenticated /mcp/public endpoint or the public directory for explicitly public roles and résumés.',object(),()=>aiApi('/api/matches'),true);
  tool('hireferry_get_introduced_job','Read your own role or a role introduced to you by HireFerry. Unrelated job IDs are not accessible.',idSchema,x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}`),true);
  tool('hireferry_get_introduced_person','Read a job seeker introduced to you by HireFerry. Unrelated profiles are not accessible.',idSchema,x=>aiApi(`/api/people/${encodeURIComponent(x.id)}`),true);
  tool('hireferry_list_orders','Read your own payment orders.',object(),()=>aiApi('/api/orders'),true);
  tool('hireferry_checkout_order','Retrieve a checkout link for your pending order. The human completes payment with the provider; this action does not pay.',idSchema,x=>aiApi(`/api/orders/${encodeURIComponent(x.id)}/checkout`,{method:'POST',body:{}}));
  if(state.user.role==='seeker'){
    tool('hireferry_get_mandate','Read your private search brief and recorded external-outreach consent.',object(),()=>aiApi('/api/mandate'),true);
    tool('hireferry_update_mandate','Submit or update your own search brief. Set allow_external=true only if the user explicitly authorizes HireFerry to contact external parties for this brief. Amounts are integer minor units of salary_currency; omitted currency keeps an existing brief’s currency or uses the site default for a new one. The brief is not publicly listed.',object({target_industry:str,target_roles:str,salary_currency:currencySchema,salary_min_cents:{type:['integer','null'],minimum:0,maximum:100000000},salary_max_cents:{type:['integer','null'],minimum:0,maximum:100000000},location:str,preferences:str,status:{type:'string',enum:['active','paused','closed']},allow_external:bool},['target_industry','target_roles','salary_min_cents','salary_max_cents','location','preferences','status','allow_external']),x=>aiApi('/api/mandate',{method:'PUT',body:x}));
    tool('hireferry_get_outreach','Read HireFerry’s recorded research and contact progress for your search. Records are actual team updates, not automatic search results.',object(),()=>aiApi('/api/outreach'),true);
  }
  if(state.user.role==='employer'){
    tool('hireferry_list_my_jobs','Read your own job records, including drafts.',object(),()=>aiApi('/api/jobs/mine'),true);
    tool('hireferry_create_job',`Create a role using fixed monthly BASE PAY, excluding bonuses and commission. salary_cents means integer minor units of currency. Read site config for currency precision and fees. Current publication rules: ${currencies().map(meta=>meta.currency+': '+feeRule('publish',meta)).join('; ')}. Free mode publishes immediately; paid mode creates a pending order. An approved promo_code discounts publication; 100% is free. Supply request_key when using a code and reuse it for retries. Provider is required only for a paid operation. Headcount defaults to 1, is free to change, and does not affect the fee; zero pauses new introductions. Publication locks the title, currency, and salary.`,object({title:str,jd_md:str,currency:currencySchema,salary_cents:{type:'integer',minimum:1,maximum:1000000000},provider:{type:'string',enum:['stripe','waffo','']},promo_code:{type:'string',minLength:6,maxLength:64},request_key:{type:'string',minLength:8,maxLength:128,pattern:'^[A-Za-z0-9_-]+$'},expected_amount_cents:{type:'integer',minimum:0},headcount:{...headcountSchema,default:1}},['title','jd_md','salary_cents']),x=>aiApi('/api/jobs',{method:'POST',body:{...x,headcount:headcountInput(x.headcount??1)}}));
    tool('hireferry_set_job_headcount','Set the hiring target for your own draft or published role, free with unlimited changes. Zero pauses new introductions; a positive number resumes them for a paid, published role. Existing introductions and chats remain. This changes no title, salary, JD version, fee, order, or paid edit allowance. A target of zero keeps employer AI eligibility when the paid salary requirement is met. Closed or deleted roles cannot resume hiring this way. The count is maintained manually.',object({id:str,headcount:headcountSchema},['id','headcount']),x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}/headcount`,{method:'PATCH',body:{headcount:headcountInput(x.headcount)}}));
    tool('hireferry_list_deleted_jobs','Read your own roles in the trash that can be restored once. Permanently deleted roles cannot be recovered.',object(),()=>aiApi('/api/my/jobs/trash'),true);
    tool('hireferry_delete_job','Delete your own role only with explicit human authorization. On first deletion it enters the trash and can be restored once; after restoration the next deletion permanently erases its title and description. Read delete_count and the current version first, and clearly explain which action will happen. Free, no automatic refund, no paid description update used. Deletion revokes shared introduction access and can remove employer AI eligibility.',versionedIdSchema,x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}`,{method:'DELETE',body:{expected_version:x.expected_version}}),false,true);
    tool('hireferry_restore_job','Restore your own role from its first deletion once, free of charge, with human authorization and its current version. Existing introductions may regain access under current rules. Without valid payment rights it returns as closed. Its next deletion is permanent. Private platform chats remain separate.',versionedIdSchema,x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}/restore`,{method:'POST',body:{expected_version:x.expected_version}}));
    tool('hireferry_permanently_delete_job','Permanently erase the title and description of your own role in the trash, only with explicit human authorization and its current version. This cannot be restored. Free, no automatic refund, no paid description update used. Necessary order and security records remain.',versionedIdSchema,x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}/permanent`,{method:'DELETE',body:{expected_version:x.expected_version}}),false,true);
    tool('hireferry_edit_job','Request a description update. The title, currency, and salary are locked. Read the job’s frozen terms for edit_mode, edit_fixed_minor, edit_bps, and max_edits. Free mode applies immediately; a paid update keeps the existing description until payment is confirmed. Provider is required only for a paid operation.',object({id:str,jd_md:str,provider:{type:'string',enum:['stripe','waffo','']}},['id','jd_md']),x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}/edit`,{method:'POST',body:{jd_md:x.jd_md,provider:x.provider||''}}));
    tool('hireferry_close_job','Permanently close your own role to new introductions, only with human authorization. To pause temporarily use set_job_headcount with zero instead. Closed roles cannot be reopened by increasing headcount. Closing the last qualifying role also ends employer AI eligibility.',idSchema,x=>aiApi(`/api/jobs/${encodeURIComponent(x.id)}/close`,{method:'POST',body:{}}),false,true);
  }
  return tools;
}
render();

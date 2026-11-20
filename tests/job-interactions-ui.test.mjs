import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {createJobInteractionsUI, supportsJobInteractions} from '../static/job-interactions.js';
import {createNavigationHarness} from './helpers/navigation-harness.mjs';
import {fileURLToPath} from 'node:url';

const esc = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const job = {id:'role-1', status:'published', headcount:2};
const summary = (extra = {}) => ({upvotes:3, downvotes:1, my_vote:0, comments:[], total:0, offset:0, limit:20, ...extra});
const deferred = () => {let resolve, reject;const promise = new Promise((yes,no) => {resolve=yes;reject=no;});return {promise,resolve,reject};};
const flush = () => new Promise(resolve => setImmediate(resolve));
function fixture({user = {id:'member'}, respond = () => summary(), ensureSession = async () => {}} = {}) {
  const dom = new JSDOM('<main><h1>Role details remain visible</h1><div id="root"></div></main>');
  const root = dom.window.document.querySelector('#root');
  const state = {user, csrf:user ? 'csrf-token' : null};
  const calls = [];
  const ui = createJobInteractionsUI({state,api:async (path,options = {}) => {calls.push({path,...options});return respond(path,options,calls);}, t:(_zh,en)=>en,esc,date:value=>String(value),ensureSession});
  root.innerHTML = ui.placeholder(job);
  const click = selector => root.querySelector(selector).click();
  const input = value => {const field=root.querySelector('textarea');field.value=value;field.dispatchEvent(new dom.window.Event('input',{bubbles:true}));};
  const submit = () => root.querySelector('form').dispatchEvent(new dom.window.Event('submit',{bubbles:true,cancelable:true}));
  return {dom,root,state,calls,ui,click,input,submit,close(){ui.destroy();dom.window.close();}};
}

test('interactions are limited to public, active jobs and anonymous text is escaped', async t => {
  assert.equal(supportsJobInteractions(job), true);
  for (const record of [{...job,status:'draft'},{...job,status:'closed'},{...job,headcount:0},{...job,headcount:-1},{...job,employer:{banned:true}}]) assert.equal(supportsJobInteractions(record),false);
  const f = fixture({user:null,respond:()=>summary({comments:[{id:1,author_name:'<img src=x onerror=alert(1)>',body:'</p><script>alert(1)</script>\nsecond line',created_at:123,can_delete:true}],total:1})});t.after(()=>f.close());
  assert.equal(f.ui.placeholder({...job,headcount:0}), '');
  await f.ui.mount(f.root);
  assert.match(f.root.textContent, /Sign in to vote or comment/);
  assert.match(f.root.textContent, /Comments are visible to everyone/);
  assert.equal(f.root.querySelector('script, img'),null);
  assert.equal(f.root.querySelector('.job-comment-body').textContent,'</p><script>alert(1)</script>\nsecond line');
  assert.equal(f.root.querySelector('[data-job-interaction="delete"]'),null);
  assert.ok([...f.root.querySelectorAll('.job-vote')].every(button=>button.disabled));
  assert.equal(f.calls[0].path,'/api/public/jobs/role-1/interactions?offset=0');
});

test('selected vote toggles off, switching uses server counts, and busy suppresses duplicate writes', async t => {
  const pending=deferred();
  const f=fixture({respond:(_path,options)=>options.method==='PUT'?pending.promise:summary({my_vote:1})});t.after(()=>f.close());
  await f.ui.mount(f.root);
  assert.equal(f.root.querySelector('[data-vote="1"]').getAttribute('aria-pressed'),'true');
  f.click('[data-vote="1"]');
  assert.deepEqual(f.calls[1].body,{vote:0});
  assert.equal(f.calls[1].method,'PUT');
  assert.equal(f.calls[1].path,'/api/jobs/role-1/vote');
  f.click('[data-vote="-1"]');
  assert.equal(f.calls.length,2);
  assert.equal(f.root.querySelector('[data-job-interactions]').getAttribute('aria-busy'),'true');
  pending.resolve(summary({upvotes:2,my_vote:0}));await flush();
  assert.equal(f.root.querySelector('[data-vote="1"] span').textContent,'2');
  f.click('[data-vote="-1"]');await flush();
  assert.deepEqual(f.calls[2].body,{vote:-1});
});

test('comments trim Unicode text, validate codepoints, preserve failed drafts and use author-only delete controls', async t => {
  let fail=true;
  const f=fixture({respond:(_path,options)=>{
    if(options.method==='POST') {if(fail)throw new Error('<save failed>');return summary({comments:[{id:42,author_name:'会员',body:options.body.body,created_at:1,can_delete:true},{id:43,author_name:'Another member',body:'Public',created_at:2,can_delete:false}],total:2});}
    if(options.method==='DELETE')return summary();
    return summary();
  }});t.after(()=>f.close());
  await f.ui.mount(f.root);
  f.input('   ');f.submit();assert.equal(f.calls.length,1);
  f.input('😀'.repeat(2001));f.submit();assert.equal(f.calls.length,1);
  const body='😀'.repeat(2000);
  f.input(`  ${body}  `);f.submit();await flush();
  assert.equal(f.calls[1].body.body,body);
  assert.equal(f.root.querySelector('textarea').value,`  ${body}  `);
  assert.match(f.root.querySelector('[role="alert"]').textContent,/<save failed>/);
  assert.equal(f.root.querySelector('save'),null);
  fail=false;f.submit();await flush();
  assert.equal(f.root.querySelector('textarea').value,'');
  assert.equal(f.root.querySelectorAll('[data-job-interaction="delete"]').length,1);
  f.click('[data-job-interaction="delete"]');await flush();
  assert.equal(f.calls.at(-1).method,'DELETE');
  assert.equal(f.calls.at(-1).path,'/api/jobs/role-1/comments/42');
  assert.equal(f.root.querySelectorAll('.job-comment').length,0);
});

test('pagination stays bounded, recovers a shrinking last page and mutations return to page one', async t => {
  let reads=0;
  const f=fixture({respond:(path,options)=>{
    if(options.method==='PUT')return summary({total:21});
    reads++;
    if(path.endsWith('offset=20'))return summary({total:1,offset:20});
    return summary({total:reads===1?21:1});
  }});t.after(()=>f.close());
  await f.ui.mount(f.root);
  assert.equal(f.root.querySelector('[data-job-interaction="previous"]').disabled,true);
  f.click('[data-job-interaction="next"]');await flush();
  assert.deepEqual(f.calls.map(call=>call.path),['/api/public/jobs/role-1/interactions?offset=0','/api/public/jobs/role-1/interactions?offset=20','/api/public/jobs/role-1/interactions?offset=0']);
  assert.equal(f.root.querySelector('[data-job-interaction="next"]'),null);
  f.click('[data-vote="1"]');await flush();
  assert.match(f.root.querySelector('.job-comments-pages').textContent,/Page 1 of 2/);
  f.click('[data-job-interaction="next"]');await flush();
  assert.ok(f.calls.filter(call=>call.method==='GET').every(call=>/offset=(0|20)$/.test(call.path)));
});

test('read errors stay in the panel with retry and 404 removes active controls', async t => {
  let failure=Object.assign(new Error('<network offline>'),{status:503});
  const f=fixture({respond:()=>{if(failure)throw failure;return summary();}});t.after(()=>f.close());
  await f.ui.mount(f.root);
  assert.equal(f.dom.window.document.querySelector('h1').textContent,'Role details remain visible');
  assert.match(f.root.textContent,/<network offline>/);
  failure=null;f.click('[data-job-interaction="retry"]');await flush();
  assert.ok(f.root.querySelector('form'));
  failure=Object.assign(new Error('Gone'),{status:404});
  f.click('[data-vote="1"]');await flush();
  assert.match(f.root.textContent,/no longer available/);
  assert.equal(f.root.querySelector('form, .job-vote'),null);
});

test('session check finishes before personalized read and failure still permits public reads', async t => {
  const session=deferred();
  const f=fixture({user:null,ensureSession:()=>session.promise});t.after(()=>f.close());
  const mounted=f.ui.mount(f.root);assert.equal(f.calls.length,0);
  f.state.user={id:'member'};f.state.csrf='csrf';session.resolve();await mounted;
  assert.ok(f.root.querySelector('form'));
  const failed=fixture({ensureSession:async()=>{throw new Error('offline');}});t.after(()=>failed.close());
  await failed.ui.mount(failed.root);
  assert.equal(failed.calls.length,1);
  assert.match(failed.root.textContent,/Sign-in status could not be checked/);
  assert.equal(failed.root.querySelector('form'),null);
});

test('destroy and remount ignore stale responses even when the DOM panel is reused', async t => {
  const old=deferred();let reads=0;
  const f=fixture({respond:()=>++reads===1?old.promise:summary({upvotes:9})});t.after(()=>f.close());
  const first=f.ui.mount(f.root);await flush();
  const oldSignal=f.calls[0].signal;
  await f.ui.mount(f.root);
  assert.equal(oldSignal.aborted,true);
  old.resolve(summary({upvotes:100}));await first;
  assert.equal(f.root.querySelector('[data-vote="1"] span').textContent,'9');
});

test('identity changes discard personalized results and block writes from the old summary', async t => {
  const pending=deferred();let reads=0;
  const f=fixture({respond:()=>++reads===1?pending.promise:summary({my_vote:-1})});t.after(()=>f.close());
  const first=f.ui.mount(f.root);await flush();f.state.user={id:'another-member'};
  pending.resolve(summary({my_vote:1}));await first;await flush();
  assert.equal(f.root.querySelector('[data-vote="-1"]').getAttribute('aria-pressed'),'true');
  f.state.user={id:'third-member'};f.click('[data-vote="1"]');await flush();
  assert.ok(f.calls.every(call=>call.method!=='PUT'));
});

test('public and introduced job routes share the panel and suppress it for paused jobs', async t => {
  const h=await createNavigationHarness(fileURLToPath(new URL('../static/',import.meta.url)));t.after(()=>h.dispose());
  const record={...job,title:'Role interaction fixture',salary_cents:1000000,currency:'CNY',created_at:1,employer:{name:'Public employer'}};
  h.overrides.set('/api/public/jobs/role-1',record);
  h.overrides.set('/api/jobs/role-1',{job:record,employer:record.employer,jd_html:'<p>Role content</p>'});
  await h.app.go('/discover/jobs/role-1');assert.match(h.root.innerHTML,/data-job-interactions="role-1"/);
  await h.app.go('/jobs/role-1');assert.match(h.root.innerHTML,/data-job-interactions="role-1"/);
  h.overrides.set('/api/jobs/role-1',{job:{...record,headcount:0},employer:record.employer});
  await h.app.render();assert.doesNotMatch(h.root.innerHTML,/data-job-interactions=/);
  h.overrides.set('/api/jobs/role-1',{job:{...record,status:'draft'},employer:record.employer});
  await h.app.render();assert.doesNotMatch(h.root.innerHTML,/data-job-interactions=/);
});

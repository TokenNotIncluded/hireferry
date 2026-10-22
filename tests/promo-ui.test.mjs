import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const source=await readFile(new URL('../static/promo-ui.js',import.meta.url),'utf8');
const {createPromoUI,normalizePromoCode}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const escape=value=>String(value??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const node=()=>({textContent:'',innerHTML:'',hidden:false,disabled:false,classList:{add(){},remove(){}}});
const promo={id:'promo-one',code:'HIRING-ONE',label:'Hiring support',free_activation_limit:1,activation_used:0,activation_remaining:1,publish_limit:3,publish_used:0,publish_reserved:0,publish_remaining:3,discount_percent:100,enabled:true,expires_at:null};

function harness(api=async()=>({})) {
  const state={user:{id:'employer-one',role:'employer',employer_paid:false},lang:'en'};
  const calls=[],actions=[];
  let id=0,ui;
  const dependencies={state,api:async(path,options)=>{calls.push({path,...options});return api(path,options);},t:(_zh,en)=>en,esc:escape,icon:()=>'',money:(n,record)=>`${record?.currency||'CNY'} ${n}`,date:String,currency:()=>({currency:'CNY'}),currencySelect:()=>'<select name="currency"></select>',centsInput:value=>{const n=Number(value)*100;if(!Number.isSafeInteger(n)||n<=0)throw new Error('Invalid salary');return n;},heading:(title,description='',action='')=>`<h1>${title}</h1><p>${description}</p>${action}`,shell:body=>body,empty:(title,description)=>`<h3>${title}</h3><p>${description}</p>`,notice:text=>`<p>${text}</p>`,openDialog:html=>actions.push({dialog:html}),closeDialog:()=>{},notify:message=>actions.push({message}),go:async path=>actions.push({go:path}),render:async()=>{},session:async()=>{},confirmDialog:(_title,_description,action)=>action(),createMessageId:()=>`request-${++id}`,refreshJobPrice:form=>{form.nodes['[data-job-submit]'].disabled=true;form.nodes['[data-job-submit]'].innerHTML='Continue to payment';form.nodes['[data-payment-options]'].hidden=false;form.nodes['#job-cost'].textContent='CNY 60000';ui.refreshJob(form);}};
  ui=createPromoUI(dependencies);
  return {ui,state,calls,actions,dependencies};
}

function form(kind='job') {
  const elements=kind==='job'?{promo_code:{value:' hiring-one ',name:'promo_code'},salary:{value:'6000',name:'salary'},currency:{value:'CNY',name:'currency'}}:{code:{value:' hiring-one ',name:'code'},currency:{value:'CNY',name:'currency'}};
  const nodes=Object.fromEntries(['[data-promo-quote]','[data-job-submit]','[data-payment-options]','[data-job-payment-note]','#job-cost','[type="submit"]'].map(selector=>[selector,node()]));
  nodes['[data-job-submit]'].disabled=true;nodes['[type="submit"]'].disabled=true;
  const value={elements,nodes,dataset:{form:kind,exhausted:'false'},isConnected:true,querySelector:selector=>nodes[selector]};
  for(const element of Object.values(elements))element.closest=()=>value;
  return value;
}

const quote=(amount=0)=>({promo_code:promo,original_amount_cents:60000,discount_percent:amount===0?100:50,amount_cents:amount,currency:'CNY',currency_decimals:2});

test('a verified 100% discount enables publication without a payment provider',async()=>{
  const h=harness(async()=>quote()),f=form();
  await h.ui.preview(f,'publish');
  assert.deepEqual(h.calls[0].body,{code:'HIRING-ONE',purpose:'publish',currency:'CNY',salary_cents:600000});
  assert.equal(f.nodes['[data-job-submit]'].disabled,false);
  assert.equal(f.nodes['[data-payment-options]'].hidden,true);
  assert.equal(f.nodes['#job-cost'].textContent,'CNY 0');
  assert.match(f.nodes['[data-job-submit]'].innerHTML,/Confirm free publication/);
  assert.equal(f.nodes['[data-job-payment-note]'].textContent,'This publication is free and takes effect on confirmation.');
  const body=h.ui.prepareJob(f,{title:'Engineer',jd_md:'Build things.',salary_cents:600000,currency:'CNY',provider:'stripe',headcount:1});
  assert.equal(body.provider,'');assert.equal(body.promo_code,'HIRING-ONE');
  assert.equal(body.expected_amount_cents,0);
  assert.equal(body.request_key,'request-1');
});

test('one editor retains its request key even if content changes after an uncertain response',async()=>{
  const h=harness(async()=>quote()),f=form();await h.ui.preview(f,'publish');
  const body={title:'Engineer',jd_md:'First description',salary_cents:600000,currency:'CNY',provider:'',headcount:1};
  const first=h.ui.prepareJob(f,body),second=h.ui.prepareJob(f,body);
  assert.equal(first.request_key,second.request_key);
  const changed=h.ui.prepareJob(f,{...body,jd_md:'Updated description'});
  assert.equal(changed.request_key,first.request_key);
});

test('a changed-price rejection can be requoted with a new expected amount and the same request key',async()=>{
  let amount=0;
  const h=harness(async()=>quote(amount)),f=form();
  await h.ui.preview(f,'publish');
  const body={title:'Engineer',jd_md:'Description',salary_cents:600000,currency:'CNY',provider:'stripe',headcount:1};
  const first=h.ui.prepareJob(f,body);
  assert.equal(first.expected_amount_cents,0);
  // The server rejects a changed amount before reserving a use or creating an order.
  amount=30000;await h.ui.preview(f,'publish');
  const retried=h.ui.prepareJob(f,body);
  assert.equal(retried.expected_amount_cents,30000);
  assert.equal(retried.request_key,first.request_key);
  assert.equal(retried.provider,'stripe');
  assert.equal(f.nodes['[data-payment-options]'].hidden,false);
});

test('changing salary, currency or code invalidates the quote and prevents a stale free submission',async()=>{
  for(const [field,value] of [['salary','7000'],['currency','JPY'],['promo_code','OTHER-CODE']]) {
    const h=harness(async()=>quote()),f=form();await h.ui.preview(f,'publish');
    f.elements[field].value=value;h.ui.changed(f.elements[field]);h.dependencies.refreshJobPrice(f);
    assert.equal(f.nodes['[data-job-submit]'].disabled,true);
    assert.equal(h.ui.matchingQuote(f,'publish'),null);
    assert.throws(()=>h.ui.prepareJob(f,{title:'Engineer'}),/Check the promotion again/);
    assert.equal(f.nodes['[data-payment-options]'].hidden,false);
  }
});

test('invalid and stale previews cannot enable publication or fall back to full-price submission',async()=>{
  const failed=harness(async()=>{throw new Error('Promotion code is unavailable');}),f=form();
  await assert.rejects(failed.ui.preview(f,'publish'),/unavailable/);
  assert.equal(f.nodes['[data-job-submit]'].disabled,true);
  assert.throws(()=>failed.ui.prepareJob(f,{title:'Engineer'}),/Check the promotion again/);
  let resolve;
  const h=harness(()=>new Promise(done=>{resolve=done;})),other=form();
  const pending=h.ui.preview(other,'publish');
  other.elements.promo_code.value='CHANGED-CODE';h.ui.changed(other.elements.promo_code);resolve(quote());await pending;
  assert.equal(h.ui.matchingQuote(other,'publish'),null);
  assert.equal(other.nodes['[data-job-submit]'].disabled,true);
});

test('a partial discount keeps payment visible and does not enable unavailable providers',async()=>{
  const h=harness(async()=>quote(30000)),f=form();await h.ui.preview(f,'publish');
  assert.equal(f.nodes['[data-payment-options]'].hidden,false);
  assert.equal(f.nodes['[data-job-submit]'].disabled,true);
  assert.equal(f.nodes['#job-cost'].textContent,'CNY 30000');
  assert.match(f.nodes['[data-job-payment-note]'].textContent,/Pay the discounted amount/);
  assert.equal(h.ui.prepareJob(f,{provider:'stripe'}).provider,'stripe');
});

test('free activation requires a current preview and refreshes the authenticated account',async()=>{
  const h=harness(async path=>path.endsWith('/preview')?quote():{user:{id:'employer-one',role:'employer',employer_paid:true},order:{amount_cents:0,status:'paid'}}),f=form('promo-activation');
  await assert.rejects(h.ui.submit(f,new FormData()),/Check the code before/);
  await h.ui.preview(f,'activation');
  assert.equal(f.nodes['[type="submit"]'].disabled,false);
  await h.ui.submit(f,new FormData());
  assert.equal(h.state.user.employer_paid,true);
  assert.deepEqual(h.calls.at(-1).body,{code:'HIRING-ONE',currency:'CNY'});
  assert.ok(h.actions.some(action=>action.go==='/app?tab=jobs'));
});

test('activation allowance can be disabled independently and discount bounds reject malformed input',()=>{
  const h=harness(),data=new FormData();
  for(const [key,value]of Object.entries({label:'Hiring',include_activation:'on',free_activation_limit:'1',publish_limit:'3',discount_percent:'100',enabled:'on'}))data.set(key,value);
  assert.deepEqual(h.ui.settings(data),{label:'Hiring',free_activation_limit:1,publish_limit:3,discount_percent:100,enabled:true});
  data.delete('include_activation');data.delete('free_activation_limit');
  assert.equal(h.ui.settings(data).free_activation_limit,0);assert.equal(h.ui.settings(data).publish_limit,3);
  for(const value of ['101','-1','1.5','Infinity','1e2','']){data.set('discount_percent',value);assert.throws(()=>h.ui.settings(data),/integer from 0 to 100/);}
  assert.equal(normalizePromoCode('  hiring-one\n'),'HIRING-ONE');
});

test('application results show issued codes and escape user-controlled notes',async()=>{
  const h=harness(async()=>({items:[{id:'app-one',reason:'<script>alert(1)</script>',status:'approved',review_note:'<img onerror=evil()>',created_at:1,promo_code:promo}],total:1,page:1,limit:50}));
  const html=await h.ui.memberPage();
  assert.match(html,/Approved/);assert.match(html,/HIRING-ONE/);assert.match(html,/Free publication/);
  assert.match(html,/&lt;script&gt;/);assert.match(html,/&lt;img onerror=evil\(\)&gt;/);
  assert.doesNotMatch(html,/<script>|<img onerror/);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source=readFileSync(new URL('../static/requests.js',import.meta.url),'utf8');
const {createRequestClient,createSessionReader,navigationReads,RequestTimeoutError}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};

test('concurrent GETs share one request without sharing mutable result objects or retaining chat history',async()=>{
  let calls=0;const gate=deferred();
  const client=createRequestClient(async()=>{calls++;await gate.promise;return {items:[{body:'private'}]};});
  const a=client.request('/api/chat/conversations/a/messages');
  const b=client.request('/api/chat/conversations/a/messages');
  gate.resolve();const [first,second]=await Promise.all([a,b]);
  assert.equal(calls,1);first.items[0].body='changed';assert.equal(second.items[0].body,'private');
  await client.request('/api/chat/conversations/a/messages');assert.equal(calls,2);
});

test('prefetch is read-only and limited to a person’s own profile, brief and outreach',async()=>{
  const calls=[];const client=createRequestClient(async path=>{calls.push(path);return {};});
  for(const path of ['/api/tokens','/api/session','/api/orders','/api/people/other','/api/chat/conversations','/api/admin/users'])await client.request(path,{prefetch:true});
  await client.request('/api/profile',{prefetch:true,method:'POST'});
  assert.deepEqual(calls,[]);
  await client.request('/api/profile',{prefetch:true});assert.deepEqual(calls,['/api/profile']);
  const user={id:'one',role:'seeker'};
  assert.deepEqual(navigationReads(new URL('https://test/app?tab=mandate'),user),['/api/mandate','/api/outreach']);
  for(const href of ['/app?tab=messages','/admin?tab=profile','/app?tab=profile&user=other','/billing/return'])assert.deepEqual(navigationReads(new URL(href,'https://test'),user),[]);
});

test('completed own-screen prefetch is consumed once and expires after a short deadline',async()=>{
  let clock=100,calls=0;const client=createRequestClient(async()=>({count:++calls}),{now:()=>clock});
  await client.request('/api/profile',{prefetch:true});
  assert.equal((await client.request('/api/profile')).count,1);
  assert.equal((await client.request('/api/profile')).count,2);
  await client.request('/api/profile',{prefetch:true});clock+=3001;
  assert.equal((await client.request('/api/profile')).count,4);
});

test('a click that joins a pending prefetch consumes it instead of leaving a second cached copy',async()=>{
  const gate=deferred();let calls=0;const client=createRequestClient(async()=>{const call=++calls;await gate.promise;return {call};});
  const hovered=client.request('/api/profile',{prefetch:true});const clicked=client.request('/api/profile');
  gate.resolve();await hovered;assert.equal((await clicked).call,1);
  assert.equal((await client.request('/api/profile')).call,2);
});

test('AI attribution and custom credentials never borrow an ordinary user response',async()=>{
  const gate=deferred();let calls=0;const client=createRequestClient(async(_path,options)=>{calls++;await gate.promise;return {ai:options.headers?.['X-HireFerry-AI']||false,credentials:options.credentials||'same-origin'};});
  const ordinary=client.request('/api/profile',{prefetch:true});
  const ai=client.request('/api/profile',{headers:{'X-HireFerry-AI':'webmcp'}});
  const guest=client.request('/api/profile',{credentials:'omit'});
  gate.resolve();const results=await Promise.all([ordinary,ai,guest]);
  assert.equal(calls,3);assert.equal(results[1].ai,'webmcp');assert.equal(results[2].credentials,'omit');
});

test('a mutation cannot leave a pre-mutation prefetch available for the next route',async()=>{
  const gate=deferred();let calls=0;
  const client=createRequestClient(async()=>{const call=++calls;if(call===1)await gate.promise;return {version:call};});
  const old=client.request('/api/mandate',{prefetch:true});await Promise.resolve();
  client.invalidate();gate.resolve();await old;
  assert.equal((await client.request('/api/mandate')).version,2);
  await client.request('/api/mandate',{prefetch:true});client.invalidate();
  assert.equal((await client.request('/api/mandate')).version,4);
});

test('logout aborts outstanding reads even when a transport ignores cancellation',async()=>{
  const gate=deferred();let signal;
  const client=createRequestClient(async(_path,options)=>{signal=options.signal;return gate.promise;});
  const response=client.request('/api/profile');await Promise.resolve();client.invalidate({abort:true});
  await assert.rejects(response,{name:'AbortError'});assert.equal(signal.aborted,true);
  gate.resolve({account:'old'});
});

test('rapid navigation cancels obsolete route reads without canceling bootstrap or a submitted write',async()=>{
  const gates=new Map();const signals=new Map();
  const client=createRequestClient((path,options)=>{const gate=deferred();gates.set(path,gate);signals.set(path,options.signal);return gate.promise;});
  const old=client.request('/api/profile',{group:'navigation:1'});
  const session=client.request('/api/session');
  const write=client.request('/api/mandate',{method:'PUT'});
  await Promise.resolve();client.cancel('navigation:1');
  await assert.rejects(old,{name:'AbortError'});
  assert.equal(signals.get('/api/session').aborted,false);assert.equal(signals.get('/api/mandate').aborted,false);
  gates.get('/api/session').resolve({user:{id:'member'}});gates.get('/api/mandate').resolve({saved:true});await Promise.all([session,write]);
  const next=client.request('/api/profile',{group:'navigation:2'});await Promise.resolve();gates.get('/api/profile').resolve({name:'Current'});
  assert.deepEqual(await next,{name:'Current'});
});

test('reads and writes stop waiting at their deadline without an automatic retry',async()=>{
  let calls=0;const signals=[];const client=createRequestClient((_path,options)=>{calls++;signals.push(options.signal);return new Promise(()=>{});},{readTimeout:10,writeTimeout:10});
  await assert.rejects(client.request('/api/profile'),error=>error instanceof RequestTimeoutError&&!error.uncertain);
  await assert.rejects(client.request('/api/orders',{method:'POST'}),error=>error instanceof RequestTimeoutError&&error.uncertain);
  assert.equal(calls,2);assert.ok(signals.every(signal=>signal.aborted));
});

test('explicit freshness bypasses intent caching and an existing in-flight GET',async()=>{
  let calls=0;const gate=deferred();const client=createRequestClient(async()=>{const call=++calls;if(call===1)await gate.promise;return {call};});
  const old=client.request('/api/profile',{prefetch:true});await Promise.resolve();
  assert.equal((await client.request('/api/profile',{fresh:true})).call,2);
  gate.resolve();await old;
  assert.equal((await client.request('/api/profile',{fresh:true})).call,3);
});

test('GET failure is not retained and a later attempt can recover',async()=>{
  let calls=0;const client=createRequestClient(async()=>{if(++calls===1)throw Object.assign(new Error('revoked'),{status:403});return {ok:true};});
  await assert.rejects(client.request('/api/profile',{prefetch:true}),{status:403});
  assert.deepEqual(await client.request('/api/profile'),{ok:true});assert.equal(calls,2);
});

test('page changes reuse a validated session, while expiry and explicit refresh recheck it',async()=>{
  let clock=100,calls=0;const session=createSessionReader(async()=>({user:{id:'member',name:'Saved'},version:++calls}),{now:()=>clock});
  const initial=await session.read();initial.user.name='Local edit';
  assert.equal((await session.read()).user.name,'Saved');assert.equal(calls,1);
  clock+=30001;assert.equal((await session.read()).version,2);
  assert.equal((await session.read({force:true})).version,3);
});

test('simultaneous navigations do not duplicate the initial session check',async()=>{
  let calls=0;const gate=deferred();const session=createSessionReader(async()=>{calls++;await gate.promise;return {user:{id:'member'}};});
  const first=session.read(),second=session.read();gate.resolve();
  assert.deepEqual(await first,await second);assert.equal(calls,1);
});

test('a session result from before logout cannot restore the former account',async()=>{
  const gate=deferred();let calls=0;const session=createSessionReader(async()=>++calls===1?gate.promise:{user:null});
  const old=session.read();await Promise.resolve();session.invalidate();gate.resolve({user:{id:'old-account'}});
  await assert.rejects(old,{name:'AbortError'});assert.deepEqual(await session.read(),{user:null});
});

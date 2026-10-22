import test from 'node:test';
import assert from 'node:assert/strict';
import {writeFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {resolve} from 'node:path';
import {createNavigationHarness} from './helpers/navigation-harness.mjs';

const current=fileURLToPath(new URL('../static/',import.meta.url));
const pause=ms=>new Promise(yes=>setTimeout(yes,ms));
const deferred=()=>{let resolve;const promise=new Promise(yes=>resolve=yes);return {promise,resolve};};
async function until(condition){for(let i=0;i<200;i++){if(condition())return;await pause(5);}assert.ok(condition(),'Expected request or navigation state was not reached');}

test('cold signed-in entry keeps configuration and session reads parallel when the session finishes first',async()=>{
  const cases=[
    {identity:{user:null,admin:true,super_admin:true,staff:null,csrf:'owner-csrf'},path:'/admin?tab=team',marker:'data-form="team-invite"'},
    {identity:{user:{id:'staff-user',role:'unassigned',email:'staff@example.test'},admin:true,super_admin:false,staff:{id:'staff-one',name:'QA staff',kind:'human',active:true},csrf:'staff-csrf'},path:'/admin?tab=users',marker:'QA staff'},
    {identity:{user:{id:'fixture-user',role:'seeker'},admin:false,super_admin:false,staff:null,csrf:'member-csrf'},path:'/app?tab=profile',marker:'QA_PROFILE_READY'},
  ];
  for(const scenario of cases){
    const config=deferred();
    const harness=await createNavigationHarness(current,{beforeResponse:path=>path==='/api/config'?config.promise:undefined});
    let navigation;
    try{
      harness.app.state.config=null;harness.overrides.set('/api/session',scenario.identity);
      navigation=harness.app.go(scenario.path);
      await until(()=>harness.network.some(value=>value.path==='/api/session'&&value.finished!==null));
      config.resolve();await navigation;
      assert.ok(harness.root.innerHTML.includes(scenario.marker),harness.root.innerHTML.slice(-600));
      assert.doesNotMatch(harness.root.innerHTML,/route-error/);
      const configReads=harness.network.filter(value=>value.path==='/api/config');
      const sessionReads=harness.network.filter(value=>value.path==='/api/session');
      assert.equal(configReads.length,1);assert.equal(sessionReads.length,1);
      assert.ok(configReads[0].began<sessionReads[0].finished,'configuration starts before session finishes');
      assert.ok(sessionReads[0].finished<configReads[0].finished,'the test releases configuration only after session completes');
      assert.equal(configReads[0].aborted,false);
      assert.equal(harness.app.state.admin,scenario.identity.admin);
      assert.equal(harness.app.state.super_admin,scenario.identity.super_admin);
    }finally{config.resolve();await navigation;harness.dispose();}
  }
});

test('rapid cold navigation shares startup reads and publishes only the newest destination',async()=>{
  const config=deferred();
  const harness=await createNavigationHarness(current,{beforeResponse:path=>path==='/api/config'?config.promise:undefined});
  let first,second;
  try{
    harness.app.state.config=null;
    harness.overrides.set('/api/session',{user:null,admin:true,super_admin:true,staff:null,csrf:'owner-csrf'});
    first=harness.app.go('/admin?tab=team');
    await until(()=>harness.network.some(value=>value.path==='/api/session'&&value.finished!==null));
    second=harness.app.go('/admin?tab=users');
    config.resolve();await Promise.all([first,second]);
    assert.equal(harness.root.dataset.pageKey,'admin:users');
    assert.doesNotMatch(harness.root.innerHTML,/data-form="team-invite"|route-error/);
    assert.equal(harness.network.filter(value=>value.path==='/api/config').length,1);
    assert.equal(harness.network.filter(value=>value.path==='/api/session').length,1);
    assert.equal(harness.network.some(value=>value.path==='/api/admin/team'),false);
    assert.equal(harness.network.some(value=>value.path.startsWith('/api/admin/users')),true);
    assert.equal(harness.network.some(value=>value.aborted),false);
  }finally{config.resolve();await Promise.all([first,second]);harness.dispose();}
});

test('cross-tab sign-out during cold startup cannot restore the already-read former session',async()=>{
  const config=deferred();let configReads=0;
  const harness=await createNavigationHarness(current,{beforeResponse:path=>path==='/api/config'&&++configReads===1?config.promise:undefined});
  let initial;
  try{
    harness.app.state.config=null;
    initial=harness.app.go('/app?tab=profile');
    await until(()=>harness.network.some(value=>value.path==='/api/session'&&value.finished!==null));
    harness.setUser(null);harness.dispatch('storage',{key:'hireferry-session-change'});
    await until(()=>harness.location.pathname==='/login'&&harness.root.innerHTML.includes('data-form="email-start"'));
    config.resolve();await initial;
    assert.equal(harness.app.state.user,null);assert.equal(harness.app.state.admin,false);
    assert.doesNotMatch(harness.root.innerHTML,/QA_PROFILE_READY|route-error/);
    assert.equal(harness.network.some(value=>value.path==='/api/profile'),false);
    assert.ok(harness.network.some(value=>value.path==='/api/config'&&value.aborted));
  }finally{config.resolve();await initial;harness.dispose();}
});

test('a later identity change still cancels in-flight private reads and removes their view',async()=>{
  const profile=deferred();
  const harness=await createNavigationHarness(current,{beforeResponse:path=>path==='/api/profile'?profile.promise:undefined});
  let pending;
  try{
    await harness.prime();
    pending=harness.app.go('/app?tab=profile');
    await until(()=>harness.network.some(value=>value.path==='/api/profile'));
    harness.setUser(null);await harness.app.refreshVisibleSession();await pending;
    assert.equal(harness.app.state.user,null);assert.equal(harness.location.pathname,'/login');
    assert.ok(harness.network.some(value=>value.path==='/api/profile'&&value.aborted));
    assert.match(harness.root.innerHTML,/data-form="email-start"/);
    assert.doesNotMatch(harness.root.innerHTML,/QA_PROFILE_READY/);
  }finally{profile.resolve();await pending;harness.dispose();}
});

test('a background 401 immediately removes private conversation DOM and clears identity before chat disposal',async()=>{
  const harness=await createNavigationHarness(current);
  try{
    await harness.prime();await harness.route('/app?tab=messages','<p>QA_MESSAGE_READY</p>');
    const poll='/api/chat/conversations/platform_fixture-user/messages?after=1&limit=100';
    harness.overrides.set(poll,{httpStatus:401,body:{error:'Session expired'}});
    await assert.rejects(harness.app.api(poll),{status:401});
    assert.equal(harness.app.state.user,null);assert.equal(harness.location.pathname,'/login');
    assert.doesNotMatch(harness.root.innerHTML,/QA_MESSAGE_READY|data-form="chat"/);
    assert.match(harness.root.innerHTML,/data-form="email-start"/);
    assert.ok(harness.stops.slice(-2).every(value=>value.user===null&&!value.admin));
  }finally{harness.dispose();}
});

test('session expiry discovered on returning to the tab cannot leave private content visible',async()=>{
  for(const failure of [false,true]){
    const harness=await createNavigationHarness(current);
    try{
      await harness.prime();await harness.route('/app?tab=messages','<p>QA_MESSAGE_READY</p>');
      if(failure)harness.overrides.set('/api/session',{httpStatus:401,body:{error:'Expired'}});else harness.setUser(null);
      await harness.app.refreshVisibleSession();
      assert.equal(harness.location.pathname,'/login');
      assert.doesNotMatch(harness.root.innerHTML,/QA_MESSAGE_READY|data-form="chat"/);
      assert.match(harness.root.innerHTML,/data-form="email-start"/);
    }finally{harness.dispose();}
  }
});

test('the actual route pipeline removes serial session and chat round trips',async t=>{
  const latencyMs=Math.max(5,Number(process.env.HIREFERRY_NAV_LATENCY_MS)||20);
  const count=Math.max(1,Math.min(10,Number(process.env.HIREFERRY_NAV_SAMPLES)||1));
  const baseline=process.env.HIREFERRY_BASELINE_STATIC?resolve(process.env.HIREFERRY_BASELINE_STATIC):null;
  const variants=baseline?[['baseline',baseline],['revised',current]]:[['revised',current]];
  const routes=[['profile','/app?tab=profile','QA_PROFILE_READY'],['messages','/app?tab=messages','<p>QA_MESSAGE_READY</p>'],['mandate','/app?tab=mandate','QA_MANDATE_READY']];
  const report={environment:'Node route execution with a minimal DOM mount adapter; no browser automation',limitations:['Synthetic fixtures; every fetch adds the same requested delay, including writes.','Measures real app/service request scheduling and HTML publication, not browser layout, paint, transfer size, or production backend latency.','Authentication is primed before timing, representing ordinary signed-in navigation. No hover/focus prefetch is simulated.','Browser event binding, chat read receipts and polling are excluded; chat timing requires the actual message body in published HTML.'],latencyMs,samples:count,sourceHashes:{},variants:{}};
  for(let sample=0;sample<count;sample++)for(const [name,directory]of variants){
    const harness=await createNavigationHarness(directory,{latencyMs});
    try{
      await harness.prime();
      report.sourceHashes[name]||=harness.sourceHashes;
      report.variants[name]||={};
      for(const [route,target,marker]of routes){
        const result=await harness.route(target,marker);
        (report.variants[name][route]||=[]).push(result);
      }
    }finally{harness.dispose();}
  }
  const summary={};
  const describe=values=>{const sorted=[...values].sort((a,b)=>a-b);return {medianMs:+sorted[Math.floor(sorted.length/2)].toFixed(1),minMs:+sorted[0].toFixed(1),maxMs:+sorted.at(-1).toFixed(1)};};
  for(const [name,routes]of Object.entries(report.variants)){
    summary[name]={};
    for(const [route,samples]of Object.entries(routes))summary[name][route]={shell:describe(samples.map(value=>value.shellMs)),content:describe(samples.map(value=>value.contentMs)),requestsBeforeContent:[...new Set(samples.map(value=>value.requestsBeforeContent))],totalRouteRequests:[...new Set(samples.map(value=>value.requests.length))]};
  }
  report.summary=summary;
  assert.deepEqual(summary.revised.profile.requestsBeforeContent,[1]);
  assert.deepEqual(summary.revised.messages.requestsBeforeContent,[2]);
  assert.deepEqual(summary.revised.mandate.requestsBeforeContent,[2]);
  for(const samples of Object.values(report.variants.revised))for(const sample of samples){
    assert.ok(sample.requests.every(value=>value.path!=='/api/session'),'no serial session recheck');
    assert.ok(sample.requests.every(value=>value.path!=='/api/chat/platform'),'no repeated room creation');
  }
  if(baseline){
    assert.deepEqual(summary.baseline.profile.requestsBeforeContent,[2]);
    assert.deepEqual(summary.baseline.messages.requestsBeforeContent,[4]);
    assert.deepEqual(summary.baseline.mandate.requestsBeforeContent,[3]);
    // Counts above assert the scheduling change without fragile machine-speed thresholds.
    report.improvementPercent=Object.fromEntries(routes.map(([route])=>[route,+((1-summary.revised[route].content.medianMs/summary.baseline[route].content.medianMs)*100).toFixed(1)]));
  }
  if(process.env.HIREFERRY_NAV_REPORT)await writeFile(process.env.HIREFERRY_NAV_REPORT,JSON.stringify(report,null,2)+'\n');
  t.diagnostic(JSON.stringify({latencyMs,samples:count,summary,improvementPercent:report.improvementPercent}));
});

test('rapid route changes cancel obsolete reads and cannot publish the old private page',async()=>{
  const harness=await createNavigationHarness(current,{latencyMs:30});
  try{
    await harness.prime();
    const first=harness.app.go('/app?tab=profile');await pause(5);
    const second=harness.app.go('/app?tab=mandate');await Promise.all([first,second]);
    assert.match(harness.root.innerHTML,/QA_MANDATE_READY/);
    assert.doesNotMatch(harness.root.innerHTML,/QA_PROFILE_READY/);
    assert.ok(harness.network.some(value=>value.path==='/api/profile'&&value.aborted));
    assert.equal(harness.root.dataset.pageKey,'app:mandate');
  }finally{harness.dispose();}
});

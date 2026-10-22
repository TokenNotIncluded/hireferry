import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import {createHash} from 'node:crypto';

// Executes the real route and service code. This DOM sink records published HTML;
// it deliberately does not estimate layout, rendering, network transfer, or browser paint.
const classes=()=>{const values=new Set();return {add:value=>values.add(value),remove:value=>values.delete(value),toggle(value,on){if(on??!values.has(value))values.add(value);else values.delete(value);},contains:value=>values.has(value)};};
const node=()=>({dataset:{},innerHTML:'',classList:classes(),style:{setProperty(name,value){this[name]=value;},removeProperty(name){delete this[name];}},isConnected:true,querySelector:()=>null,querySelectorAll:()=>[],setAttribute(){},removeAttribute(){},replaceChildren(){this.innerHTML='';},focus(){}});
const moduleCache=new Map();
const sourceCache=new Map();
function sourceSnapshot(file){if(!sourceCache.has(file))sourceCache.set(file,readFile(file,'utf8'));return sourceCache.get(file);}
async function sourceModule(directory,name){
  const file=resolve(directory,name);
  if(!moduleCache.has(file))moduleCache.set(file,sourceSnapshot(file).then(source=>import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'))));
  return moduleCache.get(file);
}
export const fixtureUser={id:'fixture-user',role:'seeker',name:'Synthetic QA member',email:'member@example.test',public_email:'',resume_md:'QA_PROFILE_READY',employer_paid:false};
export const fixtureConfig={name:'HireFerry QA',default_language:'en',humangrant_enabled:false,google_enabled:false,email_enabled:true,development:false,pricing:{currency:'CNY',currency_decimals:2,onboarding_cents:10000,publish_mode:'salary_percent',publish_bps:1000,edit_mode:'salary_percent',edit_bps:100,max_edits:3,allowed_currencies:['CNY']},limits:{list_page_size:50,workbench_page_size:25},providers:[]};

export async function createNavigationHarness(directory,{latencyMs=0,mountChat=false,beforeResponse=()=>{},initialStorage=[],useFixtureConfig=true}={}) {
  const [source,services,money,navigation,humanGrant,requests,cliAuth,team,cliGuide,billing,promo,staff,browserCache,cliMotion,controls,finance,peopleJobs,seo,webmcp]=await Promise.all([
    sourceSnapshot(resolve(directory,'app.js')),sourceModule(directory,'service-ui.js'),sourceModule(directory,'money.js'),sourceModule(directory,'navigation.js'),sourceModule(directory,'humangrant-ui.js'),sourceModule(directory,'requests.js').catch(()=>({})),sourceModule(directory,'cli-auth-return.js').catch(error=>{if(error.code==='ENOENT')return {};throw error;}),
    sourceModule(directory,'team-ui.js'),sourceModule(directory,'cli-guide.js'),sourceModule(directory,'billing-ui.js'),sourceModule(directory,'promo-ui.js'),sourceModule(directory,'staff-ui.js'),sourceModule(directory,'browser-cache.js'),sourceModule(directory,'cli-motion.js'),sourceModule(directory,'controls.js'),sourceModule(directory,'finance-ui.js'),sourceModule(directory,'people-jobs-ui.js'),sourceModule(directory,'seo.js'),sourceModule(directory,'webmcp-lifecycle.js'),
  ]);
  const sourceHashes=Object.fromEntries(await Promise.all(['app.js','service-ui.js','requests.js','cli-auth-return.js','billing-ui.js','disclosure.js'].map(async name=>[name,await sourceSnapshot(resolve(directory,name)).then(value=>createHash('sha256').update(value).digest('hex')).catch(()=>null)])));
  const root=node(),dialogs=node(),toast=node(),region=node();
  root.querySelector=selector=>['.app-content','main'].includes(selector)?region:null;
  const listeners=new Map();
  const listen=(type,fn)=>{if(!listeners.has(type))listeners.set(type,[]);listeners.get(type).push(fn);};
  const document={hidden:false,title:'',body:node(),documentElement:node(),activeElement:null,addEventListener:listen,removeEventListener(){},getElementById:()=>null,querySelector:selector=>({'#root':root,'#dialog-root':dialogs,'#toast-root':toast}[selector]||null),querySelectorAll:()=>[]};
  const head=[];
  const bodyQuery=document.querySelector;
  document.head={append:element=>head.push(element)};
  document.createElement=tag=>({tag,setAttribute(name,value){this[name]=value;},remove(){head.splice(head.indexOf(this),1);}});
  document.querySelector=selector=>bodyQuery(selector)||head.find(element=>{const match=selector.match(/^(meta|link)\[(name|property|rel)="([^"]+)"\]$/);return match&&element.tag===match[1]&&element[match[2]]===match[3];})||null;
  let url=new URL('https://hireferry.test/contact');
  const location={get pathname(){return url.pathname;},get search(){return url.search;},get hash(){return url.hash;},get href(){return url.href;},get origin(){return url.origin;},assign(value){url=new URL(value,url);}};
  const history={pushState(_state,_title,value){url=new URL(value,url);}};
  const savedDocument=globalThis.document,savedLocation=globalThis.location;
  globalThis.document=document;globalThis.location=location;
  const storage=new Map([['hireferry-language','en'],...initialStorage]);
  const localStorage={getItem:key=>storage.get(key)||null,setItem:(key,value)=>storage.set(key,value),removeItem:key=>storage.delete(key)};
  const network=[],mounts=[],stops=[];
  const overrides=new Map();
  let currentUser=structuredClone(fixtureUser);
  const conversation={id:'platform_fixture-user',kind:'platform',user_id:'fixture-user',title:'HireFerry team',writable:true,participants:[fixtureUser],unread_count:0,last_message_preview:'Synthetic preview',last_message_at:1800000000};
  function response(path){
    if(overrides.has(path)){const value=overrides.get(path);return typeof value==='function'?value():value;}
    if(path==='/api/config')return fixtureConfig;
    if(path==='/api/session')return {user:currentUser,admin:false,bootstrap:false,csrf:currentUser?'fixture-csrf':null};
    if(path==='/api/profile')return fixtureUser;
    if(path==='/api/mandate')return {mandate:{user_id:'fixture-user',target_roles:'QA_MANDATE_READY',target_industry:'Software',status:'active',salary_currency:'CNY',salary_min_cents:2000000,salary_max_cents:3000000,allow_external:false,preferences:'Synthetic fixture'}};
    if(path==='/api/outreach')return {items:[]};
    if(path==='/api/chat/platform'||path==='/api/chat/conversations/platform_fixture-user')return conversation;
    if(path==='/api/chat/conversations')return {items:[conversation]};
    if(path.startsWith('/api/chat/conversations/platform_fixture-user/messages'))return {items:[{id:1,sender_kind:'platform',sender_id:null,sender_name:'HireFerry',body:'QA_MESSAGE_READY',body_html:'<p>QA_MESSAGE_READY</p>',created_at:1800000000}],next_cursor:1,first_cursor:1,has_older:false};
    if(path==='/api/ai/eligibility')return {eligible:false};
    return {items:[]};
  }
  const fetch=async(path,options={})=>{
    const record={path,method:options.method||'GET',began:performance.now(),finished:null,aborted:false};network.push(record);
    const gate=beforeResponse(path,record);
    await new Promise((yes,no)=>{
      const fail=()=>{clearTimeout(timer);record.aborted=true;record.finished=performance.now();no(options.signal?.reason||new DOMException('Canceled','AbortError'));};
      const timer=setTimeout(()=>{Promise.resolve(gate).then(()=>{options.signal?.removeEventListener('abort',fail);yes();},no);},latencyMs);
      if(options.signal?.aborted)fail();else options.signal?.addEventListener('abort',fail,{once:true});
    });
    const data=response(path);record.finished=performance.now();
    const status=data?.httpStatus||200;
    return {ok:status>=200&&status<300,status,json:async()=>structuredClone(data?.httpStatus?data.body:data)};
  };
  const mountPage=(target,html,pageKey,{busy=false}={})=>{target.innerHTML=html;target.dataset.pageKey=pageKey;target.classList.toggle('page-pending',busy);mounts.push({at:performance.now(),pageKey,busy,html,requestCount:network.length});};
  const createServiceUI=dependencies=>{
    const service=services.createServiceUI(dependencies);
    const original=service.stopChat;
    service.stopChat=()=>{stops.push({user:dependencies.state.user?.id||null,admin:dependencies.state.admin});original();};
    // Browser event binding/polling is outside this route delivery benchmark.
    if(!mountChat)service.mountChat=()=>{};
    return service;
  };
  // This HTML sink has no layout or native disclosure events. Their behavior is
  // exercised with a dedicated event fixture in disclosure.test.mjs.
  const createDisclosureUI=()=>({sync(){},reset(){},dispose(){}});
  const createPublicConfigCache=load=>browserCache.createPublicConfigCache(load,{storage:localStorage});
  // The dedicated content-refresh suite and browser gate exercise actual DOM animation.
  const animateContentRefresh=async(_root,_html,commit,{isCurrent})=>{if(!isCurrent())return false;commit();return true;};
  const dependencies={document,navigator:{},window:{addEventListener:listen,scrollTo(){}},localStorage,sessionStorage:localStorage,location,history,fetch,requestAnimationFrame:callback=>setImmediate(()=>callback(performance.now())),...services,...money,...navigation,...humanGrant,...requests,...cliAuth,...team,...cliGuide,...billing,...promo,...staff,...browserCache,...cliMotion,...controls,...finance,...peopleJobs,...seo,...webmcp,createPublicConfigCache,animateContentRefresh,mountPage,createServiceUI,createDisclosureUI,renderLegal:()=>''};
  const executable=source.replace(/^import .*;\n/gm,'').replace(/render\(\);\s*$/,'');
  const app=new Function(...Object.keys(dependencies),executable+'\nreturn {state,api,session,render,go,service,configCache,refreshPublicPage,refreshVisibleSession:typeof refreshVisibleSession==="function"?refreshVisibleSession:null};')(...Object.values(dependencies));
  if(useFixtureConfig)Object.assign(app.state,{config:structuredClone(fixtureConfig),lang:'en'});
  return {app,root,document,network,mounts,stops,location,overrides,sourceHashes,localStorage,setUser:user=>{currentUser=user;},dispatch(type,event={}){for(const listener of listeners.get(type)||[])listener(event);},async prime(){await app.session();await app.render();network.length=0;mounts.length=0;},async route(target,marker){
    const began=performance.now(),start=network.length,mountStart=mounts.length;
    await app.go(target);
    const relevant=mounts.slice(mountStart),content=relevant.find(value=>!value.busy&&value.html.includes(marker));
    if(!content)throw new Error(`Route did not publish ${marker}: ${root.innerHTML.slice(0,400)}`);
    return {shellMs:relevant[0]?.at-began,contentMs:content.at-began,completedMs:performance.now()-began,requestsBeforeContent:content.requestCount-start,requests:network.slice(start).map(value=>({path:value.path,method:value.method,aborted:value.aborted,beganMs:value.began-began,finishedMs:value.finished===null?null:value.finished-began}))};
  },dispose(){app.service.stopChat();globalThis.document=savedDocument;globalThis.location=savedLocation;}};
}

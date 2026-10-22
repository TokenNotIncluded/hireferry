// Optional browser gate using only synthetic public configuration and guest sessions.
// No Rust build or real user account is needed. Run with installed Playwright:
// HFY_PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs \
// HFY_BROWSER_EXECUTABLE=/path/to/chromium \
// HFY_BROWSER_ARGS_MODULE=/path/to/chromium/build/index.js \
// HFY_BROWSER_SCREENSHOTS=/tmp/hfy-cli-shots node tests/cli-polish-browser.mjs
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir} from 'node:fs/promises';
import {dirname,resolve,isAbsolute,extname} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {setTimeout as pause} from 'node:timers/promises';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const load=specifier=>import(isAbsolute(specifier)?pathToFileURL(specifier).href:specifier);
const {chromium}=await load(process.env.HFY_PLAYWRIGHT_MODULE||'playwright');
const screenshots=process.env.HFY_BROWSER_SCREENSHOTS;
if(screenshots)await mkdir(screenshots,{recursive:true});
let configDelay=0,sessionDelay=0,holdConfig=false,holdSession=false;
const waitingConfig=new Set(),waitingSession=new Set();
function releaseNetwork(){holdConfig=false;holdSession=false;for(const release of [...waitingConfig,...waitingSession])release();waitingConfig.clear();waitingSession.clear();}
let config={name:'HireFerry · 聘渡',public_origin:'https://hireferry.lmm.best',default_language:'zh',
  google_enabled:false,humangrant_enabled:false,email_enabled:true,development:false,
  cli_distribution:{release_repository:'TokenNotIncluded/hireferry',release_tag_prefix:'cli-v'},
  pricing:{currency:'CNY',currency_decimals:2},limits:{list_page_size:50},providers:[]};
const unexpected=[],pageErrors=[],metrics={};
const contentTypes={'.js':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.svg':'image/svg+xml','.html':'text/html; charset=utf-8','.json':'application/json'};
const server=createServer(async(req,res)=>{
  const url=new URL(req.url,'http://fixture.local');
  try {
    if(url.pathname==='/api/config'||url.pathname==='/api/session') {
      const isConfig=url.pathname==='/api/config',payload=isConfig?structuredClone(config):{user:null,admin:false,super_admin:false,staff:null,staff_role:null,bootstrap:false,csrf:null};
      const waiting=isConfig?waitingConfig:waitingSession;
      if(isConfig?holdConfig:holdSession)await new Promise(resolve_=>waiting.add(resolve_));
      await pause(isConfig?configDelay:sessionDelay);
      res.writeHead(200,{'content-type':'application/json','cache-control':'no-store'});res.end(JSON.stringify(payload));return;
    }
    if(url.pathname.startsWith('/api/')) {unexpected.push(url.pathname);res.writeHead(403);res.end('{}');return;}
    const relative=url.pathname.startsWith('/static/')?url.pathname.slice(1):'static/index.html';
    const file=resolve(root,relative);
    if(!file.startsWith(resolve(root,'static')+'/')){res.writeHead(404);res.end();return;}
    const content=await readFile(file);
    res.writeHead(200,{'content-type':contentTypes[extname(file)]||'application/octet-stream','cache-control':'no-store'});res.end(content);
  }catch{res.writeHead(404);res.end();}
});
await new Promise((resolve_,reject)=>{server.once('error',reject);server.listen(0,'127.0.0.1',resolve_);});
const origin=`http://127.0.0.1:${server.address().port}`;
let browser;
try {
  let args=['--no-sandbox'];
  if(process.env.HFY_BROWSER_ARGS_MODULE){const module=await load(process.env.HFY_BROWSER_ARGS_MODULE);args=(module.default||module).args;}
  args=args.filter(value=>!['--disable-web-security','--disable-site-isolation-trials','--allow-running-insecure-content'].includes(value.split('=')[0]));
  args.push('--disk-cache-size=1048576','--media-cache-size=1048576');
  browser=await chromium.launch({headless:true,executablePath:process.env.HFY_BROWSER_EXECUTABLE||undefined,args});
  async function contextFor(theme='dark',width=1440,reducedMotion='no-preference') {
    const context=await browser.newContext({viewport:{width,height:980},reducedMotion,locale:'zh-CN'});
    await context.route('**/*',route=>new URL(route.request().url()).origin===origin?route.continue():route.abort());
    await context.addInitScript(value=>{if(!localStorage.getItem('hireferry-theme'))localStorage.setItem('hireferry-theme',value);localStorage.setItem('hireferry-palette','neutral');},theme);
    const page=await context.newPage();
    page.on('pageerror',error=>pageErrors.push(error.message));
    return {context,page};
  }
  async function ready(page){await page.locator('.cli-guide').waitFor();await page.waitForFunction(()=>document.querySelector('#root')?.dataset.navigationState==='ready');}
  async function noOverflow(page,label){const size=await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth,body:document.body.scrollWidth}));assert.ok(size.scroll<=size.width+1&&size.body<=size.width+1,`${label}: horizontal overflow ${JSON.stringify(size)}`);}

  const {context,page}=await contextFor();
  await context.grantPermissions(['clipboard-read','clipboard-write'],{origin});
  configDelay=100;holdSession=true;
  let started=Date.now();await page.goto(origin+'/cli');await ready(page);
  metrics.cold_first_content_ms=Date.now()-started;
  assert.equal(await page.locator('#root').getAttribute('data-config-state'),'checking','public CLI should paint while its session request is held');
  releaseNetwork();
  await page.waitForFunction(()=>document.querySelector('[data-cli-typed]')?.textContent.length>3);
  await page.locator('[data-cli-motion-toggle]').click();
  let text=await page.locator('[data-cli-typed]').textContent();await pause(150);
  assert.equal(await page.locator('[data-cli-typed]').textContent(),text,'paused demo continued typing');
  assert.equal(await page.locator('.cli-cursor').evaluate(node=>getComputedStyle(node).animationName),'none');
  await page.locator('[data-cli-motion-toggle]').click();
  await page.waitForFunction(previous=>document.querySelector('[data-cli-typed]').textContent!==previous,text);
  await page.locator('#cli-skill').scrollIntoViewIfNeeded();
  await page.waitForFunction(()=>document.querySelector('[data-cli-terminal]').dataset.paused==='true');
  text=await page.locator('[data-cli-typed]').textContent();await pause(150);
  assert.equal(await page.locator('[data-cli-typed]').textContent(),text,'off-screen demo continued typing');
  await page.evaluate(()=>scrollTo(0,0));
  await page.waitForFunction(()=>document.querySelector('[data-cli-terminal]').dataset.paused==='false');

  await page.locator('#cli-tab-windows').click();
  assert.equal(await page.locator('#cli-panel-windows').isVisible(),true);
  await page.locator('#cli-tab-windows').press('ArrowLeft');
  assert.equal(await page.locator('#cli-tab-unix').getAttribute('aria-selected'),'true');
  await page.locator('#cli-tab-unix').press('ArrowRight');
  await page.locator('#cli-panel-windows .cli-copy').click();
  assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),`irm ${config.public_origin}/install.ps1 | iex`);
  assert.equal(await page.locator('#cli-panel-windows .cli-copy').getAttribute('aria-label'),'已复制');
  await noOverflow(page,'desktop interactions');
  process.stdout.write('ok - typing, pause, off-screen suspension, keyboard platform tabs, complete clipboard copy\n');

  await page.waitForFunction(()=>document.querySelector('#root').dataset.configState==='fresh');
  configDelay=0;holdConfig=true;holdSession=true;
  started=Date.now();await page.reload();await ready(page);
  metrics.warm_first_content_ms=Date.now()-started;
  assert.equal(await page.locator('#root').getAttribute('data-config-state'),'checking','cached CLI should paint while both requests are held');
  await pause(Math.max(0,1400-(Date.now()-started)));releaseNetwork();
  await page.waitForFunction(()=>document.querySelector('#root').dataset.configState==='fresh');
  process.stdout.write('ok - cached public page paints before delayed config and session requests\n');

  config={...config,public_origin:'https://updated.hireferry.example'};configDelay=900;sessionDelay=0;
  await page.reload();await ready(page);
  await page.locator('#cli-tab-windows').click();
  await page.locator('[data-cli-motion-toggle]').click();
  await page.waitForFunction(()=>document.querySelector('[data-refresh-key="cli-install-windows"]').classList.contains('content-refresh-out'));
  await page.waitForFunction(()=>document.querySelector('[data-refresh-key="cli-install-windows"]').textContent.includes('updated.hireferry.example'));
  assert.equal(await page.locator('#cli-tab-windows').getAttribute('aria-selected'),'true','refresh lost chosen platform');
  assert.equal(await page.locator('[data-cli-motion-toggle]').getAttribute('aria-pressed'),'true','refresh resumed a manually paused demo');
  await page.locator('#cli-panel-windows .cli-copy').click();
  assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),`irm ${config.public_origin}/install.ps1 | iex`);
  await page.waitForFunction(()=>!document.querySelector('.content-refresh-out,.content-refresh-in'));
  process.stdout.write('ok - changed cached command animates, updates copy data, and retains selected platform\n');
  process.stdout.write(JSON.stringify({metrics,fixture:true})+'\n');

  config={...config,public_origin:'https://hireferry.lmm.best'};configDelay=0;sessionDelay=0;
  for(const theme of ['light','dark']) {
    for(const width of [1440,390,320]) {
      const p=page;
      await p.setViewportSize({width,height:980});await p.emulateMedia({reducedMotion:'reduce'});
      await p.evaluate(value=>localStorage.setItem('hireferry-theme',value),theme);
      await p.goto(origin+'/cli');await ready(p);
      await p.waitForFunction(()=>document.querySelector('#root').dataset.configState==='fresh');
      assert.equal(await p.locator('html').getAttribute('data-theme'),theme);
      const command=await p.locator('[data-cli-typed]').textContent();
      assert.ok(command.includes('auth login --device'),'reduced motion did not show complete command');
      await pause(100);assert.equal(await p.locator('[data-cli-typed]').textContent(),command);
      assert.equal(await p.locator('.cli-cursor').evaluate(node=>getComputedStyle(node).animationName),'none');
      await p.locator('#cli-tab-windows').click();await noOverflow(p,`${theme} ${width} Windows`);
      await p.locator('#cli-tab-unix').click();await noOverflow(p,`${theme} ${width} Unix`);
      await p.locator('.cli-details').first().locator('summary').click();
      await p.locator('.cli-skill-details summary').click();await noOverflow(p,`${theme} ${width} expanded`);
      await p.locator('.cli-details').first().locator('summary').click();
      await p.locator('.cli-skill-details summary').click();await p.evaluate(()=>scrollTo(0,0));
      if(screenshots&&width!==320) {
        const desktop=width===1440,extension=(desktop&&theme==='light')||(!desktop&&theme==='dark')?'png':'jpg';
        const path=resolve(screenshots,`cli-${desktop?'desktop':'mobile'}-${theme}.${extension}`);
        await p.screenshot({path,fullPage:true,type:extension==='jpg'?'jpeg':'png',...(extension==='jpg'?{quality:68}:{}),animations:'disabled'});
      }
      process.stdout.write(`ok - ${theme} ${width}px, reduced motion, both platforms, expanded sections, no overflow\n`);
    }
  }
  assert.deepEqual(unexpected,[],'fixture received unexpected API requests');
  assert.deepEqual(pageErrors,[],'browser reported uncaught JavaScript errors');
  process.stdout.write(JSON.stringify({metrics,screenshots: screenshots||null})+'\n');
}finally{releaseNetwork();await browser?.close();server.closeAllConnections();await new Promise(resolve_=>server.close(resolve_));}

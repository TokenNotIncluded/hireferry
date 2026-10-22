// Real SPA, synthetic guest config and release records. No external requests or account data.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir} from 'node:fs/promises';
import {dirname,resolve,isAbsolute,extname} from 'node:path';
import {fileURLToPath,pathToFileURL} from 'node:url';
import {catalogFixture as snapshot} from './helpers/downloads-fixture.mjs';

const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
const load=path=>import(isAbsolute(path)?pathToFileURL(path).href:path);
const {chromium}=await load(process.env.HFY_PLAYWRIGHT_MODULE||'playwright');
const repository='https://github.com/TokenNotIncluded/hireferry/releases';
const catalogRelease={tag_name:'cli-v0.1.4',html_url:repository+'/tag/cli-v0.1.4',draft:false,prerelease:false,published_at:'2026-10-22T00:00:00Z',assets:snapshot.assets.filter(a=>a.product==='cli').map(a=>({name:a.name.replace('0.1.3','0.1.4'),size:a.size,state:'uploaded',digest:'sha256:'+a.sha256,browser_download_url:a.url.replaceAll('0.1.3','0.1.4')}))};
const config={name:'HireFerry · 聘渡',public_origin:'https://hireferry.lmm.best',default_language:'zh',source_url:'https://github.com/TokenNotIncluded/hireferry',google_enabled:false,humangrant_enabled:false,email_enabled:true,development:false,cli_distribution:{release_repository:'TokenNotIncluded/hireferry',release_tag_prefix:'cli-v'},pricing:{currency:'CNY',currency_decimals:2},limits:{list_page_size:50},providers:[]};
const contentTypes={'.js':'text/javascript; charset=utf-8','.css':'text/css; charset=utf-8','.svg':'image/svg+xml','.html':'text/html; charset=utf-8','.json':'application/json'};
let manifestAvailable=true,manifestReads=0,externalReads=0,externalStatus=200;
const errors=[],unexpected=[];
const server=createServer(async(req,res)=>{
  const url=new URL(req.url,'http://fixture.test');
  try{
    res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' https://api.github.com; font-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'");
    if(url.pathname==='/api/config'||url.pathname==='/api/session'){
      const data=url.pathname==='/api/config'?config:{user:null,admin:false,super_admin:false,staff:null,staff_role:null,bootstrap:false,csrf:null};
      res.writeHead(200,{'content-type':'application/json'});res.end(JSON.stringify(data));return;
    }
    if(url.pathname==='/static/downloads.json'){
      manifestReads++;res.writeHead(manifestAvailable?200:404,{'content-type':'application/json','cache-control':'no-cache'});res.end(JSON.stringify(manifestAvailable?snapshot:{}));return;
    }
    if(url.pathname.startsWith('/api/')){unexpected.push(url.pathname);res.writeHead(404);res.end('{}');return;}
    const file=resolve(root,url.pathname.startsWith('/static/')?url.pathname.slice(1):'static/index.html');
    if(!file.startsWith(resolve(root,'static')+'/'))throw new Error('outside static');
    res.writeHead(200,{'content-type':contentTypes[extname(file)]||'application/octet-stream','cache-control':'no-store'});res.end(await readFile(file));
  }catch{res.writeHead(404);res.end();}
});
await new Promise((yes,no)=>{server.once('error',no);server.listen(0,'127.0.0.1',yes);});
const origin=`http://127.0.0.1:${server.address().port}`;
let browser;
try{
  browser=await chromium.launch({headless:true,executablePath:process.env.HFY_BROWSER_EXECUTABLE||undefined,args:['--no-sandbox','--disk-cache-size=1048576','--media-cache-size=1048576']});
  async function createContext(){
    const context=await browser.newContext({viewport:{width:1440,height:1050},reducedMotion:'reduce',locale:'zh-CN'});
    await context.route('**/*',async route=>{
      const url=new URL(route.request().url());
      if(url.origin===origin)return route.continue();
      if(url.href==='https://api.github.com/repos/TokenNotIncluded/hireferry/releases?per_page=10'){
        externalReads++;return route.fulfill({status:externalStatus,contentType:'application/json',headers:{'access-control-allow-origin':'*','x-ratelimit-reset':String(Math.floor(Date.now()/1000)+600)},body:JSON.stringify(externalStatus===200?[catalogRelease]:{})});
      }
      if(url.href==='https://api.github.com/repos/TokenNotIncluded/hireferry')return route.fulfill({contentType:'application/json',headers:{'access-control-allow-origin':'*'},body:JSON.stringify({stargazers_count:423})});
      unexpected.push(url.href);return route.abort();
    });
    await context.addInitScript(()=>{localStorage.setItem('hireferry-theme','light');localStorage.setItem('hireferry-palette','neutral');});
    const page=await context.newPage();page.on('pageerror',error=>errors.push(error.message));return {context,page};
  }
  async function ready(page){await page.locator('[data-downloads-page]').waitFor();await page.waitForFunction(()=>document.querySelector('[data-download-results]')?.getAttribute('aria-busy')==='false');await page.waitForFunction(()=>document.querySelector('#root')?.dataset.configState==='fresh');}
  async function noOverflow(page,label){const bounds=await page.evaluate(()=>({viewport:innerWidth,document:document.documentElement.scrollWidth,body:document.body.scrollWidth}));assert.ok(bounds.document<=bounds.viewport+1&&bounds.body<=bounds.viewport+1,`${label} overflow ${JSON.stringify(bounds)}`);}
  const {context,page}=await createContext();
  await page.goto(origin+'/cli');await page.locator('.cli-guide').waitFor();
  assert.equal(await page.locator('.cli-download').getAttribute('href'),'/downloads');
  await page.locator('.cli-download').click();await ready(page);
  assert.equal(await page.locator('[data-download-asset]').count(),8);assert.equal(externalReads,0);assert.equal(manifestReads,1);
  for(const product of ['desktop','android','ios']){
    const section=page.locator(`[data-download-product="${product}"]`);
    assert.match(await section.textContent(),/尚未发布/);assert.equal(await section.locator('.download-file').count(),0);
  }
  await page.locator('[data-download-platform="Windows"]').click();
  assert.equal(await page.locator('[data-download-asset]').count(),2);
  await page.locator('[data-action="language"]').click();await ready(page);
  assert.equal(await page.locator('[data-download-platform="Windows"]').getAttribute('aria-pressed'),'true');
  assert.equal(await page.locator('[data-download-asset]').count(),2);assert.equal(await page.locator('#downloads-title').textContent(),'Choose how you work.');
  await page.locator('.download-actions summary').first().click();
  assert.match(await page.locator('.download-file-info code').first().textContent(),/^[0-9a-f]{64}$/);
  await page.locator('[data-download-platform="Android"]').focus();await page.keyboard.press('Enter');
  assert.equal(await page.locator('[data-download-asset]').count(),0);assert.equal(await page.locator('[data-download-product]').count(),1);
  assert.equal(await page.locator('[data-download-platform="Android"]').getAttribute('aria-pressed'),'true');
  await page.locator('[data-download-platform="all"]').click();
  process.stdout.write('ok - public navigation, exact eight packages, unpublished native platforms, keyboard filters and language preservation\n');

  const screenshots=process.env.HFY_BROWSER_SCREENSHOTS;
  if(screenshots)await mkdir(screenshots,{recursive:true});
  for(const theme of ['light','dark'])for(const width of [1440,390,320]){
    await page.setViewportSize({width,height:1050});
    await page.evaluate(value=>document.documentElement.dataset.theme=value,theme);
    await noOverflow(page,`${theme} ${width}`);
    await page.locator('.download-actions summary').first().click();await noOverflow(page,`${theme} ${width} checksum`);
    await page.locator('.download-actions summary').first().click();
    if(screenshots&&((theme==='light'&&width===1440)||(theme==='dark'&&width===390)))await page.screenshot({path:resolve(screenshots,`downloads-${theme}-${width}.png`),fullPage:true,animations:'disabled'});
  }
  process.stdout.write('ok - light/dark at 1440, 390 and 320 pixels, expanded checksums without horizontal overflow\n');

  await page.setViewportSize({width:1440,height:1050});
  await page.locator('[data-download-refresh]').click();await page.waitForFunction(()=>document.querySelector('[data-download-product="cli"] .download-version')?.textContent==='v0.1.4');
  assert.equal(externalReads,1);assert.equal(await page.locator('[data-download-product="server"] [data-download-asset]').count(),2);
  await page.locator('[data-download-refresh]').click();assert.equal(externalReads,1);
  await page.goto(origin+'/');await page.locator('[data-github-stars]').waitFor();
  await page.waitForFunction(()=>document.querySelector('[data-github-stars]')?.textContent==='423');
  await page.locator('.site-header .nav a[href="/downloads"]').click();await ready(page);assert.equal(await page.locator('[data-download-asset]').count(),8);
  await context.close();
  process.stdout.write('ok - explicit bounded GitHub update retains server release, caches checks, and preserves home star behavior\n');

  manifestAvailable=false;
  const missing=await createContext();await missing.page.goto(origin+'/downloads');await ready(missing.page);
  assert.equal(await missing.page.locator('[data-download-asset]').count(),0);assert.equal(externalReads,1);
  assert.match(await missing.page.locator('[data-download-source]').textContent(),/无法读取发布清单/);
  await missing.page.locator('[data-download-refresh]').click();await missing.page.locator('[data-download-asset]').first().waitFor();
  assert.equal(await missing.page.locator('[data-download-asset]').count(),6);assert.equal(externalReads,2);
  await missing.context.close();
  process.stdout.write('ok - missing local catalog makes no automatic API call and recovers on explicit check\n');

  manifestAvailable=true;externalStatus=403;
  const limited=await createContext();await limited.page.goto(origin+'/downloads');await ready(limited.page);
  await limited.page.locator('[data-download-refresh]').click();await limited.page.waitForFunction(()=>document.querySelector('[data-download-source]')?.textContent.includes('限制请求'));
  assert.equal(await limited.page.locator('[data-download-asset]').count(),8);assert.equal(externalReads,3);
  await limited.page.reload();await ready(limited.page);await limited.page.locator('[data-download-refresh]').click();
  assert.equal(externalReads,3);assert.equal(await limited.page.locator('[data-download-asset]').count(),8);
  await limited.context.close();
  assert.deepEqual(unexpected,[]);assert.deepEqual(errors,[]);
  process.stdout.write('ok - rate-limited refresh retains eight packages and respects backoff after reload; no unexpected requests/errors\n');
}finally{await browser?.close();await new Promise(resolve_=>server.close(resolve_));}

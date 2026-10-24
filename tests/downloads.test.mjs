import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {catalogFixture as snapshot} from './helpers/downloads-fixture.mjs';

const source = await readFile(new URL('../static/downloads.js',import.meta.url),'utf8');
const {normalizeDownloadManifest:normalize,normalizeGitHubReleases:github,mergeDownloadCatalogs:merge,createDownloadSource,renderDownloads,DOWNLOAD_RELEASES} = await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const memory = () => {const values=new Map();return {getItem:key=>values.get(key)||null,setItem:(key,value)=>values.set(key,value)};};
const response = (data,status=200,headers={}) => new Response(JSON.stringify(data),{status,headers:{'content-type':'application/json',...headers}});
function release(version='0.1.4',extra={}) {
  const tag='cli-v'+version;
  return {tag_name:tag,html_url:DOWNLOAD_RELEASES+'/tag/'+tag,draft:false,prerelease:false,published_at:'2026-10-22T00:00:00Z',
    assets:snapshot.assets.filter(a=>a.product==='cli').map(a=>({name:a.name.replace('0.1.3',version),size:a.size,state:'uploaded',digest:'sha256:'+a.sha256,browser_download_url:a.url.replaceAll('0.1.3',version)})),...extra};
}

test('the deployed small catalog is exactly the published snapshot, never the planned targets',async()=>{
  const deployed=JSON.parse(await readFile(new URL('../static/downloads.json',import.meta.url),'utf8'));
  const published=JSON.parse(await readFile(new URL('../release/latest.json',import.meta.url),'utf8'));
  assert.deepEqual(deployed,published);
  const catalog=normalize(deployed);
  assert.equal(catalog.assets.length,published.assets.length);
  for(const asset of catalog.assets) assert.equal(catalog.versions[asset.product],asset.version);
});

test('engineering manifests, external URLs, mismatched formats and tags fail closed',()=>{
  for(const mutate of [
    x=>x.distribution='engineering',x=>x.repository='Other/repo',x=>x.schema_version=2,
    x=>x.assets[0].url='https://example.test/untrusted.zip',x=>x.assets[0].url+='?token=secret',
    x=>x.assets[0].tag='cli-v0.1.3-rc.1',x=>x.assets[0].version='0.1.4',
    x=>x.assets[0].format='exe',x=>x.assets[0].sha256='not-a-checksum',
    x=>x.assets.push(x.assets[0]),
  ]) { const value=structuredClone(snapshot);mutate(value);assert.throws(()=>normalize(value),/invalid_manifest/); }
});

test('GitHub fallback accepts uploaded assets of published stable releases only',()=>{
  const valid=release();
  const older=release('0.1.2');
  const catalog=github([release('0.2.0',{draft:true}),release('0.3.0',{prerelease:true}),release('0.4.0',{published_at:null}),valid,older]);
  assert.equal(catalog.versions.cli,'0.1.4');assert.equal(catalog.assets.length,6);
  const invalid=release();
  invalid.assets[0].browser_download_url='https://evil.test/pkg';
  invalid.assets[1].state='new';
  invalid.assets[2].name='hireferry-cli-0.1.4-debug-aarch64-unknown-linux-musl.tar.gz';
  assert.equal(github([invalid]).assets.length,3);
  assert.throws(()=>github(Array(11).fill(valid)),/invalid_manifest/);
});

test('one recent API page cannot downgrade or remove another product from the snapshot',()=>{
  const base=normalize(snapshot);
  assert.deepEqual(merge(base,github([release('0.1.2')])),base);
  const updated=merge(base,github([release()]));
  assert.equal(updated.versions.cli,'0.1.4');assert.equal(updated.versions.server,'0.1.0');assert.equal(updated.assets.length,8);
  assert.deepEqual(merge(updated,github([])),updated);
});

test('normal page load uses one same-origin manifest and never requests a GitHub attachment list',async()=>{
  const calls=[];
  const source=createDownloadSource({fetcher:async(url,options)=>{calls.push({url,options});return response(snapshot);}});
  const [first,second]=await Promise.all([source.load(),source.load()]);
  assert.equal(first,second);assert.equal(first.source,'snapshot');assert.equal(first.catalog.assets.length,8);
  await source.load();assert.equal(calls.length,1);assert.equal(calls[0].url,'/static/downloads.json');
  assert.equal(calls[0].options.credentials,'omit');
});

test('missing manifest stays explicit until a user requests a bounded GitHub check; cached result avoids re-fetch',async()=>{
  let now=1800000000000;const calls=[],storage=memory();
  const source=createDownloadSource({storage,now:()=>now,fetcher:async url=>{calls.push(url);return url.startsWith('/')?response({},404):response([release()]);}});
  assert.equal((await source.load()).catalog,null);assert.equal(calls.length,1);
  const fresh=await source.refresh();assert.equal(fresh.status,'checked');assert.equal(fresh.catalog.versions.cli,'0.1.4');
  assert.equal(calls[1],'https://api.github.com/repos/TokenNotIncluded/hireferry/releases?per_page=10');
  assert.equal((await source.refresh()).status,'cached');assert.equal(calls.length,2);
  const cached=createDownloadSource({storage,now:()=>now,fetcher:async url=>{calls.push(url);return response({},404);}});
  assert.equal((await cached.load()).catalog.assets.length,6);assert.equal((await cached.refresh()).status,'cached');
  assert.equal(calls.length,3);
  now+=8*24*60*60*1000;
  const expired=createDownloadSource({storage,now:()=>now,fetcher:async()=>response({},404)});
  assert.equal((await expired.load()).catalog,null);
});

test('GitHub rate limits retain the catalog and block repeated checks across a reload',async()=>{
  const now=1800000000000,storage=memory();let external=0;
  const fetcher=async url=>url.startsWith('/')?response(snapshot):(external++,response({},403,{'x-ratelimit-reset':String(now/1000+600)}));
  const first=createDownloadSource({storage,now:()=>now,fetcher});await first.load();
  const limited=await first.refresh();assert.equal(limited.status,'rate_limited');assert.equal(limited.catalog.assets.length,8);
  await first.refresh();assert.equal(external,1);
  const second=createDownloadSource({storage,now:()=>now,fetcher});await second.load();
  assert.equal((await second.refresh()).status,'rate_limited');assert.equal(external,1);
});

test('oversized responses fail closed and network failures are not mislabeled as rate limits',async()=>{
  const source=createDownloadSource({fetcher:async url=>url.startsWith('/')?response(snapshot):response([],200,{'content-length':String(2*1024*1024)})});
  await source.load();assert.equal((await source.refresh()).status,'unavailable');
  assert.equal((await source.refresh()).status,'unavailable');assert.equal(source.peek().catalog.assets.length,8);
});

test('public page distinguishes AUR source recipes, checksum integrity, and explicit update checks in both languages',()=>{
  for(const lang of ['zh','en']){
    const html=renderDownloads({lang});
    assert.match(html,/data-downloads-page/);assert.match(html,/data-download-refresh/);assert.match(html,/data-download-platform="Android"/);
    assert.match(html,/data-download-platform="iOS"/);assert.match(html,/href="\/cli" data-nav/);
    assert.match(html,/AUR/);assert.doesNotMatch(html,/actions\/runs|latest\/download|href="[^"]+\.(apk|ipa)"/);
  }
});

test('downloads page exposes mobile web and Nix channels without inventing native assets',()=>{
  const html=renderDownloads({lang:'zh',icon:name=>`<i data-icon="${name}"></i>`});
  assert.match(html,/移动版与 Nix/);
  assert.match(html,/当前没有原生 APK \/ IPA/);
  assert.match(html,/nix profile install github:TokenNotIncluded\/hireferry#cli/);
  assert.match(html,/docs\/mobile\.md/);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../static/theme.js',import.meta.url),'utf8');
function browser(saved=null,systemDark=false,storageBlocked=false,savedPalette=null) {
  const root={dataset:{},classList:{values:new Set(),toggle(name,on){on?this.values.add(name):this.values.delete(name);}}};
  const controls=[{value:'light'}],paletteControls=[{value:'blue'}],metadata={};
  const documentEvents={},windowEvents={},media={matches:systemDark,addEventListener(_,fn){this.changed=fn;}};
  const stored={'hireferry-theme':saved,'hireferry-palette':savedPalette};let mutation;
  vm.runInNewContext(source,{
    document:{documentElement:root,querySelector:()=>({setAttribute:(k,v)=>metadata[k]=v}),
      querySelectorAll:selector=>selector==='[data-palette-select]'?paletteControls:controls,
      addEventListener:(name,fn)=>documentEvents[name]=fn},
    window:{matchMedia:()=>media,addEventListener:(name,fn)=>windowEvents[name]=fn},
    localStorage:{getItem(key){if(storageBlocked)throw Error('blocked');return stored[key];},setItem(key,value){if(storageBlocked)throw Error('blocked');stored[key]=value;}},
    MutationObserver:class{constructor(fn){mutation=fn;}observe(){}},
  });
  function change(kind,value){const select={value,closest(selector){return selector===`[data-${kind}-select]`?this:null;}};documentEvents.change({target:select});}
  return {root,metadata,controls,paletteControls,stored,
    choose:value=>change('theme',value),choosePalette:value=>change('palette',value),
    system(dark){media.matches=dark;media.changed();},
    storage(value,key='hireferry-theme'){windowEvents.storage({key,newValue:value});},
    renderHeader(){controls.push({value:''});paletteControls.push({value:''});mutation();},
  };
}
test('new visitors get light blue even on a dark OS; explicit mode choices persist',()=>{
  const b=browser(null,true);
  assert.equal(b.root.dataset.theme,'light');assert.equal(b.root.dataset.palette,'blue');
  assert.equal(b.metadata.content,'#f9f9ff');
  b.choose('dark');assert.equal(b.stored['hireferry-theme'],'dark');b.system(false);
  assert.equal(b.root.dataset.theme,'dark');b.renderHeader();assert.ok(b.controls.every(x=>x.value==='dark'));
  const reloaded=browser(b.stored['hireferry-theme'],false);assert.equal(reloaded.root.dataset.theme,'dark');
  b.choose('system');assert.equal(b.root.dataset.theme,'light');b.system(true);assert.equal(b.root.dataset.theme,'dark');
});
test('palette changes retain appearance, survive navigation and reload, and sync across tabs',()=>{
  const b=browser('dark');b.choosePalette('violet');
  assert.equal(b.root.dataset.palette,'violet');assert.equal(b.root.dataset.theme,'dark');
  b.renderHeader();assert.ok(b.paletteControls.every(x=>x.value==='violet'));
  assert.equal(browser('dark',false,false,b.stored['hireferry-palette']).root.dataset.palette,'violet');
  b.storage('rose','hireferry-palette');assert.equal(b.root.dataset.palette,'rose');
  b.storage('light');assert.equal(b.root.dataset.theme,'light');assert.equal(b.root.dataset.palette,'rose');
  b.storage('invalid','hireferry-palette');assert.equal(b.root.dataset.palette,'blue');
  b.storage('invalid');assert.equal(b.root.dataset.themePreference,'light');
});
test('blocked storage still permits independent appearance and palette changes for this visit',()=>{
  const b=browser(null,true,true);b.choose('dark');b.choosePalette('rose');b.renderHeader();
  assert.equal(b.root.dataset.theme,'dark');assert.equal(b.root.dataset.palette,'rose');
  assert.ok(b.controls.every(x=>x.value==='dark'));assert.ok(b.paletteControls.every(x=>x.value==='rose'));
});
test('all Material palette text and button pairs meet normal-text contrast',()=>{
  const {palettes}=JSON.parse(readFileSync(new URL('../static/theme-palettes.json',import.meta.url)));
  function luminance(hex){const rgb=hex.slice(1).match(/../g).map(x=>parseInt(x,16)/255).map(x=>x<=.04045?x/12.92:((x+.055)/1.055)**2.4);return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;}
  for(const [name,p]of Object.entries(palettes))for(const mode of ['light','dark'])for(const [fg,bg]of [['onSurface','surface'],['onSurfaceVariant','surface'],['onPrimary','primary'],['onPrimaryContainer','primaryContainer'],['onErrorContainer','errorContainer']]){
    const a=luminance(p[mode][fg]),b=luminance(p[mode][bg]);const contrast=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
    assert.ok(contrast>=4.5,`${name}/${mode}/${fg}: ${contrast}`);
  }
});

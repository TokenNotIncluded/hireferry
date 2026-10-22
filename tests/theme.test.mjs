import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../static/theme.js',import.meta.url),'utf8');
function browser(saved=null,systemDark=false,storageBlocked=false,savedPalette=null) {
  const root={dataset:{},classList:{values:new Set(),toggle(name,on){on?this.values.add(name):this.values.delete(name);}}};
  const button=(kind,value)=>({dataset:{[kind]:value},attrs:{},getAttribute(k){return this.attrs[k];},setAttribute(k,v){this.attrs[k]=v;}});
  const controls=[button('themeChoice','dark')],paletteControls=[button('paletteChoice','rose')],metadata={};
  const documentEvents={},windowEvents={},media={matches:systemDark,addEventListener(_,fn){this.changed=fn;}};
  const menu={open:true,focused:false,querySelector(){return{focus:()=>this.focused=true};}};
  const stored={'hireferry-theme':saved,'hireferry-palette':savedPalette};let mutation;
  vm.runInNewContext(source,{
    document:{documentElement:root,querySelector:()=>({setAttribute:(k,v)=>metadata[k]=v}),
      querySelectorAll:selector=>selector==='[data-palette-choice]'?paletteControls:selector==='[data-theme-choice]'?controls:menu.open?[menu]:[],
      addEventListener:(name,fn)=>documentEvents[name]=fn},
    window:{matchMedia:()=>media,addEventListener:(name,fn)=>windowEvents[name]=fn},
    localStorage:{getItem(key){if(storageBlocked)throw Error('blocked');return stored[key];},setItem(key,value){if(storageBlocked)throw Error('blocked');stored[key]=value;}},
    MutationObserver:class{constructor(fn){mutation=fn;}observe(){}},
  });
  function change(kind,value){const chosen=button(kind==='theme'?'themeChoice':'paletteChoice',value);documentEvents.click({target:{closest:selector=>selector===`[data-${kind}-choice]`?chosen:selector==='.theme-menu'?menu:null}});}
  return {root,metadata,controls,paletteControls,stored,menu,
    choose:value=>change('theme',value),choosePalette:value=>change('palette',value),
    system(dark){media.matches=dark;media.changed();},
    storage(value,key='hireferry-theme'){windowEvents.storage({key,newValue:value});},
    renderHeader(){controls.push(button('themeChoice','dark'));paletteControls.push(button('paletteChoice','rose'));mutation();},
    outside(){documentEvents.click({target:{closest:()=>null}});},
    escape(){documentEvents.keydown({key:'Escape'});},
  };
}
test('new visitors get paper and ink; explicit mode choices persist',()=>{
  const b=browser(null,true);
  assert.equal(b.root.dataset.theme,'light');assert.equal(b.root.dataset.palette,'neutral');
  assert.equal(b.metadata.content,'#f8f7f3');
  b.choose('dark');assert.equal(b.stored['hireferry-theme'],'dark');b.system(false);
  assert.equal(b.root.dataset.theme,'dark');assert.equal(b.root.dataset.palette,'neutral');assert.equal(b.metadata.content,'#171715');
  b.renderHeader();assert.ok(b.controls.every(x=>x.attrs['aria-pressed']==='true'));
  const reloaded=browser(b.stored['hireferry-theme'],false);assert.equal(reloaded.root.dataset.theme,'dark');
  b.choose('system');assert.equal(b.root.dataset.theme,'light');b.system(true);assert.equal(b.root.dataset.theme,'dark');
});
test('palette changes retain appearance, survive navigation and reload, and sync across tabs',()=>{
  const b=browser('dark');b.choosePalette('violet');
  assert.equal(b.root.dataset.palette,'violet');assert.equal(b.root.dataset.theme,'dark');
  b.renderHeader();assert.ok(b.paletteControls.every(x=>x.attrs['aria-pressed']==='false'));
  assert.equal(browser('dark',false,false,b.stored['hireferry-palette']).root.dataset.palette,'violet');
  b.storage('rose','hireferry-palette');assert.equal(b.root.dataset.palette,'rose');
  b.storage('light');assert.equal(b.root.dataset.theme,'light');assert.equal(b.root.dataset.palette,'rose');
  b.storage('invalid','hireferry-palette');assert.equal(b.root.dataset.palette,'neutral');
  b.storage('invalid');assert.equal(b.root.dataset.themePreference,'light');
});
test('saved accent choices are respected and clearing preferences restores paper and ink',()=>{
  const b=browser('dark',false,false,'blue');
  assert.equal(b.root.dataset.palette,'blue');
  b.choosePalette('neutral');assert.equal(b.stored['hireferry-palette'],'neutral');
  assert.equal(browser('dark',false,false,b.stored['hireferry-palette']).root.dataset.palette,'neutral');
  b.storage(null,null);assert.equal(b.root.dataset.theme,'light');assert.equal(b.root.dataset.palette,'neutral');
  assert.equal(browser(null,false,false,'unknown').root.dataset.palette,'neutral');
});
test('blocked storage still permits independent appearance and palette changes for this visit',()=>{
  const b=browser(null,true,true);b.choose('dark');b.choosePalette('rose');b.renderHeader();
  assert.equal(b.root.dataset.theme,'dark');assert.equal(b.root.dataset.palette,'rose');
  assert.ok(b.controls.every(x=>x.attrs['aria-pressed']==='true'));assert.ok(b.paletteControls.every(x=>x.attrs['aria-pressed']==='true'));
});
test('all palette text and button pairs meet normal-text contrast',()=>{
  const {palettes}=JSON.parse(readFileSync(new URL('../static/theme-palettes.json',import.meta.url)));
  function luminance(hex){const rgb=hex.slice(1).match(/../g).map(x=>parseInt(x,16)/255).map(x=>x<=.04045?x/12.92:((x+.055)/1.055)**2.4);return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722;}
  for(const [name,p]of Object.entries(palettes))for(const mode of ['light','dark'])for(const [fg,bg]of [['onSurface','surface'],['onSurfaceVariant','surface'],['onSurfaceVariant','surfaceContainerLow'],['primary','surface'],['onPrimary','primary'],['onPrimaryContainer','primaryContainer'],['onSecondaryContainer','secondaryContainer'],['onTertiaryContainer','tertiaryContainer'],['onErrorContainer','errorContainer']]){
    const a=luminance(p[mode][fg]),b=luminance(p[mode][bg]);const contrast=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
    assert.ok(contrast>=4.5,`${name}/${mode}/${fg}: ${contrast}`);
  }
});

test('paper and ink and all accent swatches work and dismiss on outside click or Escape',()=>{
  const b=browser();
  for(const palette of ['neutral','blue','violet','rose','amber','teal']){b.choosePalette(palette);assert.equal(b.root.dataset.palette,palette);assert.equal(b.menu.open,true);}
  b.outside();assert.equal(b.menu.open,false);assert.equal(b.menu.focused,false);
  b.menu.open=true;b.escape();assert.equal(b.menu.open,false);assert.equal(b.menu.focused,true);
});
test('every accent shares warm neutral surfaces with distinct paper and ink defaults',()=>{
  const {palettes}=JSON.parse(readFileSync(new URL('../static/theme-palettes.json',import.meta.url)));
  assert.equal(palettes.neutral.light.primary,'#252420');
  assert.equal(palettes.neutral.dark.primary,'#eeeae2');
  for(const palette of Object.values(palettes))for(const mode of ['light','dark']) {
    assert.equal(palette[mode].surface,palettes.neutral[mode].surface);
    assert.equal(palette[mode].onSurface,palettes.neutral[mode].onSurface);
    assert.notEqual(palette.light.primary,palette.dark.primary);
  }
});

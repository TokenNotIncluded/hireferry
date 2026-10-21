import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';

const source = readFileSync(new URL('../static/theme.js', import.meta.url), 'utf8');
function browser(saved, systemDark = false, storageBlocked = false) {
  const root = {dataset:{}, classList:{values:new Set(), toggle(name, on) { on ? this.values.add(name) : this.values.delete(name); }}};
  const controls = [{value:'system'}];
  const metadata = {};
  const documentEvents = {}, windowEvents = {};
  const media = {matches:systemDark, addEventListener(_, fn) { this.changed = fn; }};
  let stored = saved, mutation;
  const localStorage = {
    getItem() { if (storageBlocked) throw new Error('blocked'); return stored; },
    setItem(_, value) { if (storageBlocked) throw new Error('blocked'); stored = value; },
  };
  vm.runInNewContext(source, {
    document:{documentElement:root, querySelector:()=>({setAttribute:(key,value)=>metadata[key]=value}),
      querySelectorAll:()=>controls, addEventListener:(name,fn)=>documentEvents[name]=fn},
    window:{matchMedia:()=>media, addEventListener:(name,fn)=>windowEvents[name]=fn},
    localStorage,
    MutationObserver:class {constructor(fn) {mutation=fn;} observe() {}},
  });
  return {root, controls, metadata, media, stored:()=>stored,
    choose(value) {const select={value,closest(){return this;}}; documentEvents.change({target:select});},
    system(dark) {media.matches=dark; media.changed();},
    storage(value) {windowEvents.storage({key:'hireferry-theme',newValue:value});},
    renderHeader() {controls.push({value:'system'});mutation();},
  };
}

test('appearance follows the system before rendering, while explicit choices survive navigation',()=>{
  const b=browser(null,true);
  assert.equal(b.root.dataset.theme,'dark');
  assert.equal(b.metadata.content,'#111713');
  b.choose('light');
  assert.equal(b.root.dataset.theme,'light');
  assert.equal(b.stored(),'light');
  b.system(false); b.system(true);
  assert.equal(b.root.dataset.theme,'light');
  b.renderHeader();
  assert.ok(b.controls.every(x=>x.value==='light'));
  const reloaded=browser(b.stored(),true);
  assert.equal(reloaded.root.dataset.theme,'light');
  b.choose('system');
  assert.equal(b.root.dataset.theme,'dark');
  b.system(false);
  assert.equal(b.root.dataset.theme,'light');
});

test('theme changes from another tab synchronize and malformed preferences fall back safely',()=>{
  const b=browser('dark',false);
  b.storage('light');
  assert.equal(b.root.dataset.theme,'light');
  b.storage('unexpected');
  assert.equal(b.root.dataset.themePreference,'system');
  b.system(true);
  assert.equal(b.root.dataset.theme,'dark');
  assert.ok(b.root.classList.values.has('dark'));
  assert.ok(!b.root.classList.values.has('light'));
  b.choose('unexpected');
  assert.equal(b.root.dataset.themePreference,'system');
});

test('blocked persistent storage does not prevent choosing an appearance for this visit',()=>{
  const b=browser(null,false,true);
  assert.equal(b.root.dataset.theme,'light');
  b.choose('dark');
  assert.equal(b.root.dataset.theme,'dark');
  b.renderHeader();
  assert.ok(b.controls.every(x=>x.value==='dark'));
});

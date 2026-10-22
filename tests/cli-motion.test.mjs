import test from 'node:test';
import assert from 'node:assert/strict';
import {createCliTypewriter,copyCliCommand,mountCliGuide} from '../static/cli-motion.js';

function animation() {
  let sequence=0;
  const tasks=new Map(),frames=[];
  const examples=[
    {command:' --server https://hire.example whoami',label:'Identity',description:'Check your identity.'},
    {command:' --server https://hire.example capabilities',label:'Permissions',description:'Inspect your permissions.'},
  ];
  const writer=createCliTypewriter(examples,frame=>frames.push(frame),{
    setTimer(callback,delay){const id=++sequence;tasks.set(id,{callback,delay});return id;},
    clearTimer(id){tasks.delete(id);},
  });
  return {writer,tasks,examples,frames,tick(){const [id,task]=tasks.entries().next().value||[];assert.ok(task,'animation should have a scheduled tick');tasks.delete(id);task.callback();},frame(){return frames.at(-1);}};
}

test('typewriter starts at hfy, completes configured commands, and rotates without changing command content',()=>{
  const run=animation();
  assert.equal(run.frame().text,'');
  assert.equal(run.tasks.size,0);
  run.writer.setActive(true);
  for(let i=0;i<run.examples[0].command.length;i++)run.tick();
  assert.equal(run.frame().text,run.examples[0].command);
  assert.equal(run.frame().typing,false);
  assert.equal([...run.tasks.values()][0].delay,2400);
  for(let i=0;i<100&&run.frame().index===0;i++)run.tick();
  assert.equal(run.frame().index,1);
  assert.equal(run.frame().text,'');
  for(let i=0;i<run.examples[1].command.length;i++)run.tick();
  assert.equal(run.frame().text,run.examples[1].command);
  run.writer.dispose();
});

test('hidden or paused demos retain their position with no timer, and disposal cannot restart them',()=>{
  const run=animation();run.writer.setActive(true);run.tick();run.tick();
  const before=run.frame().text;
  run.writer.setActive(false);
  assert.equal(run.tasks.size,0);
  assert.equal(run.frame().text,before);
  run.writer.setActive(true);run.writer.setActive(true);
  assert.equal(run.tasks.size,1,'resume does not create duplicate timers');
  run.tick();assert.ok(run.frame().text.length>before.length);
  run.writer.dispose();assert.equal(run.tasks.size,0);
  run.writer.setActive(true);assert.equal(run.tasks.size,0);
});

test('reduced motion immediately presents the whole command without any ongoing animation',()=>{
  const run=animation();run.writer.setActive(true);run.tick();
  run.writer.setReducedMotion(true);
  assert.equal(run.tasks.size,0);
  assert.equal(run.frame().text,run.examples[0].command);
  assert.equal(run.frame().typing,false);
  run.writer.setActive(true);assert.equal(run.tasks.size,0);
  run.writer.setReducedMotion(false);assert.equal(run.tasks.size,1);
  assert.equal(run.frame().text,'');
  run.writer.dispose();
});

test('copy preserves the full command and succeeds without relying on visible wrapped text',async()=>{
  let copied;
  const command='hfy --server https://hire.example auth login --device';
  const button={dataset:{copy:command},ownerDocument:{defaultView:{navigator:{clipboard:{async writeText(value){copied=value;}}}}}};
  assert.equal(await copyCliCommand(button),command);
  assert.equal(copied,command);
});

test('clipboard rejection falls back to a temporary plain-text selection, restores focus, and reports failure',async()=>{
  const command='irm https://hire.example/install.ps1 | iex';
  let selected=false,removed=false,focused=false,appended=false;
  const input={style:{},setAttribute(){},select(){selected=true;},remove(){removed=true;}};
  const doc={
    defaultView:{navigator:{clipboard:{async writeText(){throw new Error('blocked');}}}},
    activeElement:{focus(){focused=true;}},createElement(){return input;},
    body:{append(){appended=true;}},execCommand(){assert.equal(input.value,command);return true;},
  };
  const button={dataset:{copy:command,copyError:'Please copy the command manually.'},ownerDocument:doc};
  assert.equal(await copyCliCommand(button),command);
  assert.ok(selected&&removed&&focused&&appended);
  doc.execCommand=()=>false;
  await assert.rejects(copyCliCommand(button),/Please copy the command manually/);
  doc.execCommand=()=>{throw new Error('not supported');};
  await assert.rejects(copyCliCommand(button),/Please copy the command manually/);
});


test('mounting the guide again disposes prior work and retains the chosen platform and manual pause',()=>{
  const target=()=>({
    listeners:new Map(),
    addEventListener(type,handler){if(!this.listeners.has(type))this.listeners.set(type,new Set());this.listeners.get(type).add(handler);},
    removeEventListener(type,handler){this.listeners.get(type)?.delete(handler);},
    dispatch(type,event){for(const handler of this.listeners.get(type)||[])handler(event);},
  });
  const node=dataset=>({...target(),dataset:dataset||{},attributes:{},textContent:'',
    setAttribute(name,value){this.attributes[name]=value;},classList:{toggle(){}},focus(){},
  });
  let sequence=0;
  const timers=new Map(),motion={...target(),matches:false};
  const win={navigator:{platform:'Linux'},matchMedia:()=>motion,setTimeout(callback){const id=++sequence;timers.set(id,callback);return id;},clearTimeout(id){timers.delete(id);}};
  const doc={...target(),defaultView:win,hidden:false};
  const tabs=['unix','windows'].map(platform=>node({cliPlatform:platform}));
  const panels=['unix','windows'].map(platform=>node({cliPanel:platform}));
  const terminal=node({cliExamples:JSON.stringify([{command:' --server https://hire.example whoami',label:'Identity',description:'Example.'}])});
  const toggle=node({pauseLabel:'Pause',playLabel:'Play'});
  const selectors={'[data-cli-terminal]':terminal,'[data-cli-motion-toggle]':toggle,'[data-cli-typed]':node(),'[data-cli-description]':node(),'[data-cli-example-label]':node()};
  const guide={...target(),ownerDocument:doc,dataset:{},contains:()=>true,
    querySelector:selector=>selectors[selector],
    querySelectorAll:selector=>({'[data-cli-platform]':tabs,'[data-cli-panel]':panels,'[data-cli-example-dot]':[node()],'.cli-copy':[]}[selector]||[]),
  };
  const root={dataset:{},querySelector:()=>guide};
  const first=mountCliGuide(root);
  assert.equal(root.dataset.cliPlatform,'unix');
  assert.equal(timers.size,1);
  guide.dispatch('click',{target:{closest:()=>tabs[1]}});
  assert.equal(root.dataset.cliPlatform,'windows');
  const second=mountCliGuide(root);
  assert.equal(root.dataset.cliPlatform,'windows');
  assert.equal(tabs[1].attributes['aria-selected'],'true');
  assert.equal(panels[0].hidden,true);assert.equal(panels[1].hidden,false);
  assert.equal(timers.size,1,'one active mount owns one animation timer');
  assert.equal(guide.listeners.get('click').size,1);
  assert.equal(doc.listeners.get('visibilitychange').size,1);
  first.dispose();assert.equal(timers.size,1,'an already disposed mount cannot stop its replacement');
  toggle.dispatch('click',{});
  assert.equal(root.dataset.cliDemoPaused,'true');
  assert.equal(timers.size,0);
  const third=mountCliGuide(root);
  assert.equal(toggle.attributes['aria-pressed'],'true');
  assert.equal(terminal.dataset.paused,'true');
  assert.equal(timers.size,0,'background remount must respect the manual pause');
  second.dispose();
  toggle.dispatch('click',{});
  assert.equal(root.dataset.cliDemoPaused,'false');assert.equal(timers.size,1);
  third.dispose();third.dispose();
  assert.equal(timers.size,0);assert.equal(guide.listeners.get('click').size,0);
  assert.equal(doc.listeners.get('visibilitychange').size,0);
});

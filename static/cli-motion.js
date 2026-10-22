// One timer, no animation frames: the demo only runs while it can be seen.
export function createCliTypewriter(examples, paint, {
  setTimer = setTimeout, clearTimer = clearTimeout,
} = {}) {
  let index=0, position=0, phase='typing', timer=null, active=false, reduced=false, disposed=false;
  const draw=()=>paint({ ...examples[index], index, text:examples[index].command.slice(0,position), typing:phase==='typing' });
  const cancel=()=>{if(timer!==null)clearTimer(timer);timer=null;};
  const schedule=delay=>{if(!disposed&&active&&!reduced&&timer===null)timer=setTimer(tick,delay);};
  function tick() {
    timer=null;
    if(disposed||!active||reduced)return;
    const example=examples[index];
    if(phase==='typing') {
      position++;
      if(position>=example.command.length){position=example.command.length;phase='hold';draw();schedule(2400);}
      else {draw();schedule(42);}
    } else if(phase==='hold') {
      phase='deleting';draw();schedule(22);
    } else {
      position=Math.max(0,position-2);
      if(!position){index=(index+1)%examples.length;phase='typing';draw();schedule(650);}
      else {draw();schedule(22);}
    }
  }
  draw();
  return {
    setActive(value) {active=Boolean(value);if(active)schedule(500);else cancel();},
    setReducedMotion(value) {
      if(reduced===Boolean(value))return;
      reduced=Boolean(value);cancel();position=reduced?examples[index].command.length:0;phase=reduced?'hold':'typing';draw();schedule(650);
    },
    dispose() {disposed=true;active=false;cancel();},
  };
}

const guideMounts=new WeakMap();
const copyFeedback=new WeakMap();

function resetCopyFeedback(button) {
  const saved=copyFeedback.get(button);
  if(!saved)return;
  clearTimeout(saved.timer);
  button.classList.remove('is-copied');
  button.setAttribute('aria-label',saved.label);
  button.setAttribute('title',saved.title);
  const text=button.querySelector('[data-copy-label-text]'),status=button.querySelector('[data-copy-status]');
  if(text)text.textContent=button.dataset.copyLabel;
  if(status)status.textContent='';
  copyFeedback.delete(button);
}

function markCopied(button) {
  if(!button.dataset.copySuccess||!button.isConnected)return;
  const original=copyFeedback.get(button)||{label:button.getAttribute('aria-label')||'',title:button.getAttribute('title')||''};
  clearTimeout(original.timer);
  button.classList.add('is-copied');
  button.setAttribute('aria-label',button.dataset.copySuccess);
  button.setAttribute('title',button.dataset.copySuccess);
  const text=button.querySelector('[data-copy-label-text]'),status=button.querySelector('[data-copy-status]');
  if(text)text.textContent=button.dataset.copySuccess;
  if(status)status.textContent=button.dataset.copySuccess;
  copyFeedback.set(button,{...original,timer:setTimeout(()=>resetCopyFeedback(button),1800)});
}

export async function copyCliCommand(button) {
  const command=button.dataset.copy,doc=button.ownerDocument;
  if(typeof command!=='string'||!command)throw new Error(button.dataset.copyError||'No command to copy.');
  let copied=false;
  try {
    const clipboard=doc.defaultView?.navigator?.clipboard;
    if(clipboard?.writeText){await clipboard.writeText(command);copied=true;}
  } catch {/* Fall back to a selected plain-text field if clipboard access is denied. */}
  if(!copied) {
    const active=doc.activeElement,input=doc.createElement('textarea');
    input.value=command;input.readOnly=true;input.tabIndex=-1;input.setAttribute('aria-hidden','true');
    input.style.cssText='position:fixed;left:-9999px;top:0;opacity:0';
    doc.body.append(input);
    try {input.select();copied=doc.execCommand?.('copy')===true;}
    catch {copied=false;}
    finally {input.remove();active?.focus({preventScroll:true});}
  }
  if(!copied)throw new Error(button.dataset.copyError||'Could not copy. Please copy the command manually.');
  markCopied(button);
  return command;
}

export function mountCliGuide(root) {
  guideMounts.get(root)?.dispose();
  const guide=root.querySelector('.cli-guide');
  if(!guide)return {dispose(){}};
  const doc=guide.ownerDocument,win=doc.defaultView;
  const tabs=[...guide.querySelectorAll('[data-cli-platform]')],panels=[...guide.querySelectorAll('[data-cli-panel]')];
  function selectPlatform(platform,focus=false) {
    if(!tabs.some(tab=>tab.dataset.cliPlatform===platform))return;
    guide.dataset.cliPlatform=platform;
    if(root.dataset)root.dataset.cliPlatform=platform;
    for(const tab of tabs){const selected=tab.dataset.cliPlatform===platform;tab.setAttribute('aria-selected',String(selected));tab.tabIndex=selected?0:-1;if(selected&&focus)tab.focus();}
    for(const panel of panels)panel.hidden=panel.dataset.cliPanel!==platform;
  }
  selectPlatform(root.dataset?.cliPlatform||guide.dataset.cliPlatform||(/win/i.test(win.navigator.userAgentData?.platform||win.navigator.platform||'')?'windows':'unix'));
  const onClick=event=>{
    const tab=event.target.closest('[data-cli-platform]');
    if(tab&&guide.contains(tab))selectPlatform(tab.dataset.cliPlatform);
  };
  const onKey=event=>{
    const tab=event.target.closest('[data-cli-platform]');
    if(!tab||!tabs.includes(tab)||!['ArrowLeft','ArrowRight','Home','End'].includes(event.key))return;
    event.preventDefault();
    const current=tabs.indexOf(tab),last=tabs.length-1;
    const index=event.key==='Home'?0:event.key==='End'?last:(current+(event.key==='ArrowRight'?1:-1)+tabs.length)%tabs.length;
    selectPlatform(tabs[index].dataset.cliPlatform,true);
  };
  guide.addEventListener('click',onClick);guide.addEventListener('keydown',onKey);

  const terminal=guide.querySelector('[data-cli-terminal]'),toggle=guide.querySelector('[data-cli-motion-toggle]');
  const typed=guide.querySelector('[data-cli-typed]'),description=guide.querySelector('[data-cli-description]'),label=guide.querySelector('[data-cli-example-label]');
  const dots=[...guide.querySelectorAll('[data-cli-example-dot]')];
  const motion=win.matchMedia('(prefers-reduced-motion: reduce)');
  let paused=root.dataset?.cliDemoPaused==='true',visible=true,observer;
  const examples=JSON.parse(terminal.dataset.cliExamples);
  const writer=createCliTypewriter(examples,frame=>{
    typed.textContent=frame.text;description.textContent=frame.description;label.textContent=frame.label;
    terminal.dataset.typing=String(frame.typing);
    dots.forEach((dot,index)=>dot.classList.toggle('active',index===frame.index));
  },{setTimer:win.setTimeout.bind(win),clearTimer:win.clearTimeout.bind(win)});
  const sync=()=>{
    const stopped=paused||doc.hidden||!visible||motion.matches;
    terminal.dataset.paused=String(stopped);
    toggle.setAttribute('aria-pressed',String(paused));
    toggle.setAttribute('aria-label',paused?toggle.dataset.playLabel:toggle.dataset.pauseLabel);
    writer.setReducedMotion(motion.matches);writer.setActive(!stopped);
  };
  const onToggle=()=>{paused=!paused;if(root.dataset)root.dataset.cliDemoPaused=String(paused);sync();};
  toggle.addEventListener('click',onToggle);
  doc.addEventListener('visibilitychange',sync);motion.addEventListener('change',sync);
  if(win.IntersectionObserver){observer=new win.IntersectionObserver(entries=>{visible=entries.some(entry=>entry.isIntersecting);sync();});observer.observe(terminal);}
  sync();
  let disposed=false;
  const controller={dispose(){
    if(disposed)return;
    disposed=true;
    writer.dispose();observer?.disconnect();
    guide.removeEventListener('click',onClick);guide.removeEventListener('keydown',onKey);
    toggle.removeEventListener('click',onToggle);doc.removeEventListener('visibilitychange',sync);motion.removeEventListener('change',sync);
    guide.querySelectorAll('.cli-copy').forEach(resetCopyFeedback);
    if(guideMounts.get(root)===controller)guideMounts.delete(root);
  }};
  guideMounts.set(root,controller);
  return controller;
}

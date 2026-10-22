// Progressive enhancement: native fields still own form values and validation.
const arrow = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true"><path d="m7 10 5 5 5-5"/></svg>';
const calendar = '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="4" y="5" width="16" height="16" rx="3"/><path d="M8 3v4m8-4v4M4 11h16"/></svg>';
const dayKey = date => `${date.getUTCFullYear()}-${String(date.getUTCMonth()+1).padStart(2,'0')}-${String(date.getUTCDate()).padStart(2,'0')}`;

export function createControlsUI({doc = document, win = window} = {}) {
  if (!doc.createElement) return {sync(){}, clear(){}, close(){}, dispose(){}};
  const fields = new Map();
  let active = null, sequence = 0, scheduled = false;
  const english = () => doc.documentElement.lang.startsWith('en');
  const text = (zh, en) => english() ? en : zh;
  const make = (tag, className, content) => {
    const element = doc.createElement(tag);
    if (className) element.className = className;
    if (content !== undefined) element.textContent = content;
    return element;
  };
  const name = source => source.getAttribute('aria-label') || [...(source.labels || [])].map(label => {
    const copy = label.cloneNode(true);
    copy.querySelectorAll('select,input,button,textarea').forEach(node => node.remove());
    return copy.textContent.trim();
  }).join(' ') || source.name;

  function close(restore = false) {
    if (!active) return;
    const previous = active;
    active = null;
    previous.panel.remove();
    previous.trigger.setAttribute('aria-expanded','false');
    previous.trigger.removeAttribute('aria-activedescendant');
    if (restore && previous.trigger.isConnected) previous.trigger.focus();
  }

  function positionPopup(trigger, panel) {
    const box = trigger.getBoundingClientRect();
    const width = Math.min(Math.max(box.width, panel.classList.contains('calendar-menu') ? 292 : 188), win.innerWidth - 24);
    panel.style.width = `${width}px`;
    panel.style.left = `${Math.max(12, Math.min(box.left, win.innerWidth-width-12))}px`;
    const availableBelow = win.innerHeight-box.bottom-18;
    const above = availableBelow < Math.min(panel.scrollHeight, 260) && box.top > availableBelow;
    panel.style.maxHeight = `${Math.max(100, above ? box.top-18 : availableBelow)}px`;
    panel.style.top = ''; panel.style.bottom = '';
    if (above) panel.style.bottom = `${win.innerHeight-box.top+7}px`;
    else panel.style.top = `${box.bottom+7}px`;
  }

  function popup(source, trigger, panel) {
    close();
    const parent = source.closest('[role="dialog"]') || doc.body;
    parent.append(panel);
    trigger.setAttribute('aria-expanded','true');
    positionPopup(trigger,panel);
    active = {source, trigger, panel};
  }

  function wrap(source, className) {
    const wrapper = make('span',className);
    source.before(wrapper);
    wrapper.append(source);
    source.dataset.controlEnhanced = 'true';
    return wrapper;
  }

  function enhanceSelect(source) {
    const wrapper = wrap(source,'choice-control');
    const previousTabIndex = source.getAttribute('tabindex');
    const previousHidden = source.getAttribute('aria-hidden');
    source.classList.add('choice-native');
    source.tabIndex = -1;
    source.setAttribute('aria-hidden','true');
    const trigger = make('button','choice-trigger');
    trigger.type = 'button';
    trigger.setAttribute('role','combobox');
    trigger.setAttribute('aria-haspopup','listbox');
    trigger.setAttribute('aria-expanded','false');
    trigger.setAttribute('aria-autocomplete','none');
    const label = make('span','choice-value');
    trigger.append(label);
    trigger.insertAdjacentHTML('beforeend',arrow);
    wrapper.append(trigger);
    const id = `choice-menu-${++sequence}`;
    trigger.setAttribute('aria-controls',id);
    let highlighted = -1, typed = '', typedAt = 0, renderedOptions = [], renderedKey = '';
    const options = () => [...source.options];
    const enabled = option => !!option && !option.disabled && !option.parentElement?.disabled && !option.hidden;
    const optionKey = () => JSON.stringify(options().map((option,index) => [option.label,option.value,enabled(option),option.hidden,option.parentElement?.label,index===source.selectedIndex]));
    const sync = () => {
      const selected = source.selectedOptions[0];
      const value = selected?.label || text('请选择','Select');
      if (label.textContent !== value) label.textContent = value;
      trigger.disabled = source.disabled;
      trigger.setAttribute('aria-label',`${name(source)}: ${label.textContent}`);
      trigger.setAttribute('aria-required',String(source.required));
      if (source.getAttribute('aria-describedby')) trigger.setAttribute('aria-describedby',source.getAttribute('aria-describedby'));
      trigger.setAttribute('aria-invalid',source.getAttribute('aria-invalid') || 'false');
      if (active?.source === source) {
        if (source.disabled) { close();return; }
        const panel = active.panel;
        panel.setAttribute('aria-label',name(source));
        if (optionKey() !== renderedKey) {
          const previous = renderedOptions[highlighted], scrollTop = panel.scrollTop;
          drawOptions(panel);
          const index = options().indexOf(previous);
          highlight(index >= 0 && enabled(previous) ? index : options().findIndex(enabled), false);
          panel.scrollTop = scrollTop;
        }
        positionPopup(trigger,panel);
      }
    };
    function highlight(index, scroll = true) {
      highlighted = index;
      const panel = active?.source === source && active.panel;
      if (!panel) return;
      panel.querySelectorAll('[role="option"]').forEach(node => node.classList.toggle('is-highlighted',Number(node.dataset.index)===index));
      const item = panel.querySelector(`[data-index="${index}"]`);
      if (item) {
        trigger.setAttribute('aria-activedescendant',item.id);
        // Only scroll the options, never the workspace behind the popup.
        if (scroll && item.offsetTop < panel.scrollTop) panel.scrollTop = item.offsetTop;
        else if (scroll && item.offsetTop+item.offsetHeight > panel.scrollTop+panel.clientHeight) panel.scrollTop = item.offsetTop+item.offsetHeight-panel.clientHeight;
      }
    }
    function choose(index) {
      if (!enabled(options()[index])) return;
      const changed = source.selectedIndex !== index;
      source.selectedIndex = index;
      source.setAttribute('aria-invalid','false');
      sync(); close(true);
      if (changed) { source.dispatchEvent(new Event('input',{bubbles:true})); source.dispatchEvent(new Event('change',{bubbles:true})); }
    }
    function drawOptions(panel) {
      panel.replaceChildren();
      let previousGroup;
      options().forEach((option,index) => {
        if (option.hidden) return;
        const group = option.parentElement.tagName === 'OPTGROUP' ? option.parentElement.label : null;
        if (group && group !== previousGroup) panel.append(make('div','choice-group',group));
        previousGroup = group;
        const row = make('div','choice-option',option.label);
        row.id = `${id}-${index}`; row.dataset.index = String(index); row.setAttribute('role','option');
        row.setAttribute('aria-selected',String(index===source.selectedIndex));
        row.setAttribute('aria-disabled',String(!enabled(option)));
        row.addEventListener('pointerdown',event => event.preventDefault());
        row.addEventListener('pointermove',() => { if (enabled(option)) highlight(index); });
        row.addEventListener('click',() => choose(index));
        panel.append(row);
      });
      renderedOptions = options();renderedKey = optionKey();
    }
    function open() {
      if (source.disabled) return;
      sync();
      const panel = make('div','control-menu choice-menu');
      panel.id = id; panel.setAttribute('role','listbox'); panel.setAttribute('aria-label',name(source));
      drawOptions(panel);
      popup(source,trigger,panel);
      highlight(source.selectedIndex>=0 ? source.selectedIndex : options().findIndex(enabled));
      trigger.focus();
    }
    trigger.addEventListener('click',() => active?.source===source ? close() : open());
    trigger.addEventListener('keydown',event => {
      const isOpen = active?.source===source;
      const available = options().map((option,index)=>enabled(option) ? index : -1).filter(index=>index>=0);
      if (['ArrowDown','ArrowUp','Home','End'].includes(event.key)) {
        event.preventDefault();
        if (!isOpen) open();
        const position = available.indexOf(highlighted);
        const next = event.key==='Home' ? available[0] : event.key==='End' ? available.at(-1) : available[Math.max(0,Math.min(available.length-1,position+(event.key==='ArrowDown'?1:-1)))];
        if (next!==undefined) highlight(next);
      } else if (event.key==='Enter' || event.key===' ') {
        event.preventDefault(); isOpen ? choose(highlighted) : open();
      } else if (event.key==='Escape' && isOpen) { event.preventDefault(); event.stopPropagation(); close(true); }
      else if (event.key==='Tab') close();
      else if (event.key.length===1 && !event.ctrlKey && !event.altKey && !event.metaKey) {
        const now = Date.now(); typed = now-typedAt>700 ? event.key : typed+event.key; typedAt = now;
        const match = available.find(index=>options()[index].label.toLocaleLowerCase().startsWith(typed.toLocaleLowerCase()));
        if (match!==undefined) { event.preventDefault(); if (!isOpen) open(); highlight(match); }
      }
    });
    const onChange = () => sync();
    const onInvalid = event => {event.preventDefault(); trigger.setAttribute('aria-invalid','true'); trigger.focus();};
    const onFocus = () => trigger.focus();
    source.addEventListener('change',onChange); source.addEventListener('invalid',onInvalid); source.addEventListener('focus',onFocus);
    sync();
    return {sync, remove(){
      source.removeEventListener('change',onChange); source.removeEventListener('invalid',onInvalid); source.removeEventListener('focus',onFocus);
      source.classList.remove('choice-native');
      if (previousTabIndex===null) source.removeAttribute('tabindex'); else source.setAttribute('tabindex',previousTabIndex);
      if (previousHidden===null) source.removeAttribute('aria-hidden'); else source.setAttribute('aria-hidden',previousHidden);
      if (wrapper.parentNode) {wrapper.before(source); wrapper.remove();}
    }};
  }

  function enhanceDate(source) {
    const wrapper = wrap(source,'date-control');
    const trigger = make('button','date-trigger');
    trigger.type='button'; trigger.innerHTML=calendar;
    trigger.setAttribute('aria-haspopup','dialog'); trigger.setAttribute('aria-expanded','false');
    wrapper.append(trigger);
    const sync = () => {trigger.disabled=source.disabled || source.readOnly;trigger.setAttribute('aria-label',`${text('选择日期','Choose date')} · ${name(source)}`);};
    function open() {
      if (trigger.disabled) return;
      const today = dayKey(new Date());
      let focused = source.value || today;
      let month = new Date(`${focused.slice(0,7)}-01T12:00:00Z`);
      const panel = make('div','control-menu calendar-menu');
      panel.id=`calendar-menu-${++sequence}`; panel.setAttribute('role','dialog'); panel.setAttribute('aria-label',`${text('选择日期','Choose date')} · ${name(source)}`);
      trigger.setAttribute('aria-controls',panel.id);
      const isAllowed = key => (!source.min || key>=source.min) && (!source.max || key<=source.max);
      const commit = value => {source.value=value;close(true);source.dispatchEvent(new Event('input',{bubbles:true}));source.dispatchEvent(new Event('change',{bubbles:true}));};
      const button = (className,label,fn) => {const b=make('button',className,label);b.type='button';b.addEventListener('click',fn);return b;};
      function draw(focusDay=false) {
        panel.replaceChildren();
        const header=make('div','calendar-heading');
        const previous=button('calendar-month-step','‹',()=>{month.setUTCMonth(month.getUTCMonth()-1);draw();panel.querySelector('.calendar-month-step').focus();});
        const next=button('calendar-month-step','›',()=>{month.setUTCMonth(month.getUTCMonth()+1);draw();panel.querySelectorAll('.calendar-month-step')[1].focus();});
        previous.setAttribute('aria-label',text('上个月','Previous month'));next.setAttribute('aria-label',text('下个月','Next month'));
        const title=make('strong','',month.toLocaleDateString(english()?'en-US':'zh-CN',{year:'numeric',month:'long',timeZone:'UTC'}));
        title.id=`${panel.id}-title`;title.setAttribute('aria-live','polite');
        header.append(previous,title,next);panel.append(header);
        const week=make('div','calendar-week');week.setAttribute('aria-hidden','true');
        (english()?['M','T','W','T','F','S','S']:['一','二','三','四','五','六','日']).forEach(day=>week.append(make('span','',day)));
        panel.append(week);
        const grid=make('div','calendar-grid');grid.setAttribute('role','group');grid.setAttribute('aria-labelledby',title.id);
        const start=new Date(month);start.setUTCDate(1-((month.getUTCDay()+6)%7));
        for(let index=0;index<42;index++) {
          const date=new Date(start);date.setUTCDate(start.getUTCDate()+index);
          const key=dayKey(date),outside=date.getUTCMonth()!==month.getUTCMonth();
          const day=button(`calendar-day${outside?' outside':''}${key===today?' today':''}`,String(date.getUTCDate()),()=>commit(key));
          day.dataset.date=key;day.disabled=!isAllowed(key);day.tabIndex=key===focused?0:-1;
          day.setAttribute('aria-label',date.toLocaleDateString(english()?'en-US':'zh-CN',{year:'numeric',month:'long',day:'numeric',timeZone:'UTC'}));
          day.setAttribute('aria-pressed',String(key===source.value));
          if(key===today)day.setAttribute('aria-current','date');
          day.addEventListener('keydown',event=>{
            const steps={ArrowLeft:-1,ArrowRight:1,ArrowUp:-7,ArrowDown:7,Home:-((date.getUTCDay()+6)%7),End:6-((date.getUTCDay()+6)%7)};
            if(!(event.key in steps) && !['PageUp','PageDown'].includes(event.key))return;
            event.preventDefault();const target=new Date(date);
            if(event.key in steps)target.setUTCDate(date.getUTCDate()+steps[event.key]);else target.setUTCMonth(date.getUTCMonth()+(event.key==='PageUp'?-1:1));
            if(!isAllowed(dayKey(target)))return;
            focused=dayKey(target);month=new Date(`${focused.slice(0,7)}-01T12:00:00Z`);draw(true);
          });
          grid.append(day);
        }
        if(!grid.querySelector('[tabindex="0"]:not(:disabled)'))grid.querySelector('.calendar-day:not(.outside):not(:disabled)')?.setAttribute('tabindex','0');
        panel.append(grid);
        const footer=make('div','calendar-footer');
        const clear=button('calendar-text',text('清除','Clear'),()=>commit(''));clear.disabled=source.required;
        const now=button('calendar-text',text('今天','Today'),()=>commit(today));now.disabled=!isAllowed(today);
        footer.append(clear,now);panel.append(footer);
        if(focusDay)panel.querySelector('[tabindex="0"]')?.focus();
      }
      draw();popup(source,trigger,panel);panel.querySelector('[tabindex="0"]')?.focus();
      panel.addEventListener('keydown',event=>{if(event.key==='Escape'){event.preventDefault();event.stopPropagation();close(true);}});
    }
    trigger.addEventListener('click',()=>active?.source===source?close():open());
    const onKey=event=>{if(event.altKey&&event.key==='ArrowDown'){event.preventDefault();open();}};
    source.addEventListener('keydown',onKey);sync();
    return {sync,remove(){source.removeEventListener('keydown',onKey);if(wrapper.parentNode){wrapper.before(source);wrapper.remove();}}};
  }

  function clear(scope=doc) {
    for(const [source,widget] of fields) if(!source.isConnected || scope===doc || scope.contains(source)) {
      if(active?.source===source)close();
      widget.remove();delete source.dataset.controlEnhanced;fields.delete(source);
    }
  }
  function sync(scope=doc) {
    for(const [source,widget] of fields) {
      if(!source.isConnected || !source.closest('.choice-control,.date-control')) {if(active?.source===source)close();widget.remove();delete source.dataset.controlEnhanced;fields.delete(source);}
      else widget.sync();
    }
    scope.querySelectorAll('select:not([multiple]):not([data-control-enhanced]),input[type="date"]:not([data-control-enhanced])').forEach(source=>{
      if(source.tagName==='SELECT' && source.size>1)return;
      fields.set(source,source.tagName==='SELECT'?enhanceSelect(source):enhanceDate(source));
    });
  }
  const outside=event=>{if(active && !active.panel.contains(event.target) && !active.trigger.contains(event.target))close();};
  const moved=event=>{
    if(!active || active.panel.contains(event.target))return;
    const box=active.trigger.getBoundingClientRect();
    if(box.bottom<=0 || box.top>=win.innerHeight || box.right<=0 || box.left>=win.innerWidth)close();
    else positionPopup(active.trigger,active.panel);
  };
  const focusChanged=event=>{if(active && !active.panel.contains(event.target) && !active.trigger.contains(event.target))close();};
  const reset=()=>win.setTimeout(()=>sync(),0);
  doc.addEventListener('pointerdown',outside);doc.addEventListener('focusin',focusChanged);doc.addEventListener('scroll',moved,true);doc.addEventListener('reset',reset);win.addEventListener('resize',moved);
  const affectsField = record => {
    const target = record.target.nodeType === 1 ? record.target : record.target.parentElement;
    if (target === doc.documentElement && record.attributeName === 'lang') return true;
    if (target?.matches('select,option,optgroup,input[type="date"]') || target?.closest('option,optgroup')) return true;
    // Native labels can change without replacing their fields. Ignore our own
    // generated trigger text, including controls nested inside a label.
    if (target?.closest('label') && !target.closest('.choice-trigger,.date-trigger,.control-menu')) return true;
    return record.type === 'childList' && [...record.addedNodes,...record.removedNodes].some(node=>node.nodeType===1 && (node.matches('select,option,optgroup,label,input[type="date"],.choice-control,.date-control') || node.querySelector('select,label,input[type="date"]')));
  };
  const Observer = win.MutationObserver || globalThis.MutationObserver;
  const observer=typeof Observer==='function'?new Observer(records=>{
    if(scheduled || !records.some(affectsField))return;
    scheduled=true;queueMicrotask(()=>{scheduled=false;sync();});
  }):null;
  observer?.observe(doc.body,{childList:true,subtree:true,characterData:true,attributes:true,attributeFilter:['disabled','selected','label','min','max','value','aria-label','aria-describedby','aria-invalid','required','for']});
  if (doc.documentElement) observer?.observe(doc.documentElement,{attributes:true,attributeFilter:['lang']});
  return {sync,clear,close,dispose(){observer?.disconnect();close();clear();doc.removeEventListener('pointerdown',outside);doc.removeEventListener('focusin',focusChanged);doc.removeEventListener('scroll',moved,true);doc.removeEventListener('reset',reset);win.removeEventListener('resize',moved);}};
}

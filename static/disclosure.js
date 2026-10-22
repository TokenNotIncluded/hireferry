// Only the workspace chrome moves with scrolling. Reading content stays in place.
export function createDisclosureUI({ root, doc = document, win = window }) {
  const choices = new Map();
  const panelStorageKey = 'hireferry-workspace-panels';
  const panels = { navigation: false, inbox: false };
  try {
    const saved = JSON.parse(win.localStorage?.getItem(panelStorageKey) || '{}');
    for (const name of Object.keys(panels)) panels[name] = saved?.[name] === true;
  } catch {}
  let scope, pageKey, scroller, frame = null, lastY = 0, distance = 0, direction = 0;
  const keyFor = node => `${root.dataset.pageKey || ''}:${node.dataset.disclosure}`;
  const position = () => Math.max(0, scroller?.scrollTop || 0);
  const resetTravel = () => { lastY = position(); distance = 0; direction = 0; };
  const reveal = () => { root.classList.remove('workspace-chrome-collapsed'); resetTravel(); };
  const remember = (key, open) => {
    choices.delete(key);
    choices.set(key, open);
    if (choices.size > 256) choices.delete(choices.keys().next().value);
  };

  function syncPanels() {
    for (const name of Object.keys(panels)) {
      root.classList[panels[name] ? 'add' : 'remove'](`workspace-${name}-collapsed`);
    }
    for (const button of root.querySelectorAll('[data-workspace-panel]')) {
      const collapsed = panels[button.dataset.workspacePanel];
      const label = collapsed ? button.dataset.labelExpand : button.dataset.labelCollapse;
      button.setAttribute('aria-expanded', String(!collapsed));
      button.setAttribute('aria-label', label);
      button.title = label;
    }
  }

  function onPanelClick(event) {
    const button = event.target.closest?.('[data-workspace-panel]');
    const name = button?.dataset.workspacePanel;
    if (!Object.hasOwn(panels, name)) return;
    event.preventDefault();
    panels[name] = !panels[name];
    try { win.localStorage?.setItem(panelStorageKey, JSON.stringify(panels)); } catch {}
    syncPanels();
    if (name === 'inbox') {
      root.querySelector(panels.inbox
        ? '[data-workspace-panel="inbox"][data-panel-restore]'
        : '[data-workspace-panel="inbox"]:not([data-panel-restore])')?.focus({ preventScroll: true });
    }
  }

  function shouldStayOpen() {
    const active = doc.activeElement;
    return !!(
      root.querySelector('.workspace-header')?.contains(active) ||
      active?.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"]), [role="textbox"]') ||
      root.querySelector('.theme-menu[open], .workspace-more[open]') ||
      doc.querySelector('[role="dialog"][aria-modal="true"], dialog[open]')
    );
  }

  function updateScroll() {
    frame = null;
    if (!scroller) return;
    const y = position();
    if (y <= 48 || shouldStayOpen()) { reveal(); return; }
    const delta = y - lastY;
    // Accumulate tiny movements instead of responding to each touchpad tremor.
    if (Math.abs(delta) < 2) return;
    const nextDirection = Math.sign(delta);
    distance = nextDirection === direction ? distance + Math.abs(delta) : Math.abs(delta);
    direction = nextDirection;
    lastY = y;
    if (direction < 0 && distance >= 24) { reveal(); return; }
    if (direction > 0 && y >= 120 && distance >= 48) {
      const header = root.querySelector('.workspace-header');
      // Expanding the reading area must not clamp scrollTop at the page bottom.
      const room = scroller.scrollHeight - scroller.clientHeight - y;
      if (header && room > header.offsetHeight + 32) root.classList.add('workspace-chrome-collapsed');
    }
  }

  function schedule() {
    if (frame === null) frame = win.requestAnimationFrame(updateScroll);
  }

  function onToggle(event) {
    const node = event.target;
    if (node.matches?.('details[data-disclosure]')) {
      remember(keyFor(node), node.open);
    } else if (node.open && node.matches?.('.theme-menu, .workspace-more')) {
      reveal();
    }
  }

  function onFocus() { reveal(); }

  function onInvalid(event) {
    // Native validation must be able to reveal and focus a field in a folded form.
    let node = event.target.closest?.('details[data-disclosure]');
    while (node) {
      node.open = true;
      remember(keyFor(node), true);
      node = node.parentElement?.closest('details[data-disclosure]');
    }
  }

  function detachScroller() {
    scroller?.removeEventListener('scroll', schedule);
    scroller = null;
    if (frame !== null) win.cancelAnimationFrame(frame);
    frame = null;
  }

  function sync({ scope: nextScope = '' } = {}) {
    syncPanels();
    if (scope !== nextScope) { choices.clear(); scope = nextScope; reveal(); }
    const nextPageKey = root.dataset.pageKey;
    const nextScroller = root.querySelector('.workspace-layout > .app-content');
    if (scroller !== nextScroller) {
      detachScroller();
      scroller = nextScroller;
      scroller?.addEventListener('scroll', schedule, { passive: true });
      reveal();
    }
    if (pageKey !== nextPageKey || !scroller) { pageKey = nextPageKey; reveal(); }
    for (const node of root.querySelectorAll('details[data-disclosure]')) {
      const key = keyFor(node);
      if (choices.has(key)) node.open = choices.get(key);
      else remember(key, node.open);
    }
    // A freshly rendered menu or dialog can appear while the header is collapsed.
    if (shouldStayOpen()) reveal();
  }

  function reset() {
    choices.clear();
    scope = undefined;
    pageKey = undefined;
    detachScroller();
    reveal();
  }

  root.addEventListener('toggle', onToggle, true);
  root.addEventListener('invalid', onInvalid, true);
  root.addEventListener('click', onPanelClick);
  doc.addEventListener('focusin', onFocus);
  return {
    sync, reset,
    dispose() {
      reset();
      root.removeEventListener('toggle', onToggle, true);
      root.removeEventListener('invalid', onInvalid, true);
      root.removeEventListener('click', onPanelClick);
      doc.removeEventListener('focusin', onFocus);
    },
  };
}

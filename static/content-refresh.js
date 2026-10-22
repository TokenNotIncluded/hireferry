/* Animate only explicitly keyed, changed text. Forms and live interaction remain immediate. */
const active = new WeakMap();
const blocked = 'form,input,textarea,select,option,[contenteditable]';
const maximum = 12;

function protectedText(node, doc) {
  if (node.closest(blocked) || node.querySelector(blocked) || node.contains(doc.activeElement)) return true;
  const selection = doc.getSelection?.();
  if (selection && !selection.isCollapsed) {
    for (let i = 0; i < Math.min(selection.rangeCount, 4); i++) {
      try { if (selection.getRangeAt(i).intersectsNode(node)) return true; } catch { return true; }
    }
  }
  return false;
}

function keyed(scope) {
  const nodes = new Map();
  for (const node of scope.querySelectorAll('[data-refresh-key]')) {
    const key = node.getAttribute('data-refresh-key');
    if (key && !nodes.has(key)) nodes.set(key, node);
  }
  return nodes;
}

export async function animateContentRefresh(root, nextHTML, commit, { isCurrent = () => true } = {}) {
  active.get(root)?.cancel();
  const current = () => root.isConnected !== false && isCurrent();
  if (!current()) return false;
  const doc = root.ownerDocument || document;
  const reduced = doc.defaultView?.matchMedia?.('(prefers-reduced-motion: reduce)');
  if (doc.hidden || reduced?.matches) { commit(); return true; }
  const template = doc.createElement('template');
  template.innerHTML = nextHTML;
  const old = keyed(root), pairs = [];
  for (const [key, next] of keyed(template.content)) {
    const node = old.get(key);
    if (!node || node.textContent === next.textContent || protectedText(node, doc) ||
        next.closest(blocked) || next.querySelector(blocked) ||
        pairs.some(pair => pair.node.contains(node) || node.contains(pair.node))) continue;
    pairs.push({ key, node, text: next.textContent, skip: false });
    if (pairs.length === maximum) break;
  }
  if (!pairs.length) { if (!current()) return false; commit(); return true; }

  let timer, release, canceled = false, cleaned = false, motionStopped = false, phase = [];
  const decorated = new Set();
  const strip = node => node.classList.remove('content-refresh-out', 'content-refresh-in');
  const finishWait = value => { clearTimeout(timer); const done = release; release = null; done?.(value); };
  const wait = delay => new Promise(resolve => { release = resolve; timer = setTimeout(() => finishWait(true), delay); });
  const cleanup = () => {
    if (cleaned) return;
    cleaned = true;
    clearTimeout(timer);
    for (const node of decorated) strip(node);
    for (const type of ['focusin', 'selectionchange', 'visibilitychange']) doc.removeEventListener(type, interaction);
    reduced?.removeEventListener?.('change', interaction);
    if (active.get(root) === task) active.delete(root);
  };
  const task = { cancel() { canceled = true; finishWait(false); cleanup(); } };
  function interaction() {
    if (!current()) { task.cancel(); return; }
    motionStopped ||= doc.hidden || reduced?.matches;
    for (const item of phase) {
      if (motionStopped || protectedText(item.node, doc)) {
        item.pair.skip = true;
        strip(item.node);
      }
    }
    if (phase.every(item => item.pair.skip)) finishWait(true);
  }
  active.set(root, task);
  for (const type of ['focusin', 'selectionchange', 'visibilitychange']) doc.addEventListener(type, interaction);
  reduced?.addEventListener?.('change', interaction);
  try {
    phase = pairs.map(pair => ({ node: pair.node, pair }));
    for (const { node } of phase) { decorated.add(node); node.classList.add('content-refresh-out'); }
    if (!await wait(200) || canceled || !current()) return false;
    for (const { node } of phase) strip(node);
    commit();
    if (!current() || motionStopped || doc.hidden || reduced?.matches) return true;
    const updated = keyed(root);
    phase = pairs.filter(pair => !pair.skip).map(pair => ({ pair, node: updated.get(pair.key) })).filter(({ node, pair }) => node && node.textContent === pair.text && !protectedText(node, doc));
    for (const { node } of phase) { decorated.add(node); node.classList.add('content-refresh-in'); }
    if (phase.length) await wait(180);
    return true;
  } finally { cleanup(); }
}

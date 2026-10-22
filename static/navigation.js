/* Update the page in place. Shared navigation keeps its DOM, focus and scroll. */
const elementKey = node => {
  if (node.nodeType !== 1) return '';
  return node.id || node.getAttribute('data-dom-key') ||
    (node.tagName === 'A' ? `a:${node.getAttribute('href')}` : '') ||
    (node.hasAttribute('data-action') ? `action:${node.getAttribute('data-action')}` : '');
};

function updateNode(current, next) {
  if (current.isEqualNode(next)) return current;
  if (current.nodeType !== next.nodeType || current.nodeName !== next.nodeName) {
    current.replaceWith(next);
    return next;
  }
  if (current.nodeType !== 1) {
    current.nodeValue = next.nodeValue;
    return current;
  }
  // A new workspace page must not inherit a form value from the previous page.
  const changedPage = current.getAttribute('data-page-key') !== next.getAttribute('data-page-key');
  const changedValue = current.getAttribute('value') !== next.getAttribute('value');
  const changedChecked = current.hasAttribute('checked') !== next.hasAttribute('checked');
  for (const attribute of [...current.attributes]) {
    if (!next.hasAttribute(attribute.name)) current.removeAttribute(attribute.name);
  }
  for (const attribute of [...next.attributes]) {
    if (current.getAttribute(attribute.name) !== attribute.value) current.setAttribute(attribute.name, attribute.value);
  }
  if (changedPage) current.replaceChildren(...next.childNodes);
  else reconcileChildren(current, next);
  if (changedValue && current.tagName === 'INPUT') current.value = next.getAttribute('value') || '';
  if (changedChecked && current.tagName === 'INPUT') current.checked = next.hasAttribute('checked');
  return current;
}

export function reconcileChildren(parent, nextParent) {
  const old = [...parent.childNodes];
  const used = new Set();
  const keyed = new Map();
  const unkeyed = new Map();
  // Index siblings once. Repeatedly scanning the old list for every unkeyed
  // child makes mixed, long lists quadratic even when their nodes can be kept.
  for (const node of old) {
    const key = elementKey(node);
    if (key) keyed.set(key, node);
    else {
      let group = unkeyed.get(node.nodeName);
      if (!group) unkeyed.set(node.nodeName, group = { nodes: [], index: 0 });
      group.nodes.push(node);
    }
  }
  let position = parent.firstChild;
  for (const next of [...nextParent.childNodes]) {
    const key = elementKey(next);
    const group = key ? null : unkeyed.get(next.nodeName);
    let current = key ? keyed.get(key) : group?.nodes[group.index++];
    if (current && used.has(current)) current = null;
    if (!current) {
      parent.insertBefore(next, position);
      continue;
    }
    used.add(current);
    if (current !== position) parent.insertBefore(current, position);
    current = updateNode(current, next);
    position = current.nextSibling;
  }
  for (const node of old) if (!used.has(node) && node.parentNode === parent) node.remove();
}

export function mountPage(root, html, pageKey, { busy = false } = {}) {
  const template = document.createElement('template');
  template.innerHTML = html;
  const content = template.content.querySelector('.app-content');
  if (content) content.setAttribute('data-page-key', pageKey);
  reconcileChildren(root, template.content);
  root.dataset.pageKey = pageKey;
  const region = root.querySelector('.app-content') || root.querySelector('main');
  region?.setAttribute('aria-busy', String(busy));
  root.classList.toggle('page-pending', busy);
}

export function pageIdentity(path, tab) {
  if (path === '/admin') return `admin:${tab || 'overview'}`;
  if (path === '/app') return `app:${tab || 'messages'}`;
  return path;
}

export function pagePlaceholder(tab, label) {
  const line = width => `<span class="skeleton-line" style="width:${width}%"></span>`;
  const row = `<div class="skeleton-row">${line(26)}${line(45)}${line(15)}</div>`;
  let content;
  if (tab === 'messages') {
    content = `<div class="skeleton-chat"><aside>${row.repeat(5)}</aside><section>${line(45)}${line(64)}${line(38)}<div class="skeleton-composer"></div></section></div>`;
  } else if (tab === 'matches') {
    content = `<div class="skeleton-matching"><aside class="panel">${row.repeat(4)}</aside><section class="panel">${line(60)}${row.repeat(6)}</section></div>`;
  } else if (['mandate', 'profile', 'editor'].includes(tab)) {
    content = `<section class="panel skeleton-form">${Array.from({length:6}, () => `<div>${line(35)}<div class="skeleton-input"></div></div>`).join('')}<div class="skeleton-textarea"></div></section>`;
  } else if (tab === 'overview') {
    content = `<div class="skeleton-metrics">${Array.from({length:3}, () => `<section class="panel">${line(60)}${line(35)}</section>`).join('')}</div><section class="panel">${row.repeat(5)}</section>`;
  } else {
    content = `<section class="panel">${row.repeat(6)}</section>`;
  }
  return `<div class="page-placeholder"><span class="sr-only" role="status">${label}</span><div aria-hidden="true"><div class="page-heading skeleton-heading">${line(38)}${line(65)}</div>${content}</div></div>`;
}

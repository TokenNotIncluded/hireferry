/*
 * Load CSS that belongs to the current route without making the landing page
 * wait for workspace, chat, admin, CLI, or directory styles. The server keeps
 * this module and its sibling assets under one immutable release directory.
 */
const assetBase = new URL('./', import.meta.url);
const loaded = new Map();

const routeStyleNames = path => {
  const styles = new Set();
  const add = (...names) => names.forEach(name => styles.add(name));

  if (path === '/' || path === '/for-employers' || path === '/for-job-tools') {
    add('promo.css', 'particle-type.css', 'brand-motion.css');
  }
  if (
    path === '/discover' ||
    /^\/discover\/(jobs|people)\/[^/]+$/.test(path) ||
    /^\/u\/[A-Za-z0-9_-]+$/.test(path)
  ) {
    add('people-jobs.css', 'public-directory.css');
  }
  if (path === '/cli') add('cli-guide.css', 'cli-auth.css');
  if (path === '/downloads') add('downloads.css');
  if (path === '/app' || path.startsWith('/app/')) {
    add('workspace.css', 'people-jobs.css', 'promo.css');
    const tab = new URLSearchParams(location.search).get('tab');
    if (!tab || tab === 'messages') add('conversation.css', 'chat-renderer.css');
  }
  if (path === '/admin' || path.startsWith('/staff/invite/')) {
    add(
      'workspace.css',
      'staff.css',
      'finance.css',
      'people-jobs.css',
      'conversation.css',
      'chat-renderer.css',
      'promo.css',
    );
  }

  return [...styles];
};

function assetUrl(name) {
  return new URL(name, assetBase).href;
}

function loadStylesheet(name) {
  if (loaded.has(name)) return loaded.get(name);

  let link = document.head.querySelector(
    `link[data-route-style="${name}"], link[rel="stylesheet"][href$="/${name}"]`,
  );
  if (!link) {
    link = document.createElement('link');
    link.rel = 'stylesheet';
    link.dataset.routeStyle = name;
    link.href = assetUrl(name);
    document.head.append(link);
  }

  const promise = new Promise(resolve => {
    if (link.sheet) {
      resolve({name, ok: true});
      return;
    }
    link.addEventListener('load', () => resolve({name, ok: true}), {once: true});
    link.addEventListener('error', () => resolve({name, ok: false}), {once: true});
  });
  loaded.set(name, promise);
  return promise;
}

export function routeStyles(pathname = location.pathname) {
  return routeStyleNames(pathname).map(loadStylesheet);
}

export async function ensureRouteStyles(pathname = location.pathname) {
  const results = await Promise.all(routeStyles(pathname));
  return {failed: results.filter(result => !result.ok).map(result => result.name)};
}

// Keep a small compatibility hook for browser diagnostics and future shell
// code. The application render path is the authority and awaits the promise.
window.hireferryEnsureRouteStyles = ensureRouteStyles;
window.hireferryLoadRouteStyles = path => void ensureRouteStyles(path);

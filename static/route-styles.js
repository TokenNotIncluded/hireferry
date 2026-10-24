/*
 * Keep the landing page's critical CSS small without making the SPA depend on
 * a build step. The server rewrites /static/ URLs to the immutable release
 * prefix, so these links participate in the same cache/version boundary as
 * the rest of the module graph.
 */
const coreStyles = [
  'app.css',
  'theme-palettes.css',
  'theme.css',
  'navigation.css',
  'controls.css',
];
const assetBase = new URL('./', import.meta.url);

const routeStyles = path => {
  // The feedback controller is shared by every rendered page, but its small
  // stylesheet is still allowed to arrive after the core shell.
  const styles = new Set(['feedback.css']);
  const add = (...names) => names.forEach(name => styles.add(name));

  if (path === '/') {
    add('particle-type.css', 'brand-motion.css', 'promo.css');
  } else if (path === '/login') {
    add('auth.css');
  } else if (path === '/discover' || /^\/discover\/(jobs|people)\/[^/]+$/.test(path)) {
    add('public-directory.css');
  } else if (path === '/cli') {
    add('cli-guide.css', 'cli-auth.css');
  } else if (path === '/downloads') {
    add('downloads.css');
  } else if (path === '/app' || path.startsWith('/app/')) {
    add('workspace.css');
    const tab = new URLSearchParams(location.search).get('tab');
    if (!tab || tab === 'messages') add('conversation.css', 'chat-renderer.css');
    if (tab === 'promos') add('promo.css');
  } else if (path === '/admin') {
    add('auth.css', 'workspace.css');
    const tab = new URLSearchParams(location.search).get('tab');
    if (tab === 'messages') add('conversation.css', 'chat-renderer.css');
    if (['users', 'jobs', 'matches', 'mandates'].includes(tab)) add('people-jobs.css');
    if (['overview', 'orders'].includes(tab)) add('finance.css');
    if (['team', 'staff'].includes(tab)) add('staff.css');
  }

  return [...styles];
};

const loaded = new Set();

function loadStyle(name, {async = true} = {}) {
  if (loaded.has(name) || document.querySelector(`link[data-route-style="${name}"], link[rel="stylesheet"][href$="/${name}"]`)) {
    loaded.add(name);
    return;
  }
  const link = document.createElement('link');
  link.dataset.routeStyle = name;
  link.href = new URL(name, assetBase).href;
  if (async) {
    link.rel = 'preload';
    link.as = 'style';
    link.addEventListener('load', () => { link.rel = 'stylesheet'; }, {once: true});
    link.addEventListener('error', () => { loaded.delete(name); link.remove(); }, {once: true});
  } else {
    link.rel = 'stylesheet';
  }
  document.head.append(link);
  loaded.add(name);
}

export function loadRouteStyles(path = location.pathname) {
  coreStyles.forEach(name => loadStyle(name, {async: false}));
  routeStyles(path).forEach(name => loadStyle(name));
}

window.hireferryLoadRouteStyles = loadRouteStyles;
loadRouteStyles();

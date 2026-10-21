/* Apply the saved appearance before loading styles to avoid a light flash. */
(() => {
  'use strict';
  const key = 'hireferry-theme';
  const root = document.documentElement;
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const valid = value => ['system', 'light', 'dark'].includes(value);
  let preference = 'system';
  try {
    const saved = localStorage.getItem(key);
    if (valid(saved)) preference = saved;
  } catch { /* A browser may deny storage; appearance still works this visit. */ }

  function syncControls() {
    document.querySelectorAll('[data-theme-select]').forEach(select => {
      if (select.value !== preference) select.value = preference;
    });
  }
  function apply() {
    const theme = preference === 'system' ? (media.matches ? 'dark' : 'light') : preference;
    root.dataset.theme = theme;
    root.dataset.themePreference = preference;
    root.classList.toggle('dark', theme === 'dark');
    root.classList.toggle('light', theme === 'light');
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', theme === 'dark' ? '#111713' : '#f4f3ee');
    syncControls();
  }
  apply();
  document.addEventListener('change', event => {
    const select = event.target.closest?.('[data-theme-select]');
    if (!select || !valid(select.value)) return;
    preference = select.value;
    try { localStorage.setItem(key, preference); } catch { /* See above. */ }
    apply();
  });
  media.addEventListener('change', () => {
    if (preference === 'system') apply();
  });
  window.addEventListener('storage', event => {
    if (event.key !== key && event.key !== null) return;
    preference = valid(event.newValue) ? event.newValue : 'system';
    apply();
  });
  // The navigation is rendered again after a route or language change.
  new MutationObserver(syncControls).observe(root, {childList: true, subtree: true});
})();

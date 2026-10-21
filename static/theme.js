/* Apply the saved appearance before loading styles to avoid a light flash. */
(() => {
  'use strict';
  const key = 'hireferry-theme';
  const paletteKey = 'hireferry-palette';
  const root = document.documentElement;
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const valid = value => ['system', 'light', 'dark'].includes(value);
  const palettes = ['blue', 'violet', 'rose'];
  const surfaces = {"blue": {"light": "#f9f9ff", "dark": "#0f131c"}, "violet": {"light": "#fef7ff", "dark": "#15121c"}, "rose": {"light": "#fff8f7", "dark": "#1d1012"}};
  let preference = 'light', palette = 'blue';
  try {
    const saved = localStorage.getItem(key);
    if (valid(saved)) preference = saved;
    const savedPalette = localStorage.getItem(paletteKey);
    if (palettes.includes(savedPalette)) palette = savedPalette;
  } catch { /* A browser may deny storage; appearance still works this visit. */ }

  function syncControls() {
    document.querySelectorAll('[data-theme-select]').forEach(select => {
      if (select.value !== preference) select.value = preference;
    });
    document.querySelectorAll('[data-palette-select]').forEach(select => {
      if (select.value !== palette) select.value = palette;
    });
  }
  function apply() {
    const theme = preference === 'system' ? (media.matches ? 'dark' : 'light') : preference;
    root.dataset.theme = theme;
    root.dataset.themePreference = preference;
    root.dataset.palette = palette;
    root.classList.toggle('dark', theme === 'dark');
    root.classList.toggle('light', theme === 'light');
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', surfaces[palette][theme]);
    syncControls();
  }
  apply();
  document.addEventListener('change', event => {
    const paletteSelect = event.target.closest?.('[data-palette-select]');
    if (paletteSelect) {
      if (!palettes.includes(paletteSelect.value)) return;
      palette = paletteSelect.value;
      try { localStorage.setItem(paletteKey, palette); } catch { /* This visit still works. */ }
      apply();
      return;
    }
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
    if (![key, paletteKey, null].includes(event.key)) return;
    if (event.key === key) preference = valid(event.newValue) ? event.newValue : 'light';
    if (event.key === paletteKey) palette = palettes.includes(event.newValue) ? event.newValue : 'blue';
    if (event.key === null) { preference = 'light'; palette = 'blue'; }
    apply();
  });
  // The navigation is rendered again after a route or language change.
  new MutationObserver(syncControls).observe(root, {childList: true, subtree: true});
})();

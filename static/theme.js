/* Apply saved appearance before first paint; preferences stay in this browser. */
(() => {
  'use strict';
  const key = 'hireferry-theme', paletteKey = 'hireferry-palette';
  const root = document.documentElement;
  const media = window.matchMedia('(prefers-color-scheme: dark)');
  const valid = value => ['system', 'light', 'dark'].includes(value);
  const palettes = ['neutral', 'blue', 'violet', 'rose', 'amber', 'teal'];
  let preference = 'light', palette = 'neutral';
  try {
    const saved = localStorage.getItem(key), savedPalette = localStorage.getItem(paletteKey);
    if (valid(saved)) preference = saved;
    if (palettes.includes(savedPalette)) palette = savedPalette;
  } catch { /* Storage may be unavailable; controls still work for this visit. */ }
  function syncControls() {
    for (const [selector, property, selected] of [
      ['[data-theme-choice]', 'themeChoice', preference],
      ['[data-palette-choice]', 'paletteChoice', palette],
    ]) document.querySelectorAll(selector).forEach(button => {
      const pressed = String(button.dataset[property] === selected);
      if (button.getAttribute('aria-pressed') !== pressed) button.setAttribute('aria-pressed', pressed);
    });
  }
  function apply() {
    const theme = preference === 'system' ? (media.matches ? 'dark' : 'light') : preference;
    root.dataset.theme = theme;
    root.dataset.themePreference = preference;
    root.dataset.palette = palette;
    root.classList.toggle('dark', theme === 'dark');
    root.classList.toggle('light', theme === 'light');
    document.querySelector('meta[name="theme-color"]')?.setAttribute('content', theme === 'dark' ? '#171715' : '#f8f7f3');
    syncControls();
  }
  function closeMenus(restoreFocus = false) {
    document.querySelectorAll('.theme-menu[open]').forEach(menu => {
      menu.open = false;
      if (restoreFocus) menu.querySelector('summary')?.focus();
    });
  }
  apply();
  document.addEventListener('click', event => {
    const color = event.target.closest?.('[data-palette-choice]');
    const mode = event.target.closest?.('[data-theme-choice]');
    if (color && palettes.includes(color.dataset.paletteChoice)) {
      palette = color.dataset.paletteChoice;
      try { localStorage.setItem(paletteKey, palette); } catch { /* This visit still works. */ }
      apply();
    } else if (mode && valid(mode.dataset.themeChoice)) {
      preference = mode.dataset.themeChoice;
      try { localStorage.setItem(key, preference); } catch { /* This visit still works. */ }
      apply();
    } else if (!event.target.closest?.('.theme-menu')) closeMenus();
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape') closeMenus(true);
  });
  media.addEventListener('change', () => { if (preference === 'system') apply(); });
  window.addEventListener('storage', event => {
    if (![key, paletteKey, null].includes(event.key)) return;
    if (event.key === key) preference = valid(event.newValue) ? event.newValue : 'light';
    if (event.key === paletteKey) palette = palettes.includes(event.newValue) ? event.newValue : 'neutral';
    if (event.key === null) { preference = 'light'; palette = 'neutral'; }
    apply();
  });
  new MutationObserver(syncControls).observe(root, {childList: true, subtree: true});
})();

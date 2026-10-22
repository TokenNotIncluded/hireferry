// SPDX-License-Identifier: AGPL-3.0-or-later

/** One delegated pointer listener survives the retained navigation shell. */
export function mountBrandMotion(root = document) {
  const media = matchMedia('(prefers-reduced-motion: reduce)');
  const fine = matchMedia('(hover: hover) and (pointer: fine)');
  const documentRoot = document.documentElement;
  const brands = new Set();
  const visibility = new WeakMap();
  let active = null, frame = null, disposed = false, pending = null, paused = false;
  const clear = brand => {
    if (!brand) return;
    for (const property of ['--brand-x', '--brand-y', '--brand-turn']) brand.style.removeProperty(property);
  };
  const reset = () => {
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null; pending = null; clear(active); active = null;
    documentRoot.dataset.brandMotion = document.hidden || media.matches || paused ? 'paused' : 'playing';
  };
  const move = event => {
    if (disposed || media.matches || !fine.matches || paused || document.hidden) return;
    const brand = event.target.closest?.('.brand');
    if (!brand) return;
    if (active !== brand) { clear(active); active = brand; }
    pending = { brand, x: event.clientX, y: event.clientY };
    if (frame === null) frame = requestAnimationFrame(() => {
      frame = null;
      const { brand: target, x, y } = pending;
      const bounds = target.getBoundingClientRect();
      const horizontal = Math.max(-1, Math.min(1, (x - bounds.left) / bounds.width * 2 - 1));
      const vertical = Math.max(-1, Math.min(1, (y - bounds.top) / bounds.height * 2 - 1));
      target.style.setProperty('--brand-x', `${horizontal * 4}px`);
      target.style.setProperty('--brand-y', `${vertical * 3 - 2}px`);
      target.style.setProperty('--brand-turn', `${horizontal * 11}deg`);
    });
  };
  const leave = event => {
    if (active && !active.contains(event.relatedTarget)) reset();
  };
  const intersection = new IntersectionObserver(entries => {
    if (disposed) return;
    for (const entry of entries) {
      if (!brands.has(entry.target)) continue;
      visibility.set(entry.target, String(entry.isIntersecting));
      entry.target.dataset.brandVisible = String(entry.isIntersecting);
    }
  });
  const sync = () => {
    if (disposed) return;
    for (const brand of brands) if (!brand.isConnected) {
      intersection.unobserve(brand); brands.delete(brand); visibility.delete(brand);
    }
    root.querySelectorAll('.brand').forEach(brand => {
      if (!brands.has(brand)) { brands.add(brand); intersection.observe(brand); }
      // Navigation reconciles attributes even when it retains this element.
      const visible = visibility.get(brand) ?? 'false';
      if (brand.dataset.brandVisible !== visible) brand.dataset.brandVisible = visible;
    });
  };
  const observer = new MutationObserver(sync);
  observer.observe(root === document ? document.body : root, {
    childList: true, subtree: true, attributes: true, attributeFilter: ['data-brand-visible'],
  });
  root.addEventListener('pointermove', move, { passive: true });
  root.addEventListener('pointerout', leave, { passive: true });
  document.addEventListener('visibilitychange', reset);
  media.addEventListener('change', reset);
  fine.addEventListener('change', reset);
  sync(); reset();
  return {
    pause(value = !paused) { paused = Boolean(value); reset(); },
    dispose() {
      disposed = true; reset(); intersection.disconnect(); observer.disconnect();
      root.removeEventListener('pointermove', move); root.removeEventListener('pointerout', leave);
      document.removeEventListener('visibilitychange', reset);
      media.removeEventListener('change', reset); fine.removeEventListener('change', reset);
      brands.forEach(brand => { clear(brand); delete brand.dataset.brandVisible; });
      brands.clear(); delete documentRoot.dataset.brandMotion;
    },
  };
}

(() => {
  'use strict';
  // A convenience control, not a security boundary. All authorization remains
  // in Rust and the local-only Tauri capability. No credentials enter this script.
  const local = location.hostname === 'tauri.localhost' || location.protocol === 'tauri:';
  if (local) {
    window.__HIREFERRY_MOBILE__ = true;
    return;
  }
  if (!/^https?:$/.test(location.protocol) || window !== window.top) return;
  // Mobile WebViews do not support Tauri's desktop on_new_window callback.
  // Route ordinary target=_blank links through the same native navigation
  // policy instead; the selected service stays in-app, other HTTPS origins
  // open in the system browser, and non-web schemes are denied by Rust.
  document.addEventListener('click', event => {
    const anchor = event.target instanceof Element ? event.target.closest('a[target="_blank"]') : null;
    if (!anchor || event.defaultPrevented || !/^https?:$/.test(anchor.protocol)) return;
    event.preventDefault();
    location.assign(anchor.href);
  });
  const addReturn = () => {
    if (document.getElementById('hireferry-app-settings')) return;
    const link = document.createElement('a');
    link.id = 'hireferry-app-settings';
    // Rust substitutes a fixed bundled page for this build's platform. Let the
    // WebView perform this navigation normally; no native command, redirect
    // payload, or second navigate() inside its navigation callback is needed.
    link.href = '__HIREFERRY_LAUNCHER_URL__';
    const zh = navigator.language.startsWith('zh');
    link.textContent = zh ? '连接' : 'Connection';
    link.setAttribute('aria-label', zh ? '返回聘渡应用连接设置' : 'Return to HireFerry connection settings');
    link.style.cssText = 'position:fixed;left:max(12px,env(safe-area-inset-left));bottom:calc(78px + env(safe-area-inset-bottom));z-index:79;box-sizing:border-box;display:flex;align-items:center;justify-content:center;min-width:64px;min-height:44px;padding:8px 12px;border:1px solid #cbcfc9;border-radius:24px;background:#f8f7f3;color:#292824;font:600 13px system-ui,sans-serif;text-decoration:none;box-shadow:0 2px 12px #0002;';
    document.body.append(link);
    // The site's feedback control lives on the right and its bottom navigation
    // occupies the first 64px. Keep this compact control on the left; when a
    // conversation composer grows, lift it above the composer rather than
    // covering the text field. Site dialogs/feedback panels remain above it.
    let frame = 0;
    const observed = new WeakSet();
    const liftedFeedback = new WeakSet();
    const resize = new ResizeObserver(() => schedule());
    const position = () => {
      frame = 0;
      let bottom = 78;
      let composerVisible = false;
      document.querySelectorAll('.chat-composer').forEach(composer => {
        if (!observed.has(composer)) { observed.add(composer); resize.observe(composer); }
        const rect = composer.getBoundingClientRect();
        if (rect.height && rect.bottom > 0 && rect.top < innerHeight) {
          composerVisible = true;
          bottom = Math.max(bottom, innerHeight - rect.top + 8);
        }
      });
      link.style.bottom = `max(calc(78px + env(safe-area-inset-bottom)),${Math.min(bottom, Math.max(78, innerHeight - 60))}px)`;
      // Preserve a feedback button position the user has dragged. Its default
      // mobile position otherwise covers the chat send button, so lift both
      // default controls together while the composer is visible.
      document.querySelectorAll('.feedback-launch').forEach(feedback => {
        if (feedback.style.top && feedback.style.top !== 'auto') return;
        if (composerVisible) {
          feedback.style.bottom = link.style.bottom;
          liftedFeedback.add(feedback);
        } else if (liftedFeedback.has(feedback)) {
          feedback.style.removeProperty('bottom');
          liftedFeedback.delete(feedback);
        }
      });
    };
    const schedule = () => { if (!frame) frame = requestAnimationFrame(position); };
    resize.observe(document.body);
    new MutationObserver(schedule).observe(document.body, { childList: true, subtree: true });
    addEventListener('resize', schedule);
    addEventListener('scroll', schedule, { passive: true, capture: true });
    visualViewport?.addEventListener('resize', schedule);
    schedule();
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', addReturn, { once: true });
  else addReturn();
})();

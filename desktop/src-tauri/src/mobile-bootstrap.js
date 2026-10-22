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
    link.href = 'hireferry-settings://return';
    const zh = navigator.language.startsWith('zh');
    link.textContent = zh ? '聘渡 · 连接设置' : 'HireFerry · Connection';
    link.setAttribute('aria-label', zh ? '返回聘渡应用连接设置' : 'Return to HireFerry connection settings');
    link.style.cssText = 'position:fixed;right:max(12px,env(safe-area-inset-right));bottom:max(12px,env(safe-area-inset-bottom));z-index:2147483647;display:flex;align-items:center;min-height:44px;padding:8px 14px;border:1px solid #cbcfc9;border-radius:24px;background:#f8f7f3;color:#292824;font:600 13px system-ui,sans-serif;text-decoration:none;box-shadow:0 2px 12px #0002;';
    document.body.append(link);
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', addReturn, { once: true });
  else addReturn();
})();

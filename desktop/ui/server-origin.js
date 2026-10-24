(() => {
  'use strict';
  function canonicalServerOrigin(input) {
    let url;
    try { url = new URL(input.trim()); } catch { throw 'invalid_server'; }
    const loopback = url.hostname === 'localhost' || url.hostname === '[::1]' || /^127\.\d+\.\d+\.\d+$/.test(url.hostname);
    if ((url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback)) ||
        url.username || url.password || url.port === '0' || url.pathname !== '/' ||
        url.search || url.hash || /[?#]/.test(url.href)) throw 'invalid_server';
    return url.origin;
  }
  // Input hygiene before persistence. Rust independently enforces navigation
  // policy and the capability system enforces native authorization.
  window.HireFerryConnection = Object.freeze({ canonicalServerOrigin });
})();

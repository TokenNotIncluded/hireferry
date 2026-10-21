// Continue a CLI request only on this site's consent and connection pages. This is never
// an arbitrary return URL and never grants permission without a separate click.
export const CLI_RETURN_KEY = 'hireferry-cli-return';

export function safeCliPath(value, origin) {
  if (typeof value !== 'string' || value.length > 8192) return null;
  try {
    const url = new URL(value, origin);
    if (url.origin !== origin || url.username || url.password || url.hash) return null;
    if (!['/oauth/device', '/oauth/authorize', '/oauth/connections'].includes(url.pathname)) return null;
    return url.pathname + url.search;
  } catch { return null; }
}

export function rememberCliReturn(storage, path, origin, afterAuthOnly = false, now = Date.now()) {
  const safe = safeCliPath(path, origin);
  if (!safe) return false;
  storage.setItem(CLI_RETURN_KEY, JSON.stringify({path: safe, expiresAt: now + 600000, afterAuthOnly}));
  return true;
}

export function takeCliReturn(storage, origin, identity, authenticated = false, now = Date.now()) {
  let request;
  try { request = JSON.parse(storage.getItem(CLI_RETURN_KEY)); } catch { storage.removeItem(CLI_RETURN_KEY); }
  if (!request) return null;
  const path = safeCliPath(request.path, origin);
  if (!path || !Number.isFinite(request.expiresAt) || request.expiresAt <= now || request.expiresAt > now + 600000) {
    storage.removeItem(CLI_RETURN_KEY);
    return null;
  }
  // Basic CLI identity does not require an AI-eligible role or a paid job.
  const ready = !identity.bootstrap && (identity.admin || identity.user);
  if (!ready || (request.afterAuthOnly && !authenticated)) return null;
  storage.removeItem(CLI_RETURN_KEY);
  return path;
}

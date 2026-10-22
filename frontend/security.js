// Model output and peer messages are untrusted. Rendering never grants API access.
export const MAX_SOURCE_LENGTH = 65536;
export const MAX_ACTION_LENGTH = 4096;

export function safeLink(value, base = globalThis.location?.href || 'https://hireferry.invalid/') {
  if (typeof value !== 'string' || !value || /[\u0000-\u0020\u007f]/u.test(value)) return undefined;
  try {
    const url = new URL(value, base);
    if (!['https:', 'http:', 'mailto:'].includes(url.protocol) || url.username || url.password) return undefined;
    return url.href;
  } catch {
    return undefined;
  }
}

export function safeImage(value, base = globalThis.location?.href || 'https://hireferry.invalid/') {
  if (typeof value !== 'string') return undefined;
  if (/^data:image\/(png|jpeg|gif|webp);base64,[a-z0-9+/=]+$/iu.test(value) && value.length <= MAX_SOURCE_LENGTH) return value;
  const link = safeLink(value, base);
  if (!link) return undefined;
  const url = new URL(link);
  return url.origin === new URL(base).origin ? url.href : undefined;
}

export function draftAction(event) {
  if (!event || event.type !== 'continue_conversation' || typeof event.humanFriendlyMessage !== 'string') return null;
  const text = event.humanFriendlyMessage.trim();
  if (!text || text.length > MAX_ACTION_LENGTH) return null;
  // Keep only short scalar form values. Hidden context, URLs, and tool instructions
  // supplied in an action are never implicitly executed or sent to another user.
  const formState = Object.create(null);
  for (const [key, value] of Object.entries(event.formState || {}).slice(0, 32)) {
    if (key.length > 80 || ['__proto__', 'constructor', 'prototype'].includes(key)) continue;
    if (typeof value === 'string') formState[key] = value.slice(0, 1024);
    else if (typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) formState[key] = value;
  }
  return { type: 'draft', text, formState };
}

export function boundedNumber(value, fallback = 0, min = 0, max = 96) {
  return typeof value === 'number' && Number.isFinite(value) ? Math.min(max, Math.max(min, value)) : fallback;
}

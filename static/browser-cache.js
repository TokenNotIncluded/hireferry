/* Only public site configuration survives a reload. Identity, messages, tokens,
 * profiles and payment results stay behind the existing live request/session checks. */
export const PUBLIC_CONFIG_CACHE_KEY = 'hireferry-public-config-v1';
const schema = 1;
// Four localized welcome screens and their bounded Markdown can exceed 64 KiB.
const maxLength = 512 * 1024;
const strings = ['name', 'public_origin', 'default_language', 'support_email', 'source_url'];
const flags = ['google_enabled', 'humangrant_enabled', 'email_enabled', 'email_notifications_enabled', 'development'];
const maps = ['payment_currencies', 'payment_modes', 'waffo_regions', 'cli_distribution', 'pricing', 'limits', 'chat'];
const fields = [...strings, ...flags, ...maps, 'providers'];
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const stringList = value => Array.isArray(value) && value.every(item => typeof item === 'string');
const copy = value => JSON.parse(JSON.stringify(value));
const stable = value => JSON.stringify(value, function (_key, item) {
  return object(item) ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item;
});
export const samePublicConfig = (left, right) => stable(left) === stable(right);

function publicChat(value) {
  if (!object(value.welcome)) throw new TypeError('Invalid public chat configuration');
  const audiences = ['platform_seeker', 'platform_employer', 'matched', 'staff'];
  const textLimits = { title: 160, title_en: 160, body_md: 4000, body_md_en: 4000, body_html: 65536, body_html_en: 65536 };
  const promptLimits = { label: 80, label_en: 80, body: 2000, body_en: 2000 };
  const textFields = (source, limits) => {
    const selected = {};
    for (const [key, limit] of Object.entries(limits)) {
      if (!Object.hasOwn(source, key)) continue;
      const text = source[key];
      if (text !== null && (typeof text !== 'string' || [...text].length > limit)) throw new TypeError('Invalid public chat text');
      selected[key] = text;
    }
    return selected;
  };
  return { welcome: Object.fromEntries(Object.entries(value.welcome).map(([audience, entry]) => {
    if (!audiences.includes(audience) || !object(entry)) throw new TypeError('Invalid public chat audience');
    const selected = textFields(entry, textLimits);
    if (entry.prompts !== undefined && entry.prompts !== null) {
      if (!Array.isArray(entry.prompts) || entry.prompts.length > 6) throw new TypeError('Invalid public chat prompts');
      selected.prompts = entry.prompts.map(prompt => {
        if (!object(prompt) || typeof prompt.label !== 'string' || !prompt.label.trim() || typeof prompt.body !== 'string' || !prompt.body.trim()) throw new TypeError('Invalid public chat prompt');
        return textFields(prompt, promptLimits);
      });
    }
    return [audience, selected];
  })) };
}

function publicConfig(value) {
  if (!object(value) || typeof value.name !== 'string' || !value.name.trim()) throw new TypeError('Invalid public configuration');
  // An explicit allowlist prevents future API additions from accidentally persisting account data.
  const selected = Object.fromEntries(fields.filter(key => Object.hasOwn(value, key)).map(key => [key, value[key]]));
  if (strings.some(key => key in selected && typeof selected[key] !== 'string') ||
      flags.some(key => key in selected && typeof selected[key] !== 'boolean') ||
      maps.some(key => key in selected && !object(selected[key])) ||
      ('providers' in selected && !stringList(selected.providers))) throw new TypeError('Invalid public configuration');
  const pricing = selected.pricing;
  if (pricing && (['allowed_currencies'].some(key => key in pricing && !stringList(pricing[key])) ||
      ['currency_options', 'activation_currency_options'].some(key => key in pricing &&
        (!Array.isArray(pricing[key]) || !pricing[key].every(item => object(item) && typeof item.currency === 'string'))))) throw new TypeError('Invalid public pricing');
  if (selected.limits && !Object.values(selected.limits).every(value => Number.isSafeInteger(value) && value >= 0)) throw new TypeError('Invalid public limits');
  if (selected.payment_currencies && !Object.values(selected.payment_currencies).every(stringList)) throw new TypeError('Invalid payment currencies');
  if (selected.waffo_regions && !Object.values(selected.waffo_regions).every(value => object(value) && stringList(value.currencies) && stringList(value.payment_methods))) throw new TypeError('Invalid payment regions');
  if (selected.cli_distribution && (typeof selected.cli_distribution.release_repository !== 'string' ||
      !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(selected.cli_distribution.release_repository))) throw new TypeError('Invalid CLI source');
  if (selected.chat) selected.chat = publicChat(selected.chat);
  const encoded = JSON.stringify(selected);
  if (encoded.length > maxLength) throw new TypeError('Public configuration is too large');
  return JSON.parse(encoded);
}

function browserStorage() {
  try { return globalThis.localStorage; } catch { return null; }
}

export function createPublicConfigCache(load, { storage = browserStorage(), now = Date.now, maxAge = 24 * 60 * 60 * 1000 } = {}) {
  let entry = null;
  let pending = null;
  let revision = 0;
  let verified = false;
  const remove = () => { try { storage?.removeItem(PUBLIC_CONFIG_CACHE_KEY); } catch { /* Storage may be unavailable. */ } };
  const validTime = savedAt => Number.isFinite(savedAt) && savedAt <= now() && now() - savedAt < maxAge;
  try {
    const saved = storage?.getItem(PUBLIC_CONFIG_CACHE_KEY);
    if (saved) {
      if (saved.length > maxLength) throw new Error('Oversized cache');
      const decoded = JSON.parse(saved);
      if (decoded.schema !== schema || !validTime(decoded.savedAt)) throw new Error('Expired cache');
      entry = { savedAt: decoded.savedAt, value: publicConfig(decoded.value) };
    }
  } catch { remove(); }

  function peek() {
    if (entry && !validTime(entry.savedAt)) { entry = null; verified = false; remove(); }
    return entry ? copy(entry.value) : null;
  }

  function invalidate() {
    revision += 1;
    pending = null;
    entry = null;
    verified = false;
    remove();
  }

  async function refresh() {
    if (pending) return copy(await pending);
    const started = revision;
    const ensureCurrent = () => { if (revision !== started) throw new DOMException('Configuration changed', 'AbortError'); };
    const operation = Promise.resolve().then(() => { ensureCurrent(); return load(); }).then(result => {
      ensureCurrent();
      const value = publicConfig(result), previous = peek();
      const changed = previous !== null && !samePublicConfig(previous, value);
      entry = { savedAt: now(), value };
      verified = true;
      try { storage?.setItem(PUBLIC_CONFIG_CACHE_KEY, JSON.stringify({ schema, ...entry })); } catch { /* Keep the in-memory result when storage is full or blocked. */ }
      return { value, previous, changed };
    }).finally(() => { if (pending === operation) pending = null; });
    pending = operation;
    return copy(await operation);
  }

  // A stored snapshot is useful for first paint, never proof of current prices or permissions.
  return { peek, refresh, invalidate, isVerified: () => peek() !== null && verified };
}

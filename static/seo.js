export function syncPageMetadata({doc, path, lang, config, origin}) {
  let catalog = {};
  try { catalog = JSON.parse(doc.querySelector('meta[name="hireferry-seo"]')?.content || '{}'); } catch {}
  const page = catalog[path]?.[lang === 'en' ? 1 : 0];
  const site = config?.name || 'HireFerry · 聘渡';
  // Raw HTML previews do not pass through the server's metadata renderer.
  if (!Object.keys(catalog).length) {
    doc.title = `${site} · ${lang === 'en' ? 'People-first hiring' : '人才双向匹配服务'}`;
    return;
  }
  const title = page ? `${page[0]} · ${site}` : `${site} · ${lang === 'en' ? 'Account and workspace' : '账户与工作台'}`;
  const description = page?.[1] || (lang === 'en' ? 'Sign in to HireFerry to manage your account, matches and conversations.' : '登录聘渡 HireFerry，管理你的账户、匹配与会话。');
  const meta = (key, value, property = false) => {
    const attr = property ? 'property' : 'name';
    let element = doc.querySelector(`meta[${attr}="${key}"]`);
    if (value === null) { element?.remove(); return; }
    if (!element) { element = doc.createElement('meta'); element.setAttribute(attr, key); doc.head.append(element); }
    element.content = value;
  };
  doc.title = title;
  meta('description', description);
  meta('robots', page ? 'index, follow, max-image-preview:large' : 'noindex, follow');
  let canonical = doc.querySelector('link[rel="canonical"]');
  const url = new URL(path, config?.public_origin || origin).href;
  if (page) {
    if (!canonical) { canonical = doc.createElement('link'); canonical.rel = 'canonical'; doc.head.append(canonical); }
    canonical.href = url;
  } else canonical?.remove();
  for (const [key, value] of Object.entries({type:'website', site_name:site, title, description, url})) meta(`og:${key}`, page ? value : null, true);
  meta('twitter:card', page ? 'summary' : null);
}

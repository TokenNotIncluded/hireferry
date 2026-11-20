function plainText(value, limit = 500) {
  return String(value || '')
    .replace(/<[^>]*>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, limit);
}

function detailMetadata(path, record) {
  const isJob = path.startsWith('/discover/jobs/');
  if (isJob) {
    const title = record?.title || '公开岗位';
    const employer = record?.employer?.name || '公开招聘方';
    return {
      title,
      description: `${employer}正在公开招聘 ${title}，岗位说明和招聘人数可公开查看。`,
      structured: {
        '@context': 'https://schema.org',
        '@type': 'JobPosting',
        title,
        description: plainText(record?.jd_html),
        url: record?.url,
        datePosted: record?.created_at ? new Date(Number(record.created_at) * 1000).toISOString() : undefined,
        hiringOrganization: {'@type': 'Organization', name: employer},
        baseSalary: record?.currency && Number.isFinite(Number(record.salary_cents)) ? {
          '@type': 'MonetaryAmount',
          currency: record.currency,
          value: {
            '@type': 'QuantitativeValue',
            value: Number(record.salary_cents) / (10 ** Number(record.currency_decimals || 0)),
            unitText: 'MONTH',
          },
        } : undefined,
      },
    };
  }
  const name = record?.name || record?.handle || '公开简历';
  const links = Object.values(record?.ai_profile_links || {}).filter(url => /^https?:\/\//.test(url));
  return {
    title: name,
    description: `${name} 的公开求职主页（@${record?.handle || ''}），包含本人主动公开的简历内容和个人主页链接。`,
    structured: {
      '@context': 'https://schema.org',
      '@type': 'Person',
      name,
      identifier: record?.handle,
      url: record?.url,
      description: plainText(record?.resume_html),
      sameAs: links.length ? links : undefined,
    },
  };
}

export function syncPageMetadata({doc, path, lang, config, origin, publicRecord}) {
  let catalog = {};
  try { catalog = JSON.parse(doc.querySelector('meta[name="hireferry-seo"]')?.content || '{}'); } catch {}
  const page = catalog[path]?.[lang === 'en' ? 1 : 0];
  const detail = publicRecord && detailMetadata(path, publicRecord);
  const site = config?.name || 'HireFerry · 聘渡';
  // Raw HTML previews do not pass through the server's metadata renderer.
  if (!Object.keys(catalog).length) {
    doc.title = `${site} · ${lang === 'en' ? 'People-first hiring' : '人才双向匹配服务'}`;
    return;
  }
  const title = detail ? `${detail.title} · ${site}` : page ? `${page[0]} · ${site}` : `${site} · ${lang === 'en' ? 'Account and workspace' : '账户与工作台'}`;
  const description = detail?.description || page?.[1] || (lang === 'en' ? 'Sign in to HireFerry to manage your account, matches and conversations.' : '登录聘渡 HireFerry，管理你的账户、匹配与会话。');
  const meta = (key, value, property = false) => {
    const attr = property ? 'property' : 'name';
    let element = doc.querySelector(`meta[${attr}="${key}"]`);
    if (value === null) { element?.remove(); return; }
    if (!element) { element = doc.createElement('meta'); element.setAttribute(attr, key); doc.head.append(element); }
    element.content = value;
  };
  doc.title = title;
  meta('description', description);
  meta('robots', detail || page ? 'index, follow, max-image-preview:large' : 'noindex, follow');
  let canonical = doc.querySelector('link[rel="canonical"]');
  const url = new URL(path, config?.public_origin || origin).href;
  if (detail || page) {
    if (!canonical) { canonical = doc.createElement('link'); canonical.rel = 'canonical'; doc.head.append(canonical); }
    canonical.href = url;
  } else canonical?.remove();
  for (const [key, value] of Object.entries({type:'website', site_name:site, title, description, url})) meta(`og:${key}`, detail || page ? value : null, true);
  meta('twitter:card', detail || page ? 'summary' : null);
  doc.querySelector('script[data-hireferry-jsonld]')?.remove();
  if (detail?.structured) {
    const script = doc.createElement('script');
    script.type = 'application/ld+json';
    script.dataset.hireferryJsonld = 'true';
    script.textContent = JSON.stringify(detail.structured);
    doc.head.append(script);
  }
}

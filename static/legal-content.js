const escape = value => String(value ?? '').replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));

export function renderLegal(documentContent, lang = 'zh') {
  const t = (zh, en) => lang === 'en' ? en : zh;
  const content = document.createElement('template');
  content.innerHTML = documentContent.html;
  for (const link of content.content.querySelectorAll('a[href]')) {
    if (/^\/(?![\\/])/.test(link.getAttribute('href') || '')) link.setAttribute('data-nav', '');
  }
  const title = content.content.querySelector('h1');
  const heading = title?.textContent || t('服务说明', 'Service information');
  title?.remove();
  const headings = Array.from(content.content.querySelectorAll('h2'));
  const links = headings.map((element, index) => {
    if (!/^[a-z][a-z0-9_-]*$/.test(element.id)) element.id = `section-${index + 1}`;
    return `<a href="#${escape(element.id)}">${index + 1}. ${escape(element.textContent)}</a>`;
  }).join('');
  const metadata = [
    documentContent.version ? `${t('版本', 'Version')} ${escape(documentContent.version)}` : '',
    documentContent.effective_date ? `${t('更新及生效日期', 'Updated and effective')}: ${escape(documentContent.effective_date)}` : '',
  ].filter(Boolean).join(' · ');
  return `<main id="main" class="page-width"><article class="prose legal-document"><h1>${escape(heading)}</h1>${metadata ? `<p class="legal-meta">${metadata}</p>` : ''}<nav class="legal-toc" aria-label="${t('文档目录', 'Document contents')}">${links}</nav>${content.innerHTML}</article></main>`;
}

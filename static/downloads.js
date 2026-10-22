// Public release data only. Engineering targets and CI artifacts never become downloads.
export const DOWNLOAD_REPOSITORY = 'TokenNotIncluded/hireferry';
export const DOWNLOAD_RELEASES = `https://github.com/${DOWNLOAD_REPOSITORY}/releases`;
const API = `https://api.github.com/repos/${DOWNLOAD_REPOSITORY}/releases?per_page=10`;
const CACHE_KEY = 'hireferry-downloads-v1';
const FRESH_MS = 30 * 60 * 1000, MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000;
const PRODUCTS = ['cli', 'desktop', 'android', 'ios', 'server'];
const PLATFORMS = ['Windows', 'macOS', 'Linux', 'Android', 'iOS'];
const FORMATS = ['app.tar.gz', 'tar.gz', 'zip', 'deb', 'rpm', 'AppImage', 'dmg', 'exe', 'msi', 'flatpak', 'apk', 'aab', 'ipa'];
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const TAG = /^(desktop|cli|server|android|ios|mobile)-v((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))$/;
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const text = lang => (zh, en) => lang === 'en' ? en : zh;
const supported = {cli:['Windows','macOS','Linux'], desktop:['Windows','macOS','Linux'], android:['Android'], ios:['iOS'], server:['Linux']};

function compareVersions(a, b) {
  const left = a.split('.').map(Number), right = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) if (left[i] !== right[i]) return left[i] - right[i];
  return 0;
}

function assetRecord(asset, allowMissingDigest = false) {
  if (!asset || !PRODUCTS.includes(asset.product) || !PLATFORMS.includes(asset.platform) ||
      !supported[asset.product].includes(asset.platform) || !FORMATS.includes(asset.format) ||
      !VERSION.test(asset.version || '') || !['x86_64','aarch64','universal','armv7'].includes(asset.architecture) ||
      !/^[A-Za-z0-9_.+-]{1,180}$/.test(asset.name || '') || /(?:debug|simulator|unsigned|ad-hoc)/i.test(asset.name) ||
      !Number.isSafeInteger(asset.size) || asset.size <= 0) throw new Error('invalid_manifest');
  const tag = TAG.exec(asset.tag || '');
  if (!tag || tag[2] !== asset.version || (tag[1] !== asset.product && !(tag[1] === 'mobile' && ['android','ios'].includes(asset.product)))) throw new Error('invalid_manifest');
  const formats = ['cli','server'].includes(asset.product) ? (asset.platform === 'Windows' ? ['zip'] : ['tar.gz'])
    : {Windows:['exe','msi'], macOS:['dmg','app.tar.gz'], Linux:['deb','rpm','AppImage','flatpak'], Android:['apk','aab'], iOS:['ipa']}[asset.platform];
  if (!formats.includes(asset.format) || !asset.name.endsWith('.' + asset.format) ||
      (['cli','server','desktop'].includes(asset.product) && !asset.name.startsWith(`hireferry-${asset.product}-${asset.version}-`))) throw new Error('invalid_manifest');
  const expected = `${DOWNLOAD_RELEASES}/download/${asset.tag}/${encodeURIComponent(asset.name)}`;
  if (asset.url !== expected || (!/^[a-f0-9]{64}$/i.test(asset.sha256 || '') && !(allowMissingDigest && !asset.sha256))) throw new Error('invalid_manifest');
  return {product:asset.product, platform:asset.platform, architecture:asset.architecture,
    target:typeof asset.target === 'string' ? asset.target.slice(0,90) : '', format:asset.format,
    version:asset.version, name:asset.name, size:asset.size, sha256:asset.sha256?.toLowerCase() || null,
    tag:asset.tag, url:expected};
}

export function normalizeDownloadManifest(manifest, {allowMissingDigest = false} = {}) {
  if (manifest?.schema_version !== 1 || manifest.repository !== DOWNLOAD_REPOSITORY ||
      ![undefined,'release'].includes(manifest.distribution) || !Array.isArray(manifest.assets) ||
      manifest.assets.length > 200 || !manifest.versions || !manifest.releases) throw new Error('invalid_manifest');
  const assets = manifest.assets.map(item => {
    const product = item.product === 'mobile' ? ({Android:'android', iOS:'ios'}[item.platform] || item.product) : item.product;
    const asset = assetRecord({...item, product}, allowMissingDigest);
    const key = item.product;
    if (manifest.versions[key] !== asset.version || manifest.releases[key] !== asset.tag) throw new Error('invalid_manifest');
    return asset;
  });
  if (new Set(assets.map(a => a.url)).size !== assets.length) throw new Error('invalid_manifest');
  const versions = {}, releases = {};
  for (const asset of assets) { versions[asset.product] = asset.version; releases[asset.product] = asset.tag; }
  return {schema_version:1, repository:DOWNLOAD_REPOSITORY, distribution:'release', versions, releases, assets};
}

function recognizeAsset(product, version, asset, tag) {
  const name = asset.name || '';
  if (asset.state !== 'uploaded' || /(?:debug|simulator|unsigned|ad-hoc)/i.test(name)) return null;
  let platform, architecture;
  if (/(?:linux|\.deb$|\.rpm$|\.AppImage$|\.flatpak$)/i.test(name)) platform = 'Linux';
  else if (/(?:windows|\.exe$|\.msi$)/i.test(name)) platform = 'Windows';
  else if (/(?:darwin|\.dmg$)/i.test(name)) platform = 'macOS';
  else if (/\.apk$|\.aab$/i.test(name)) platform = 'Android';
  else if (/\.ipa$/i.test(name)) platform = 'iOS';
  if (/(?:aarch64|arm64)/i.test(name)) architecture = 'aarch64';
  else if (/(?:x86_64|amd64|x64)/i.test(name)) architecture = 'x86_64';
  else if (/universal/i.test(name)) architecture = 'universal';
  else if (/armv7/i.test(name)) architecture = 'armv7';
  const format = FORMATS.find(value => name.endsWith('.' + value));
  const actualProduct = product === 'mobile' ? ({Android:'android', iOS:'ios'}[platform]) : product;
  if (!platform || !architecture || !format || !actualProduct) return null;
  try {
    return assetRecord({product:actualProduct, version, platform, architecture, format, name,
      size:asset.size, sha256:asset.digest?.startsWith('sha256:') ? asset.digest.slice(7) : null,
      tag, url:asset.browser_download_url}, true);
  } catch { return null; }
}

// One bounded page can discover newer packages. Absence here never proves a product has no release.
export function normalizeGitHubReleases(releases) {
  if (!Array.isArray(releases) || releases.length > 10) throw new Error('invalid_manifest');
  const result = {schema_version:1, repository:DOWNLOAD_REPOSITORY, distribution:'release', versions:{}, releases:{}, assets:[]};
  for (const release of releases) {
    const tag = TAG.exec(release.tag_name || '');
    if (release.draft !== false || release.prerelease !== false || !release.published_at || !tag ||
        release.html_url !== `${DOWNLOAD_RELEASES}/tag/${release.tag_name}` || !Array.isArray(release.assets) || release.assets.length > 200) continue;
    for (const item of release.assets) {
      const asset = recognizeAsset(tag[1], tag[2], item, release.tag_name);
      if (!asset) continue;
      const previous = result.versions[asset.product];
      if (previous && compareVersions(asset.version, previous) < 0) continue;
      if (previous && compareVersions(asset.version, previous) > 0) result.assets = result.assets.filter(a => a.product !== asset.product);
      result.versions[asset.product] = asset.version; result.releases[asset.product] = asset.tag;
      if (!result.assets.some(a => a.url === asset.url)) result.assets.push(asset);
    }
  }
  return result;
}

export function mergeDownloadCatalogs(base, incoming) {
  if (!base) return incoming;
  const result = structuredClone(base);
  for (const product of PRODUCTS) {
    const version = incoming.versions[product];
    if (!version || (result.versions[product] && compareVersions(version, result.versions[product]) <= 0)) continue;
    result.assets = result.assets.filter(a => a.product !== product).concat(incoming.assets.filter(a => a.product === product));
    result.versions[product] = version; result.releases[product] = incoming.releases[product];
  }
  return result;
}

async function readJson(response, maximum) {
  if (Number(response.headers?.get('content-length')) > maximum) throw new Error('response_too_large');
  if (!response.body?.getReader) {
    const body = await response.text();
    if (new TextEncoder().encode(body).byteLength > maximum) throw new Error('response_too_large');
    return JSON.parse(body);
  }
  const reader = response.body.getReader(), parts = []; let size = 0;
  try {
    while (true) {
      const {done, value} = await reader.read(); if (done) break;
      size += value.byteLength;
      if (size > maximum) { await reader.cancel(); throw new Error('response_too_large'); }
      parts.push(value);
    }
  } finally { reader.releaseLock(); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const part of parts) { bytes.set(part, offset); offset += part.byteLength; }
  return JSON.parse(new TextDecoder().decode(bytes));
}

export function createDownloadSource({fetcher = (...args) => fetch(...args), storage = null, now = () => Date.now()} = {}) {
  let current = null, pending = null, checkedAt = 0, blockedUntil = 0, blockedStatus = 'rate_limited', loaded = false;
  const stored = (() => {
    try {
      const value = JSON.parse(storage?.getItem(CACHE_KEY) || 'null');
      if (Number.isFinite(value?.blockedUntil)) blockedUntil = Math.min(value.blockedUntil, now() + 60 * 60 * 1000);
      blockedStatus = value?.blockedStatus === 'unavailable' ? 'unavailable' : 'rate_limited';
      if (!value || !Number.isFinite(value.checkedAt) || value.checkedAt > now() || now() - value.checkedAt > MAX_AGE_MS) return null;
      const catalog = normalizeDownloadManifest(value.catalog, {allowMissingDigest:true});
      checkedAt = value.checkedAt;
      return {catalog, source:'cache', checkedAt, status:'cached'};
    } catch { return null; }
  })();
  async function request(url, maximum) {
    const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 7000);
    try {
      const response = await fetcher(url, {credentials:'omit', referrerPolicy:'no-referrer', signal:controller.signal,
        headers:{Accept:'application/json'}, cache:'no-cache'});
      if (!response.ok) {
        if ([403,429].includes(response.status)) {
          const reset = Number(response.headers?.get('x-ratelimit-reset')) * 1000;
          const retry = Number(response.headers?.get('retry-after')) * 1000 + now();
          blockedUntil = Math.min(now() + 60 * 60 * 1000, Math.max(now() + 60000, reset || 0, retry || 0));
          blockedStatus = 'rate_limited';
          throw new Error('rate_limited');
        }
        throw new Error('unavailable');
      }
      return await readJson(response, maximum);
    } finally { clearTimeout(timer); }
  }
  function save() {
    try { storage?.setItem(CACHE_KEY, JSON.stringify({catalog:current?.catalog || null, checkedAt, blockedUntil, blockedStatus})); } catch {}
  }
  return {
    peek: () => current,
    load() {
      if (loaded) return Promise.resolve(current);
      if (pending) return pending;
      pending = (async () => {
        try {
          const catalog = normalizeDownloadManifest(await request('/static/downloads.json', 256 * 1024));
          current = {catalog:mergeDownloadCatalogs(catalog, stored?.catalog || catalog), source:stored ? 'snapshot-cache' : 'snapshot', checkedAt, status:stored ? 'cached' : 'ready'};
        } catch { current = stored || {catalog:null, source:'none', checkedAt:0, status:'unavailable'}; }
        loaded = true; return current;
      })().finally(() => { pending = null; });
      return pending;
    },
    async refresh() {
      if (pending) await pending;
      if (pending) return pending;
      if (blockedUntil > now()) return current = {...current, status:blockedStatus};
      if (checkedAt && now() - checkedAt < FRESH_MS) return current = {...current, status:'cached'};
      pending = (async () => {
        try {
          const incoming = normalizeGitHubReleases(await request(API, 1024 * 1024));
          checkedAt = now(); blockedUntil = 0;
          current = {catalog:mergeDownloadCatalogs(current?.catalog, incoming), source:'github', checkedAt, status:'checked'};
          save();
        } catch (error) {
          if (error.message !== 'rate_limited') { blockedUntil = now() + 60000; blockedStatus = 'unavailable'; }
          current = {...current, status:error.message === 'rate_limited' ? 'rate_limited' : 'unavailable'}; save();
        }
        return current;
      })().finally(() => { pending = null; });
      return pending;
    },
  };
}

function productInfo(lang) {
  const t = text(lang);
  return {
    cli:{name:'hfy CLI', note:t('在终端登录、查询，以及连接 AI 助手。','Sign in, query your account and connect AI assistants from a terminal.'), icon:'key', link:'/cli', action:t('安装与使用教程','Installation guide')},
    desktop:{name:t('桌面客户端','Desktop app'), note:'Windows · macOS · Linux', icon:'monitor'},
    android:{name:'Android', note:t('手机与平板客户端','Phone and tablet app'), icon:'panel'},
    ios:{name:'iOS', note:t('iPhone 与 iPad 客户端','iPhone and iPad app'), icon:'panel'},
    server:{name:t('自托管服务端','Self-hosted server'), note:t('在自己的 Linux 服务器部署聘渡。','Run HireFerry on your own Linux server.'), icon:'globe', link:`https://github.com/${DOWNLOAD_REPOSITORY}/blob/main/docs/self-hosting.md`, action:t('部署指南','Deployment guide')},
  };
}

function formatSize(bytes) { return `${(bytes / 1024 / 1024).toFixed(2)} MiB`; }

function packageTable(assets, lang) {
  const t = text(lang), arch = {x86_64:'x64', aarch64:'ARM64', universal:t('通用','Universal'), armv7:'ARMv7'};
  return `<table class="download-table"><caption class="sr-only">${t('正式发布的安装包','Published release packages')}</caption><thead><tr><th scope="col">${t('系统 / 架构','System / architecture')}</th><th scope="col">${t('格式','Format')}</th><th scope="col">${t('大小','Size')}</th><th scope="col">${t('下载与校验','Download & checksum')}</th></tr></thead><tbody>${assets.sort((a,b) => PLATFORMS.indexOf(a.platform)-PLATFORMS.indexOf(b.platform) || a.architecture.localeCompare(b.architecture) || a.format.localeCompare(b.format)).map(a => `<tr data-download-asset="${escape(a.name)}"><th scope="row"><strong>${escape(a.platform)}</strong><span>${arch[a.architecture]}</span></th><td class="download-format">.${escape(a.format)}</td><td class="download-size">${formatSize(a.size)}</td><td class="download-actions"><a class="download-file" href="${escape(a.url)}" rel="noopener noreferrer" aria-label="${escape(`${t('下载','Download')} ${a.product} ${a.version} · ${a.platform} ${arch[a.architecture]} .${a.format}`)}">${t('下载','Download')} <span aria-hidden="true">↓</span></a><details><summary>${t('文件信息','File details')}</summary><div class="download-file-info"><span>${escape(a.name)}</span>${a.sha256 ? `<span>SHA-256</span><code>${a.sha256}</code>` : `<span>${t('接口未提供 SHA-256；请查看该版本发布说明。','The API did not provide a SHA-256 digest. Check the release notes.')}</span>`}<a href="${DOWNLOAD_RELEASES}/tag/${escape(a.tag)}" target="_blank" rel="noopener noreferrer">${t('发布说明','Release notes')} ↗</a></div></details></td></tr>`).join('')}</tbody></table>`;
}

function resultsMarkup(result, lang, platform, icon) {
  const t = text(lang), catalog = result?.catalog, info = productInfo(lang);
  return PRODUCTS.filter(product => platform === 'all' || supported[product].includes(platform)).map(product => {
    const all = catalog?.assets.filter(a => a.product === product) || [];
    const assets = all.filter(a => platform === 'all' || a.platform === platform);
    const data = info[product], version = catalog?.versions[product];
    return `<section class="download-product ${assets.length ? 'has-downloads' : ''}" data-download-product="${product}"><div class="download-product-heading"><span class="download-product-icon" aria-hidden="true">${icon(data.icon)}</span><div><h3>${escape(data.name)}${version ? `<span class="download-version">v${escape(version)}</span>` : ''}</h3><p>${escape(data.note)}</p></div><span class="download-release-status ${assets.length ? 'is-published' : ''}">${assets.length ? t('已发布','Published') : !catalog ? t('待确认','Not confirmed') : all.length ? t('此平台暂无包','No package for this platform') : result.source === 'snapshot' ? t('尚未发布','Not yet released') : t('清单暂无包','No package in catalog')}</span></div>${assets.length ? packageTable(assets, lang) : `<p class="download-unavailable">${!catalog ? t('发布清单暂时无法读取。请检查 GitHub 正式发布记录。','The release catalog is unavailable. Check the official GitHub releases.') : all.length ? t('这个版本没有提供所选系统的正式安装包。','This version has no published package for the selected system.') : t('当前清单中尚无正式安装包。构建验证中的产物不作为正式下载提供。','No official package is in the current catalog. Build-validation artifacts are not offered as releases.')}</p>`}${data.link ? `<a class="download-guide" href="${escape(data.link)}" ${data.link.startsWith('/') ? 'data-nav' : 'target="_blank" rel="noopener noreferrer"'}>${escape(data.action)} <span aria-hidden="true">↗</span></a>` : ''}</section>`;
  }).join('');
}

function sourceMessage(result, lang) {
  const t = text(lang);
  if (!result) return t('正在读取发布清单…','Reading the release catalog…');
  const messages = {
    ready:t('来源：随站点提供的正式发布清单。可用版本以清单为准。','Source: the official release snapshot shipped with this site. Versions reflect this snapshot.'),
    checked:t('已检查 GitHub 最近 10 条发布记录，并保留清单中已有的较新版本。','Checked the 10 most recent GitHub releases; newer versions already in the catalog are retained.'),
    cached:t('使用已验证格式的发布清单缓存。GitHub 检查结果缓存 30 分钟。','Using the validated release catalog cache. GitHub checks are cached for 30 minutes.'),
    rate_limited:t('GitHub 暂时限制请求。保留现有清单，请稍后检查或直接打开正式发布页。','GitHub is limiting requests. The existing catalog is retained; try later or open the official releases page.'),
    unavailable:result.catalog ? t('暂时无法更新；继续显示已有发布清单。可直接查看 GitHub 正式发布页。','Could not refresh; showing the existing catalog. You can check the official GitHub releases directly.') : t('无法读取发布清单，暂不推测可下载的安装包。请检查 GitHub 正式发布页。','The release catalog could not be read. No package availability is assumed; check the official releases page.'),
  };
  return messages[result.status] || messages.unavailable;
}

export function renderDownloads({lang = 'zh', icon = () => ''} = {}) {
  const t = text(lang);
  return `<main id="main" class="page-width downloads-page" data-downloads-page>
    <section class="downloads-hero" aria-labelledby="downloads-title"><div><p class="eyebrow">HIREFERRY / DOWNLOADS</p><h1 id="downloads-title">${t('选择你的使用方式。','Choose how you work.')}</h1><p class="downloads-intro">${t('浏览器、终端，或自己的服务器。<br>在这里查看各平台正式发布的版本与安装包。','Your browser, terminal, or your own server.<br>Find published versions and packages for each platform.')}</p><a class="download-inline-link" href="/cli" data-nav>${t('CLI 与 AI 技能使用教程','CLI & AI skill guide')} ${icon('arrow')}</a></div><aside class="download-web"><span class="download-product-icon">${icon('globe')}</span><div><span class="download-web-label">${t('随时开始','READY WHEN YOU ARE')}</span><h2>${t('网页工作台','Web workspace')}</h2><p>${t('无需安装，电脑与手机都能使用。','No installation. Works on desktop and mobile.')}</p></div><a class="button" href="/app" data-nav>${t('打开工作台','Open workspace')} ${icon('arrow')}</a></aside></section>
    <section class="download-catalog" aria-labelledby="download-matrix-title"><div class="download-section-heading"><div><p class="eyebrow">PLATFORMS & RELEASES</p><h2 id="download-matrix-title">${t('平台与版本','Platforms & versions')}</h2></div><a href="${DOWNLOAD_RELEASES}" target="_blank" rel="noopener noreferrer">GitHub Releases <span aria-hidden="true">↗</span></a></div>
      <div class="download-filters" role="group" aria-label="${t('按系统筛选下载','Filter downloads by system')}">${['all',...PLATFORMS].map(platform => `<button type="button" data-download-platform="${platform}" aria-pressed="${platform === 'all'}">${platform === 'all' ? t('全部系统','All systems') : platform}</button>`).join('')}</div>
      <div class="download-source"><p role="status" aria-live="polite" data-download-source>${sourceMessage(null,lang)}</p><button type="button" class="download-refresh" data-download-refresh>${t('检查 GitHub 发布','Check GitHub releases')}</button></div>
      <div class="download-products" data-download-results aria-busy="true"><p class="download-loading">${t('正在读取轻量发布清单…','Loading the small release catalog…')}</p></div>
    </section>
    <section class="download-source-build" aria-labelledby="download-source-title"><span class="download-product-icon">${icon('file')}</span><div><h2 id="download-source-title">${t('Arch Linux / AUR 与源码','Arch Linux / AUR & source')}</h2><p>${t('AUR 的 PKGBUILD 是社区构建配方，需要自行构建或获取它指定的上游包。源码配方不等于官方安装包；AUR 上架状态请以包页面为准。','AUR PKGBUILDs are community build recipes. They build from source or fetch a specified upstream package. A source recipe is not an official installer; check the package page for AUR availability.')}</p><div><a href="https://github.com/${DOWNLOAD_REPOSITORY}/blob/main/docs/aur.md" target="_blank" rel="noopener noreferrer">${t('AUR 安装说明','AUR instructions')} ↗</a><a href="https://github.com/${DOWNLOAD_REPOSITORY}" target="_blank" rel="noopener noreferrer">${t('查看源代码','View source')} ↗</a></div></div></section>
    <p class="download-footnote">${t('默认只读取小型发布清单，不加载完整附件列表。检查更新由你主动触发；限流或离线时保留可用缓存（最多 7 天）。校验值用于核对文件完整性，不代表平台签名或公证。','The page loads a small release catalog by default, not the full attachment list. Update checks run only on request; usable cached checks last up to 7 days. Checksums verify file integrity, not platform signing or notarization.')}</p>
  </main>`;
}

let sharedSource, selectedPlatform = 'all';
export function mountDownloads(root, {lang = 'zh', icon = () => '', source} = {}) {
  const page = root.querySelector('[data-downloads-page]'); if (!page) return {dispose(){}};
  if (!source) {
    let storage; try { storage = globalThis.localStorage; } catch {}
    source = sharedSource ||= createDownloadSource({storage});
  }
  let active = true, result = source.peek(), platform = selectedPlatform, loading = false;
  const filters = page.querySelector('.download-filters'), results = page.querySelector('[data-download-results]');
  const status = page.querySelector('[data-download-source]'), refresh = page.querySelector('[data-download-refresh]');
  function paint() {
    if (!active || !page.isConnected) return;
    results.innerHTML = resultsMarkup(result, lang, platform, icon); results.setAttribute('aria-busy', String(loading));
    status.textContent = sourceMessage(result, lang); refresh.disabled = loading;
    for (const button of filters.querySelectorAll('button')) button.setAttribute('aria-pressed', String(button.dataset.downloadPlatform === platform));
  }
  function filter(event) { const button = event.target.closest('[data-download-platform]'); if (button && filters.contains(button)) { platform = selectedPlatform = button.dataset.downloadPlatform; paint(); } }
  async function check() { if (loading) return; loading = true; paint(); result = await source.refresh(); loading = false; paint(); }
  filters.addEventListener('click', filter); refresh.addEventListener('click', check);
  if (result) paint();
  void source.load().then(value => { result = value; paint(); });
  return {dispose(){active = false; filters.removeEventListener('click',filter); refresh.removeEventListener('click',check);}};
}

#!/usr/bin/env python3
"""One release catalog drives CI, verified assets, Markdown, JSON and SVG.

Uses Python's standard library. Build jobs have no publishing privileges; only a
complete, checksum-verified set can be assembled for the publishing job.
"""
import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import shutil
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / 'release/targets.json'
SNAPSHOT = ROOT / 'release/latest.json'
VERSION_RE = re.compile(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?')
REPOSITORY_RE = re.compile(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+')
ARCH_LABELS = {'x86_64':'x64', 'aarch64':'ARM64'}
GROUPS = ('desktop', 'flatpak', 'cli', 'server')
PRODUCTS = ('desktop', 'cli', 'server')
PRODUCT_LABELS = {'desktop': '桌面版', 'cli': 'hfy CLI', 'server': '服务端'}
CHANNELS = ('mobile', 'nix')


def group_product(group):
    return 'desktop' if group == 'flatpak' else group

LABELS = {'exe':'NSIS .exe', 'msi':'.msi', 'dmg':'.dmg', 'app.tar.gz':'.app.tar.gz',
          'deb':'.deb', 'rpm':'.rpm', 'AppImage':'.AppImage', 'flatpak':'.flatpak',
          'tar.gz':'.tar.gz', 'zip':'.zip'}


def version(value):
    match = VERSION_RE.fullmatch(value) if isinstance(value, str) else None
    if not match or any(p.isdigit() and len(p) > 1 and p[0] == '0' for p in (match[4] or '').split('.')):
        raise ValueError(f'Invalid release version: {value!r}')
    return value


def repository(value):
    if not isinstance(value, str) or not REPOSITORY_RE.fullmatch(value) or '..' in value:
        raise ValueError('Invalid GitHub repository')
    return value


def validate_channels(data):
    """Validate non-archive distribution channels kept beside package targets."""
    channels = data.get('channels', {})
    if not isinstance(channels, dict) or set(channels) - set(CHANNELS):
        raise ValueError('Unexpected release channel fields')
    mobile = channels.get('mobile')
    if mobile is not None:
        if (not isinstance(mobile, dict) or set(mobile) != {'kind', 'platforms', 'url', 'docs'} or
                mobile.get('kind') != 'responsive-web' or
                mobile.get('platforms') != ['Android', 'iOS'] or
                not isinstance(mobile.get('url'), str) or
                not re.fullmatch(r'https://[A-Za-z0-9][A-Za-z0-9._~:/?#\[\]@!$&()*+,;=%-]{0,240}', mobile['url']) or
                not isinstance(mobile.get('docs'), str) or not re.fullmatch(r'docs/[A-Za-z0-9._/-]+\.md', mobile['docs'])):
            raise ValueError('Invalid mobile release channel')
    nix = channels.get('nix')
    if nix is not None:
        if (not isinstance(nix, dict) or set(nix) != {'kind', 'systems', 'packages', 'flake', 'docs'} or
                nix.get('kind') != 'flake' or
                not isinstance(nix.get('systems'), list) or
                nix['systems'] != ['x86_64-linux', 'aarch64-linux'] or
                not isinstance(nix.get('packages'), list) or
                nix['packages'] != ['server', 'cli'] or
                not isinstance(nix.get('flake'), str) or
                not re.fullmatch(r'github:[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', nix['flake']) or
                not isinstance(nix.get('docs'), str) or not re.fullmatch(r'docs/[A-Za-z0-9._/-]+\.md', nix['docs'])):
            raise ValueError('Invalid Nix release channel')


def validate_catalog(data):
    """Validate both checked-in targets and lower-trust published snapshots."""
    if not isinstance(data, dict) or data.get('schema_version') != 1:
        raise ValueError('Unsupported release catalog schema')
    required = {'schema_version', 'repository', 'desktop', 'flatpak', 'cli'}
    if not required <= set(data) or set(data) - required - {'server', 'channels'}:
        raise ValueError('Unexpected release catalog fields')
    repository(data['repository'])
    validate_channels(data)
    native_formats = {'Windows':{'exe','msi'}, 'macOS':{'dmg','app.tar.gz'},
                      'Linux':{'deb','rpm','AppImage'}}
    system_marker = {'Windows':'-pc-windows-', 'macOS':'-apple-darwin', 'Linux':'-unknown-linux-'}
    bundle_name = {'exe':'nsis', 'msi':'msi', 'dmg':'dmg', 'app.tar.gz':'app',
                   'deb':'deb', 'rpm':'rpm', 'AppImage':'appimage'}
    for group in GROUPS:
        rows = data.get(group, [])
        if not isinstance(rows, list):
            raise ValueError('Catalog targets must be lists')
        targets = set()
        for row in rows:
            required = {'runner', 'platform', 'architecture', 'target', 'formats'}
            if group == 'desktop': required.add('bundles')
            if group == 'flatpak': required.add('arch')
            if not isinstance(row, dict) or set(row) != required:
                raise ValueError('Unexpected target fields')
            if not isinstance(row['platform'], str) or row['platform'] not in native_formats:
                raise ValueError('Invalid target platform')
            arch, target = row['architecture'], row['target']
            if not isinstance(arch, str) or not re.fullmatch(r'[a-z0-9_]{1,32}', arch):
                raise ValueError('Invalid target architecture')
            if (not isinstance(target, str) or not re.fullmatch(r'[a-z0-9_]+(?:-[a-z0-9_]+)+', target)
                    or not target.startswith(arch + '-') or system_marker[row['platform']] not in target):
                raise ValueError('Invalid target triple')
            if target in targets:
                raise ValueError(f'Duplicate {group} target')
            targets.add(target)
            if not isinstance(row['runner'], str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,79}', row['runner']):
                raise ValueError('Invalid runner')
            formats = row['formats']
            if (not isinstance(formats, list) or not formats or
                    any(not isinstance(fmt, str) or fmt not in LABELS for fmt in formats) or
                    len(formats) != len(set(formats))):
                raise ValueError('Unknown or duplicate package format')
            allowed = native_formats[row['platform']] if group == 'desktop' else (
                {'flatpak'} if group == 'flatpak' else {'zip' if row['platform'] == 'Windows' else 'tar.gz'})
            if not set(formats) <= allowed:
                raise ValueError('Package format does not match platform')
            if group == 'flatpak' and (row['platform'] != 'Linux' or row['arch'] != arch):
                raise ValueError('Invalid Flatpak architecture')
            if group == 'server' and row['platform'] != 'Linux':
                raise ValueError('Server packages require Linux')
            if group == 'desktop':
                bundles = row['bundles']
                if (not isinstance(bundles, str) or len(bundles.split(',')) != len(formats) or
                        set(bundles.split(',')) != {bundle_name[fmt] for fmt in formats}):
                    raise ValueError('Bundle list does not match formats')
    return data


def catalog():
    return validate_catalog(json.loads(CATALOG.read_text()))


def package_version(product):
    path = ROOT / {'desktop':'desktop/src-tauri/Cargo.toml', 'cli':'crates/hireferry-cli/Cargo.toml', 'server':'Cargo.toml'}[product]
    match = re.search(r'^version\s*=\s*"([^"]+)"\s*$', path.read_text(), re.M)
    if not match:
        raise ValueError(f'Missing package version in {path}')
    return version(match[1])


def filename(product, release_version, target, fmt):
    return f'hireferry-{product}-{version(release_version)}-{target}.{fmt}'


def expected(data, versions, products=('desktop', 'cli')):
    result = {}
    for group in GROUPS:
        product = group_product(group)
        if product not in products:
            continue
        for row in data.get(group, []):
            for fmt in row['formats']:
                name = filename(product, versions[product], row['target'], fmt)
                result[name] = {k:row[k] for k in ('platform', 'architecture', 'target')}
                result[name].update(product=product, version=versions[product], format=fmt)
    return result


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as file:
        for chunk in iter(lambda: file.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def checksums(folder):
    files = sorted(p for p in folder.iterdir() if p.is_file() and p.name != 'SHA256SUMS')
    (folder / 'SHA256SUMS').write_text(''.join(f'{digest(p)}  {p.name}\n' for p in files))


def find_target(data, group, target):
    return next((row for row in data[group] if row['target'] == target), None)


def stage_desktop(args):
    row = find_target(catalog(), 'desktop', args.target)
    if row is None:
        raise ValueError('Target is not in the desktop catalog')
    release_version = version(args.version or package_version('desktop'))
    sources = {}
    for fmt in row['formats']:
        pattern = '*.app' if fmt == 'app.tar.gz' else '*.' + fmt
        found = list(args.bundle_dir.rglob(pattern))
        if len(found) != 1 or found[0].is_symlink():
            raise ValueError(f'Expected exactly one {fmt} bundle, got {found}')
        source = found[0]
        if (fmt == 'app.tar.gz' and not source.is_dir()) or (fmt != 'app.tar.gz' and (not source.is_file() or source.stat().st_size == 0)):
            raise ValueError(f'Invalid bundle: {source}')
        sources[fmt] = source
    args.out.mkdir(parents=True, exist_ok=False)
    for fmt, source in sources.items():
        destination = args.out / filename('desktop', release_version, args.target, fmt)
        if fmt == 'app.tar.gz':
            with tarfile.open(destination, 'w:gz') as archive:
                archive.add(source, arcname=source.name)
        else:
            shutil.copyfile(source, destination)
    checksums(args.out)


def stage_flatpak(args):
    if not find_target(catalog(), 'flatpak', args.target):
        raise ValueError('Target is not in the Flatpak catalog')
    if not args.binary.is_file() or args.binary.is_symlink() or args.binary.stat().st_size == 0:
        raise ValueError('Flatpak bundle must be a nonempty regular file')
    release_version = version(args.version or package_version('desktop'))
    args.out.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(args.binary, args.out / filename('desktop', release_version, args.target, 'flatpak'))
    checksums(args.out)


def stage_server(args):
    if not find_target(catalog(), 'server', args.target):
        raise ValueError('Target is not in the server catalog')
    if not args.binary.is_file() or args.binary.is_symlink() or not args.binary.stat().st_size:
        raise ValueError('Server binary must be a nonempty regular file')
    release_version = version(args.version or package_version('server'))
    args.out.mkdir(parents=True, exist_ok=False)
    name = filename('server', release_version, args.target, 'tar.gz')
    with tempfile.TemporaryDirectory(prefix='hireferry-server-') as temporary:
        package = Path(temporary) / name.removesuffix('.tar.gz')
        package.mkdir()
        shutil.copyfile(args.binary, package/'hireferry')
        (package/'hireferry').chmod(0o755)
        for path in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES', 'config.example.toml', 'config.development.toml'):
            shutil.copyfile(ROOT/path, package/path)
        for path in ('docs', 'deploy', 'legal'):
            shutil.copytree(ROOT/path, package/path)
        (package/'BUILD-INFO.txt').write_text(f'package=hireferry\nversion={release_version}\ntarget={args.target}\n')
        with tarfile.open(args.out/name, 'w:gz') as archive:
            archive.add(package, arcname=package.name)
    checksums(args.out)


def verified_inputs(source, wanted):
    """Reject missing/extra packages, duplicate assets, traversal and stale digests."""
    found = {}
    for checksum in sorted(source.rglob('SHA256SUMS')):
        if checksum.is_symlink():
            raise ValueError('Checksum manifest must be a regular file')
        for line in checksum.read_text().splitlines():
            match = re.fullmatch(r'([0-9a-fA-F]{64}) [ *]([^/\\]+)', line)
            if not match:
                raise ValueError(f'Malformed checksum in {checksum}')
            name = match[2]
            if name not in wanted or name in found:
                raise ValueError(f'Unexpected or duplicate package: {name}')
            package = checksum.parent / name
            if not package.is_file() or package.is_symlink() or package.stat().st_size == 0:
                raise ValueError(f'Missing/invalid package: {name}')
            if digest(package) != match[1].lower():
                raise ValueError(f'Checksum mismatch: {name}')
            found[name] = package
        declared = {p.name for p in found.values() if p.parent == checksum.parent}
        actual = {p.name for p in checksum.parent.iterdir() if p.name != 'SHA256SUMS'}
        if actual != declared:
            raise ValueError(f'Unmanifested files in {checksum.parent}: {sorted(actual - declared)}')
    if set(found) != set(wanted):
        raise ValueError(f'Incomplete release; missing: {sorted(set(wanted) - set(found))}')
    all_files = {p for p in source.rglob('*') if p.is_file() and p.name != 'SHA256SUMS'}
    if all_files != set(found.values()):
        raise ValueError('Unmanifested files in input artifact tree')
    return found


def make_manifest(data, wanted, files, tag, repo, commit=''):
    published_catalog = json.loads(json.dumps(data))
    published_catalog['repository'] = repository(repo)
    validate_catalog(published_catalog)
    products = {item['product'] for item in wanted.values()}
    for group in GROUPS:
        if group_product(group) not in products and group in published_catalog:
            published_catalog[group] = []
    assets = []
    for name, item in sorted(wanted.items()):
        assets.append(dict(item, name=name, size=files[name].stat().st_size,
                           sha256=digest(files[name]), tag=tag,
                           url=f'https://github.com/{repo}/releases/download/{tag}/{name}'))
    return {'schema_version':1, 'repository':repo, 'catalog':published_catalog, 'source_commit':commit,
            'source_commits':{product:commit for product in sorted(products)},
            'versions':{a['product']:a['version'] for a in assets},
            'releases':{a['product']:tag for a in assets}, 'assets':assets}


def validate_manifest(manifest):
    if not isinstance(manifest, dict) or manifest.get('schema_version') != 1:
        raise ValueError('Unsupported download manifest schema')
    repo = repository(manifest['repository'])
    data = validate_catalog(manifest['catalog'])
    if data['repository'] != repo:
        raise ValueError('Snapshot catalog repository mismatch')
    if not isinstance(manifest['versions'], dict) or not isinstance(manifest['releases'], dict):
        raise ValueError('Invalid product metadata')
    products = set(manifest['versions'])
    if not products or products - set(PRODUCTS):
        raise ValueError('Unknown manifest products')
    if set(manifest['releases']) != products:
        raise ValueError('Release products differ from version products')
    represented = {group_product(group) for group in GROUPS if data.get(group)}
    if represented != products:
        raise ValueError('Catalog products differ from published products')
    commits = manifest.get('source_commits', {product:manifest.get('source_commit', '') for product in products})
    if (not isinstance(commits, dict) or set(commits) != products or
            any(not isinstance(commit, str) or not re.fullmatch(r'[0-9a-f]{0,64}', commit) for commit in commits.values())):
        raise ValueError('Invalid per-product source commits')
    for value in manifest['versions'].values():
        version(value)
    wanted = expected(data, manifest['versions'], products)
    assets = manifest['assets']
    if (not isinstance(assets, list) or any(not isinstance(a, dict) or not isinstance(a.get('name'), str) for a in assets) or
            len(assets) != len(wanted) or {a.get('name') for a in assets} != set(wanted)):
        raise ValueError('Manifest does not match the complete release catalog')
    for asset in assets:
        name = asset['name']
        if any(asset.get(k) != v for k, v in wanted[name].items()):
            raise ValueError(f'Incorrect asset metadata: {name}')
        tag = asset['tag']
        if not isinstance(tag, str) or not re.fullmatch(r'(desktop|cli|server)-v[^/]+', tag):
            raise ValueError('Invalid release tag')
        version(tag.split('-v', 1)[1])
        if tag != manifest['releases'][asset['product']]:
            raise ValueError('Release tag differs from product metadata')
        if ((asset['product'] == 'desktop' and tag != 'desktop-v' + asset['version']) or
                (asset['product'] == 'cli' and (tag.startswith('server-') or (tag.startswith('cli-') and tag != 'cli-v' + asset['version']))) or
                (asset['product'] == 'server' and tag != 'server-v' + asset['version'])):
            raise ValueError('Release tag version mismatch')
        if asset['url'] != f'https://github.com/{repo}/releases/download/{tag}/{name}':
            raise ValueError('Invalid asset download URL')
        if not isinstance(asset['size'], int) or isinstance(asset['size'], bool) or asset['size'] <= 0:
            raise ValueError('Invalid package size')
        if not isinstance(asset['sha256'], str) or not re.fullmatch(r'[0-9a-f]{64}', asset['sha256']):
            raise ValueError('Invalid package checksum')
    return manifest


def stable_version(value):
    if '-' in version(value):
        raise ValueError('Only stable releases update the download documentation')
    return tuple(map(int, value.split('.')))


def merge_manifests(current, incoming):
    """Keep the newest stable release of each independently versioned product."""
    validate_manifest(incoming)
    if current:
        validate_manifest(current)
        if current['repository'] != incoming['repository']:
            raise ValueError('Cannot merge releases from different repositories')
    selected = {}
    for source in (current, incoming):
        if source is None:
            continue
        for product, product_version in source['versions'].items():
            candidate = stable_version(product_version)
            if product not in selected or candidate > stable_version(selected[product]['versions'][product]):
                selected[product] = source
    repo = incoming['repository']
    merged = {'schema_version':1, 'repository':repo,
              'catalog':{'schema_version':1, 'repository':repo, **{group:[] for group in GROUPS}},
              'source_commit':'', 'source_commits':{}, 'versions':{}, 'releases':{}, 'assets':[]}
    for source in (incoming, current):
        if source and source['catalog'].get('channels'):
            merged['catalog']['channels'] = json.loads(json.dumps(source['catalog']['channels']))
            break
    for product in PRODUCTS:
        if product not in selected:
            continue
        source = selected[product]
        merged['versions'][product] = source['versions'][product]
        merged['releases'][product] = source['releases'][product]
        merged['source_commits'][product] = source.get('source_commits', {}).get(product, source.get('source_commit', ''))
        for group in (('desktop', 'flatpak') if product == 'desktop' else (product,)):
            merged['catalog'][group] = source['catalog'][group]
        merged['assets'].extend(asset for asset in source['assets'] if asset['product'] == product)
    merged['assets'].sort(key=lambda asset: asset['name'])
    commits = set(merged['source_commits'].values())
    if len(commits) == 1:
        merged['source_commit'] = next(iter(commits))
    return validate_manifest(json.loads(json.dumps(merged)))


def human_size(size):
    return f'{size / 1048576:.2f} MiB' if size >= 1048576 else f'{size / 1024:.1f} KiB'


def svg(data, manifest=None):
    """Accessible, repository-native vector artwork; values come from the catalog."""
    if manifest: data = manifest['catalog']
    count = len(manifest['assets']) if manifest else sum(len(row['formats']) for group in GROUPS for row in data.get(group, []))
    subtitle = ('RELEASE ASSETS VERIFIED · ' + ' / '.join(dict.fromkeys(manifest['releases'][p] for p in PRODUCTS if p in manifest['releases']))) if manifest else 'BUILD TARGETS · DOWNLOADS APPEAR AFTER A COMPLETE RELEASE'
    subtitle = html.escape(subtitle)
    headline = 'HireFerry · Every desktop, one ferry.' if not manifest or 'desktop' in manifest['versions'] else ('HireFerry · Your service. Your ferry.' if 'server' in manifest['versions'] else 'HireFerry · One CLI. Every platform.')
    cards = []
    for i, (platform, symbol) in enumerate((('Windows','window'),('macOS','apple'),('Linux','terminal'))):
        x = 36 + i * 302
        if symbol == 'window':
            mark = f'<path d="M{x+24} 132h16v16h-16zm20 0h16v16h-16zm-20 20h16v16h-16zm20 0h16v16h-16z" fill="#c3d9cb"/>'
        elif symbol == 'apple':
            mark = f'<rect x="{x+23}" y="132" width="39" height="28" rx="4" fill="none" stroke="#c3d9cb" stroke-width="2"/><path d="M{x+17} 166h51m-27-6v6" stroke="#c3d9cb" stroke-width="2"/>'
        else:
            mark = f'<rect x="{x+22}" y="132" width="42" height="34" rx="6" fill="none" stroke="#c3d9cb" stroke-width="2"/><path d="m{x+31} 142 7 7-7 7m13 0h12" fill="none" stroke="#c3d9cb" stroke-width="2"/>'
        desktop_rows = [row for group in ('desktop', 'flatpak') for row in data[group] if row['platform'] == platform]
        cli_rows = [row for row in data['cli'] if row['platform'] == platform]
        server_rows = [row for row in data.get('server', []) if row['platform'] == platform]
        rows = desktop_rows or cli_rows or server_rows
        formats = ' · '.join(dict.fromkeys('Flatpak' if fmt == 'flatpak' else LABELS[fmt] for row in rows for fmt in row['formats'])) or 'No release assets'
        architectures = ' + '.join(dict.fromkeys(ARCH_LABELS.get(row['architecture'], row['architecture']) for row in desktop_rows + cli_rows + server_rows))
        products = ' + '.join(label for label, present in (('Desktop', desktop_rows), ('hfy CLI', cli_rows), ('Server', server_rows)) if present) or 'No published packages'
        cards.append(f'<rect x="{x}" y="112" width="278" height="206" rx="18" fill="#343b35"/>{mark}<text x="{x+24}" y="200" class="platform">{platform}</text><text x="{x+24}" y="225" class="detail">{html.escape(architectures)}</text><text x="{x+24}" y="258" class="formats">{html.escape(formats)}</text><text x="{x+24}" y="286" class="detail">{products}</text>')
    channels = data.get('channels', {})
    channel_note = ' · Mobile Web · Nix Flake' if channels.get('mobile') and channels.get('nix') else ''
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="960" height="392" viewBox="0 0 960 392" role="img" aria-labelledby="title description">
<title id="title">HireFerry release matrix</title><desc id="description">Package availability by operating system and architecture; {count} package variants{channel_note}. {'Published package checksums are verified.' if manifest else 'Configured build targets, not a claim of published or tested packages.'}</desc>
<defs><linearGradient id="paper" x2="1" y2="1"><stop stop-color="#f5f2e9"/><stop offset="1" stop-color="#e2e6db"/></linearGradient></defs>
<style>text{{font-family:Inter,system-ui,sans-serif}}.platform{{font-size:25px;font-weight:650;fill:#fbfaf6}}.detail{{font-size:14px;fill:#bfcac0}}.formats{{font-size:12px;fill:#dce6de}}</style>
<rect width="960" height="392" rx="24" fill="url(#paper)"/>
<path d="M38 42h40L68 57H48zM56 21v17H41zM62 25l13 13H62z" fill="#343b35"/>
<text x="94" y="47" font-size="26" font-weight="650" fill="#29332c">{headline}</text>
<text x="38" y="82" font-size="10.5" letter-spacing="1.2" fill="#526455">{subtitle}</text>
{''.join(cards)}
<text x="38" y="353" font-size="13" fill="#455747">{count} package variants · SHA-256 verification · one release catalog{channel_note}</text><path d="M682 350h236" stroke="#809484" stroke-dasharray="3 7"/><circle cx="918" cy="350" r="4" fill="#526d59"/>
</svg>
'''


def markdown(data, manifest=None, image='assets/release-matrix.svg', docs_url=''):
    if manifest: data = manifest['catalog']
    repo = manifest['repository'] if manifest else data['repository']
    guide = docs_url + 'desktop.md'
    cli_guide = docs_url + 'cli.md'
    mobile = data.get('channels', {}).get('mobile')
    nix = data.get('channels', {}).get('nix')
    mobile_guide = docs_url + 'mobile.md'
    nix_guide = docs_url + 'nix.md'
    lines = ['<!-- Generated by scripts/release.py; edit release/targets.json, not this file. -->',
             '# 聘渡 · 桌面版、移动版、CLI、服务端与 Nix', '', f'![HireFerry 发布矩阵]({image})', '',
             f'[安装与发布说明]({guide}) · [移动版说明]({mobile_guide}) · [Nix 安装]({nix_guide}) · [CLI 使用指南]({cli_guide}) · '
             f'[所有 Releases](https://github.com/{repo}/releases)', '']
    if manifest:
        lines += ['下表来自已汇总校验的发布资产；大小为实际下载体积，SHA-256 见对应 Release 的 `SHA256SUMS`。', '']
        if 'desktop' not in manifest['versions']:
            lines += ['**尚未收录已发布的桌面安装包。** 桌面构建目标见 `release/targets.json`。', '']
        for product in PRODUCTS:
            if product in manifest['versions']:
                tag = manifest['releases'][product]
                lines.append(f"- **{PRODUCT_LABELS[product]} {manifest['versions'][product]}**：[Release](https://github.com/{repo}/releases/tag/{tag}) · [校验文件](https://github.com/{repo}/releases/download/{tag}/SHA256SUMS)")
    else:
        lines += ['**当前展示 CI 配置的构建目标，尚未记录一次完整桌面 Release。** 这些条目不是实机验证记录；完整构建、校验并发布后，工作流会替换为实际下载链接、版本和包大小。', '']
    lines += ['', '## 渠道矩阵', '', '| 渠道 | 当前交付 | 入口 |', '| --- | --- | --- |']
    if mobile:
        lines.append(f"| 移动版 Web（Android / iOS 浏览器） | 已提供响应式工作台；当前没有原生 APK / IPA | [打开移动版]({mobile['url']}) · [说明]({mobile_guide}) |")
    if nix:
        lines.append(f"| Nix / NixOS | Flake 提供 `server` 与 `cli`，支持 x86_64-linux / aarch64-linux | [`{nix['flake']}`]({nix_guide}) |")
    lines.append('| 原生 Android / iOS | 当前没有可验证的正式安装包 | 继续使用移动版 Web |')
    lines += ['', '## 已发布安装包', '', '| 组件 | 系统 | 架构 | 软件包 | 下载体积 |', '| --- | --- | --- | --- | --- |']
    if manifest:
        for a in sorted(manifest['assets'], key=lambda a:(a['product'],a['platform'],a['architecture'],a['format'])):
            lines.append(f"| {PRODUCT_LABELS[a['product']]} | {a['platform']} | {ARCH_LABELS.get(a['architecture'], a['architecture'])} | [{LABELS[a['format']]}]({a['url']}) | {human_size(a['size'])} |")
    else:
        for group in GROUPS:
            for row in data.get(group, []):
                lines.append(f"| {PRODUCT_LABELS[group_product(group)]} | {row['platform']} | {ARCH_LABELS.get(row['architecture'], row['architecture'])} | {'、'.join(LABELS[f] for f in row['formats'])} | 发布后生成 |")
    lines += ['', '## Arch Linux / AUR', '',
              '服务端与客户端 CLI 独立安装，支持 x86_64 和 aarch64。同一组件的 `-git` 与 `-bin` 互斥，服务端与 CLI 可以共存。', '',
              '| 组件 | 跟随 main 编译 | 安装 Release 二进制 |', '| --- | --- | --- |']
    for product in ('server', 'cli'):
        names = [f'hireferry-{product}-{variant}' for variant in ('git', 'bin')]
        lines.append(f"| {PRODUCT_LABELS[product]} | [{names[0]}](https://aur.archlinux.org/packages/{names[0]}) | [{names[1]}](https://aur.archlinux.org/packages/{names[1]}) |")
    lines += ['', f'安装、服务初始化与发布流程见 [AUR 指南]({docs_url}aur.md)。AUR 配方不计入上面的 Release 二进制数量。', '']
    lines += ['', '## Linux 选包建议', '',
              '| 发行版系列 | 桌面版优先选择 | 适用范围 |', '| --- | --- | --- |',
              '| Ubuntu、Debian、Linux Mint、Pop!_OS、Zorin、elementary | `.deb`；依赖不匹配时用 Flatpak | Ubuntu 22.04 / Debian 12 基线及提供 WebKitGTK 4.1 的后续版本 |',
              '| Fedora、openSUSE、RHEL / Rocky / Alma 衍生版 | `.rpm`；缺少所需库时用 Flatpak | 安装器依赖检查通过的版本；旧企业版不承诺原生运行 |',
              '| Arch、Manjaro、EndeavourOS、Gentoo | AppImage 或 Flatpak | AppImage 仍受宿主 glibc 与图形环境限制 |',
              '| NixOS | Nix Flake 的 `server` / `cli`；桌面版可用 Flatpak | `nix profile install github:TokenNotIncluded/hireferry#cli`；桌面版仍需图形运行时 |',
              '| 不可变桌面、SteamOS | Flatpak | 先配置发行版支持的 Flatpak 服务、运行时和桌面 portal |',
              '| Alpine / musl 系统 | CLI 使用 musl 包；桌面优先 Flatpak | 原生 glibc 桌面包不适用；Flatpak 可用性取决于发行版配置 |',
              '', 'Linux CLI 的静态 musl 包可用于 glibc 与 musl 发行版；桌面版需要图形会话。Flatpak 首次安装共享运行时需要额外空间，后续应用可复用。',
              '', 'Windows 的 NSIS 包按需下载 WebView2；macOS 使用系统 WebKit。桌面版不附带服务器或数据库，业务页面连接所选 HireFerry 站点。',
              '', f'**兼容范围不是“所有历史发行版本均实测”。** 架构、运行时和系统依赖满足要求才适用；具体要求、安装命令与签名说明见 [桌面版文档]({guide})。',
              '', '维护者：`release/targets.json` 同时定义二进制目标和移动 Web / Nix 渠道；已发布下载表固定使用 `release/latest.json` 内的目录快照。桌面版、CLI 与服务端按各自的稳定版本更新，旧版本不会覆盖较新的产品条目；修改构建目标不会改写历史包。运行 `python3 scripts/release.py docs --check` 检查生成文件。', '']
    return '\n'.join(lines)


def assemble(args, products=('desktop', 'cli')):
    data = catalog()
    versions = {product:package_version(product) for product in products}
    primary = next(product for product in PRODUCTS if product in products)
    if args.tag != primary + '-v' + versions[primary]:
        raise ValueError(f'Release tag must match the {primary} Cargo package version')
    repo = repository(args.repository or data['repository'])
    wanted = expected(data, versions, products)
    files = verified_inputs(args.source, wanted)
    manifest = validate_manifest(make_manifest(data, wanted, files, args.tag, repo, args.commit))
    args.destination.mkdir(parents=True, exist_ok=False)
    for name, path in files.items():
        shutil.copyfile(path, args.destination/name)
    (args.destination/'VERSION').write_text(versions.get('cli', versions[primary]) + '\n')
    (args.destination/'downloads.json').write_text(json.dumps(manifest, indent=2) + '\n')
    (args.destination/'downloads.md').write_text(markdown(data, manifest, f'https://github.com/{repo}/releases/download/{args.tag}/release-matrix.svg', f'https://github.com/{repo}/blob/{args.tag}/docs/'))
    (args.destination/'release-matrix.svg').write_text(svg(data, manifest))
    checksums(args.destination)
    print(f'Verified {len(files)} package assets for {args.tag}')


def assemble_cli(args):
    assemble(args, ('cli',))


def assemble_server(args):
    assemble(args, ('server',))


def docs(args):
    manifest_path = args.manifest or (SNAPSHOT if SNAPSHOT.exists() else None)
    manifest = validate_manifest(json.loads(manifest_path.read_text())) if manifest_path else None
    if args.manifest:
        current = json.loads(SNAPSHOT.read_text()) if SNAPSHOT.exists() else None
        manifest = merge_manifests(current, manifest)
    data = manifest['catalog'] if manifest else catalog()
    outputs = {ROOT/'docs/downloads.md':markdown(data, manifest), ROOT/'docs/assets/release-matrix.svg':svg(data, manifest)}
    if args.manifest:
        outputs[SNAPSHOT] = json.dumps(manifest, indent=2) + '\n'
    stale = []
    for path, content in outputs.items():
        if args.check:
            if not path.exists() or path.read_text() != content:
                stale.append(str(path.relative_to(ROOT)))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
    if stale:
        raise ValueError('Regenerate release docs: ' + ', '.join(stale))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    matrix = commands.add_parser('matrix', help='Emit a GitHub Actions strategy matrix')
    matrix.add_argument('product', choices=GROUPS)
    matrix.set_defaults(run=lambda a: print(json.dumps({'include':catalog()[a.product]}, separators=(',',':'))))
    for command, fn in (('stage-desktop',stage_desktop),('stage-flatpak',stage_flatpak),('stage-server',stage_server)):
        stage = commands.add_parser(command)
        stage.add_argument('--target', required=True)
        stage.add_argument('--version')
        stage.add_argument('--out', type=Path, required=True)
        stage.add_argument('--bundle-dir' if command == 'stage-desktop' else '--binary', type=Path, required=True)
        stage.set_defaults(run=fn)
    for command, fn in (('assemble', assemble), ('assemble-cli', assemble_cli), ('assemble-server', assemble_server)):
        build = commands.add_parser(command)
        build.add_argument('source', type=Path)
        build.add_argument('destination', type=Path)
        build.add_argument('--tag', required=True)
        build.add_argument('--repository')
        build.add_argument('--commit', default='')
        build.set_defaults(run=fn)
    generate = commands.add_parser('docs')
    generate.add_argument('--manifest', type=Path)
    generate.add_argument('--check', action='store_true')
    generate.set_defaults(run=docs)
    args = parser.parse_args()
    try:
        args.run(args)
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f'release: {error}\n')


if __name__ == '__main__':
    main()

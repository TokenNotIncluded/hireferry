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
import subprocess
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


def validate_catalog(data):
    """Validate both checked-in targets and lower-trust published snapshots."""
    if not isinstance(data, dict) or data.get('schema_version') != 1:
        raise ValueError('Unsupported release catalog schema')
    required = {'schema_version', 'repository', 'desktop', 'flatpak', 'cli'}
    if not required <= set(data) or set(data) - required - {'server'}:
        raise ValueError('Unexpected release catalog fields')
    repository(data['repository'])
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
            if sys.platform == 'darwin':
                # Preserve Apple metadata/stapled tickets. This is a manual-install
                # archive, not an updater payload. Verification reopens this archive.
                subprocess.run(['tar', '-czf', str(destination.resolve()), '-C',
                                str(source.parent.resolve()), source.name], check=True)
            else:
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


def signing_report_name(target):
    return f'desktop-signing-{target}.json'


def validate_signing_report(report, wanted, files=None, require_signed=False):
    if (not isinstance(report, dict) or set(report) != {'schema_version', 'target', 'assets'}
            or report['schema_version'] != 1 or not isinstance(report['assets'], dict)
            or set(report['assets']) != set(wanted) or not wanted
            or any(item['target'] != report['target'] for item in wanted.values())):
        raise ValueError('Invalid or incomplete desktop signing report')
    allowed = {'Windows':{'authenticode','unsigned'},
               'macOS':{'developer-id-notarized','ad-hoc'}, 'Linux':{'checksum-only'}}
    required = {'Windows':'authenticode', 'macOS':'developer-id-notarized', 'Linux':'checksum-only'}
    for name, entry in report['assets'].items():
        if (not isinstance(entry, dict) or not {'sha256','size','verification'} <= set(entry)
                or set(entry) - {'sha256','size','verification','publisher'}
                or not isinstance(entry['sha256'], str) or not re.fullmatch(r'[0-9a-f]{64}', entry['sha256'])
                or not isinstance(entry['size'], int) or isinstance(entry['size'], bool) or entry['size'] <= 0
                or not isinstance(entry['verification'], str)
                or entry['verification'] not in allowed[wanted[name]['platform']]):
            raise ValueError(f'Invalid native verification metadata: {name}')
        if entry['verification'] in ('authenticode','developer-id-notarized'):
            publisher = entry.get('publisher')
            if not isinstance(publisher, str) or not publisher or len(publisher) > 1000 or '\n' in publisher:
                raise ValueError('Native signing publisher must be recorded')
        elif 'publisher' in entry:
            raise ValueError('Unsigned/checksum-only packages cannot claim a signing publisher')
        if require_signed and entry['verification'] != required[wanted[name]['platform']]:
            raise ValueError(f'Formal desktop release requires native publisher signing: {name}')
        if files is not None and (entry['sha256'] != digest(files[name]) or entry['size'] != files[name].stat().st_size):
            raise ValueError(f'Signing report does not match package bytes: {name}')
    return report


def validate_desktop_signing(manifest, require_signed=False):
    reports = manifest.get('desktop_signing')
    targets = {row['target'] for row in manifest['catalog']['desktop']}
    if not isinstance(reports, dict) or set(reports) != targets or not targets:
        raise ValueError('Complete native desktop signing reports are required')
    for target, report in reports.items():
        wanted = {asset['name']:asset for asset in manifest['assets']
                  if asset['product']=='desktop' and asset['target']==target and asset['format']!='flatpak'}
        validate_signing_report(report, wanted, require_signed=require_signed)
        for name, entry in report['assets'].items():
            if entry['sha256'] != wanted[name]['sha256'] or entry['size'] != wanted[name]['size']:
                raise ValueError('Manifest and desktop signing report differ')


def require_publishable(manifest):
    if manifest.get('distribution', 'release') != 'release':
        raise ValueError('Engineering artifacts cannot update public downloads or AUR binary packages')
    if 'desktop' in manifest['versions']:
        validate_desktop_signing(manifest, require_signed=True)
    return manifest


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
    if manifest.get('distribution', 'release') not in ('release','engineering'):
        raise ValueError('Invalid distribution state')
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
    if 'desktop_signing' in manifest:
        if 'desktop' not in products:
            raise ValueError('Signing reports require desktop assets')
        validate_desktop_signing(manifest, require_signed=manifest.get('distribution','release')=='release')
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
        if product == 'desktop' and 'desktop_signing' in source:
            merged['desktop_signing'] = source['desktop_signing']
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
    if manifest and manifest.get('distribution') == 'engineering':
        subtitle = 'ENGINEERING ARTIFACTS · NOT A PUBLISHED RELEASE'
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
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="960" height="392" viewBox="0 0 960 392" role="img" aria-labelledby="title description">
<title id="title">HireFerry release matrix</title><desc id="description">Package availability by operating system and architecture; {count} package variants. {'Package integrity recorded with SHA-256.' if manifest else 'Configured build targets, not a claim of published or tested packages.'}</desc>
<defs><linearGradient id="paper" x2="1" y2="1"><stop stop-color="#f5f2e9"/><stop offset="1" stop-color="#e2e6db"/></linearGradient></defs>
<style>text{{font-family:Inter,system-ui,sans-serif}}.platform{{font-size:25px;font-weight:650;fill:#fbfaf6}}.detail{{font-size:14px;fill:#bfcac0}}.formats{{font-size:12px;fill:#dce6de}}</style>
<rect width="960" height="392" rx="24" fill="url(#paper)"/>
<path d="M38 42h40L68 57H48zM56 21v17H41zM62 25l13 13H62z" fill="#343b35"/>
<text x="94" y="47" font-size="26" font-weight="650" fill="#29332c">{headline}</text>
<text x="38" y="82" font-size="10.5" letter-spacing="1.2" fill="#526455">{subtitle}</text>
{''.join(cards)}
<text x="38" y="353" font-size="13" fill="#455747">{count} package variants · SHA-256 verification · one release catalog</text><path d="M682 350h236" stroke="#809484" stroke-dasharray="3 7"/><circle cx="918" cy="350" r="4" fill="#526d59"/>
</svg>
'''


def markdown(data, manifest=None, image='assets/release-matrix.svg', docs_url=''):
    if manifest: data = manifest['catalog']
    repo = manifest['repository'] if manifest else data['repository']
    guide = docs_url + 'desktop.md'
    cli_guide = docs_url + 'cli.md'
    engineering = manifest and manifest.get('distribution') == 'engineering'
    lines = ['<!-- Generated by scripts/release.py; edit release/targets.json, not this file. -->',
             '# 聘渡 · 桌面版、CLI 与服务端下载', '', f'![HireFerry 发布矩阵]({image})', '',
             f'[安装与发布说明]({guide}) · [CLI 使用指南]({cli_guide}) · '
             f'[所有 Releases](https://github.com/{repo}/releases)', '']
    if manifest:
        lines += [('**工程验证产物，未发布为正式安装包。** 仅用于本次 CI 验证；不能导入公开下载页或生成 AUR 二进制配方。'
                   if engineering else '下表来自已汇总校验的发布资产；大小为实际下载体积，SHA-256 见对应 Release 的 `SHA256SUMS`。'), '']
        if 'desktop' not in manifest['versions']:
            lines += ['**尚未收录已发布的桌面安装包。** 桌面构建目标见 `release/targets.json`。', '']
        for product in PRODUCTS:
            if product in manifest['versions'] and not engineering:
                tag = manifest['releases'][product]
                lines.append(f"- **{PRODUCT_LABELS[product]} {manifest['versions'][product]}**：[Release](https://github.com/{repo}/releases/tag/{tag}) · [校验文件](https://github.com/{repo}/releases/download/{tag}/SHA256SUMS)")
    else:
        lines += ['**当前展示 CI 配置的构建目标，尚未记录一次完整桌面 Release。** 这些条目不是实机验证记录；完整构建、校验并发布后，工作流会替换为实际下载链接、版本和包大小。', '']
    lines += ['', '| 组件 | 系统 | 架构 | 软件包 | 下载体积 |', '| --- | --- | --- | --- | --- |']
    if manifest:
        for a in sorted(manifest['assets'], key=lambda a:(a['product'],a['platform'],a['architecture'],a['format'])):
            package = f"`{a['name']}`" if engineering else f"[{LABELS[a['format']]}]({a['url']})"
            lines.append(f"| {PRODUCT_LABELS[a['product']]} | {a['platform']} | {ARCH_LABELS.get(a['architecture'], a['architecture'])} | {package} | {human_size(a['size'])} |")
    else:
        for group in GROUPS:
            for row in data.get(group, []):
                lines.append(f"| {PRODUCT_LABELS[group_product(group)]} | {row['platform']} | {ARCH_LABELS.get(row['architecture'], row['architecture'])} | {'、'.join(LABELS[f] for f in row['formats'])} | 发布后生成 |")
    lines += ['', '## Arch Linux / AUR', '',
              '服务端、CLI 与桌面 GUI 独立安装，支持 x86_64 和 aarch64。同一组件的 `-git` 与 `-bin` 互斥，不同组件可以共存。', '',
              '| 组件 | 跟随 main 编译 | 安装 Release 二进制 |', '| --- | --- | --- |']
    for product in ('server', 'cli'):
        names = [f'hireferry-{product}-{variant}' for variant in ('git', 'bin')]
        lines.append(f"| {PRODUCT_LABELS[product]} | [{names[0]}](https://aur.archlinux.org/packages/{names[0]}) | [{names[1]}](https://aur.archlinux.org/packages/{names[1]}) |")
    lines += ['| 桌面 GUI | 仓库提供 `hireferry-desktop-git` 配方；AUR 可用性见指南 | `hireferry-desktop-bin` 必须依据正式桌面 Release 生成；可用性见指南 |']
    if manifest and 'desktop_signing' in manifest:
        labels = {'authenticode':'已验 Authenticode 与时间戳', 'developer-id-notarized':'已验 Developer ID 与公证票据',
                  'checksum-only':'SHA-256；无平台发布者签名', 'unsigned':'未签名工程构建', 'ad-hoc':'临时本地签名工程构建'}
        lines += ['', '## 桌面包签名与更新', '', '| 系统 / 目标 | 实际包验签结果 |', '| --- | --- |']
        for target, report in sorted(manifest['desktop_signing'].items()):
            states = sorted({labels[entry['verification']] for entry in report['assets'].values()})
            lines.append(f"| `{target}` | {'、'.join(states)} |")
        lines += ['', '桌面更新通过下载并安装新版本完成；未启用应用内自动更新。Flatpak 附件更新需重新安装新附件。SHA-256 不代替平台签名，签名也不承诺免除 Windows SmartScreen 的信誉提示。']
    lines += ['', f'安装、服务初始化与发布流程见 [AUR 指南]({docs_url}aur.md)。AUR 配方不计入上面的 Release 二进制数量。', '']
    lines += ['', '## Linux 选包建议', '',
              '| 发行版系列 | 桌面版优先选择 | 适用范围 |', '| --- | --- | --- |',
              '| Ubuntu、Debian、Linux Mint、Pop!_OS、Zorin、elementary | `.deb`；依赖不匹配时用 Flatpak | Ubuntu 22.04 / Debian 12 基线及提供 WebKitGTK 4.1 的后续版本 |',
              '| Fedora、openSUSE、RHEL / Rocky / Alma 衍生版 | `.rpm`；缺少所需库时用 Flatpak | 安装器依赖检查通过的版本；旧企业版不承诺原生运行 |',
              '| Arch、Manjaro、EndeavourOS、Gentoo | AppImage 或 Flatpak | AppImage 仍受宿主 glibc 与图形环境限制 |',
              '| NixOS、不可变桌面、SteamOS | Flatpak | 先配置发行版支持的 Flatpak 服务、运行时和桌面 portal |',
              '| Alpine / musl 系统 | CLI 使用 musl 包；桌面优先 Flatpak | 原生 glibc 桌面包不适用；Flatpak 可用性取决于发行版配置 |',
              '', 'Linux CLI 的静态 musl 包可用于 glibc 与 musl 发行版；桌面版需要图形会话。Flatpak 首次安装共享运行时需要额外空间，后续应用可复用。',
              '', 'Windows 的 NSIS 包按需下载 WebView2；macOS 使用系统 WebKit。桌面版不附带服务器或数据库，业务页面连接所选 HireFerry 站点。',
              '', f'**兼容范围不是“所有历史发行版本均实测”。** 架构、运行时和系统依赖满足要求才适用；具体要求、安装命令与签名说明见 [桌面版文档]({guide})。',
              '', '维护者：`release/targets.json` 定义下一次构建目标；已发布下载表固定使用 `release/latest.json` 内的目录快照。桌面版、CLI 与服务端按各自的稳定版本更新，旧版本不会覆盖较新的产品条目；修改构建目标不会改写历史包。运行 `python3 scripts/release.py docs --check` 检查生成文件。', '']
    return '\n'.join(lines)


def assemble(args, products=('desktop', 'cli')):
    data = catalog()
    versions = {product:package_version(product) for product in products}
    primary = next(product for product in PRODUCTS if product in products)
    if args.tag != primary + '-v' + versions[primary]:
        raise ValueError(f'Release tag must match the {primary} Cargo package version')
    repo = repository(args.repository or data['repository'])
    wanted = expected(data, versions, products)
    reports = {signing_report_name(row['target']):row for row in data['desktop']} if 'desktop' in products else {}
    files = verified_inputs(args.source, {**wanted, **reports})
    manifest = make_manifest(data, wanted, files, args.tag, repo, args.commit)
    manifest['distribution'] = getattr(args, 'distribution', 'release')
    if reports:
        manifest['desktop_signing'] = {}
        for name, row in reports.items():
            target = row['target']
            report = json.loads(files[name].read_text())
            target_assets = {n:m for n,m in wanted.items() if m['product']=='desktop' and m['target']==target and m['format']!='flatpak'}
            validate_signing_report(report, target_assets, files, require_signed=manifest['distribution']=='release')
            manifest['desktop_signing'][target] = report
    validate_manifest(manifest)
    if manifest['distribution'] == 'release':
        require_publishable(manifest)
    args.destination.mkdir(parents=True, exist_ok=False)
    for name, path in files.items():
        shutil.copyfile(path, args.destination/name)
    (args.destination/'VERSION').write_text(versions.get('cli', versions[primary]) + '\n')
    (args.destination/'downloads.json').write_text(json.dumps(manifest, indent=2) + '\n')
    image = 'release-matrix.svg' if manifest['distribution']=='engineering' else f'https://github.com/{repo}/releases/download/{args.tag}/release-matrix.svg'
    (args.destination/'downloads.md').write_text(markdown(data, manifest, image, f'https://github.com/{repo}/blob/{args.commit if manifest["distribution"]=="engineering" and args.commit else args.tag}/docs/'))
    (args.destination/'release-matrix.svg').write_text(svg(data, manifest))
    checksums(args.destination)
    print(f'Verified {len(wanted)} package assets for {args.tag} ({manifest["distribution"]})')


def assemble_cli(args):
    assemble(args, ('cli',))


def assemble_server(args):
    assemble(args, ('server',))


def verify_publication(args):
    manifest = require_publishable(validate_manifest(json.loads((args.directory/'downloads.json').read_text())))
    allowed = {asset['name']:asset for asset in manifest['assets']}
    allowed.update({name:{} for name in ('VERSION','downloads.json','downloads.md','release-matrix.svg')})
    if 'desktop_signing' in manifest:
        allowed.update({signing_report_name(target):{} for target in manifest['desktop_signing']})
    files = verified_inputs(args.directory, allowed)
    for asset in manifest['assets']:
        if digest(files[asset['name']]) != asset['sha256'] or files[asset['name']].stat().st_size != asset['size']:
            raise ValueError('Download manifest does not match final package bytes')
    for target, report in manifest.get('desktop_signing', {}).items():
        if json.loads(files[signing_report_name(target)].read_text()) != report:
            raise ValueError('Final desktop signing report differs from download manifest')
    print('Complete formal release and native verification reports are ready to publish')


def docs(args):
    manifest_path = args.manifest or (SNAPSHOT if SNAPSHOT.exists() else None)
    manifest = validate_manifest(json.loads(manifest_path.read_text())) if manifest_path else None
    if manifest and manifest.get('distribution', 'release') != 'release':
        raise ValueError('Engineering artifacts cannot update public downloads or AUR binary packages')
    if args.manifest:
        require_publishable(manifest)
        current = json.loads(SNAPSHOT.read_text()) if SNAPSHOT.exists() else None
        manifest = merge_manifests(current, manifest)
    data = manifest['catalog'] if manifest else catalog()
    outputs = {ROOT/'docs/downloads.md':markdown(data, manifest), ROOT/'docs/assets/release-matrix.svg':svg(data, manifest)}
    if manifest:
        outputs[ROOT/'static/downloads.json'] = json.dumps(manifest, indent=2) + '\n'
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
        build.add_argument('--distribution', choices=('release','engineering'), default='release')
        build.set_defaults(run=fn)
    verify = commands.add_parser('verify-publication')
    verify.add_argument('directory', type=Path)
    verify.set_defaults(run=verify_publication)
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

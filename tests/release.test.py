#!/usr/bin/env python3
"""Exercise the release gate with fake package bytes; no platform builds required."""
import argparse
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('release', ROOT/'scripts/release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = release.catalog()
        self.versions = {'desktop':'0.1.0','cli':'0.1.2'}
        self.wanted = release.expected(self.data, self.versions)
        self.source = self.root/'artifacts'
        for i, name in enumerate(self.wanted):
            folder = self.source/f'job-{i}'
            folder.mkdir(parents=True)
            (folder/name).write_bytes(('real bytes for '+name).encode())
            release.checksums(folder)

    def test_complete_release_has_exact_links_hashes_and_sizes(self):
        files = release.verified_inputs(self.source, self.wanted)
        self.assertEqual(len(files), 21)
        manifest = release.make_manifest(self.data, self.wanted, files, 'desktop-v0.1.0', self.data['repository'])
        self.assertEqual(release.validate_manifest(manifest), manifest)
        output = release.markdown(self.data, manifest)
        self.assertNotIn('发布后生成', output)
        for item in manifest['assets']:
            self.assertIn(item['url'], output)
            self.assertEqual(item['size'], files[item['name']].stat().st_size)
            self.assertEqual(item['sha256'], release.digest(files[item['name']]))
        ET.fromstring(release.svg(self.data, manifest))

    def test_missing_platform_cannot_publish(self):
        folder = next(self.source.iterdir())
        for path in folder.iterdir(): path.unlink()
        folder.rmdir()
        with self.assertRaisesRegex(ValueError, 'Incomplete release'):
            release.verified_inputs(self.source, self.wanted)

    def test_corrupted_package_cannot_publish(self):
        next(p for p in self.source.rglob('*') if p.is_file() and p.name!='SHA256SUMS').write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'Checksum mismatch'):
            release.verified_inputs(self.source, self.wanted)

    def test_unmanifested_file_cannot_publish(self):
        (self.source/'leftover.zip').write_bytes(b'old release')
        with self.assertRaisesRegex(ValueError, 'Unmanifested files'):
            release.verified_inputs(self.source, self.wanted)

    def test_duplicate_package_cannot_publish(self):
        folder = next(self.source.iterdir())
        duplicate = self.source/'duplicate'
        duplicate.mkdir()
        for p in folder.iterdir(): (duplicate/p.name).write_bytes(p.read_bytes())
        with self.assertRaisesRegex(ValueError, 'duplicate package'):
            release.verified_inputs(self.source, self.wanted)

    def test_traversal_checksum_is_rejected(self):
        (next(self.source.iterdir())/'SHA256SUMS').write_text('0'*64+'  ../bad.zip\n')
        with self.assertRaisesRegex(ValueError, 'Malformed checksum'):
            release.verified_inputs(self.source, self.wanted)

    def test_windows_binary_mode_checksums_are_supported(self):
        for file in self.source.rglob('SHA256SUMS'):
            file.write_text(file.read_text().replace('  ',' *'))
        self.assertEqual(len(release.verified_inputs(self.source, self.wanted)),21)

    def test_bad_manifest_url_and_metadata_are_rejected(self):
        files = release.verified_inputs(self.source,self.wanted)
        base = release.make_manifest(self.data,self.wanted,files,'desktop-v0.1.0',self.data['repository'])
        for key, value in (('url','https://example.test/fake'),('architecture','riscv64'),('size',0),('tag','desktop-v9.0.0')):
            manifest = json.loads(json.dumps(base))
            manifest['assets'][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.validate_manifest(manifest)

    def test_failed_assembly_writes_no_partial_release(self):
        args=argparse.Namespace(source=self.source,destination=self.root/'out',tag='desktop-v0.1.0',repository=None,commit='abc')
        next(self.source.rglob('SHA256SUMS')).write_text('bad')
        with patch.object(release,'package_version',side_effect=lambda product:self.versions[product]), self.assertRaises(ValueError):
            release.assemble(args)
        self.assertFalse(args.destination.exists())

    def test_assembly_preserves_cli_version_and_hashes_metadata(self):
        args=argparse.Namespace(source=self.source,destination=self.root/'out',tag='desktop-v0.1.0',repository=None,commit='abc')
        with patch.object(release,'package_version',side_effect=lambda product:self.versions[product]): release.assemble(args)
        self.assertEqual((args.destination/'VERSION').read_text(),'0.1.2\n')
        sums=(args.destination/'SHA256SUMS').read_text()
        for name in ('VERSION','downloads.md','downloads.json','release-matrix.svg'):
            self.assertIn(release.digest(args.destination/name)+'  '+name,sums)
        self.assertEqual(json.loads((args.destination/'downloads.json').read_text())['source_commit'],'abc')

    def test_macos_app_archive_is_created_without_updater(self):
        bundles=self.root/'bundle'; app=bundles/'macos/HireFerry.app/Contents/MacOS'
        app.mkdir(parents=True); (app/'hireferry-desktop').write_bytes(b'binary')
        (bundles/'HireFerry.dmg').write_bytes(b'dmg')
        args=argparse.Namespace(target='aarch64-apple-darwin',version='0.1.0',bundle_dir=bundles,out=self.root/'stage')
        release.stage_desktop(args)
        archive=next(args.out.glob('*.app.tar.gz'))
        with release.tarfile.open(archive) as contents:
            self.assertIn('HireFerry.app/Contents/MacOS/hireferry-desktop',contents.getnames())

    def test_published_catalog_survives_new_planned_formats_and_targets(self):
        files = release.verified_inputs(self.source, self.wanted)
        manifest = release.make_manifest(self.data, self.wanted, files, 'desktop-v0.1.0', self.data['repository'])
        planned = json.loads(json.dumps(self.data))
        arm_windows = next(row for row in planned['desktop'] if row['target'] == 'aarch64-pc-windows-msvc')
        arm_windows['formats'].append('msi')
        arm_windows['bundles'] += ',msi'
        planned['cli'].append(dict(planned['cli'][0], target='riscv64gc-unknown-linux-musl', architecture='riscv64gc'))
        release.validate_catalog(planned)
        self.assertEqual(len(release.expected(planned, self.versions)), 23)
        self.assertEqual(release.validate_manifest(manifest), manifest)
        published_markdown = release.markdown(self.data, manifest)
        self.assertEqual(release.markdown(planned, manifest), published_markdown)
        self.assertEqual(release.svg(planned, manifest), release.svg(self.data, manifest))
        self.assertNotIn('riscv64gc', published_markdown)
        fixture = self.root/'docs-fixture'
        fixture.mkdir()
        snapshot = fixture/'latest.json'
        snapshot.write_text(json.dumps(manifest))
        current = fixture/'targets.json'
        current.write_text(json.dumps(planned))
        with patch.object(release, 'ROOT', fixture), patch.object(release, 'SNAPSHOT', snapshot), patch.object(release, 'CATALOG', current):
            release.docs(argparse.Namespace(manifest=None, check=False))
            release.docs(argparse.Namespace(manifest=None, check=True))
            self.assertEqual(release.catalog(), planned)
            self.assertEqual((fixture/'docs/downloads.md').read_text(), published_markdown)

    def test_embedded_catalog_rejects_injection_and_inconsistent_metadata(self):
        files = release.verified_inputs(self.source, self.wanted)
        base = release.make_manifest(self.data, self.wanted, files, 'desktop-v0.1.0', self.data['repository'])
        for key, value in (('platform', '[evil](https://evil.test)'),
                           ('architecture', 'x64 | injected'), ('target', 'x64)\n![image](url)'),
                           ('formats', ['[download](url)']), ('runner', 'ubuntu; echo injected'),
                           ('bundles', 'deb,rpm,appimage;true')):
            manifest = json.loads(json.dumps(base))
            manifest['catalog']['desktop'][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.validate_manifest(manifest)
        base['catalog']['repository'] = 'another/repository'
        with self.assertRaisesRegex(ValueError, 'repository mismatch'):
            release.validate_manifest(base)

    def test_manifest_catalog_uses_effective_repository_and_is_frozen(self):
        files = release.verified_inputs(self.source, self.wanted)
        manifest = release.make_manifest(self.data, self.wanted, files, 'desktop-v0.1.0', 'example/fork')
        self.assertEqual(manifest['catalog']['repository'], 'example/fork')
        self.assertEqual(release.validate_manifest(manifest), manifest)
        self.data['desktop'][0]['formats'].clear()
        self.assertEqual(manifest['catalog']['desktop'][0]['formats'], ['deb','rpm','AppImage'])

    def released_fixture(self, versions, tag, data=None, commit='abc'):
        data = data or self.data
        wanted = release.expected(data, versions, tuple(versions))
        source = Path(tempfile.mkdtemp(dir=self.root, prefix='published-'))
        for i, name in enumerate(wanted):
            folder = source/f'job-{i}'
            folder.mkdir()
            (folder/name).write_bytes(('package '+name).encode())
            release.checksums(folder)
        files = release.verified_inputs(source, wanted)
        return release.make_manifest(data, wanted, files, tag, data['repository'], commit), source

    def test_standalone_cli_assembly_has_six_assets_and_no_desktop_claim(self):
        _, source = self.released_fixture({'cli':'0.1.2'}, 'cli-v0.1.2')
        args = argparse.Namespace(source=source, destination=self.root/'cli-release',
                                  tag='cli-v0.1.2', repository=None, commit='ccc')
        with patch.object(release, 'package_version', return_value='0.1.2'):
            release.assemble_cli(args)
        manifest = release.validate_manifest(json.loads((args.destination/'downloads.json').read_text()))
        self.assertEqual(len(manifest['assets']), 6)
        self.assertEqual(manifest['versions'], {'cli':'0.1.2'})
        self.assertEqual(manifest['source_commits'], {'cli':'ccc'})
        self.assertEqual(manifest['catalog']['desktop'], [])
        self.assertEqual(manifest['catalog']['flatpak'], [])
        artwork = (args.destination/'release-matrix.svg').read_text()
        self.assertIn('6 package variants', artwork)
        self.assertNotIn('Desktop + hfy CLI', artwork)
        self.assertNotIn('Every desktop', artwork)
        self.assertNotIn('NSIS', artwork)
        ET.fromstring(artwork)
        self.assertIn('尚未收录已发布的桌面安装包', (args.destination/'downloads.md').read_text())
        sums = (args.destination/'SHA256SUMS').read_text()
        self.assertIn(release.digest(args.destination/'downloads.json')+'  downloads.json', sums)

    def test_cli_first_then_desktop_keeps_newer_cli_and_frozen_catalog(self):
        future = json.loads(json.dumps(self.data))
        future['cli'].append(dict(future['cli'][0], target='riscv64gc-unknown-linux-musl', architecture='riscv64gc'))
        cli, _ = self.released_fixture({'cli':'0.1.10'}, 'cli-v0.1.10', future, 'ccc')
        desktop, _ = self.released_fixture({'desktop':'0.2.0','cli':'0.1.9'}, 'desktop-v0.2.0', commit='ddd')
        fixture = self.root/'merged-docs'
        fixture.mkdir()
        snapshot = fixture/'latest.json'
        incoming = fixture/'incoming.json'
        with patch.object(release, 'ROOT', fixture), patch.object(release, 'SNAPSHOT', snapshot):
            incoming.write_text(json.dumps(cli))
            release.docs(argparse.Namespace(manifest=incoming, check=False))
            self.assertEqual(json.loads(snapshot.read_text())['versions'], {'cli':'0.1.10'})
            incoming.write_text(json.dumps(desktop))
            release.docs(argparse.Namespace(manifest=incoming, check=False))
            release.docs(argparse.Namespace(manifest=None, check=True))
            merged = release.validate_manifest(json.loads(snapshot.read_text()))
            self.assertEqual(merged['versions'], {'desktop':'0.2.0','cli':'0.1.10'})
            self.assertEqual(merged['releases'], {'desktop':'desktop-v0.2.0','cli':'cli-v0.1.10'})
            self.assertEqual(merged['source_commits'], {'desktop':'ddd','cli':'ccc'})
            self.assertEqual(len(merged['catalog']['cli']), 7)
            self.assertEqual(len(merged['assets']), 22)
            self.assertEqual([a for a in merged['assets'] if a['product']=='cli'], cli['assets'])
            before = snapshot.read_text()
            release.docs(argparse.Namespace(manifest=incoming, check=False))
            self.assertEqual(snapshot.read_text(), before)
            self.assertIn('22 package variants', (fixture/'docs/assets/release-matrix.svg').read_text())

    def test_retained_cli_can_come_from_an_older_desktop_tag(self):
        old, _ = self.released_fixture({'desktop':'0.1.0','cli':'0.1.10'}, 'desktop-v0.1.0', commit='aaa')
        incoming, _ = self.released_fixture({'desktop':'0.2.0','cli':'0.1.9'}, 'desktop-v0.2.0', commit='bbb')
        merged = release.merge_manifests(old, incoming)
        self.assertEqual(merged['versions'], {'desktop':'0.2.0','cli':'0.1.10'})
        self.assertEqual(merged['releases']['cli'], 'desktop-v0.1.0')
        self.assertEqual(merged['source_commits'], {'desktop':'bbb','cli':'aaa'})
        self.assertEqual(release.validate_manifest(merged), merged)
        for asset in merged['assets']:
            if asset['product'] == 'cli': self.assertIn('/desktop-v0.1.0/', asset['url'])

    def test_newer_cli_updates_without_replacing_desktop(self):
        desktop, _ = self.released_fixture({'desktop':'0.1.0','cli':'0.1.2'}, 'desktop-v0.1.0')
        newer, _ = self.released_fixture({'cli':'0.1.10'}, 'cli-v0.1.10', commit='ddd')
        older, _ = self.released_fixture({'cli':'0.1.9'}, 'cli-v0.1.9', commit='eee')
        merged = release.merge_manifests(desktop, newer)
        self.assertEqual(merged['versions'], {'desktop':'0.1.0','cli':'0.1.10'})
        self.assertEqual(release.merge_manifests(merged, older), merged)
        self.assertEqual([a for a in merged['assets'] if a['product']=='desktop'],
                         [a for a in desktop['assets'] if a['product']=='desktop'])

    def test_documentation_merge_rejects_prereleases_and_cross_repository(self):
        stable, _ = self.released_fixture({'cli':'0.1.2'}, 'cli-v0.1.2')
        prerelease, _ = self.released_fixture({'cli':'0.1.3-rc.1'}, 'cli-v0.1.3-rc.1')
        with self.assertRaisesRegex(ValueError, 'Only stable releases'):
            release.merge_manifests(stable, prerelease)
        fork = json.loads(json.dumps(self.data)); fork['repository'] = 'example/fork'
        another, _ = self.released_fixture({'cli':'0.1.3'}, 'cli-v0.1.3', fork)
        with self.assertRaisesRegex(ValueError, 'different repositories'):
            release.merge_manifests(stable, another)

    def test_invalid_versions_rejected(self):
        for value in ('v0.1.0','0.1','01.2.3','0.1.0/evil','0.1.0-01','0.1.0+build','0.1.0;echo'):
            with self.subTest(value=value),self.assertRaises(ValueError): release.version(value)
        for value in ('0.1.0','1.2.3-rc.1'): self.assertEqual(release.version(value),value)

    def test_server_release_is_independent_and_merges_with_both_clients(self):
        server, source = self.released_fixture({'server':'0.1.0'}, 'server-v0.1.0', commit='aaa')
        args = argparse.Namespace(source=source, destination=self.root/'server-release',
                                  tag='server-v0.1.0', repository=None, commit='aaa')
        with patch.object(release, 'package_version', return_value='0.1.0'):
            release.assemble_server(args)
        self.assertEqual(len(server['assets']), 2)
        self.assertEqual(server['catalog']['cli'], [])
        self.assertEqual((args.destination/'VERSION').read_text(), '0.1.0\n')
        clients, _ = self.released_fixture(self.versions, 'desktop-v0.1.0', commit='bbb')
        merged = release.merge_manifests(clients, server)
        self.assertEqual(merged['versions'], dict(self.versions, server='0.1.0'))
        self.assertEqual(len(merged['assets']), 23)
        self.assertEqual(release.merge_manifests(merged, clients), merged)
        self.assertIn('服务端', release.markdown(self.data, merged))
        self.assertIn('hireferry-server-bin', release.markdown(self.data, merged))
        ET.fromstring(release.svg(self.data, merged))

    def test_server_archive_contains_configuration_and_service_without_cli(self):
        binary = self.root/'hireferry'
        binary.write_bytes(b'server executable')
        args = argparse.Namespace(binary=binary, target='x86_64-unknown-linux-gnu',
                                  version='0.1.0', out=self.root/'server-stage')
        release.stage_server(args)
        archive = next(args.out.glob('*.tar.gz'))
        with release.tarfile.open(archive) as package:
            names = {name.split('/', 1)[1] for name in package.getnames() if '/' in name}
            self.assertIn('hireferry', names)
            self.assertIn('config.example.toml', names)
            self.assertIn('deploy/hireferry.service', names)
            self.assertNotIn('hfy', names)
        self.assertEqual(len(release.verified_inputs(args.out, {archive.name:{}})), 1)

    def test_old_catalogs_remain_readable_without_server(self):
        old = json.loads(json.dumps(self.data))
        old.pop('server')
        release.validate_catalog(old)
        manifest, _ = self.released_fixture(self.versions, 'desktop-v0.1.0', old)
        self.assertEqual(release.validate_manifest(manifest), manifest)

    def test_mobile_and_nix_channels_are_catalogued_without_fake_packages(self):
        self.assertEqual(self.data['channels']['mobile']['kind'], 'responsive-web')
        self.assertEqual(self.data['channels']['nix']['packages'], ['server', 'cli'])
        manifest, _ = self.released_fixture({'cli':'0.1.3'}, 'cli-v0.1.3')
        output = release.markdown(self.data, manifest)
        self.assertIn('移动版 Web（Android / iOS 浏览器）', output)
        self.assertIn('Nix / NixOS', output)
        self.assertIn('当前没有原生 APK / IPA', output)
        for key, value in [('kind', 'script'), ('url', 'http://evil.test')]:
            invalid = json.loads(json.dumps(self.data))
            invalid['channels']['mobile'][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                release.validate_catalog(invalid)


if __name__=='__main__': unittest.main()

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

    def test_invalid_versions_rejected(self):
        for value in ('v0.1.0','0.1','01.2.3','0.1.0/evil','0.1.0-01','0.1.0+build','0.1.0;echo'):
            with self.subTest(value=value),self.assertRaises(ValueError): release.version(value)
        for value in ('0.1.0','1.2.3-rc.1'): self.assertEqual(release.version(value),value)


if __name__=='__main__': unittest.main()

#!/usr/bin/env python3
"""Exercise actual PKGBUILD package functions in an isolated filesystem."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'scripts'))
import aur
import release


class AurPackages(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='hireferry-aur-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def manifest(self, product):
        version = release.package_version(product)
        wanted = release.expected(release.catalog(), {product:version}, (product,))
        files = {}
        for name in wanted:
            path = self.root/name
            path.write_bytes(b'archive bytes')
            files[name] = path
        return release.make_manifest(release.catalog(), wanted, files, f'{product}-v{version}', release.catalog()['repository'])

    def test_binary_recipes_install_only_their_component_and_can_coexist(self):
        paths = {}
        for product in ('server','cli'):
            manifest = self.manifest(product)
            target = 'x86_64-unknown-linux-' + ('gnu' if product == 'server' else 'musl')
            name, recipe = aur.render(product, 'bin', manifest['versions'][product], manifest)
            folder = self.root/name
            source = folder/'src'
            package = source/f'hireferry-{product}-{manifest["versions"][product]}-{target}'
            package.mkdir(parents=True)
            binary = package/('hireferry' if product == 'server' else 'hfy')
            binary.write_text('#!/bin/sh\necho fixture\n')
            for file in ('LICENSE','THIRD_PARTY_NOTICES','config.example.toml'):
                shutil.copyfile(ROOT/file, package/file)
            shutil.copytree(ROOT/'docs', package/'docs')
            if product == 'cli':
                shutil.copytree(ROOT/'skills', package/'skills')
                (package/'scripts').mkdir()
                shutil.copyfile(ROOT/'scripts/install-skill.sh',package/'scripts/install-skill.sh')
            else:
                for file in (ROOT/'packaging/arch').iterdir():
                    shutil.copyfile(file, source/file.name)
            (folder/'PKGBUILD').write_text(recipe)
            out = folder/'pkg'
            subprocess.run(['bash','-eu','-c','CARCH=x86_64; source ./PKGBUILD; srcdir=$1; pkgdir=$2; package',
                            'package-test',str(source),str(out)],cwd=folder,check=True,capture_output=True)
            paths[product] = {p.relative_to(out).as_posix() for p in out.rglob('*') if p.is_file()}
            self.assertNotIn('cargo ',recipe)
            self.assertIn('sha256sums_aarch64=',recipe)
            self.assertNotIn('SKIP',recipe)
        self.assertIn('usr/bin/hireferry',paths['server'])
        self.assertNotIn('usr/bin/hfy',paths['server'])
        self.assertIn('usr/bin/hfy',paths['cli'])
        self.assertFalse(any('systemd/' in path or 'sysusers.d/' in path for path in paths['cli']))
        self.assertFalse(paths['server'] & paths['cli'])
        self.assertNotIn('etc/hireferry/config.toml',paths['server'])

    def test_git_recipes_build_only_requested_cargo_package(self):
        for product, package in (('server','hireferry'),('cli','hireferry-cli')):
            _, text = aur.render(product,'git','0.1.0.r1.gabc')
            self.assertIn(f'--target "$_target" -p {package}\n',text)
            self.assertIn('cargo fetch --locked',text)
            self.assertIn('cargo build --frozen',text)
            self.assertNotIn('--workspace',text)

    def test_binary_generator_rejects_missing_or_wrong_architecture(self):
        manifest = self.manifest('server')
        manifest['assets'] = manifest['assets'][:1]
        with self.assertRaisesRegex(ValueError,'exactly x86_64 and aarch64'):
            aur.render('server','bin','0.1.0',manifest)
        manifest = self.manifest('cli')
        next(a for a in manifest['assets'] if a['platform']=='Linux')['target'] = 'x86_64-unknown-linux-gnu'
        with self.assertRaisesRegex(ValueError,'Unexpected AUR binary target'):
            aur.render('cli','bin','0.1.0',manifest)

    def test_checked_in_srcinfo_matches_makepkg(self):
        if not shutil.which('makepkg'):
            self.skipTest('makepkg is not available on this platform')
        for folder in (ROOT/'packaging/aur').glob('hireferry-*'):
            generated = subprocess.check_output(['makepkg','--printsrcinfo'],cwd=folder,text=True)
            self.assertEqual(generated,(folder/'.SRCINFO').read_text(),str(folder))


if __name__ == '__main__':
    unittest.main()

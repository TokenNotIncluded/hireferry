"""Exercise downloads, checksum failures and installation in isolated directories."""
import hashlib
import io
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class Installer(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='hfy-installer-test-')
        self.base = Path(self.temp.name)
        self.assets = self.base/'assets'
        self.assets.mkdir()
        self.bin = self.base/'mock-bin'
        self.bin.mkdir()
        self.destination = self.base/'install with spaces'
        self.env = dict(os.environ, HFY_RELEASE_REPO='Fixture/hireferry', HFY_TAG_PREFIX='cli-v', HFY_NO_PATH='1', HFY_INSTALL_DIR=str(self.destination), HFY_TEST_ASSETS=str(self.assets), HFY_TEST_BASE=str(self.base))
        self.env.pop('HFY_VERSION', None)
        self.env['PATH'] = str(self.bin)+os.pathsep+self.env['PATH']
        self.executable('curl', '''#!/bin/sh
set -eu
url=''; output=''
while [ "$#" -gt 0 ]; do
 case "$1" in https://*) url=$1; shift;; -o) output=$2; shift 2;; *) shift;; esac
done
printf '%s\n' "$url" >> "$HFY_TEST_BASE/requests"
cp "$HFY_TEST_ASSETS/${url##*/}" "$output"
''')
        self.executable('sysctl', '#!/bin/sh\nexit 1\n')
        self.platform('Linux', 'x86_64')

    def tearDown(self): self.temp.cleanup()
    def executable(self, name, text):
        p=self.bin/name; p.write_text(text); p.chmod(0o755)
    def platform(self, system, arch):
        self.executable('uname', f'#!/bin/sh\ncase "$1" in -s) echo {system};; -m) echo {arch};; esac\n')
    def package(self, target='x86_64-unknown-linux-musl', version='1.2.3'):
        name=f'hireferry-cli-{version}-{target}'
        data=b'#!/bin/sh\nprintf "hfy 1.2.3\\n"\n'
        archive=self.assets/(name+'.tar.gz')
        with tarfile.open(archive,'w:gz') as handle:
            entry=tarfile.TarInfo(name+'/hfy');entry.size=len(data);entry.mode=0o755
            handle.addfile(entry,io.BytesIO(data))
        (self.assets/'VERSION').write_text(version+'\n')
        (self.assets/'SHA256SUMS').write_text(hashlib.sha256(archive.read_bytes()).hexdigest()+'  '+archive.name+'\n')
    def run_install(self, success=True):
        result=subprocess.run(['sh',str(ROOT/'static/install.sh')],env=self.env,capture_output=True,text=True,timeout=30)
        self.assertEqual(result.returncode==0,success,result.stdout+result.stderr)
        return result

    def test_latest_installs_to_spaced_path_and_pins_all_asset_requests(self):
        self.package();self.run_install()
        self.assertTrue((self.destination/'hfy').is_file())
        requests=(self.base/'requests').read_text().splitlines()
        self.assertIn('/latest/download/VERSION',requests[0])
        self.assertTrue(all('/download/cli-v1.2.3/' in url for url in requests[1:]))
        self.assertFalse(list(self.destination.glob('.hfy-install.*')))

    def test_checksum_failure_preserves_existing_binary(self):
        self.package();self.destination.mkdir();old=self.destination/'hfy';old.write_bytes(b'keep existing executable')
        (self.assets/'SHA256SUMS').write_text('0'*64+'  '+next(self.assets.glob('*.tar.gz')).name+'\n')
        self.run_install(False);self.assertEqual(old.read_bytes(),b'keep existing executable')

    def test_mac_arm_selection_and_version_override(self):
        self.platform('Darwin','arm64');self.package('aarch64-apple-darwin')
        self.env['HFY_VERSION']='1.2.3';self.run_install()
        self.assertNotIn('/latest/',(self.base/'requests').read_text())

    def test_unsupported_architecture_and_symlink_destination_do_not_install(self):
        self.platform('Linux','riscv64');self.run_install(False)
        self.assertFalse(self.destination.exists())
        self.platform('Linux','x86_64');self.package()
        outside=self.base/'outside';outside.mkdir();self.destination.symlink_to(outside,target_is_directory=True)
        self.run_install(False);self.assertFalse(list(outside.iterdir()))

    @unittest.skipUnless(os.environ.get('HFY_INSTALLER_PACKAGE_DIR'), 'release smoke test runs with native archive')
    def test_real_native_release_installs_and_runs(self):
        (self.bin/'uname').unlink();(self.bin/'sysctl').unlink()
        folder=Path(os.environ['HFY_INSTALLER_PACKAGE_DIR'])
        for file in folder.iterdir():
            if file.is_file():shutil.copyfile(file,self.assets/file.name)
        version=__import__('re').search(r'^version = "([^"]+)"$',(ROOT/'crates/hireferry-cli/Cargo.toml').read_text(),__import__('re').M)[1]
        (self.assets/'VERSION').write_text(version+'\n')
        self.run_install()
        result=subprocess.run([str(self.destination/'hfy'),'--version'],capture_output=True,text=True,check=True)
        self.assertIn(version,result.stdout)

if __name__=='__main__':unittest.main()

"""Exercise installation boundaries and package contents in isolated directories."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest
import zipfile


REPO = Path(__file__).resolve().parents[1]


class CliScripts(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hireferry-cli-scripts-")
        self.base = Path(self.temp.name)
        self.source = self.base / "source"
        self.project = self.base / "project with spaces"
        self.project.mkdir()
        (self.source / "scripts").mkdir(parents=True)
        for script in ("install-skill.sh", "package-cli.sh"):
            shutil.copy2(REPO / "scripts" / script, self.source / "scripts" / script)
        shutil.copytree(REPO / "skills/hireferry", self.source / "skills/hireferry")
        self.env = os.environ.copy()
        self.env["HOME"] = str(self.base / "isolated-user")
        self.env.pop("CODEX_HOME", None)
        self.target = self.project / ".agents/skills/hireferry"

    def tearDown(self):
        self.temp.cleanup()

    def run_script(self, script, *args, ok=True, env=None):
        result = subprocess.run(
            ["bash", str(self.source / "scripts" / script), *map(str, args)],
            cwd=self.project,
            env=self.env if env is None else env,
            text=True,
            capture_output=True,
            timeout=15,
        )
        if ok:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def test_project_install_upgrade_and_uninstall_preserve_other_skills(self):
        other = self.project / ".agents/skills/other/SKILL.md"
        other.parent.mkdir(parents=True)
        other.write_text("keep me")
        self.run_script("install-skill.sh")
        manifest = self.target / "SKILL.md"
        original = manifest.read_text()
        manifest.write_text("local customization")
        self.run_script("install-skill.sh", ok=False)
        self.assertEqual(manifest.read_text(), "local customization")
        self.run_script("install-skill.sh", "--force")
        self.assertEqual(manifest.read_text(), original)
        self.run_script("install-skill.sh", "--uninstall")
        self.assertFalse(self.target.exists())
        self.assertEqual(other.read_text(), "keep me")
        self.run_script("install-skill.sh", "--uninstall")

    def test_force_and_uninstall_refuse_an_unmanaged_skill(self):
        self.target.mkdir(parents=True)
        manifest = self.target / "SKILL.md"
        manifest.write_text("a different skill")
        self.run_script("install-skill.sh", "--force", ok=False)
        self.run_script("install-skill.sh", "--uninstall", ok=False)
        self.assertEqual(manifest.read_text(), "a different skill")

    def test_global_paths_and_explicit_destination_are_isolated(self):
        self.run_script("install-skill.sh", "--global")
        global_target = Path(self.env["HOME"]) / ".agents/skills/hireferry"
        self.assertTrue((global_target / "SKILL.md").is_file())
        custom_env = self.env | {"CODEX_HOME": str(self.base / "custom-codex")}
        self.run_script("install-skill.sh", "--global", env=custom_env)
        custom_target = Path(custom_env["CODEX_HOME"]) / "skills/hireferry"
        self.assertTrue((custom_target / "SKILL.md").is_file())
        selected = self.base / "selected/hireferry"
        self.run_script("install-skill.sh", "--dest", selected)
        self.assertTrue((selected / "SKILL.md").is_file())
        self.assertFalse(self.target.exists())

    def test_symbolic_destination_and_parent_are_rejected(self):
        outside = self.base / "outside"
        outside.mkdir()
        self.target.parent.mkdir(parents=True)
        self.target.symlink_to(outside, target_is_directory=True)
        self.run_script("install-skill.sh", "--force", ok=False)
        self.target.unlink()
        shutil.rmtree(self.project / ".agents")
        (self.project / ".agents").symlink_to(outside, target_is_directory=True)
        self.run_script("install-skill.sh", ok=False)
        self.assertEqual(list(outside.iterdir()), [])

    def test_symlinks_in_source_or_existing_skill_are_rejected(self):
        outside = self.base / "outside.txt"
        outside.write_text("do not touch")
        self.run_script("install-skill.sh")
        (self.target / "extra").symlink_to(outside)
        self.run_script("install-skill.sh", "--force", ok=False)
        self.run_script("install-skill.sh", "--uninstall", ok=False)
        (self.target / "extra").unlink()
        self.run_script("install-skill.sh", "--uninstall")
        (self.source / "skills/hireferry/extra").symlink_to(outside)
        self.run_script("install-skill.sh", ok=False)
        self.assertEqual(outside.read_text(), "do not touch")

    def test_invalid_destinations_and_conflicting_flags_are_rejected(self):
        self.run_script("install-skill.sh", "--dest", self.base, ok=False)
        self.run_script("install-skill.sh", "--dest", "../hireferry", ok=False)
        self.run_script("install-skill.sh", "--dest", self.target, "--global", ok=False)
        self.run_script("install-skill.sh", "--force", "--uninstall", ok=False)
        self.assertFalse(self.target.exists())

    def prepare_package(self):
        (self.source / "crates/hireferry-cli").mkdir(parents=True)
        (self.source / "crates/hireferry-cli/Cargo.toml").write_text('version = "0.1.0"\n')
        (self.source / "LICENSE").write_text("test license")
        (self.source / "docs").mkdir()
        for name in ("cli.md", "auth.md", "api.md"):
            (self.source / "docs" / name).write_text(f"# {name}\n")
        (self.source / "config.toml").write_text("not-for-distribution")
        binary = self.base / "fixture-hfy"
        binary.write_bytes(b"test binary bytes: package contents only")
        return binary

    def test_package_has_checked_contents_and_no_server_configuration(self):
        binary = self.prepare_package()
        output = self.base / "output with spaces"
        args = ("--binary", binary, "--target", "x86_64-unknown-linux-gnu", "--out-dir", output)
        self.run_script("package-cli.sh", *args)
        archive = next(output.glob("*.tar.gz"))
        checksum = (output / "SHA256SUMS").read_text().split()
        self.assertEqual(checksum, [hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name])
        with tarfile.open(archive) as handle:
            names = handle.getnames()
            prefix = "hireferry-cli-0.1.0-x86_64-unknown-linux-gnu/"
            for name in ("hfy", "LICENSE", "README.md", "docs/cli.md", "skills/hireferry/SKILL.md", "scripts/install-skill.sh"):
                self.assertIn(prefix + name, names)
            self.assertFalse(any("config.toml" in name for name in names))
            self.assertEqual(handle.extractfile(prefix + "hfy").read(), binary.read_bytes())
        before = archive.read_bytes()
        self.run_script("package-cli.sh", *args, ok=False)
        self.assertEqual(archive.read_bytes(), before)

    def test_package_rejects_missing_target_and_symlink_checksum(self):
        binary = self.prepare_package()
        self.run_script("package-cli.sh", "--binary", binary, ok=False)
        output = self.base / "packages"
        output.mkdir()
        outside = self.base / "outside-checksum"
        outside.write_text("untouched")
        (output / "SHA256SUMS").symlink_to(outside)
        self.run_script("package-cli.sh", "--binary", binary, "--target", "x86_64-unknown-linux-gnu", "--out-dir", output, ok=False)
        self.assertEqual(outside.read_text(), "untouched")

    def test_windows_package_is_a_native_zip_with_executable_and_skill(self):
        binary = self.prepare_package()
        output = self.base / "windows packages"
        self.run_script("package-cli.sh", "--binary", binary, "--target", "aarch64-pc-windows-msvc", "--out-dir", output)
        archive = next(output.glob("*.zip"))
        with zipfile.ZipFile(archive) as handle:
            prefix = "hireferry-cli-0.1.0-aarch64-pc-windows-msvc/"
            self.assertEqual(handle.read(prefix + "hfy.exe"), binary.read_bytes())
            self.assertIn(prefix + "skills/hireferry/SKILL.md", handle.namelist())
            self.assertFalse(any("config.toml" in name for name in handle.namelist()))
        self.assertEqual((output / "SHA256SUMS").read_text().split(), [hashlib.sha256(archive.read_bytes()).hexdigest(), archive.name])


class CliReleaseAssembly(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="hireferry-cli-release-")
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(REPO / "scripts/assemble-cli-release.py", self.root / "scripts")
        manifest = self.root / "crates/hireferry-cli/Cargo.toml"
        manifest.parent.mkdir(parents=True)
        manifest.write_text('version = "1.2.3"\n')
        self.packages = self.root / "packages"
        targets = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl",
                   "x86_64-apple-darwin", "aarch64-apple-darwin",
                   "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc")
        for target in targets:
            folder = self.packages / ("cli-release-" + target)
            folder.mkdir(parents=True)
            name = f"hireferry-cli-1.2.3-{target}." + ("zip" if "windows" in target else "tar.gz")
            data = target.encode()
            (folder / name).write_bytes(data)
            marker = " *" if "windows" in target else "  "
            (folder / "SHA256SUMS").write_bytes((hashlib.sha256(data).hexdigest() + marker + name + "\r\n").encode())

    def tearDown(self):
        self.temp.cleanup()

    def assemble(self):
        return subprocess.run(["python3", str(self.root / "scripts/assemble-cli-release.py"),
                               str(self.packages), str(self.root / "release")],
                              capture_output=True, text=True, timeout=15)

    def test_native_text_and_binary_checksum_formats_publish_one_canonical_manifest(self):
        result = self.assemble()
        self.assertEqual(result.returncode, 0, result.stderr)
        output = self.root / "release"
        self.assertEqual((output / "VERSION").read_text(), "1.2.3\n")
        lines = (output / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(lines), 6)
        for line in lines:
            digest, name = line.split("  ")
            self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)

    def test_tampered_archive_prevents_a_publishable_release(self):
        archive = next((self.packages / "cli-release-x86_64-pc-windows-msvc").glob("*.zip"))
        archive.write_bytes(b"changed after packaging")
        result = self.assemble()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Invalid checksums", result.stderr)
        self.assertFalse((self.root / "release/VERSION").exists())
        self.assertFalse((self.root / "release/SHA256SUMS").exists())


if __name__ == "__main__":
    unittest.main()

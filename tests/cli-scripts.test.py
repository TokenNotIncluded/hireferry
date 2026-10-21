"""Exercise installation boundaries and package contents in isolated directories."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


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


if __name__ == "__main__":
    unittest.main()

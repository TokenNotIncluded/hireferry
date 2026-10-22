"""Guard release identity and prevent stale/unverified mobile artifacts."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("hireferry_mobile", Path(__file__).resolve().parents[1] / "scripts" / "mobile.py")
mobile = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mobile)


class MobilePackagingTests(unittest.TestCase):
    def test_release_rejects_missing_credentials_before_build(self):
        with patch.dict(os.environ, {}, clear=True):
            with self.assertRaisesRegex(mobile.MobileError, "maintainer signing"):
                mobile.require_android_signing()

    def test_release_requires_expected_certificate_not_only_a_keystore(self):
        with tempfile.TemporaryDirectory() as directory:
            key = Path(directory) / "release.keystore"
            key.touch()
            values = {name: "test-value" for name in mobile.ANDROID_SIGNING}
            values["HF_ANDROID_KEYSTORE_PATH"] = str(key)
            with patch.dict(os.environ, values, clear=True):
                with self.assertRaisesRegex(mobile.MobileError, "SHA-256"):
                    mobile.require_android_signing()
            values["HF_ANDROID_CERT_SHA256"] = ":".join(["AB"] * 32)
            with patch.dict(os.environ, values, clear=True):
                self.assertEqual(mobile.require_android_signing(), "ab" * 32)

    def test_signing_setup_is_idempotent_and_never_writes_env_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory)
            (project / "app").mkdir()
            path = project / "app" / "build.gradle.kts"
            path.write_text('android { namespace = "best.lmm.hireferry" }\n')
            with patch.dict(os.environ, {"HF_ANDROID_KEYSTORE_PASSWORD": "do-not-write-this-secret"}):
                mobile.prepare_android_signing(project)
                mobile.prepare_android_signing(project)
            content = path.read_text()
            self.assertEqual(content.count(mobile.SIGNING_MARKER), 1)
            self.assertNotIn("do-not-write-this-secret", content)

    def test_apk_certificate_mismatch_is_an_error(self):
        responses = ["Signer #1 certificate SHA-256 digest: " + "ab" * 32,
                     "package: name='best.lmm.hireferry.debug' versionCode='1000' versionName='0.1.0'"]
        with patch.object(mobile, "android_tool", return_value=Path("tool")), patch.object(mobile, "run", side_effect=responses):
            with self.assertRaisesRegex(mobile.MobileError, "certificate differs"):
                mobile.verify_android(Path("debug.apk"), "cd" * 32)

    def test_unsigned_aab_is_not_reported_as_verified(self):
        with patch.object(mobile, "run", return_value="jar is unsigned."):
            with self.assertRaisesRegex(mobile.MobileError, "unsigned"):
                mobile.verify_android(Path("bundle.aab"))

    def test_engineering_manifest_cannot_claim_production_distribution(self):
        with tempfile.TemporaryDirectory() as directory:
            args = types.SimpleNamespace(platform="android", variant="debug", simulator=False, target=["aarch64"])
            mobile.write_manifest(Path(directory), [{"filename": "test.apk", "sha256": "ab" * 32, "bytes": 42}], args)
            manifest = json.loads((Path(directory) / "mobile-artifacts.json").read_text())
            self.assertEqual(manifest["distribution"], "engineering-test")
            self.assertEqual((Path(directory) / "SHA256SUMS").read_text(), "ab" * 32 + "  test.apk\n")

    def test_remote_site_has_no_native_capability(self):
        capability = json.loads((mobile.TAURI / "capabilities" / "launcher.json").read_text())
        self.assertTrue(capability["local"])
        self.assertEqual(capability["windows"], ["launcher"])
        self.assertNotIn("remote", capability)
        self.assertEqual(set(capability["permissions"]), {"allow-connect", "allow-open-browser"})

    def test_apple_silicon_simulator_artifact_is_collected_from_simulator_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory) / "apple"
            application = project / "build" / "arm64-sim" / "HireFerry.app"
            application.mkdir(parents=True)
            (application / "Info.plist").write_text("simulator artifact fixture")
            output = Path(directory) / "artifacts"
            args = types.SimpleNamespace(platform="ios", variant="debug", simulator=True,
                                         target=["aarch64-sim"], output=str(output))
            with patch.object(mobile, "doctor"), patch.object(mobile, "run"), \
                    patch.object(mobile, "initialize", return_value=project), \
                    patch.object(mobile, "cli", return_value=["npm", "run", "tauri", "--"]):
                mobile.build(args)
            manifest = json.loads((output / "mobile-artifacts.json").read_text())
            self.assertTrue(manifest["simulator_only"])
            self.assertEqual(manifest["artifacts"][0]["filename"], "hireferry-ios-aarch64-sim-simulator.app.tar.gz")
            self.assertFalse(manifest["artifacts"][0]["signature_verified"])


if __name__ == "__main__":
    unittest.main()

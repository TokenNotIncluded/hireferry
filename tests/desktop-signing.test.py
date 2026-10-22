#!/usr/bin/env python3
"""Signing policy tests; mocked native commands are not evidence of OS signing."""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'scripts'))
spec = importlib.util.spec_from_file_location('signing', ROOT/'scripts/desktop-signing.py')
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class SigningPolicy(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.clean = patch.dict(os.environ, {}, clear=True)
        self.clean.start()
        self.addCleanup(self.clean.stop)

    def args(self, target):
        return argparse.Namespace(target=target, out=self.root/'config.json')

    def test_unsigned_engineering_build_has_explicit_ad_hoc_macos_config(self):
        args = self.args('aarch64-apple-darwin')
        signing.configure(args)
        self.assertEqual(json.loads(args.out.read_text()), {'bundle':{'macOS':{'signingIdentity':'-'}}})
        signing.configure(self.args('aarch64-pc-windows-msvc'))
        self.assertEqual(json.loads(args.out.read_text()), {})

    def test_formal_windows_and_macos_fail_before_build_without_credentials(self):
        os.environ['DESKTOP_REQUIRE_SIGNATURES'] = 'true'
        for target in ('x86_64-apple-darwin','aarch64-pc-windows-msvc'):
            with self.subTest(target=target), self.assertRaisesRegex(ValueError, 'Formal desktop release requires'):
                signing.configure(self.args(target))
        signing.configure(self.args('x86_64-unknown-linux-gnu'))

    def test_partial_or_ambiguous_notarization_configuration_fails(self):
        credentials = {'APPLE_CERTIFICATE':'fixture', 'APPLE_CERTIFICATE_PASSWORD':'secret',
                       'APPLE_SIGNING_IDENTITY':'Developer ID Application: Fixture',
                       'APPLE_ID':'fixture@example.invalid', 'APPLE_PASSWORD':'private', 'APPLE_TEAM_ID':'ABCDE12345'}
        with patch.dict(os.environ, {'APPLE_CERTIFICATE':'fixture'}):
            with self.assertRaisesRegex(ValueError, 'complete notarization'):
                signing.configure(self.args('aarch64-apple-darwin'))
        with patch.dict(os.environ, dict(credentials, APPLE_API_KEY='PARTIAL')):
            with self.assertRaisesRegex(ValueError, 'complete notarization'):
                signing.configure(self.args('aarch64-apple-darwin'))
        with patch.dict(os.environ, credentials):
            signing.configure(self.args('aarch64-apple-darwin'))
            config = (self.root/'config.json').read_text()
            self.assertNotIn('private', config)
            self.assertNotIn('secret', config)
            self.assertIn('Developer ID Application:', config)

    def test_api_key_is_private_temporary_file_not_in_overlay(self):
        os.environ.update({'APPLE_CERTIFICATE':'fixture', 'APPLE_CERTIFICATE_PASSWORD':'secret',
                           'APPLE_SIGNING_IDENTITY':'Developer ID Application: Fixture',
                           'APPLE_API_ISSUER':'issuer', 'APPLE_API_KEY':'keyid',
                           'APPLE_API_KEY_BASE64':base64.b64encode(b'private-key-fixture').decode(),
                           'GITHUB_ENV':str(self.root/'environment')})
        signing.configure(self.args('x86_64-apple-darwin'))
        key = self.root/'hireferry-notary-key.p8'
        self.assertEqual(key.read_bytes(), b'private-key-fixture')
        if os.name != 'nt':
            self.assertEqual(key.stat().st_mode & 0o777, 0o600)
        self.assertNotIn('private-key-fixture', (self.root/'config.json').read_text())
        self.assertIn('APPLE_API_KEY_PATH=', (self.root/'environment').read_text())

    def test_windows_certificate_import_uses_env_password_and_erases_pfx(self):
        os.environ.update({'WINDOWS_CERTIFICATE':base64.b64encode(b'pfx-fixture').decode(),
                           'WINDOWS_CERTIFICATE_PASSWORD':'not-in-argv',
                           'WINDOWS_CERTIFICATE_THUMBPRINT':'A'*40,
                           'WINDOWS_TIMESTAMP_URL':'http://timestamp.example.invalid'})
        with patch.object(signing, 'run') as native:
            signing.configure(self.args('x86_64-pc-windows-msvc'))
        args, kwargs = native.call_args
        self.assertNotIn('not-in-argv', ' '.join(args[0]))
        self.assertFalse(Path(kwargs['env']['HF_PFX_FILE']).exists())
        config = json.loads((self.root/'config.json').read_text())['bundle']['windows']
        self.assertEqual(config['certificateThumbprint'], 'A'*40)
        self.assertEqual(config['digestAlgorithm'], 'sha256')
        self.assertTrue(config['tsp'])

    def test_windows_verification_requires_valid_signature_and_timestamp(self):
        file = self.root/'test.exe'
        for status, timestamped, expected in [('NotSigned',False,'unsigned'),('Valid',True,'authenticode'),
                                               ('Valid',False,None),('HashMismatch',True,None)]:
            result = SimpleNamespace(stdout=json.dumps({'status':status, 'timestamped':timestamped, 'publisher':'CN=Fixture'}))
            with self.subTest(status=status,timestamped=timestamped), patch.object(signing,'run',return_value=result):
                if expected:
                    self.assertEqual(signing.windows_verification(file)['verification'], expected)
                else:
                    with self.assertRaises(ValueError): signing.windows_verification(file)

    def test_macos_developer_id_requires_stapled_ticket(self):
        details = SimpleNamespace(stderr='Authority=Developer ID Application: Fixture\n')
        with patch.object(signing,'run',side_effect=[SimpleNamespace(),details,SimpleNamespace()]) as native:
            result = signing.verify_app(self.root/'HireFerry.app')
            self.assertEqual(result['verification'], 'developer-id-notarized')
            self.assertEqual(native.call_args.args[0][:3], ['xcrun','stapler','validate'])
        with patch.object(signing,'run',side_effect=[SimpleNamespace(),details,ValueError('not stapled')]):
            with self.assertRaisesRegex(ValueError,'not stapled'):
                signing.verify_app(self.root/'HireFerry.app')


if __name__ == '__main__': unittest.main()

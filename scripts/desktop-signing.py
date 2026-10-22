#!/usr/bin/env python3
"""Configure native signing, then verify the final packages on their native OS.

No credentials are printed or stored in release artifacts. Unsigned builds are
engineering artifacts; a formal desktop release requires native verification.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile

import release


def required():
    return os.environ.get('DESKTOP_REQUIRE_SIGNATURES', '').lower() == 'true'


def run(command, *, check=True, env=None):
    result = subprocess.run(command, text=True, capture_output=True, env=env)
    if check and result.returncode:
        # Native signing errors can contain account details. Do not echo secrets.
        raise ValueError(f'{Path(command[0]).name} failed (exit {result.returncode}); inspect the native signing configuration')
    return result


def present(names):
    return all(os.environ.get(name) for name in names)


def configure(args):
    row = release.find_target(release.catalog(), 'desktop', args.target)
    if row is None:
        raise ValueError('Unknown desktop target')
    config = {}
    args.out.parent.mkdir(parents=True, exist_ok=True)
    platform = row['platform']
    if platform == 'Windows':
        names = ('WINDOWS_CERTIFICATE', 'WINDOWS_CERTIFICATE_PASSWORD',
                 'WINDOWS_CERTIFICATE_THUMBPRINT', 'WINDOWS_TIMESTAMP_URL')
        supplied = any(os.environ.get(name) for name in names)
        if supplied and not present(names):
            raise ValueError('Incomplete Windows signing settings: configure all four WINDOWS_* inputs')
        if required() and not supplied:
            raise ValueError('Formal desktop release requires Windows certificate, password, thumbprint and timestamp URL')
        if supplied:
            thumbprint = os.environ['WINDOWS_CERTIFICATE_THUMBPRINT'].replace(' ', '').upper()
            if not re.fullmatch(r'[0-9A-F]{40}', thumbprint):
                raise ValueError('Windows certificate thumbprint must contain 40 hexadecimal characters')
            timestamp = os.environ['WINDOWS_TIMESTAMP_URL']
            if not re.fullmatch(r'https?://[^\s]+', timestamp):
                raise ValueError('Windows timestamp URL must be an HTTP(S) timestamp service')
            # Import-PfxCertificate reads its password from the environment, never argv.
            with tempfile.TemporaryDirectory(prefix='hireferry-pfx-') as temporary:
                certificate = Path(temporary)/'certificate.pfx'
                certificate.write_bytes(base64.b64decode(os.environ['WINDOWS_CERTIFICATE'], validate=True))
                certificate.chmod(0o600)
                env = dict(os.environ, HF_PFX_FILE=str(certificate), HF_SIGN_THUMBPRINT=thumbprint)
                script = r'''$ErrorActionPreference = 'Stop'
$pw = ConvertTo-SecureString $env:WINDOWS_CERTIFICATE_PASSWORD -AsPlainText -Force
$certs = Import-PfxCertificate -FilePath $env:HF_PFX_FILE -CertStoreLocation Cert:\CurrentUser\My -Password $pw
$cert = @($certs | Where-Object { $_.Thumbprint -eq $env:HF_SIGN_THUMBPRINT -and $_.HasPrivateKey -and $_.NotAfter -gt (Get-Date) })
if ($cert.Count -ne 1) { throw 'Requested signing certificate/private key was not imported or has expired' }
'''
                run(['pwsh', '-NoProfile', '-NonInteractive', '-Command', script], env=env)
            config = {'bundle': {'windows': {'certificateThumbprint':thumbprint,
                       'digestAlgorithm':'sha256', 'timestampUrl':timestamp, 'tsp':True}}}
    elif platform == 'macOS':
        certificate = ('APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY')
        password_auth = ('APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID')
        api_auth = ('APPLE_API_ISSUER', 'APPLE_API_KEY', 'APPLE_API_KEY_BASE64')
        supplied = any(os.environ.get(name) for name in certificate + password_auth + api_auth)
        partial_password = any(os.environ.get(name) for name in password_auth) and not present(password_auth)
        partial_api = any(os.environ.get(name) for name in api_auth) and not present(api_auth)
        if supplied and (not present(certificate) or partial_password or partial_api or not (present(password_auth) ^ present(api_auth))):
            raise ValueError('macOS requires a Developer ID certificate and exactly one complete notarization credential set')
        if required() and not supplied:
            raise ValueError('Formal desktop release requires Apple Developer ID signing and notarization credentials')
        if supplied:
            if not os.environ['APPLE_SIGNING_IDENTITY'].startswith('Developer ID Application:'):
                raise ValueError('Use a Developer ID Application identity for direct macOS distribution')
            # Tauri imports the certificate itself. For API authentication it expects a key file.
            if present(api_auth):
                key = args.out.parent/'hireferry-notary-key.p8'
                key.write_bytes(base64.b64decode(os.environ['APPLE_API_KEY_BASE64'], validate=True))
                key.chmod(0o600)
                with open(os.environ['GITHUB_ENV'], 'a') as output:
                    output.write(f'APPLE_API_KEY_PATH={key.resolve()}\n')
            config = {'bundle': {'macOS': {'signingIdentity':os.environ['APPLE_SIGNING_IDENTITY']}}}
        else:
            config = {'bundle': {'macOS': {'signingIdentity':'-'}}}
    args.out.write_text(json.dumps(config) + '\n')
    print(f'{platform}: {"release signing required" if required() else "engineering build; publication disabled"}')


def windows_verification(path):
    env = dict(os.environ, HF_VERIFY_FILE=str(path.resolve()))
    script = r'''$ErrorActionPreference = 'Stop'
$s = Get-AuthenticodeSignature -LiteralPath $env:HF_VERIFY_FILE
@{status=$s.Status.ToString(); publisher=if($s.SignerCertificate){$s.SignerCertificate.Subject}else{''}; timestamped=($null -ne $s.TimeStamperCertificate)} | ConvertTo-Json -Compress
'''
    result = json.loads(run(['pwsh', '-NoProfile', '-NonInteractive', '-Command', script], env=env).stdout)
    if result['status'] == 'NotSigned':
        return {'verification':'unsigned'}
    if result['status'] != 'Valid' or not result['timestamped']:
        raise ValueError(f'Windows package lacks a valid, timestamped Authenticode signature: {path.name}')
    return {'verification':'authenticode', 'publisher':result['publisher']}


def verify_app(app):
    run(['codesign', '--verify', '--deep', '--strict', str(app)])
    detail = run(['codesign', '--display', '--verbose=4', str(app)]).stderr
    authority = next((line.removeprefix('Authority=') for line in detail.splitlines()
                      if line.startswith('Authority=Developer ID Application:')), None)
    if authority:
        run(['xcrun', 'stapler', 'validate', str(app)])
        return {'verification':'developer-id-notarized', 'publisher':authority}
    if 'Signature=adhoc' in detail:
        return {'verification':'ad-hoc'}
    raise ValueError('macOS app is neither ad-hoc nor a stapled Developer ID signed app')


def macos_verification(path):
    with tempfile.TemporaryDirectory(prefix='hireferry-verify-') as temporary:
        folder = Path(temporary)
        if path.name.endswith('.app.tar.gz'):
            # This is our staged artifact. Validate paths before native extraction,
            # which also restores macOS metadata preserved by native tar.
            with tarfile.open(path) as archive:
                for member in archive.getmembers():
                    if member.name.startswith('/') or '..' in Path(member.name).parts:
                        raise ValueError('Unsafe macOS app archive path')
            run(['tar', '-xzf', str(path.resolve()), '-C', str(folder)])
            apps = list(folder.glob('*.app'))
            if len(apps) != 1:
                raise ValueError('Expected one app in macOS archive')
            return verify_app(apps[0])
        mount = folder/'mounted'
        mount.mkdir()
        run(['hdiutil', 'attach', '-nobrowse', '-readonly', '-mountpoint', str(mount), str(path.resolve())])
        try:
            apps = list(mount.glob('*.app'))
            if len(apps) != 1:
                raise ValueError('Expected one app in macOS disk image')
            return verify_app(apps[0])
        finally:
            run(['hdiutil', 'detach', str(mount)])


def verify(args):
    data = release.catalog()
    row = release.find_target(data, 'desktop', args.target)
    if row is None:
        raise ValueError('Unknown desktop target')
    name = release.signing_report_name(args.target)
    assets = {}
    wanted = release.expected(data, {'desktop':release.package_version('desktop')}, ('desktop',))
    for file, metadata in wanted.items():
        if metadata['target'] != args.target or metadata['format'] == 'flatpak':
            continue
        path = args.directory/file
        if not path.is_file() or path.is_symlink() or not path.stat().st_size:
            raise ValueError(f'Missing package to verify: {file}')
        if row['platform'] == 'Windows':
            checked = windows_verification(path)
        elif row['platform'] == 'macOS':
            checked = macos_verification(path)
        else:
            checked = {'verification':'checksum-only'}
        assets[file] = dict(checked, sha256=release.digest(path), size=path.stat().st_size)
    report = {'schema_version':1, 'target':args.target, 'assets':assets}
    release.validate_signing_report(report, {n:m for n,m in wanted.items()
                                            if m['target']==args.target and m['format']!='flatpak'},
                                    {n:args.directory/n for n in assets}, require_signed=required())
    (args.directory/name).write_text(json.dumps(report, indent=2) + '\n')
    release.checksums(args.directory)
    print(f'Native verification recorded for {len(assets)} {args.target} packages')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    config = commands.add_parser('configure')
    config.add_argument('--target', required=True)
    config.add_argument('--out', required=True, type=Path)
    config.set_defaults(run=configure)
    check = commands.add_parser('verify')
    check.add_argument('--target', required=True)
    check.add_argument('--directory', required=True, type=Path)
    check.set_defaults(run=verify)
    args = parser.parse_args()
    try:
        args.run(args)
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        parser.exit(1, f'desktop-signing: {error}\n')

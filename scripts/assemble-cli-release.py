#!/usr/bin/env python3
"""Publish only a complete, checksum-verified set of CLI packages."""
import hashlib
from pathlib import Path
import re
import shutil
import sys

TARGETS = (
    'x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl',
    'x86_64-apple-darwin', 'aarch64-apple-darwin',
    'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc',
)

def assemble(source, destination):
    root = Path(__file__).resolve().parents[1]
    version = re.search(r'^version = "([^"]+)"$', (root/'crates/hireferry-cli/Cargo.toml').read_text(), re.M)[1]
    destination.mkdir(parents=True, exist_ok=False)
    lines = []
    for target in TARGETS:
        folder = source / ('cli-release-' + target)
        suffix = 'zip' if 'windows' in target else 'tar.gz'
        name = f'hireferry-cli-{version}-{target}.{suffix}'
        archive = folder/name
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        checksums = (folder/'SHA256SUMS').read_text().splitlines()
        # GNU checksum files use a space for text mode or '*' for binary mode.
        # Git Bash on Windows defaults to the latter; both identify the same bytes.
        checksum = re.fullmatch(r'([0-9a-fA-F]{64}) [ *]' + re.escape(name), checksums[0]) if len(checksums) == 1 else None
        assert checksum and checksum[1].lower() == actual, f'Invalid checksums for {target}'
        shutil.copyfile(archive, destination/name)
        lines.append(actual+'  '+name)
    (destination/'SHA256SUMS').write_text('\n'.join(sorted(lines))+'\n')
    (destination/'VERSION').write_text(version+'\n')
    print(f'Verified six platforms for hfy {version}')

if __name__ == '__main__':
    assemble(Path(sys.argv[1]), Path(sys.argv[2]))

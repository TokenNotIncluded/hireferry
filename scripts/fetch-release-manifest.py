#!/usr/bin/env python3
"""Download and validate a published stable release's checksummed manifest."""
import argparse
import json
from pathlib import Path
import re
import subprocess

import release

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('tag')
parser.add_argument('destination', type=Path)
args = parser.parse_args()
if not re.fullmatch(r'(desktop|cli|server)-v[0-9]+\.[0-9]+\.[0-9]+', args.tag):
    parser.error('Expected a stable product release tag')
repo = release.catalog()['repository']
metadata = json.loads(subprocess.check_output(['gh','release','view',args.tag,'--repo',repo,'--json','tagName,isDraft,isPrerelease'],text=True))
if metadata['isDraft'] or metadata['isPrerelease'] or metadata['tagName'] != args.tag:
    parser.error('Only published stable releases are supported')
args.destination.mkdir(parents=True, exist_ok=False)
subprocess.run(['gh','release','download',args.tag,'--repo',repo,'--pattern','downloads.json','--pattern','SHA256SUMS','--dir',str(args.destination)],check=True)
expected = [line.split()[0] for line in (args.destination/'SHA256SUMS').read_text().splitlines()
            if len(line.split()) == 2 and line.split()[1] == 'downloads.json']
if expected != [release.digest(args.destination/'downloads.json')]:
    parser.error('Release manifest checksum mismatch')
manifest = release.validate_manifest(json.loads((args.destination/'downloads.json').read_text()))
product = args.tag.split('-v', 1)[0]
if manifest['repository'] != repo or manifest['releases'].get(product) != args.tag:
    parser.error('Manifest does not belong to the requested product release')
print(args.destination/'downloads.json')

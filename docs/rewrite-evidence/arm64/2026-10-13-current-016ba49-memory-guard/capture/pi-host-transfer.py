# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Use the guarded runner's host copy methods for owned preparation artifacts."""
import argparse
import hashlib
import json
import pathlib
import re
import sys
import time

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
RUN = ROOT / '.cache/arm64-qual/integrated-2026-10-12'
sys.path.insert(0, str(ROOT / 'tests/qualification/arm64'))
from native_vm import Guest

parser = argparse.ArgumentParser()
parser.add_argument('direction', choices=['push', 'pull'])
parser.add_argument('local', type=pathlib.Path)
parser.add_argument('remote')
args = parser.parse_args()
if not re.fullmatch(r'limeos-arm64-qual/[a-zA-Z0-9_./+-]+', args.remote) or '..' in args.remote.split('/'):
    parser.error('copy must stay within the owned host staging directory')
local = args.local.resolve()
if not local.is_relative_to(RUN.resolve()):
    parser.error('local copy must stay within this run')
guest = Guest('host-copy', 'holly@192.168.0.45', RUN / 'authorization-pi45.json')
started = time.time()
if args.direction == 'push':
    guest.host_push(local, args.remote)
else:
    guest.host_pull(args.remote, local)
record = {'started_epoch': started, 'direction': args.direction, 'local': str(local), 'remote': args.remote, 'authorization_sha256': guest.authorization['sha256'], 'bytes': local.stat().st_size, 'sha256': hashlib.file_digest(local.open('rb'), 'sha256').hexdigest()}
with (RUN / 'host-transfers.jsonl').open('a') as stream:
    stream.write(json.dumps(record)+'\n')
print(json.dumps({k:v for k,v in record.items() if k != 'local'}))

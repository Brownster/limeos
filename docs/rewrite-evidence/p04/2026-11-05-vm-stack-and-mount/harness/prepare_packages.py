#!/usr/bin/env python3
"""Package this worktree's built binaries and sign a disposable offline APT repository."""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
STATE = ROOT / '.cache/q'
BUILD = STATE / 'build'
OUT = STATE / 'candidate'
KEYS = STATE / 'signing'
VERSION = '0.4.15+stacklc.1'
IMAGE = 'sha256:e0df67a00f0f3080a0d01274c4c492c1173c6e39853913864d2cf596ad726fab'  # gpg + apt-ftparchive, no network


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


record = json.loads((BUILD / 'record.json').read_text())
assert record['exit_code'] == 0
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert record['source'] == head
for name, facts in record['binaries'].items():
    assert digest(BUILD / 'binaries' / name) == facts['sha256'], name
assert (ROOT / 'frontend/dist/index.html').is_file()
shutil.rmtree(OUT, ignore_errors=True)
shutil.rmtree(KEYS, ignore_errors=True)
OUT.mkdir()
KEYS.mkdir(mode=0o700)
packages = OUT / 'packages'
packages.mkdir()
environment = os.environ.copy()
environment['TMPDIR'] = str(STATE / 'tmp')
for profile in ('standard', 'shadow'):
    subprocess.run([sys.executable, 'packaging/build.py', '--arch', 'amd64', '--version', VERSION, '--profile', profile,
                    '--binaries', str(BUILD / 'binaries'), '--output', str(packages)], cwd=ROOT, env=environment, check=True)


def inside(path):
    return '/build/' + str(path.relative_to(ROOT))


command = ['docker', 'run', '--rm', '--network=none', '--read-only', '--memory=128m', '--memory-swap=128m',
           '--pids-limit=32', '--cpus=1', '--cap-drop=ALL', '--security-opt=no-new-privileges',
           '--security-opt=label=disable', '--user', '1000:1000',
           '--env', 'GNUPGHOME=' + inside(KEYS), '--env', 'TMPDIR=' + inside(STATE / 'tmp'),
           '--volume', str(ROOT) + ':/build:ro', '--volume', str(STATE) + ':' + inside(STATE),
           '--workdir', '/build', IMAGE]
subprocess.run([*command, 'gpg', '--batch', '--pinentry-mode', 'loopback', '--passphrase', '',
                '--quick-generate-key', 'LimeOS disposable stack/mount fixture 2026-11-05', 'ed25519', 'sign', '7d'], check=True, cwd=ROOT)
public = subprocess.check_output([*command, 'gpg', '--batch', '--with-colons', '--list-keys'], text=True)
fingerprints = [line.split(':')[9] for line in public.splitlines() if line.startswith('fpr:')]
assert len(fingerprints) == 1 and re.fullmatch('[A-F0-9]{40}', fingerprints[0])
files = sorted(packages.glob('*.deb'))
assert len(files) == 2
repository = OUT / 'repository'
subprocess.run([*command, 'packaging/repository.sh', inside(repository), fingerprints[0], *map(inside, files)], check=True, cwd=ROOT)
subprocess.run([*command, 'gpgv', '--keyring', inside(repository / 'limeos-archive-keyring.gpg'),
                inside(repository / 'dists/stable/InRelease')], check=True, cwd=ROOT)
inputs = {'source': head, 'version': VERSION, 'signing_fingerprint': fingerprints[0],
          'packages': {p.name: digest(p) for p in files}, 'binaries': record['binaries'],
          'ui_note': 'frontend/dist copied from the shared checkout (untracked build output); UI is not under test',
          'repository': {str(p.relative_to(repository)): digest(p) for p in sorted(repository.rglob('*')) if p.is_file()}}
(OUT / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
print(json.dumps({k: inputs[k] for k in ('source', 'version', 'signing_fingerprint', 'packages')}), flush=True)

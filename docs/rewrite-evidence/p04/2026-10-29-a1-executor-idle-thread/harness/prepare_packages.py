# /// script
# dependencies = []
# ///
"""Package one current source and sign a disposable, offline VM repository."""
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/stack-installed-qualification'
BUILD = STATE / 'build-7'
OUT = STATE / 'candidate-7'
KEYS = STATE / 'signing-7'
VERSION = '0.4.7+scan.1'
IMAGE = 'sha256:e0df67a00f0f3080a0d01274c4c492c1173c6e39853913864d2cf596ad726fab'

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

record = json.loads((BUILD / 'record.json').read_text())
assert record['exit_code'] == 0 and record['stop_reason'] is None
assert not record['oom_killed'] and record['inputs_unchanged'] and record['container_removed']
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() == record['source']
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
for name, expected in record['before'].items():
    assert digest(ROOT / name) == expected, name
binaries = json.loads((BUILD / 'binaries.json').read_text())
assert set(binaries) == {'limeos-core', 'limeos-password-worker', 'limeos-executor', 'limeos-stack-read-worker', 'limeosctl'}
for name, facts in binaries.items():
    assert digest(BUILD / 'binaries' / name) == facts['sha256'], name
ui_record_path = ROOT / '.cache/main-consolidation/attempts/a1-initial-census-ui-build-1/record.json'
ui_record = json.loads(ui_record_path.read_text())
assert ui_record['exit_code'] == 0 and not ui_record['guard_triggered'] and ui_record['inputs_unchanged']
assert ui_record['head'] == record['source']
for name, expected in ui_record['before'].items():
    assert digest(ROOT / name) == expected, name
ui = {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / 'frontend/dist').rglob('*')) if path.is_file()}
assert 'frontend/dist/index.html' in ui and len(ui) == 3
OUT.mkdir(exist_ok=False)
KEYS.mkdir(mode=0o700, exist_ok=False)
packages = OUT / 'packages'
packages.mkdir()
environment = os.environ.copy()
environment['TMPDIR'] = str(STATE / 'tmp')
for profile in ('standard', 'shadow'):
    subprocess.run(['python3', 'packaging/build.py', '--arch', 'amd64', '--version', VERSION,
                    '--profile', profile, '--binaries', str(BUILD / 'binaries'), '--output', str(packages)],
                   cwd=ROOT, env=environment, check=True)

def inside(path):
    return '/build/' + str(path.relative_to(ROOT))

command = ['docker', 'run', '--rm', '--network=none', '--read-only', '--memory=128m', '--memory-swap=128m',
           '--pids-limit=32', '--cpus=1', '--cap-drop=ALL', '--security-opt=no-new-privileges',
           '--security-opt=label=disable', '--user', '1000:1000',
           '--env', 'GNUPGHOME=' + inside(KEYS), '--env', 'TMPDIR=' + inside(STATE / 'tmp'),
           '--volume', str(ROOT) + ':/build:ro', '--volume', str(STATE) + ':' + inside(STATE),
           '--workdir', '/build', IMAGE]
subprocess.run([*command, 'gpg', '--batch', '--pinentry-mode', 'loopback', '--passphrase', '',
                '--quick-generate-key', 'LimeOS disposable census A1 fixture 2026-10-29', 'ed25519', 'sign', '7d'],
               check=True, cwd=ROOT)
public = subprocess.check_output([*command, 'gpg', '--batch', '--with-colons', '--list-keys'], text=True)
fingerprints = [line.split(':')[9] for line in public.splitlines() if line.startswith('fpr:')]
assert len(fingerprints) == 1 and re.fullmatch('[A-F0-9]{40}', fingerprints[0])
fingerprint = fingerprints[0]
expiry = int(next(line.split(':')[6] for line in public.splitlines() if line.startswith('pub:')))
files = sorted(packages.glob('*.deb'))
assert len(files) == 2
observed = subprocess.check_output([*command, 'sha256sum', *map(inside, files)], text=True).splitlines()
assert [line.split()[0] for line in observed] == [digest(path) for path in files]
repository = OUT / 'repository'
subprocess.run([*command, 'packaging/repository.sh', inside(repository), fingerprint, *map(inside, files)],
               check=True, cwd=ROOT)
subprocess.run([*command, 'gpgv', '--keyring', inside(repository / 'limeos-archive-keyring.gpg'),
                inside(repository / 'dists/stable/InRelease')], check=True, cwd=ROOT)
assert ui == {str(path.relative_to(ROOT)): digest(path) for path in sorted((ROOT / 'frontend/dist').rglob('*')) if path.is_file()}
inputs = {'source': record['source'], 'version': VERSION, 'scope': 'Current AMD64 standard/shadow VM fixtures; not a published release',
          'packages': {path.name: digest(path) for path in files}, 'binaries': binaries, 'ui': ui,
          'ui_build_record_sha256': digest(ui_record_path), 'packaging_sha256': digest(ROOT / 'packaging/build.py'),
          'producer_sha256': digest(Path(__file__)),
          'repository_tool_image': IMAGE, 'package_handoff': observed,
          'signing_fingerprint': fingerprint, 'signing_key_expires': expiry,
          'repository': {str(path.relative_to(repository)): digest(path) for path in sorted(repository.rglob('*')) if path.is_file()}}
(OUT / 'inputs.json').write_text(json.dumps(inputs, indent=2) + '\n')
print(json.dumps({'source': inputs['source'], 'version': VERSION, 'packages': inputs['packages'],
                  'signing_fingerprint': fingerprint, 'signing_key_expires': expiry,
                  'private_key_exported': False, 'repository_signature_verified': True}), flush=True)

# /// script
# dependencies = []
# ///
"""Revalidate the frozen candidate at actual launch time, then run one guarded VM."""
import datetime
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/a1-initial-census-diagnostic'
PACKAGES = ROOT / '.cache/stack-installed-qualification/candidate-7'
manifest = json.loads((STATE / 'inputs-1.json').read_text())
packages = json.loads((PACKAGES / 'inputs.json').read_text())
build = json.loads((ROOT / '.cache/stack-installed-qualification/build-7/record.json').read_text())

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

assert not (ROOT / '.cache/main-consolidation/attempts/a1-initial-census-vm-1').exists()
assert not (STATE / 'result-1.json').exists()
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() == manifest['product_source'] == packages['source'] == build['source']
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
assert digest(PACKAGES / 'inputs.json') == manifest['package_inputs_sha256']
for name, expected in build['before'].items():
    assert digest(ROOT / name) == expected, name
for name, expected in manifest['guest'].items():
    assert digest(STATE / 'guest-1' / name) == expected, name
for name, expected in manifest['runner_support'].items():
    assert digest(STATE / name) == expected, name
assert digest(STATE / 'run_384.py') == manifest['runner_sha256']
assert digest(ROOT / 'crates/persistence/src/lib.rs') == manifest['schema_source_sha256']
for name, expected in packages['repository'].items():
    assert digest(PACKAGES / 'repository' / name) == expected, name
busy = [row for row in subprocess.check_output(['ps', '-eo', 'comm'], text=True).splitlines() if row.strip() in ('cargo', 'rustc', 'rustdoc')]
assert not busy, 'A compiler is active; do not launch another local suite'
now = datetime.datetime.now(datetime.timezone.utc)
assert packages['signing_key_expires'] > int(now.timestamp()) + 3600, 'Disposable signing key needs renewal'
available = next(int(line.split()[1]) * 1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
admission = {'at_utc': now.isoformat(), 'source': packages['source'],
             'launcher_sha256': digest(Path(__file__)), 'available_bytes': available,
             'guest_start_threshold_bytes': 3 * 1024**3 + 512 * 1024**2,
             'runtime_stop_floor_bytes': 3 * 1024**3, 'disk_free_bytes': shutil.disk_usage(ROOT).free,
             'guest_memory_mib': 384, 'admitted': available >= 3 * 1024**3 + 512 * 1024**2}
(STATE / 'admission-1.json').write_text(json.dumps(admission, indent=2) + '\n')
print(json.dumps(admission), flush=True)
if not admission['admitted']:
    raise SystemExit(75)
os.chdir(ROOT)
os.execv(sys.executable, [sys.executable, str(ROOT / '.cache/main-consolidation/guard.py'),
    'a1-initial-census-vm-1', sys.executable, str(STATE / 'run_384.py'),
    '--image', str(ROOT / '.cache/a1-namespace-review/debian-12-genericcloud-amd64.qcow2'),
    '--checksums', str(ROOT / '.cache/a1-namespace-review/SHA512SUMS'),
    '--repository', str(PACKAGES / 'repository'),
    '--guest-script', str(STATE / 'guest-1/installed_guest.py'), '--storage-disks',
    '--output', str(STATE / 'result-1.json')])

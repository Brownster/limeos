#!/usr/bin/env python3
"""Offline Debian build of the five binaries from this worktree's HEAD (docker, no network)."""
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
IMAGE = 'sha256:af684f63adae359e95f9082cc3235bcc76208baaa9cd1a48b553784e226d5ba5'
OUT = ROOT / '.cache/q/build'
PRODUCT = ['crates', 'bins', 'Cargo.toml', 'Cargo.lock', 'packaging', 'frontend/src', 'contracts']
# Product inputs must equal HEAD; test-only files may differ.
subprocess.run(['git', 'diff', '--quiet', 'HEAD', '--', *PRODUCT], cwd=ROOT, check=True)
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
avail = next(int(line.split()[1]) * 1024 for line in open('/proc/meminfo') if line.startswith('MemAvailable'))
assert avail >= 4 * 1024**3, avail
shutil.rmtree(OUT, ignore_errors=True)
(OUT / 'binaries').mkdir(parents=True)
inner = (
    'cargo build --offline --locked --release -p limeos-core -p limeos-executor -p limeosctl '
    '&& for n in limeos-core limeos-password-worker limeos-executor limeos-stack-read-worker limeosctl; do '
    'cp /build/.cache/a1-namespace-review/debian-target/release/$n /build/.cache/q/build/binaries/$n; done'
)
cmd = [
    'docker', 'run', '--rm', '--name', 'limeos-q-build', '--network=none', '--read-only',
    '--memory=3g', '--memory-swap=3g', '--pids-limit=128', '--cpus=4', '--cap-drop=ALL',
    '--security-opt=no-new-privileges', '--security-opt=label=disable', '--user', '1000:1000',
    '--volume', f'{ROOT}:/build:ro',
    '--volume', f'{ROOT}/.cache/p04-a1-integration:/build/.cache/p04-a1-integration',
    '--volume', f'{ROOT}/.cache/a1-namespace-review/debian-target:/build/.cache/a1-namespace-review/debian-target',
    '--volume', f'{ROOT}/.cache/q:/build/.cache/q', '--workdir', '/build',
    '--env', 'CARGO_HOME=/build/.cache/p04-a1-integration/cargo-home',
    '--env', 'CARGO_TARGET_DIR=/build/.cache/a1-namespace-review/debian-target',
    '--env', 'CARGO_BUILD_JOBS=4', '--env', 'CARGO_INCREMENTAL=0', '--env', 'TMPDIR=/build/.cache/q/tmp',
    IMAGE, 'sh', '-c', inner,
]
with (OUT / 'output.log').open('wb') as log:
    rc = subprocess.run(cmd, stdout=log, stderr=subprocess.STDOUT).returncode
rec = {'source': head, 'image': IMAGE, 'exit_code': rc}
if rc == 0:
    rec['binaries'] = {
        p.name: {'sha256': hashlib.sha256(p.read_bytes()).hexdigest(), 'bytes': p.stat().st_size}
        for p in sorted((OUT / 'binaries').iterdir())
    }
(OUT / 'record.json').write_text(json.dumps(rec, indent=2) + '\n')
print(json.dumps(rec))
sys.exit(rc)

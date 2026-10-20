# /// script
# dependencies = []
# ///
"""Run the remaining fixed local validation steps sequentially under the guard."""

import json
import shutil
import subprocess
import time
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
UV = ['uv', '--cache-dir', str(ROOT / '.cache/uv'), 'run', '--offline', '--no-project']
steps = [
    ('workspace-rest', ['cargo', 'test', '--offline', '--workspace', '--exclude', 'limeos-executor-storage', '--locked', '--no-fail-fast']),
    ('storage-compile', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor-storage', '--no-run', '--message-format=json']),
    ('storage-runtime', UV + [str(STATE / 'storage_runtime.py')]),
    ('main-clippy', ['cargo', 'clippy', '--offline', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings']),
    ('main-contracts', UV + ['scripts/check_contracts.py']),
    ('prepare-stream-b', UV + ['tests/fixtures/stream-b/check-leaves.py']),
    ('stream-b-tests', ['cargo', 'test', '--offline', '--manifest-path', '.cache/stream-b-leaf/Cargo.toml']),
    ('stream-b-clippy', ['cargo', 'clippy', '--offline', '--locked', '--manifest-path', '.cache/stream-b-leaf/Cargo.toml', '--all-targets', '--', '-D', 'warnings']),
]
for name, command in steps:
    if name == 'stream-b-tests':
        shutil.copyfile(ROOT / 'Cargo.lock', ROOT / '.cache/stream-b-leaf/Cargo.lock')
    with (STATE / f'{name}.capture.log').open('wb') as log:
        result = subprocess.run(UV + [str(STATE / 'guard.py'), name, *command], cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, check=False)
    print(json.dumps({'step': name, 'exit_code': result.returncode}), flush=True)
    if result.returncode:
        print((STATE / f'{name}.capture.log').read_text()[-6000:], flush=True)
        raise SystemExit(result.returncode)
print('All fixed local validation steps passed', flush=True)

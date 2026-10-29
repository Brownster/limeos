# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run the final source-bound runtime correction checks one at a time."""
from pathlib import Path
import subprocess
import sys

ROOT = Path('/home/marc/Documents/github/lime-os')
GUARD = ROOT / '.cache/main-consolidation/guard.py'
checks = [
    ('a1-runtime-bootstrap-format-2', ['cargo', 'fmt', '--all', '--', '--check']),
    ('a1-runtime-bootstrap-executor-tests-2', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor']),
    ('a1-runtime-bootstrap-clippy-2', ['cargo', 'clippy', '--offline', '--locked', '-p', 'limeos-executor', '--all-targets', '--', '-D', 'warnings']),
    ('a1-runtime-bootstrap-repository-2', ['python3', 'scripts/check_repository.py']),
]
available = next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
assert available >= 4 * 1024**3, 'Compiler start margin is below 4 GiB'
for label, command in checks:
    with (ROOT / '.cache/main-consolidation' / (label + '-console.log')).open('wb') as stream:
        result = subprocess.run([sys.executable, str(GUARD), label, *command], cwd=ROOT,
                                stdout=stream, stderr=subprocess.STDOUT)
    print(label, result.returncode, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)

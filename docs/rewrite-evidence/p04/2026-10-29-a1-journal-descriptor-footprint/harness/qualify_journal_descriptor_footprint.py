# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Qualify the genuine journal descriptor regression on repository disk."""
from pathlib import Path
import subprocess
import sys

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
checks = [
    ('a1-journal-descriptor-format-1', ['cargo', 'fmt', '--all', '--', '--check']),
    ('a1-journal-descriptor-tests-1', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor-storage', '--lib', 'effects::tests::']),
    ('a1-journal-descriptor-clippy-1', ['cargo', 'clippy', '--offline', '--locked', '-p', 'limeos-executor-storage', '--all-targets', '--', '-D', 'warnings']),
    ('a1-journal-descriptor-repository-1', ['python3', 'scripts/check_repository.py']),
]
available = next(int(line.split()[1])*1024 for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))
assert available >= 4 * 1024**3
for label, command in checks:
    with (STATE / (label + '-console.log')).open('xb') as stream:
        result = subprocess.run([sys.executable, str(STATE / 'guard.py'), label, *command], cwd=ROOT,
                                stdout=stream, stderr=subprocess.STDOUT)
    print(label, result.returncode, flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)

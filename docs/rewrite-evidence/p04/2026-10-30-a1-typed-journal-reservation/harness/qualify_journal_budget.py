# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Qualify the typed live-journal capacity profile sequentially."""
from pathlib import Path
import subprocess
import sys

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
checks = [
    ('a1-journal-budget-format-1', ['cargo', 'fmt', '--all', '--', '--check']),
    ('a1-journal-budget-executor-tests-1', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor']),
    ('a1-journal-budget-journal-tests-1', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor-storage', '--lib', 'effects::tests::']),
    ('a1-journal-budget-storage-docs-1', ['cargo', 'test', '--offline', '--locked', '-p', 'limeos-executor-storage', '--doc']),
    ('a1-journal-budget-clippy-1', ['cargo', 'clippy', '--offline', '--locked', '-p', 'limeos-executor', '-p', 'limeos-executor-storage', '--all-targets', '--', '-D', 'warnings']),
    ('a1-journal-budget-reference-1', ['python3', '-m', 'unittest', 'tests.reference.test_storage_mount_profile']),
    ('a1-journal-budget-repository-1', ['python3', 'scripts/check_repository.py']),
]
available = next(int(v.split()[1])*1024 for v in Path('/proc/meminfo').read_text().splitlines() if v.startswith('MemAvailable:'))
assert available >= 4 * 1024**3
for label, command in checks:
    with (STATE / (label + '-console.log')).open('xb') as stream:
        status = subprocess.run([sys.executable, str(STATE / 'guard.py'), label, *command], cwd=ROOT,
                                stdout=stream, stderr=subprocess.STDOUT).returncode
    print(label, status, flush=True)
    if status:
        raise SystemExit(status)

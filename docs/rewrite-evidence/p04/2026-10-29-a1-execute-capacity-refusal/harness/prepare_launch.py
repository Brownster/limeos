# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Prepare one frozen runtime-fix candidate and launch its detached guarded VM."""
from pathlib import Path
import subprocess
import sys

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/stack-installed-qualification'
GUARD = ROOT / '.cache/main-consolidation/guard.py'
steps = [
    ('runtime-ui-8', [sys.executable, str(GUARD), 'a1-runtime-bootstrap-ui-build-1', 'npm', '--prefix', 'frontend', 'run', 'build']),
    ('runtime-packages-8', [sys.executable, str(GUARD), 'a1-runtime-bootstrap-packages-1', sys.executable, str(STATE / 'prepare_packages_8.py')]),
    ('runtime-guest-8', [sys.executable, str(STATE / 'prepare_runtime_bootstrap_guest.py')]),
    ('runtime-launch-8', [sys.executable, str(STATE / 'launch_runtime_bootstrap_vm.py')]),
]
assert not [v for v in subprocess.check_output(['ps', '-eo', 'comm'], text=True).splitlines()
            if v.strip() in ('cargo', 'rustc', 'rustdoc')]
for label, command in steps:
    with (STATE / (label + '-console.log')).open('xb') as stream:
        status = subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT).returncode
    print(label, status, flush=True)
    if status:
        raise SystemExit(status)
print('VM launched; acceptance requires the original guard terminal record and installed result', flush=True)

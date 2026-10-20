# /// script
# dependencies = []
# ///
"""Run the already compiled storage kernel fixtures with small private tmpfs data."""

import hashlib
import json
import os
import resource
import signal
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
compile_record = json.loads((STATE / 'attempts/storage-compile/record.json').read_text())
assert compile_record['exit_code'] == 0 and compile_record['inputs_unchanged']
artifacts = []
for line in (STATE / 'attempts/storage-compile/output.log').read_text().splitlines():
    try:
        row = json.loads(line)
    except json.JSONDecodeError:
        continue
    if row.get('reason') == 'compiler-artifact' and row.get('target', {}).get('name') == 'limeos_executor_storage' and row.get('profile', {}).get('test') and row.get('executable'):
        artifacts.append(Path(row['executable']))
assert len(artifacts) == 1, artifacts
executable = artifacts[0]
identity = hashlib.sha256(executable.read_bytes()).hexdigest()
parent = Path('/run/user') / str(os.getuid())
assert not parent.is_symlink() and parent.stat().st_uid == os.getuid()
assert subprocess.check_output(['stat', '-f', '-c', '%T', str(parent)], text=True).strip() == 'tmpfs'
peak = 0
limit = 16 * 1024 * 1024
with tempfile.TemporaryDirectory(prefix='limeos-storage-fixtures-', dir=parent) as directory:
    scratch = Path(directory)
    env = dict(os.environ, TMPDIR=directory)
    command = [str(executable), '--test-threads=1']
    with (STATE / 'storage-kernel-runtime.log').open('wb') as log:
        child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        resource.prlimit(child.pid, resource.RLIMIT_FSIZE, (limit, limit))
        try:
            while child.poll() is None:
                total = 0
                for path in scratch.rglob('*'):
                    try:
                        if path.is_file() and not path.is_symlink():
                            total += path.stat().st_size
                    except FileNotFoundError:
                        pass
                peak = max(peak, total)
                if total > limit:
                    raise RuntimeError('Owned runtime fixture exceeds 16 MiB sampling threshold')
                time.sleep(0.02)
            status = child.wait()
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
    assert identity == hashlib.sha256(executable.read_bytes()).hexdigest()
assert not scratch.exists()
record = {'executable': str(executable), 'sha256': identity, 'exit_code': status, 'tmpfs_fixture_sampled_peak_bytes': peak, 'sampling_threshold_bytes': limit, 'per_file_limit_bytes': limit, 'scratch_removed': True, 'compile_record': 'attempts/storage-compile/record.json', 'runtime_tmpdir': str(scratch), 'cargo_or_compilation_on_tmpfs': False}
(STATE / 'storage-kernel-runtime.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record), flush=True)
if status:
    print((STATE / 'storage-kernel-runtime.log').read_text()[-10000:], flush=True)
raise SystemExit(status)

# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture bounded local rewrite checks with disk-backed build/scratch paths."""
import datetime
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
WORK = ROOT / '.worktrees/completed-slices'
LOGS = ROOT / '.cache/rewrite-integrated/logs'
COMMANDS = {
    'fmt': ['cargo', 'fmt', '--all'],
    'fmt-check': ['cargo', 'fmt', '--all', '--', '--check'],
    'test-rest': ['cargo', 'test', '--workspace', '--exclude', 'limeos-executor-storage', '--locked', '--offline', '--no-fail-fast'],
    'storage-fixtures': ['cargo', 'test', '-p', 'limeos-executor-storage', '--locked', '--offline', '--no-fail-fast'],
    'clippy': ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--offline', '--', '-D', 'warnings'],
    'contracts': [sys.executable, 'scripts/check_contracts.py'],
    'repository': [sys.executable, 'scripts/check_repository.py'],
    'arm64-harness': [sys.executable, '-m', 'unittest', 'discover', '-s', 'tests/qualification/arm64', '-p', 'test_harness.py'],
    'fd-isolated': [str(ROOT / '.cache/rewrite-pool-dependencies/target/debug/deps/limeos_backup_archive-f73ff2fe07529e78'), '--exact', 'before_images::tests::maximum_object_fixture_reports_actual_descriptor_and_record_cost', '--nocapture', '--test-threads=1'],
    'backup-parallel': [str(ROOT / '.cache/rewrite-pool-dependencies/target/debug/deps/limeos_backup_archive-f73ff2fe07529e78'), '--nocapture', '--test-threads=8'],
}
name = sys.argv[1]
label = sys.argv[2] if len(sys.argv) > 2 else name
command = COMMANDS[name]
environment = dict(os.environ, TMPDIR=str(ROOT / '.cache/tmp'), UV_CACHE_DIR=str(ROOT / '.cache/uv'), CCACHE_DIR=str(ROOT / '.cache/ccache'), CARGO_TARGET_DIR=str(ROOT / '.cache/rewrite-pool-dependencies/target'), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='2')
if name == 'contracts':
    environment['CARGO_NET_OFFLINE'] = 'true'
log = LOGS / (label + '.txt')
started = time.monotonic()
started_utc = datetime.datetime.now(datetime.timezone.utc).isoformat()
scratch = tempfile.TemporaryDirectory(prefix='limeos-storage-fixtures-', dir='/tmp') if name == 'storage-fixtures' else None
if scratch:
    environment['TMPDIR'] = scratch.name
peak = {'bytes': 0, 'files': 0}
stop = threading.Event()
def sample():
    while not stop.wait(0.05):
        total, count = 0, 0
        for path in Path(scratch.name).rglob('*'):
            try:
                if path.is_file() and not path.is_symlink():
                    total += path.stat().st_size
                    count += 1
            except FileNotFoundError:
                pass
        peak['bytes'] = max(peak['bytes'], total)
        peak['files'] = max(peak['files'], count)
sampler = threading.Thread(target=sample, daemon=True) if scratch else None
if sampler:
    sampler.start()
with log.open('x') as output:
    output.write('$ ' + ' '.join(command) + '\n')
    output.flush()
    result = subprocess.run(command, cwd=WORK, env=environment, stdout=output, stderr=subprocess.STDOUT)
stop.set()
if sampler:
    sampler.join()
    scratch.cleanup()
metadata = {'command': command, 'cwd': str(WORK), 'exit_code': result.returncode,
            'started_utc': started_utc, 'seconds': round(time.monotonic() - started, 3),
            'environment_overrides': {key: environment[key] for key in ['TMPDIR','UV_CACHE_DIR','CCACHE_DIR','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','CARGO_INCREMENTAL','RUST_TEST_THREADS']},
            'tmpfs_fixture_observed_peak': peak if scratch else None,
            'tmpfs_fixture_removed': bool(scratch)}
if name == 'contracts':
    metadata['environment_overrides']['CARGO_NET_OFFLINE'] = 'true'
with (LOGS / (label + '.json')).open('x') as output:
    json.dump(metadata, output, indent=2)
    output.write('\n')
print(f'{label}: exit={result.returncode}, seconds={metadata["seconds"]}, log={log}', flush=True)
sys.exit(result.returncode)

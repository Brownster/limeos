# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Capture current merged-source checks without host access or general tmpfs work."""
import datetime
import gzip
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
import threading
import time

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/container-destinations'
logs = root / '.cache/rewrite-container-destinations/logs'
logs.mkdir(parents=True, exist_ok=True)
commands = {
    'destinations-focused': ['cargo', 'test', '-p', 'limeos-executor-storage', 'destinations::tests', '--locked', '--offline'],
    'precommit-focused': ['cargo', 'test', '-p', 'limeos-executor-storage', 'destinations::tests', '--locked', '--offline'],
    'test-rest': ['cargo', 'test', '--workspace', '--exclude', 'limeos-executor-storage', '--locked', '--offline', '--no-fail-fast'],
    'storage-compile-on-disk': ['cargo', 'test', '-p', 'limeos-executor-storage', '--locked', '--offline', '--no-run'],
    'storage-fixtures': ['cargo', 'test', '-p', 'limeos-executor-storage', '--locked', '--offline', '--no-fail-fast'],
    'clippy': ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--offline', '--', '-D', 'warnings'],
    'fmt': ['cargo', 'fmt', '--all', '--', '--check'],
    'contracts': [sys.executable, 'scripts/check_contracts.py'],
    'repository': [sys.executable, 'scripts/check_repository.py'],
    'arm64-harness': [sys.executable, '-m', 'unittest', 'discover', '-s', 'tests/qualification/arm64', '-p', 'test_*.py'],
}
name = sys.argv[1]
command = commands[name]
target = logs / (name + '.log.gz')
metadata_path = logs / (name + '.json')
assert not target.exists() and not metadata_path.exists(), 'use a fresh check output'
environment = dict(os.environ, TMPDIR=str(root / '.cache/tmp'), UV_CACHE_DIR=str(root / '.cache/uv'), CCACHE_DIR=str(root / '.cache/ccache'), CARGO_TARGET_DIR=str(root / '.cache/rewrite-pool-dependencies/target'), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='2', CARGO_NET_OFFLINE='true')
scratch = None
if name == 'storage-fixtures':
    compiled = json.loads((logs / 'storage-compile-on-disk.json').read_text())
    assert compiled['exit'] == 0, 'compile all storage fixtures on disk first'
    scratch = tempfile.TemporaryDirectory(prefix='limeos-container-destinations-tests-', dir='/tmp')
    environment['TMPDIR'] = scratch.name
peak = {'bytes': 0, 'files': 0}
done = threading.Event()
def sample():
    while not done.wait(0.05):
        total, count = 0, 0
        for path in pathlib.Path(scratch.name).rglob('*'):
            try:
                if path.is_file() and not path.is_symlink():
                    total += path.stat().st_size
                    count += 1
            except FileNotFoundError:
                pass
        peak['bytes'] = max(peak['bytes'], total)
        peak['files'] = max(peak['files'], count)
sampler = threading.Thread(target=sample, daemon=True) if scratch else None
started = time.monotonic()
started_utc = datetime.datetime.now(datetime.UTC).isoformat()
if sampler:
    sampler.start()
try:
    result = subprocess.run(command, cwd=work, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
finally:
    done.set()
    if sampler:
        sampler.join()
        scratch.cleanup()
target.write_bytes(gzip.compress(result.stdout, mtime=0))
metadata = {'name': name, 'command': command, 'cwd': str(work), 'qualified_source': subprocess.check_output(['git','rev-parse','HEAD'], cwd=work, text=True).strip(), 'started_utc': started_utc, 'seconds': round(time.monotonic()-started,3), 'exit': result.returncode, 'environment': {key: environment[key] for key in ('TMPDIR','UV_CACHE_DIR','CCACHE_DIR','CARGO_TARGET_DIR','CARGO_BUILD_JOBS','CARGO_INCREMENTAL','RUST_TEST_THREADS','CARGO_NET_OFFLINE')}, 'log': target.name, 'raw_sha256': hashlib.sha256(result.stdout).hexdigest(), 'compressed_sha256': hashlib.sha256(target.read_bytes()).hexdigest(), 'tmpfs_fixture_peak': peak if scratch else None, 'tmpfs_fixture_removed': bool(scratch), 'rust_cases': sum(int(n) for n in re.findall(rb'test result: ok\. (\d+) passed',result.stdout)), 'pi_accessed': False}
metadata['worktree_status'] = subprocess.check_output(['git', 'status', '--porcelain'], cwd=work, text=True)
metadata['runtime_hashes'] = {name: hashlib.sha256((work / name).read_bytes()).hexdigest() for name in ('crates/executor-storage/src/lib.rs', 'crates/executor-storage/src/dependencies.rs', 'crates/executor-storage/src/processes.rs', 'crates/executor-storage/src/sources.rs', 'crates/executor-storage/src/destinations.rs', 'crates/executor-storage/src/destinations/tests.rs')}
metadata_path.write_text(json.dumps(metadata,indent=2)+'\n')
print(json.dumps({'check':name,'exit':result.returncode,'seconds':metadata['seconds'],'rust_cases':metadata['rust_cases'],'tmpfs_fixture_peak':metadata['tmpfs_fixture_peak']}), flush=True)
if result.returncode:
    print(result.stdout.decode(errors='replace')[-14000:])
raise SystemExit(result.returncode)

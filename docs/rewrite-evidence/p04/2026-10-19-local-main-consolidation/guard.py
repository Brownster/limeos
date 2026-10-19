#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Run one frozen focused command with disk logs and a child-only memory guard."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

ROOT = Path('/home/marc/Documents/github/lime-os')
WORK = ROOT / '.worktrees/local-main-consolidation'
STATE = ROOT / '.cache/main-consolidation'
FLOOR = 3 * 1024**3

def available():
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'):
            return int(line.split()[1]) * 1024
    raise RuntimeError('MemAvailable missing')

def snapshot():
    paths = subprocess.check_output(['git', 'ls-files', '-co', '--exclude-standard', '-z'], cwd=WORK).decode().split('\0')
    prefixes = ('Cargo.', 'rust-toolchain', '.cargo/', 'crates/', 'bins/', 'contracts/generated/', 'frontend/', 'tests/', 'packaging/', 'docs/p04-storage-mounts-profile.md')
    result = {}
    for name in sorted(set(paths)):
        if name.startswith(prefixes) and (WORK / name).is_file():
            digest = hashlib.sha256()
            with (WORK / name).open('rb') as source:
                for block in iter(lambda: source.read(1024 * 1024), b''):
                    digest.update(block)
            result[name] = digest.hexdigest()
    return result

def main():
    name, *command = sys.argv[1:]
    directory = STATE / 'attempts' / name
    directory.mkdir(exist_ok=False)
    processes = subprocess.check_output(['ps', '-eo', 'pid,ppid,comm'], text=True)
    busy = [line for line in processes.splitlines() if line.split()[-1] in ('cargo', 'rustc', 'rustdoc')]
    if busy and Path(command[0]).name in ('cargo', 'rustc', 'rustdoc'):
        (directory / 'refused.txt').write_text('\n'.join(busy))
        identities = []
        inspected = set()
        for line in busy:
            pid, parent, comm = line.split()
            for selected in (pid, parent):
                if selected in inspected:
                    continue
                inspected.add(selected)
                item = {'pid': int(selected), 'observed_child': int(pid), 'observed_parent': int(parent), 'observed_child_comm': comm}
                proc = Path('/proc') / selected
                for name in ('cwd', 'exe'):
                    try:
                        item[name] = str((proc / name).readlink())
                    except OSError as error:
                        item[name + '_unavailable'] = str(error)
                identities.append(item)
        (directory / 'refused.json').write_text(json.dumps(identities, indent=2) + '\n')
        raise RuntimeError('Another Cargo/compiler process is active')
    memory = available()
    if memory < FLOOR:
        raise RuntimeError('Available memory is already below 3 GiB')
    env = os.environ.copy()
    env.update(CARGO_HOME=os.environ.get('CARGO_HOME', '/home/marc/.cargo'), CARGO_TARGET_DIR=str(ROOT / '.cache/rewrite-pool-dependencies/target'), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', RUST_TEST_THREADS='1', TMPDIR=str(STATE / 'tmp'), UV_CACHE_DIR=str(ROOT / '.cache/uv'), UV_PYTHON_DOWNLOADS='never', NPM_CONFIG_CACHE=str(STATE / 'npm-cache'))
    record = {'command': command, 'cwd': str(WORK), 'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=WORK, text=True).strip(), 'guard_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), 'started': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'environment': {k: env[k] for k in ('CARGO_HOME', 'CARGO_TARGET_DIR', 'CARGO_BUILD_JOBS', 'CARGO_INCREMENTAL', 'RUST_TEST_THREADS', 'TMPDIR', 'UV_CACHE_DIR', 'NPM_CONFIG_CACHE')}, 'before': snapshot(), 'minimum_available_bytes': memory, 'memory_floor_bytes': FLOOR, 'guard_triggered': False, 'observed_external_compilers': busy, 'non_cargo_concurrency_authorized': bool(busy)}
    metadata = directory / 'record.json'
    metadata.write_text(json.dumps(record, indent=2) + '\n')
    with (directory / 'output.log').open('wb') as output:
        child = subprocess.Popen(command, cwd=WORK, env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        record['pid'] = child.pid
        metadata.write_text(json.dumps(record, indent=2) + '\n')
        try:
            while child.poll() is None:
                memory = available()
                record['minimum_available_bytes'] = min(record['minimum_available_bytes'], memory)
                if memory < FLOOR:
                    record['guard_triggered'] = True
                    os.killpg(child.pid, signal.SIGTERM)
                    time.sleep(2)
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    break
                time.sleep(0.1)
            record['exit_code'] = child.wait()
        except BaseException:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            record['exit_code'] = child.wait()
            raise
        finally:
            record['after'] = snapshot()
            record['inputs_unchanged'] = record['before'] == record['after']
            record['ended'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
            metadata.write_text(json.dumps(record, indent=2) + '\n')
    print(json.dumps({k: record[k] for k in ('command', 'exit_code', 'minimum_available_bytes', 'guard_triggered', 'inputs_unchanged')}), flush=True)
    print((directory / 'output.log').read_text()[-24000:], flush=True)
    return 1 if record['guard_triggered'] or not record['inputs_unchanged'] else record['exit_code']

if __name__ == '__main__':
    sys.exit(main())

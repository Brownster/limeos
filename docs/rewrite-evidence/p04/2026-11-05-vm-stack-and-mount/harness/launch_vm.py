#!/usr/bin/env python3
"""Run exactly one guarded disposable VM through tests/privileged_vm/run.py.

usage: launch_vm.py LABEL GUEST_SCRIPT [--storage-disks]
Admission: MemAvailable >= 8 GiB, no other qemu process, no cargo/rustc. The child's process group is
stopped if MemAvailable < 3 GiB or after DEADLINE seconds. Results land in .cache/q/runs/LABEL/.
"""
import datetime
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
SHARED = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/q'
ADMIT, FLOOR, DEADLINE = 8 * 1024**3, 3 * 1024**3, 45 * 60


def available():
    for line in Path('/proc/meminfo').read_text().splitlines():
        if line.startswith('MemAvailable:'):
            return int(line.split()[1]) * 1024
    raise RuntimeError('MemAvailable missing')


def qemus():
    out = subprocess.run(['ps', '-eo', 'pid,args'], text=True, capture_output=True).stdout
    return [line for line in out.splitlines() if 'qemu-system' in line and 'grep' not in line]


label, guest, *flags = sys.argv[1:]
run_dir = STATE / 'runs' / label
scratch = STATE / 'tmp-vm' / label
scratch.mkdir(parents=True)
assert not qemus(), qemus()
run_dir.mkdir(parents=True, exist_ok=False)
compilers = [r for r in subprocess.check_output(['ps', '-eo', 'comm'], text=True).splitlines() if r.strip() in ('cargo', 'rustc')]
start_mem = available()
record = {'label': label, 'guest_script': guest, 'started': datetime.datetime.now(datetime.timezone.utc).isoformat(),
          'available_at_admission': start_mem, 'admit_threshold': ADMIT, 'floor': FLOOR, 'minimum_available': start_mem,
          'guard_triggered': False, 'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
          'flags': flags, 'foreign_compiler_processes_at_admission': len(compilers)}
if start_mem < ADMIT:
    record['admitted'] = False
    (run_dir / 'record.json').write_text(json.dumps(record, indent=2) + '\n')
    raise SystemExit('not admitted: MemAvailable below 8 GiB')
record['admitted'] = True
command = [sys.executable, str(ROOT / 'tests/privileged_vm/run.py'),
           '--image', str(SHARED / '.cache/p01-vm/debian-12-genericcloud-amd64.qcow2'),
           '--checksums', str(SHARED / '.cache/p01-vm/SHA512SUMS'),
           '--repository', str(STATE / 'candidate/repository'),
           '--guest-script', str(ROOT / guest), '--output', str(run_dir / 'result.json'), *flags]
env = dict(os.environ, TMPDIR=str(scratch))
record['command'] = command
with (run_dir / 'output.log').open('wb') as log:
    child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.monotonic() + DEADLINE
    try:
        while child.poll() is None:
            mem = available()
            record['minimum_available'] = min(record['minimum_available'], mem)
            if mem < FLOOR or time.monotonic() > deadline:
                record['guard_triggered'] = 'memory' if mem < FLOOR else 'deadline'
                os.killpg(child.pid, signal.SIGTERM)
                time.sleep(3)
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                break
            time.sleep(0.2)
    except BaseException:
        os.killpg(child.pid, signal.SIGTERM)
        raise
    finally:
        record['exit_code'] = child.wait()
        time.sleep(1)
        record['ended'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        shutil.rmtree(scratch, ignore_errors=True)
        record['qemu_left'] = qemus()
        record['scratch_removed'] = not scratch.exists()
        (run_dir / 'record.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps({k: record[k] for k in ('label', 'exit_code', 'guard_triggered', 'minimum_available', 'qemu_left', 'scratch_removed')}))
sys.exit(0 if record['exit_code'] == 0 and not record['guard_triggered'] else 1)

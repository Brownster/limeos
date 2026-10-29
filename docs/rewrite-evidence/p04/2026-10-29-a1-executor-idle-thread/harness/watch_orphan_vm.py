# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Restore bounded observation of the exact orphaned diagnostic runner group."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import select
import signal
import time

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/a1-initial-census-diagnostic'
original_path = ROOT / '.cache/main-consolidation/attempts/a1-initial-census-vm-1/record.json'
original = json.loads(original_path.read_text())
OUTPUT = STATE / 'supervision-recovery-1.json'
assert not OUTPUT.exists()
pid = original['pid']
now = lambda: datetime.datetime.now(datetime.timezone.utc).isoformat()
record = {'started': now(), 'original_runner_pid': pid,
          'original_record_sha256': hashlib.sha256(original_path.read_bytes()).hexdigest(),
          'watcher_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
          'memory_floor_bytes': 3 * 1024**3, 'guard_gap_invalidates_continuous_guard_acceptance': True,
          'stop_reason': None, 'samples': 0}

def save():
    OUTPUT.write_text(json.dumps(record, indent=2) + '\n')

def available():
    return next(int(v.split()[1])*1024 for v in Path('/proc/meminfo').read_text().splitlines()
                if v.startswith('MemAvailable:'))

def identity():
    proc = Path('/proc') / str(pid)
    text = (proc / 'stat').read_text()
    fields = text[text.rindex(')')+1:].split()
    args = (proc / 'cmdline').read_bytes().split(b'\0')
    expected = str(STATE / 'run_384.py').encode()
    assert expected in args, 'Runner command identity differs'
    assert os.getpgid(pid) == pid, 'Runner no longer leads the original process group'
    assert Path(os.readlink(proc / 'cwd')).resolve() == ROOT
    assert b'--storage-disks' in args
    return int(fields[19])

try:
    handle = os.pidfd_open(pid)
except ProcessLookupError:
    record.update(ended=now(), runner_absent_at_attachment=True)
    save()
    print(json.dumps(record), flush=True)
    raise SystemExit(0)
start = identity()
record.update(runner_start_ticks=start, runner_absent_at_attachment=False,
              minimum_available_bytes=available())
poller = select.poll()
poller.register(handle, select.POLLIN)
save()
deadline = time.monotonic() + 1200
try:
    while not poller.poll(250):
        memory = available()
        record['samples'] += 1
        record['minimum_available_bytes'] = min(memory, record['minimum_available_bytes'])
        if memory < record['memory_floor_bytes'] or time.monotonic() >= deadline:
            record['stop_reason'] = 'memory floor' if memory < record['memory_floor_bytes'] else 'watcher deadline'
            if not poller.poll(0):
                assert identity() == start
                os.killpg(pid, signal.SIGTERM)
                # Give the runner its existing VM cleanup interval first.
                if not poller.poll(15000):
                    assert identity() == start
                    os.killpg(pid, signal.SIGKILL)
                    poller.poll(5000)
            break
        if record['samples'] % 20 == 0:
            save()
    record['runner_pidfd_terminal'] = bool(poller.poll(0))
finally:
    os.close(handle)
    record['ended'] = now()
    record['after_inputs_unchanged'] = all(
        hashlib.sha256((ROOT / name).read_bytes()).hexdigest() == expected
        for name, expected in original['before'].items())
    save()
print(json.dumps(record), flush=True)

"""Part A wrapper: the unchanged approved positive mount flow with real Docker, plus passive observation.

Staged next to a verbatim copy of tests/privileged_vm/*.py. During the flow it only holds ONE already-open
kernel trace_pipe descriptor (scheduler fork/exec/exit events), so it opens and closes no descriptors in the
scanned process (an earlier variant that sampled /proc from this process did, and itself caused
fds.link_failed refusals; see README). After the run it prints timestamped journal lines, the process
exits/forks that immediately precede each refusal, and resolves pids named by refusal records.
"""
import collections
import datetime
import json
import os
import re
import subprocess
import threading
import time
from pathlib import Path

import p04_approved_guest as approved
import p04_mounts_guest as mounts

VERSION = '0.4.15+stacklc.1'


TRACE = collections.deque(maxlen=60000)
TRACE_STATE = {'enabled': False}


def start_trace():
    """Enable sched fork/exec/exit tracing once; hold a single trace_pipe fd for the whole run."""
    base = None
    for candidate in ('/sys/kernel/tracing', '/sys/kernel/debug/tracing'):
        if os.path.exists(candidate + '/trace_pipe'):
            base = candidate
            break
    if base is None:
        subprocess.run(['mount', '-t', 'tracefs', 'nodev', '/sys/kernel/tracing'], capture_output=True)
        if os.path.exists('/sys/kernel/tracing/trace_pipe'):
            base = '/sys/kernel/tracing'
    if base is None:
        TRACE_STATE['error'] = 'no tracefs'
        return
    try:
        Path(base + '/tracing_on').write_text('1')
        for event in ('sched_process_fork', 'sched_process_exec', 'sched_process_exit'):
            Path(base + '/events/sched/%s/enable' % event).write_text('1')
        pipe = open(base + '/trace_pipe', 'rb', buffering=0)
    except OSError as error:
        TRACE_STATE['error'] = str(error)
        return
    TRACE_STATE.update(enabled=True, base=base)

    def reader():
        buffer = b''
        while True:
            chunk = pipe.read(65536)
            if not chunk:
                return
            now = time.time()
            buffer += chunk
            *lines, buffer = buffer.split(b'\n')
            for line in lines:
                TRACE.append((now, line.decode(errors='replace')))

    threading.Thread(target=reader, daemon=True).start()


def stamp(line):
    match = re.match(r'(\w{3} \d+ \d\d:\d\d:\d\d\.\d+)', line)
    if not match:
        return None
    parsed = datetime.datetime.strptime('2026 ' + match.group(1), '%Y %b %d %H:%M:%S.%f')
    return parsed.replace(tzinfo=datetime.timezone.utc).timestamp()


def trace_before(refusals, seconds=3.0):
    """Print scheduler events (comm-pid, event) shortly before each distinct refusal time."""
    seen = set()
    for when in refusals:
        if round(when, 2) in seen:
            continue
        seen.add(round(when, 2))
        window = [(t, line) for t, line in TRACE if when - seconds <= t <= when + 0.05]
        counts = collections.Counter()
        for _, line in window:
            match = re.match(r'\s*(\S+?)-(\d+)\s+\[\d+\]\s+\S+\s+[\d.]+:\s+(sched_\w+)', line)
            if match:
                counts[(match.group(1), match.group(3))] += 1
        print('TRACE_WINDOW ' + json.dumps({'refusal_epoch': round(when, 3), 'events': len(window),
              'by_comm_event': {'%s|%s' % k: v for k, v in counts.most_common(25)}}), flush=True)
        quiet = [(t, line) for t, line in window if 'blkid' not in line]
        for t, line in quiet[-120:]:
            print('TRACE_EVENT %.3f %s' % (t, line[:200]), flush=True)


def journal(unit, lines):
    return subprocess.run(['journalctl', '-u', unit, '--no-pager', '-n', str(lines), '-o', 'short-precise'],
                          text=True, capture_output=True, timeout=15).stdout


def report(tag):
    refusals = []
    for unit, words in ((mounts.UNIT, ('storage_mount', 'scan.', 'consumer', 'refus')), ('limeos-core', ('storage_mount', 'storage target', 'reconcil'))):
        kept = 0
        for line in journal(unit, 600).splitlines():
            if len(line.encode()) < 4096 and any(word in line for word in words):
                print('MOUNT_JOURNAL ' + tag + ' ' + unit + ' ' + line, flush=True)
                if 'refus' in line and stamp(line):
                    refusals.append(stamp(line))
                kept += 1
                if kept >= 80:
                    break
    print('TRACE_STATE ' + json.dumps(dict(TRACE_STATE, buffered=len(TRACE))), flush=True)
    trace_before(sorted(set(refusals)))
    # Identify processes named by refusal lines (still alive or not): read-only /proc lookups.
    pids = set()
    for unit in (mounts.UNIT,):
        for line in journal(unit, 600).splitlines():
            if 'storage_mount' in line:
                pids.update(int(n) for n in re.findall(r'"pid":(\d+)', line))
    for pid in sorted(pids):
        info = {'pid': pid}
        try:
            info['comm'] = Path('/proc/%d/comm' % pid).read_text().strip()
            info['cmdline'] = Path('/proc/%d/cmdline' % pid).read_text().replace('\0', ' ')[:200]
            info['exe'] = os.readlink('/proc/%d/exe' % pid)
            info['threads'] = len(os.listdir('/proc/%d/task' % pid))
            fds = {}
            for fd in os.listdir('/proc/%d/fd' % pid):
                try:
                    fds[fd] = os.readlink('/proc/%d/fd/%s' % (pid, fd))
                except OSError:
                    fds[fd] = None
            info['fds'] = fds
        except OSError as error:
            info['gone'] = str(error)
        print('REFUSAL_PID ' + tag + ' ' + json.dumps(info), flush=True)
    meminfo = {l.split(':')[0]: int(l.split()[1]) for l in Path('/proc/meminfo').read_text().splitlines()}
    print('GUEST_MEM ' + tag + ' ' + json.dumps({k: meminfo[k] for k in ('MemTotal', 'MemAvailable')}), flush=True)


start_trace()
try:
    approved.main(package_version=VERSION, qualify_upgrade=False)
except BaseException:
    report('failure')
    raise
else:
    report('success')

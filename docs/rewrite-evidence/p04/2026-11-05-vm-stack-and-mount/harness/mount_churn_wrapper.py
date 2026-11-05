"""Part A wrapper: the unchanged approved positive mount flow with real Docker, plus journal capture.

Staged next to a verbatim copy of tests/privileged_vm/*.py. It adds only read-only observations
(journal lines and Docker daemon thread counts) around p04_approved_guest.main.
"""
import json
import subprocess
import sys

import p04_approved_guest as approved
import p04_mounts_guest as mounts

VERSION = '0.4.15+stacklc.1'


def journal(unit, lines=400):
    result = subprocess.run(['journalctl', '-u', unit, '--no-pager', '-n', str(lines), '-o', 'cat'],
                            text=True, capture_output=True, timeout=10)
    return result.stdout


def report(tag):
    kept = 0
    for line in journal(mounts.UNIT).splitlines():
        if len(line.encode()) >= 4096:
            continue
        if any(word in line for word in ('storage_mount', 'scan.', 'consumer', 'census', 'refus')):
            print('MOUNT_JOURNAL ' + tag + ' ' + line, flush=True)
            kept += 1
            if kept >= 60:
                break
    print('MOUNT_JOURNAL_LINES ' + tag + ' ' + str(kept), flush=True)
    for unit in ('limeos-a1-real-docker',):
        out = subprocess.run(['systemctl', 'show', unit, '-p', 'MainPID', '-p', 'NRestarts', '-p', 'ActiveState'],
                             text=True, capture_output=True).stdout
        pid = next((l.split('=')[1] for l in out.splitlines() if l.startswith('MainPID=')), '0')
        try:
            threads = len(list(__import__('pathlib').Path('/proc/%s/task' % pid).iterdir()))
        except OSError:
            threads = None
        print('DOCKERD ' + tag + ' ' + json.dumps({'show': out.split(), 'threads': threads}), flush=True)


try:
    approved.main(package_version=VERSION, qualify_upgrade=False)
except BaseException:
    report('failure')
    raise
else:
    report('success')

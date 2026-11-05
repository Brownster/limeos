#!/usr/bin/env python3
"""Summarize Part A runs from their retained output logs: stage reached, HTTP/phase refusal strings."""
import json
import re
import sys
from pathlib import Path

runs = Path(sys.argv[1])
out = {}
for log in sorted(runs.glob('mount-*/output.log')):
    text = log.read_text()
    record = json.loads((log.parent / 'record.json').read_text())
    failure = re.search(r'^A1_FAILURE (.*)$', text, re.M)
    entry = {'exit_code': record['exit_code'], 'minimum_host_available_bytes': record['minimum_available'],
             'started': record['started'], 'ended': record['ended'], 'qemu_left': record['qemu_left'],
             'scratch_removed': record['scratch_removed']}
    if failure:
        data = json.loads(failure.group(1))
        entry['stage'] = data['stage']
        entry['error'] = data.get('error')
    refusals = []
    for line in text.splitlines():
        if line.startswith('MOUNT_JOURNAL'):
            match = re.search(r'"event":"(storage_mount_[a-z_]+)","phase":"([^"]+)"(?:,"failure":"(\w+)")?', line)
            if match:
                refusals.append({'event': match.group(1), 'phase': match.group(2), 'failure': match.group(3),
                                 'at': re.search(r'(Nov \d+ [\d:.]+)', line).group(1) if 'Nov' in line else None})
            if 'storage_mount_consumer_tasks_changed' in line:
                refusals[-1:] = [{**(refusals[-1] if refusals else {}), 'detail': json.loads(line[line.index('{'):])}]
    entry['refusal_journal'] = refusals
    out[log.parent.name] = entry
print(json.dumps(out, indent=2))

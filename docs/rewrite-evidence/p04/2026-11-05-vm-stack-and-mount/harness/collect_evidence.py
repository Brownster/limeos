#!/usr/bin/env python3
"""Copy retained raw outputs into the evidence directory and seal them with SHA256SUMS."""
import gzip
import io
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

def gz(source, target):
    with open(target, 'wb') as raw_out, gzip.GzipFile(filename='', mode='wb', fileobj=raw_out, mtime=0) as out, open(source, 'rb') as src:
        shutil.copyfileobj(src, out)


ROOT = Path(__file__).resolve().parents[5]
EVIDENCE = ROOT / 'docs/rewrite-evidence/p04/2026-11-05-vm-stack-and-mount'
RUNS = ROOT / '.cache/q/runs'
raw = EVIDENCE / 'raw'
raw.mkdir(exist_ok=True)
for run in sorted(RUNS.iterdir()):
    log = run / 'output.log'
    if not log.exists():
        continue
    gz(log, raw / f'{run.name}-output.log.gz')
    shutil.copyfile(run / 'record.json', raw / f'{run.name}-record.json')
    text = log.read_text()
    begin, end = text.find('STACK_RESULT_BEGIN'), text.find('STACK_RESULT_END')
    if begin != -1 and end != -1:
        (raw / f'{run.name}-result.json').write_text(text[begin + len('STACK_RESULT_BEGIN\n'):end])
    elif (run / 'result.json').exists():
        shutil.copyfile(run / 'result.json', raw / f'{run.name}-result.json')
summary = subprocess.check_output([sys.executable, str(EVIDENCE / 'harness/summarize_mount.py'), str(RUNS)], text=True)
(raw / 'mount-summary.json').write_text(summary)
build = ROOT / '.cache/q/build'
shutil.copyfile(build / 'record.json', raw / 'build-record.json')
gz(build / 'output.log', raw / 'build-output.log.gz')
shutil.copyfile(ROOT / '.cache/q/candidate/inputs.json', raw / 'package-inputs.json')
lines = []
for path in sorted(EVIDENCE.rglob('*')):
    if path.is_file() and path.name != 'SHA256SUMS':
        lines.append(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(EVIDENCE)}')
(EVIDENCE / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')
print(len(lines), 'files sealed')

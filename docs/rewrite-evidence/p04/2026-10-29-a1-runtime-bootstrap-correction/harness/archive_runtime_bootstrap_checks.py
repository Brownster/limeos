# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Preserve the native failing bootstrap and exact final-source checks."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-29-a1-runtime-bootstrap-correction'
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
OUT.mkdir(exist_ok=False)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def archive(name, path):
    target = OUT / (name + '.gz')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(path.read_bytes(), mtime=0))

source = ROOT / 'bins/executor/src/main.rs'
formatted = '''.map(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap()
            })'''
inline = '.map(|entry| entry.unwrap().file_name().to_str().unwrap().parse().unwrap())'
assert source.read_text().count(formatted) == 1
unformatted = source.read_text().replace(formatted, inline).encode()
red_path = STATE / 'a1-runtime-bootstrap-red.rs'
checks = {}
frozen = None
for suffix in ('format-2', 'executor-tests-2', 'clippy-2', 'repository-2', 'red-1', 'green-1', 'format-1'):
    label = 'a1-runtime-bootstrap-' + suffix
    path = STATE / 'attempts' / label
    record = json.loads((path / 'record.json').read_text())
    assert record['exit_code'] == {'red-1': 101, 'format-1': 1}.get(suffix, 0)
    assert record['inputs_unchanged'] and not record['guard_triggered']
    assert record['before'] == record['after']
    if frozen is None:
        frozen = record['before']
    expected = dict(frozen)
    if suffix == 'red-1':
        expected['bins/executor/src/main.rs'] = sha(red_path.read_bytes())
    elif suffix in ('green-1', 'format-1'):
        expected['bins/executor/src/main.rs'] = sha(unformatted)
    assert record['before'] == expected, label
    checks[label] = {k: record[k] for k in ('head', 'command', 'started', 'ended', 'exit_code',
                                          'minimum_available_bytes', 'guard_triggered', 'inputs_unchanged')}
    archive('checks/' + label + '-record.json', path / 'record.json')
    archive('checks/' + label + '-output.log', path / 'output.log')
for name, expected in frozen.items():
    assert sha((ROOT / name).read_bytes()) == expected, name
    committed = subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)
    assert sha(committed) == expected, name
assert 'bootstrap added an idle worker' in (STATE / 'attempts/a1-runtime-bootstrap-red-1/output.log').read_text()
archive('source-red.rs', red_path)
archive('source-final.rs', source)
(OUT / 'source-before-format.rs.gz').write_bytes(gzip.compress(unformatted, mtime=0))
for name in ('guard.py', 'qualify_runtime_bootstrap.py', 'archive_runtime_bootstrap_checks.py'):
    target = OUT / 'harness' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes((STATE / name).read_bytes())
summary = {'source': head, 'recorded_inputs_match_commit': len(frozen), 'checks': checks,
           'final_source_checks': ['format-2', 'executor-tests-2', 'clippy-2', 'repository-2'],
           'red_and_preformat_variants_joined_by_exact_source_hash': True,
           'installed_a1_accepted': False, 'blocking_pool_ceiling_unchanged': True,
           'host_task_comparisons_unchanged': True}
(OUT / 'source-match.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({'source': head, 'recorded_inputs_match_commit': len(frozen), 'checks': len(checks)}))

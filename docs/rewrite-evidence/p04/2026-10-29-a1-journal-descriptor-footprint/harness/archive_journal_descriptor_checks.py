# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Join the measured journal footprint and final regression to committed source."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-29-a1-journal-descriptor-footprint'
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
OUT.mkdir(exist_ok=False)

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def archive(name, path):
    target = OUT / (name + '.gz')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(path.read_bytes(), mtime=0))

variants = {'probe-1': STATE / 'a1-journal-descriptor-measurement.rs',
            'format-1': STATE / 'a1-journal-descriptor-format-failed.rs'}
checks, frozen = {}, None
for suffix in ('format-2', 'tests-2', 'clippy-2', 'repository-2', 'probe-1', 'format-1'):
    label = 'a1-journal-descriptor-' + suffix
    path = STATE / 'attempts' / label
    record = json.loads((path / 'record.json').read_text())
    assert record['exit_code'] == (1 if suffix == 'format-1' else 0)
    assert record['inputs_unchanged'] and not record['guard_triggered']
    assert record['before'] == record['after']
    if frozen is None:
        frozen = record['before']
    expected = dict(frozen)
    if suffix in variants:
        expected['crates/executor-storage/src/effects/tests.rs'] = sha(variants[suffix])
    assert record['before'] == expected, label
    checks[label] = {k:record[k] for k in ('head', 'command', 'started', 'ended', 'exit_code',
                                         'minimum_available_bytes', 'guard_triggered', 'inputs_unchanged')}
    for name in ('record.json', 'output.log'):
        archive('checks/' + label + '-' + name, path / name)
for name, expected in frozen.items():
    assert sha(ROOT / name) == expected, name
    content = subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)
    assert hashlib.sha256(content).hexdigest() == expected, name
log = (STATE / 'attempts/a1-journal-descriptor-probe-1/output.log').read_text()
text = log.partition('JOURNAL_DESCRIPTOR_FOOTPRINT ')[2]
measurement, _ = json.JSONDecoder().raw_decode(text)
assert measurement == {'baseline': 4, 'created': 9, 'after_write': 9,
                       'after_create_drop': 4, 'reopened': 9, 'after_reopen_drop': 4}
for suffix, path in variants.items():
    archive('source-' + suffix + '.rs', path)
archive('source-final.rs', ROOT / 'crates/executor-storage/src/effects/tests.rs')
for name in ('guard.py', 'qualify_journal_descriptor_footprint.py',
             'qualify_journal_descriptor_footprint_2.py', 'archive_journal_descriptor_checks.py'):
    path = OUT / 'harness' / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes((STATE / name).read_bytes())
executable = ROOT / '.cache/rewrite-pool-dependencies/target/debug/deps/limeos_executor_storage-1b92f2a31d62eb89'
summary = {'source': head, 'recorded_inputs_match_commit': len(frozen), 'checks': checks,
           'measurement': measurement, 'retained_descriptor_delta': 5,
           'final_source_checks': ['format-2', 'tests-2', 'clippy-2', 'repository-2'],
           'final_test_executable_sha256': sha(executable),
           'production_capacity_changed': False, 'installed_a1_accepted': False,
           'failed_installed_admission_count_proven': False}
(OUT / 'source-match.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({'source': head, 'recorded_inputs_match_commit': len(frozen), 'retained_descriptor_delta': 5}))

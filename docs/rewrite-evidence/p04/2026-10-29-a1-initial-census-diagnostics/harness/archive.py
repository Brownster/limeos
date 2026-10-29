# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind the diagnostic's positive and negative checks to committed source."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-29-a1-initial-census-diagnostics'
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
OUT.mkdir(exist_ok=False)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def archive(name, path):
    target = OUT / (name + '.gz')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(path.read_bytes(), mtime=0))

checks = {}
frozen = None
for suffix in ('clippy', 'focused', 'storage-all', 'compile', 'runtime', 'doctests',
               'executor-clippy', 'format-check', 'repository'):
    label = 'a1-initial-census-' + suffix + '-1'
    directory = STATE / 'attempts' / label
    record = json.loads((directory / 'record.json').read_text())
    assert record['exit_code'] == (101 if suffix == 'storage-all' else 0), label
    assert record['inputs_unchanged'] and not record['guard_triggered'], label
    assert record['before'] == record['after'], label
    if frozen is None:
        frozen = record['before']
    assert record['before'] == frozen, label
    checks[label] = {key: record[key] for key in
                     ('head', 'command', 'started', 'ended', 'exit_code',
                      'minimum_available_bytes', 'guard_triggered', 'inputs_unchanged')}
    archive('checks/' + label + '-record.json', directory / 'record.json')
    archive('checks/' + label + '-output.log', directory / 'output.log')

for name, expected in frozen.items():
    assert digest(ROOT / name) == expected, name
    committed = subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)
    assert hashlib.sha256(committed).hexdigest() == expected, name

runtime_path = STATE / 'a1-initial-census-kernel-runtime.json'
runtime = json.loads(runtime_path.read_text())
assert runtime['exit_code'] == 0 and runtime['scratch_removed']
assert not Path(runtime['runtime_tmpdir']).exists()
assert not runtime['cargo_or_compilation_on_tmpfs']
assert digest(Path(runtime['executable'])) == runtime['sha256']
runner = STATE / 'a1_initial_census_runtime.py'
assert digest(runner) == runtime['runner_sha256']
log = STATE / 'a1-initial-census-kernel-runtime.log'
assert '231 passed; 0 failed' in log.read_text()
disk_log = STATE / 'attempts/a1-initial-census-storage-all-1/output.log'
assert '214 passed; 17 failed' in disk_log.read_text()
archive('kernel-runtime.json', runtime_path)
archive('kernel-runtime.log', log)
archive('source.rs', ROOT / 'crates/executor-storage/src/mount_admission/host_consumers.rs')
for name, path in {'guard.py': STATE / 'guard.py', 'fixture_runtime.py': runner,
                   'archive.py': Path(__file__)}.items():
    target = OUT / 'harness' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(path.read_bytes())
summary = {'source': head, 'recorded_inputs_match_commit': len(frozen),
           'checks': checks, 'runtime': runtime,
           'direct_disk_suite_accepted': False,
           'fixture_only_tmpfs_suite_accepted': True,
           'installed_a1_accepted': False,
           'scope': 'Initial-census diagnostics only; no changed admission policy or installed mount acceptance'}
(OUT / 'source-match.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({'source': head, 'checks': len(checks),
                  'recorded_inputs_match_commit': len(frozen),
                  'installed_a1_accepted': False}))

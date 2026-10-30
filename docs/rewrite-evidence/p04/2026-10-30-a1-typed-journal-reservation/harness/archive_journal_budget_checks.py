# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Join typed journal resource checks and the pre-fix failure to source."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-30-a1-typed-journal-reservation'
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
OUT.mkdir(exist_ok=False)

def sha(data):
    return hashlib.sha256(data).hexdigest()

def archive(name, path):
    target = OUT / (name + '.gz')
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(path.read_bytes(), mtime=0))

checks, frozen = {}, None
for suffix in ('format-1', 'executor-tests-1', 'journal-tests-1', 'storage-docs-1',
               'clippy-1', 'reference-1', 'repository-1'):
    label = 'a1-journal-budget-' + suffix
    path = STATE / 'attempts' / label
    record = json.loads((path / 'record.json').read_text())
    assert record['exit_code'] == 0 and not record['guard_triggered'] and record['inputs_unchanged']
    assert record['before'] == record['after']
    if frozen is None:
        frozen = record['before']
    assert record['before'] == frozen, label
    checks[label] = {k:record[k] for k in ('head', 'command', 'started', 'ended', 'exit_code',
                                         'minimum_available_bytes', 'guard_triggered', 'inputs_unchanged')}
    for name in ('record.json', 'output.log'):
        archive('checks/' + label + '-' + name, path / name)
for name, expected in frozen.items():
    assert sha((ROOT / name).read_bytes()) == expected, name
    assert sha(subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)) == expected, name
sources = ('bins/executor/src/storage_mount_admission.rs', 'bins/executor/src/storage_runtime.rs',
           'crates/executor-storage/src/effects/mod.rs', 'crates/executor-storage/src/effects/tests.rs')
red_path = STATE / 'attempts/a1-journal-budget-red-1'
red = json.loads((red_path / 'record.json').read_text())
assert red['exit_code'] == 101 and red['inputs_unchanged'] and not red['guard_triggered']
expected = dict(frozen)
for name in sources:
    data = (STATE / 'a1-journal-budget-red.rs').read_bytes() if name == sources[0] else subprocess.check_output(['git', 'show', f'{red["head"]}:{name}'], cwd=ROOT)
    expected[name] = sha(data)
assert red['before'] == expected == red['after']
assert 'execution journal has no reservation' in (red_path / 'output.log').read_text()
for name in ('record.json', 'output.log'):
    archive('checks/a1-journal-budget-red-1-' + name, red_path / name)
mutation = STATE / 'attempts/a1-journal-budget-format-apply-1'
record = json.loads((mutation / 'record.json').read_text())
assert record['exit_code'] == 0 and not record['guard_triggered']
assert not record['inputs_unchanged'] and record['after'] == frozen
for name in ('record.json', 'output.log'):
    archive('procedure/format-apply-' + name, mutation / name)
buffer = io.BytesIO()
with tarfile.open(fileobj=buffer, mode='w') as tar:
    for variant in ('final', 'red'):
        for name in sources:
            if variant == 'final':
                data = (ROOT / name).read_bytes()
            elif name == sources[0]:
                data = (STATE / 'a1-journal-budget-red.rs').read_bytes()
            else:
                data = subprocess.check_output(['git', 'show', f'{red["head"]}:{name}'], cwd=ROOT)
            info = tarfile.TarInfo(variant + '/' + name)
            info.size, info.mode, info.mtime = len(data), 0o644, 0
            tar.addfile(info, io.BytesIO(data))
(OUT / 'source-variants.tar.gz').write_bytes(gzip.compress(buffer.getvalue(), mtime=0))
for name in ('guard.py', 'qualify_journal_budget.py', 'archive_journal_budget_checks.py'):
    path = OUT / 'harness' / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes((STATE / name).read_bytes())
summary = {'source': head, 'recorded_inputs_match_commit': len(frozen), 'checks': checks,
           'red_source_parent': red['head'], 'red_variant_joined': True,
           'inspection_baseline_bound': 16, 'journal_baseline_bound': 21,
           'inspection_total_reservation': 213, 'journal_total_reservation': 218,
           'aggregate_fd_limit': 226, 'service_nofile_limit': 256,
           'ordinary_seventeenth_descriptor_refused': True, 'journal_twenty_second_descriptor_refused': True,
           'genuine_journal_required_by_public_composition_seam': True,
           'installed_a1_accepted': False, 'new_installed_candidate_required': True}
(OUT / 'source-match.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({k:summary[k] for k in ('source', 'recorded_inputs_match_commit', 'inspection_total_reservation', 'journal_total_reservation', 'installed_a1_accepted')}))

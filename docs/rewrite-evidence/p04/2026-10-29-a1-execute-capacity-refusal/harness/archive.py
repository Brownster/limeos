# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Preserve the continuously guarded execute-admission refusal."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/a1-runtime-bootstrap-qualification'
BUILDS = ROOT / '.cache/stack-installed-qualification'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-29-a1-execute-capacity-refusal'
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
OUT.mkdir(exist_ok=False)

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def save(name, data):
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)

def archive(name, path):
    save(name + '.gz', gzip.compress(path.read_bytes(), mtime=0))

build = json.loads((BUILDS / 'build-8/record.json').read_text())
packages = json.loads((BUILDS / 'candidate-8/inputs.json').read_text())
fixture = json.loads((STATE / 'inputs-1.json').read_text())
assert head == build['source'] == packages['source'] == fixture['product_source']
assert build['exit_code'] == 0 and build['inputs_unchanged'] and build['container_removed']
assert build['stop_reason'] is None and not build['oom_killed']
for name, expected in build['before'].items():
    assert digest(ROOT / name) == expected, name
    assert hashlib.sha256(subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)).hexdigest() == expected, name
for name, expected in packages['repository'].items():
    assert digest(BUILDS / 'candidate-8/repository' / name) == expected, name
for name, expected in fixture['guest'].items():
    assert digest(STATE / 'guest-1' / name) == expected, name
for name, expected in fixture['runner_support'].items():
    assert digest(STATE / name) == expected, name
assert digest(STATE / 'run_384.py') == fixture['runner_sha256']
vm = ROOT / '.cache/main-consolidation/attempts/a1-runtime-bootstrap-vm-1'
record = json.loads((vm / 'record.json').read_text())
assert record['head'] == head and record['before'] == build['before'] == record['after']
assert record['exit_code'] == 1 and record['ended'] and record['inputs_unchanged']
assert not record['guard_triggered']
log = (vm / 'output.log').read_text()
failure = next(json.loads(line.partition(' ')[2]) for line in log.splitlines() if line.startswith('A1_FAILURE '))
assert failure['summary'] == 'fail' and failure['stage'] == 'await_terminal_receipt'
assert failure['action'] == failure['job']['id'] and failure['job']['state'] == 'needs_intervention'
assert failure['authority_schema'] == 12 and failure['package_version'] == packages['version']
assert failure['schema_source_sha256'] == fixture['schema_source_sha256']
assert failure['fixture_sha256'] == fixture['guest']['p04_mounts_guest.py']
for name, expected in failure['binary_sha256'].items():
    assert expected == packages['binaries'][name]['sha256'], name
assert 'A1_CONSUMER_CHANGE_RECORDS=0' in log
assert '"phase":"inspect.capacity.admit","code":"Unavailable"' in failure['unit_diagnostics']
assert '"operation":"receipt","phase":"root_reply","code":"NotFound"' in failure['unit_diagnostics']
assert not (ROOT / '.cache/main-consolidation/tmp/limeos-p01-vm-u8cqj_zz').exists()
for name in ('record.json', 'output.log', 'binaries.json', 'source-before.log', 'source-after.log'):
    archive('build/' + name, BUILDS / 'build-8' / name)
for label in ('a1-runtime-bootstrap-ui-build-1', 'a1-runtime-bootstrap-packages-1', 'a1-runtime-bootstrap-vm-1'):
    directory = ROOT / '.cache/main-consolidation/attempts' / label
    for name in ('record.json', 'output.log'):
        archive('checks/' + label + '-' + name, directory / name)
archive('package-inputs.json', BUILDS / 'candidate-8/inputs.json')
archive('fixture-inputs.json', STATE / 'inputs-1.json')
for name in ('admission-1.json', 'guard-launch-1.json', 'guard-console-1.log',
             'result-1-vm-failure.txt', 'result-1-startup-console.txt'):
    archive(name, STATE / name)
save('a1-failure.json', (json.dumps(failure, indent=2) + '\n').encode())
for name, path in {
    'build_candidate.py': BUILDS / 'build_candidate_8.py', 'build_inside.py': BUILDS / 'build_inside.py',
    'prepare_packages.py': BUILDS / 'prepare_packages_8.py', 'prepare_guest.py': BUILDS / 'prepare_runtime_bootstrap_guest.py',
    'launch_vm.py': BUILDS / 'launch_runtime_bootstrap_vm.py', 'detached_guard.py': BUILDS / 'detached_guard.py',
    'prepare_launch.py': BUILDS / 'prepare_launch_runtime_vm.py',
    'guard.py': ROOT / '.cache/main-consolidation/guard.py', 'archive.py': Path(__file__),
}.items():
    save('harness/' + name, path.read_bytes())
buffer = io.BytesIO()
with tarfile.open(fileobj=buffer, mode='w') as tar:
    for path in [*sorted((STATE / 'guest-1').glob('*.py')), STATE / 'run_384.py', STATE / 'current_authority_schema.py']:
        data = path.read_bytes()
        info = tarfile.TarInfo(('guest/' if path.parent.name == 'guest-1' else 'runner/') + path.name)
        info.size, info.mode, info.mtime = len(data), 0o644, 0
        tar.addfile(info, io.BytesIO(data))
save('guest-and-runner-sources.tar.gz', gzip.compress(buffer.getvalue(), mtime=0))
for name, path in {'executor-main.rs': 'bins/executor/src/main.rs',
                   'executor-admission.rs': 'bins/executor/src/storage_mount_admission.rs',
                   'executor-runtime.rs': 'bins/executor/src/storage_runtime.rs',
                   'schema-source.rs': 'crates/persistence/src/lib.rs'}.items():
    archive(name, ROOT / path)
summary = {'source': head, 'recorded_inputs_match_commit': len(build['before']),
           'a1_accepted': False, 'continuous_guard_terminal_record_present': True,
           'vm_exit_code': record['exit_code'], 'host_guard_triggered': record['guard_triggered'],
           'minimum_available_bytes': record['minimum_available_bytes'],
           'inputs_unchanged': record['inputs_unchanged'], 'vm_scratch_removed': True,
           'stage': failure['stage'], 'job_state': failure['job']['state'],
           'action': failure['action'], 'actual_refusal': 'inspect.capacity.admit Unavailable',
           'root_receipt_reply': 'NotFound', 'task_difference_records': 0,
           'service_threads_at_start': failure['service']['status']['Threads'],
           'packages': packages['packages'], 'binaries': packages['binaries'],
           'upgrade_qualification': False, 'installed_ui_qualification': False,
           'next': 'Measure execution resource admission with the genuine already-open MountJournal; do not raise limits or retry this action'}
save('source-match.json', (json.dumps(summary, indent=2) + '\n').encode())
print(json.dumps({k:summary[k] for k in ('source','recorded_inputs_match_commit','vm_exit_code','stage','actual_refusal','a1_accepted')}))

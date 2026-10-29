# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Preserve a failed installed run without inventing a missing terminal guard."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/a1-initial-census-diagnostic'
BUILDS = ROOT / '.cache/stack-installed-qualification'
OUT = ROOT / 'docs/rewrite-evidence/p04/2026-10-29-a1-executor-idle-thread'
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

build = json.loads((BUILDS / 'build-7/record.json').read_text())
packages = json.loads((BUILDS / 'candidate-7/inputs.json').read_text())
fixture = json.loads((STATE / 'inputs-1.json').read_text())
assert build['source'] == packages['source'] == fixture['product_source'] == head
assert build['exit_code'] == 0 and build['inputs_unchanged'] and build['container_removed']
assert build['stop_reason'] is None and not build['oom_killed']
for name, expected in build['before'].items():
    assert digest(ROOT / name) == expected, name
    source = subprocess.check_output(['git', 'show', f'{head}:{name}'], cwd=ROOT)
    assert hashlib.sha256(source).hexdigest() == expected, name
for name, expected in packages['repository'].items():
    assert digest(BUILDS / 'candidate-7/repository' / name) == expected, name
for name, expected in fixture['guest'].items():
    assert digest(STATE / 'guest-1' / name) == expected, name
for name, expected in fixture['runner_support'].items():
    assert digest(STATE / name) == expected, name
assert digest(STATE / 'run_384.py') == fixture['runner_sha256']
vm = ROOT / '.cache/main-consolidation/attempts/a1-initial-census-vm-1'
record = json.loads((vm / 'record.json').read_text())
assert record['head'] == head and record['before'] == build['before']
assert 'exit_code' not in record and 'ended' not in record
log = (vm / 'output.log').read_text()
failure = next(json.loads(line.partition(' ')[2]) for line in log.splitlines() if line.startswith('A1_FAILURE '))
assert failure['summary'] == 'fail' and failure['stage'] == 'human_plan_approval_queue'
assert 'action' not in failure
assert failure['authority_schema'] == 12 and failure['package_version'] == packages['version']
assert failure['schema_source_sha256'] == fixture['schema_source_sha256']
assert failure['fixture_sha256'] == fixture['guest']['p04_mounts_guest.py']
for name, expected in failure['binary_sha256'].items():
    assert expected == packages['binaries'][name]['sha256'], name
changes = [json.loads(line.partition(' ')[2]) for line in log.splitlines() if line.startswith('A1_CONSUMER_CHANGE ')]
assert len(changes) == 1
change = changes[0]
assert change['phase'] == 'scan.recheck.user_tasks'
assert change['removed_count'] == 1 and change['added_count'] == 0
removed = change['removed'][0]
assert removed['pid'] == failure['service']['pid'] == 7189 and removed['tid'] == 7190
witness = failure['after_refusal_facts']['pre_effect_task_witness']
for side in ('before', 'after'):
    tasks = witness[side]['user_tasks']
    assert witness[side]['complete_without_observed_read_or_membership_errors']
    assert any(all(task[key] == removed[key] for key in ('pid', 'tid', 'start', 'namespace'))
               and task['cgroup'] == '/system.slice/limeos-storage-mounts.service' for task in tasks)
scratch = ROOT / '.cache/main-consolidation/tmp/limeos-p01-vm-glw75_hb'
assert not scratch.exists()
for name in ('record.json', 'output.log', 'binaries.json', 'source-before.log', 'source-after.log'):
    archive('build/' + name, BUILDS / 'build-7' / name)
for label in ('a1-initial-census-ui-build-1', 'a1-initial-census-packages-1', 'a1-initial-census-vm-1'):
    directory = ROOT / '.cache/main-consolidation/attempts' / label
    for name in ('record.json', 'output.log'):
        archive('checks/' + label + '-' + name, directory / name)
archive('package-inputs.json', BUILDS / 'candidate-7/inputs.json')
archive('fixture-inputs.json', STATE / 'inputs-1.json')
for name in ('admission-1.json', 'result-1-vm-failure.txt', 'result-1-startup-console.txt', 'supervision-recovery-1.json'):
    archive(name, STATE / name)
save('a1-failure.json', (json.dumps(failure, indent=2) + '\n').encode())
for name, path in {
    'build_candidate.py': BUILDS / 'build_candidate_7.py',
    'build_inside.py': BUILDS / 'build_inside.py',
    'prepare_packages.py': BUILDS / 'prepare_packages_7.py',
    'prepare_guest.py': BUILDS / 'prepare_initial_census_guest.py',
    'launch_vm.py': BUILDS / 'launch_initial_census_vm.py',
    'watch_orphan_vm.py': BUILDS / 'watch_orphan_census_vm.py',
    'guard.py': ROOT / '.cache/main-consolidation/guard.py',
    'archive.py': Path(__file__),
}.items():
    save('harness/' + name, path.read_bytes())
buffer = io.BytesIO()
with tarfile.open(fileobj=buffer, mode='w') as tar:
    for path in [*sorted((STATE / 'guest-1').glob('*.py')), STATE / 'run_384.py', STATE / 'current_authority_schema.py']:
        name = 'guest/' + path.name if path.parent.name == 'guest-1' else 'runner/' + path.name
        info = tarfile.TarInfo(name)
        data = path.read_bytes()
        info.size, info.mode, info.mtime = len(data), 0o644, 0
        tar.addfile(info, io.BytesIO(data))
save('guest-and-runner-sources.tar.gz', gzip.compress(buffer.getvalue(), mtime=0))
archive('executor-main.rs', ROOT / 'bins/executor/src/main.rs')
archive('schema-source.rs', ROOT / 'crates/persistence/src/lib.rs')
tokio = ROOT / '.cache/p04-a1-integration/cargo-home/registry/src/index.crates.io-1949cf8c6b5b557f/tokio-1.52.3/src/runtime/blocking/pool.rs'
summary = {'source': head, 'recorded_inputs_match_commit': len(build['before']),
           'a1_accepted': False, 'continuous_host_guard_accepted': False,
           'terminal_guard_record_missing': True, 'runner_and_vm_absent_native_observation': True,
           'vm_scratch_removed': True, 'original_vm_record_modified': False,
           'guest_reported_stage': failure['stage'], 'removed_task': removed,
           'matched_service_pid_and_witness_cgroup': True,
           'pre_effect_refusal_cases': failure['pre_effect']['cases'],
           'binary_and_package_match': True, 'packages': packages['packages'],
           'tokio_blocking_pool_source_sha256': digest(tokio),
           'hypothesis': 'Startup-only no-op creates the executor blocking worker, whose default ten-second idle retirement changes the exact host task set during a later admission',
           'upgrade_qualification': False, 'installed_ui_qualification': False}
save('source-match.json', (json.dumps(summary, indent=2) + '\n').encode())
print(json.dumps({k:summary[k] for k in ('source','recorded_inputs_match_commit','guest_reported_stage','removed_task','a1_accepted','terminal_guard_record_missing')}))

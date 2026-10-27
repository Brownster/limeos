# /// script
# dependencies = []
# ///
"""Freeze current committed A1 fixtures with source-bound current packages."""
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/a1-current-qualification'
BUILD = ROOT / '.cache/stack-installed-qualification/build-6'
PACKAGES = ROOT / '.cache/stack-installed-qualification/candidate-6'

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

assert not subprocess.check_output(['git', 'status', '--porcelain'], cwd=ROOT)
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
record = json.loads((BUILD / 'record.json').read_text())
inputs = json.loads((PACKAGES / 'inputs.json').read_text())
assert record['exit_code'] == 0 and record['inputs_unchanged'] and record['container_removed']
assert record['source'] == inputs['source'] == head
for name, expected in record['before'].items():
    assert digest(ROOT / name) == expected, name
for name, expected in inputs['packages'].items():
    assert digest(PACKAGES / 'repository/pool/main' / name) == expected, name
for name, expected in inputs['repository'].items():
    assert digest(PACKAGES / 'repository' / name) == expected, name
STATE.mkdir(exist_ok=False)
guest = STATE / 'guest-1'
guest.mkdir()
for source in sorted((ROOT / 'tests/privileged_vm').glob('*.py')):
    shutil.copyfile(source, guest / source.name)
census = ROOT / '.cache/a1-task-difference/guest/namespace_census.py'
shutil.copyfile(census, guest / census.name)
wrapper = (ROOT / '.cache/a1-fixture-readiness/installed_guest.py').read_text()
assert wrapper.count('0.4.6+taskdiff.1') == 1
(guest / 'installed_guest.py').write_text(wrapper.replace('0.4.6+taskdiff.1', inputs['version']))
original = (ROOT / 'tests/privileged_vm/run.py').read_text()
memory = '"2048" if args.build_bundle else "1024"'
assert original.count(memory) == 1
runner = STATE / 'run_384.py'
runner.write_text(original.replace(memory, '"2048" if args.build_bundle else "384"'))
support = STATE / 'current_authority_schema.py'
shutil.copyfile(ROOT / 'tests/privileged_vm/current_authority_schema.py', support)
manifest = {'fixture_source': head, 'product_source': record['source'], 'packages': inputs['packages'],
            'snapshotter_sha256': digest(Path(__file__)),
            'schema_source_sha256': digest(ROOT / 'crates/persistence/src/lib.rs'),
            'binaries': inputs['binaries'], 'ui': inputs['ui'],
            'package_inputs_sha256': digest(PACKAGES / 'inputs.json'),
            'guest_memory_mib': 384, 'runner_sha256': digest(runner),
            'runner_support': {support.name: digest(support)},
            'only_runner_change': 'non-build guest memory 1024 to 384 MiB; assertions and service ceilings unchanged',
            'upgrade_qualification': False, 'observational_census_sha256': digest(census),
            'guest': {p.name: digest(p) for p in sorted(guest.glob('*.py'))}}
(STATE / 'inputs-1.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({'source': head, 'package_version': inputs['version'],
                  'guest_files': len(manifest['guest']), 'upgrade_qualification': False}))

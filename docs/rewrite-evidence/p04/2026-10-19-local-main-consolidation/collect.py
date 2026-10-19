# /// script
# dependencies = []
# ///
"""Archive actual local consolidation outcomes without granting runtime acceptance."""

import gzip
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
WORK = ROOT / '.worktrees/local-main-consolidation'
STATE = ROOT / '.cache/main-consolidation'
PACKET = WORK / 'docs/rewrite-evidence/p04/2026-10-19-local-main-consolidation'
PACKET.mkdir(parents=True, exist_ok=True)


def digest(content):
    return hashlib.sha256(content).hexdigest()


def git(*args):
    return subprocess.check_output(['git', '-C', str(WORK), *args], text=True).strip()


source = 'b587621a2cde11442ce318728f57faad6fc384e7'
assert git('rev-parse', 'HEAD') == source
results = json.loads((STATE / 'frontend/results.json').read_text())
assert all(row['exit_code'] == 0 for row in results)
assert sum(row['counts'].get('pass', 0) for row in results) == 219
records = {}
for directory in sorted((STATE / 'attempts').iterdir()):
    record = json.loads((directory / 'record.json').read_text())
    records[directory.name] = {key: record.get(key) for key in ('head', 'command', 'exit_code', 'guard_triggered', 'minimum_available_bytes', 'inputs_unchanged')}
assert records['workspace-tests-cached']['guard_triggered']
assert records['reference-tests-host']['exit_code'] == 0
assert records['arm64-harness']['exit_code'] == 0
assert records['schema-test']['exit_code'] == 0
names = git('ls-files', '-z').split('\0')
prefixes = ('Cargo.', 'rust-toolchain', '.cargo/', 'crates/', 'bins/', 'frontend/', 'tests/', 'scripts/', 'packaging/', 'spikes/', 'contracts/', '.github/')
inputs = [name for name in names if name.startswith(prefixes) and (WORK / name).is_file()]
(PACKET / 'inputs.sha256').write_text(''.join(f'{digest((WORK / name).read_bytes())}  {name}\n' for name in sorted(inputs)))
for path in sorted(STATE.rglob('*')):
    relative = path.relative_to(STATE)
    if not path.is_file() or relative.parts[0] not in ('attempts', 'frontend') or path.suffix not in ('.json', '.log'):
        continue
    target = PACKET / 'logs' / f'{relative}.gz'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(path.read_bytes(), mtime=0))
for name in ('generate.log', 'ui-contract-red.log', 'ui-contract-green.log', 'ui-contract-direct.log', 'ui-contract-final.log', 'fmt.log', 'repository.log', 'repository-cache-refusal.log'):
    path = STATE / name
    if path.is_file():
        (PACKET / f'{name}.gz').write_bytes(gzip.compress(path.read_bytes(), mtime=0))
(PACKET / 'Cargo.lock.gz').write_bytes(gzip.compress((WORK / 'Cargo.lock').read_bytes(), mtime=0))
(PACKET / 'refs-before.txt.gz').write_bytes(gzip.compress((STATE / 'refs-before.txt').read_bytes(), mtime=0))
for name in ('collect.py', 'frontend_check.py', 'guard.py', 'cleanup.py', 'relocate_previews.py'):
    shutil.copyfile(STATE / name, PACKET / name)
manifest = {'source': source, 'base': '7df3f7530865c31b7faefd1f7b12a25aa7655860', 'input_files': len(inputs), 'frontend': results, 'attempts': records, 'full_native_rust': 'UNQUALIFIED: missing default offline dependency; existing-cache attempt guard-terminated during compilation', 'fresh_workspace_clippy': 'UNRUN', 'hosted_final_ci': 'UNRUN: publication permission pending', 'installed_vm_hardware_performance': 'UNRUN', 'phase_or_defect_closure': False, 'original_refs_bundle': str(STATE / 'refs-before.bundle'), 'original_bundle_sha256': digest((STATE / 'refs-before.bundle').read_bytes())}
(PACKET / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
(PACKET / 'artifacts.sha256').write_text(''.join(f'{digest(path.read_bytes())}  {path.relative_to(PACKET)}\n' for path in sorted(PACKET.rglob('*')) if path.is_file() and path.name != 'artifacts.sha256'))
print(json.dumps({'source': source, 'inputs': len(inputs), 'frontend_cases': 219, 'native_workspace_qualification': False}))

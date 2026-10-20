# /// script
# dependencies = []
# ///
"""Bind the final local checks and completed cleanup to unchanged product source."""

import gzip
import hashlib
import json
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path('/home/marc/Documents/github/lime-os')
STATE = ROOT / '.cache/main-consolidation'
PACKET = ROOT / 'docs/rewrite-evidence/p04/2026-10-20-local-main-validation'
PACKET.mkdir(parents=True, exist_ok=True)
source = '6466eb0236cf4599bd6c67555277b1a8cdccfc1b'
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip() == source


def save(path, data):
    target = PACKET / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(gzip.compress(data, mtime=0) if str(path).endswith('.gz') else data)


def counts(log):
    rows = [tuple(map(int, row)) for row in re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', log, re.M)]
    assert rows and all(row[1:] == (0, 0, 0, 0) for row in rows)
    return sum(row[0] for row in rows)


checks = {}
names = ('workspace-rest', 'storage-compile', 'storage-runtime', 'storage-docs', 'main-clippy', 'main-contracts', 'prepare-stream-b', 'stream-b-tests', 'stream-b-clippy')
for name in names:
    directory = STATE / 'attempts' / name
    record = json.loads((directory / 'record.json').read_text())
    assert record['head'] == source and record['exit_code'] == 0 and not record['guard_triggered'] and record['inputs_unchanged'], name
    checks[name] = {key: record[key] for key in ('command', 'exit_code', 'minimum_available_bytes', 'inputs_unchanged', 'started', 'ended')}
    save(f'checks/{name}/record.json.gz', (directory / 'record.json').read_bytes())
    save(f'checks/{name}/output.log.gz', (directory / 'output.log').read_bytes())
runtime = json.loads((STATE / 'storage-kernel-runtime.json').read_text())
assert runtime['exit_code'] == 0 and runtime['scratch_removed'] and not runtime['cargo_or_compilation_on_tmpfs']
assert not Path(runtime['runtime_tmpdir']).exists()
components = {'workspace_excluding_storage': counts((STATE / 'attempts/workspace-rest/output.log').read_text()), 'storage_unit': counts((STATE / 'storage-kernel-runtime.log').read_text()), 'storage_compile_fail': counts((STATE / 'attempts/storage-docs/output.log').read_text())}
assert components == {'workspace_excluding_storage': 906, 'storage_unit': 211, 'storage_compile_fail': 13}
leaf_count = counts((STATE / 'attempts/stream-b-tests/output.log').read_text())
assert leaf_count == 507
for name in ('storage-kernel-runtime.log', 'storage-kernel-runtime.json', 'cleanup-result.json', 'cleanup-plan.json', 'bundle-verification.log'):
    save(name + '.gz', (STATE / name).read_bytes())
for name in ('guard.py', 'followups.py', 'storage_runtime.py', 'collect_final.py'):
    shutil.copyfile(STATE / name, PACKET / name)
for path in sorted((ROOT / '.cache/stream-b-leaf').rglob('*')):
    if path.is_file() and path.name in ('Cargo.toml', 'Cargo.lock', 'lib.rs'):
        save(Path('harness') / path.relative_to(ROOT / '.cache/stream-b-leaf'), path.read_bytes())
save('Cargo.lock.gz', (ROOT / 'Cargo.lock').read_bytes())
save('inputs.sha256', (ROOT / 'docs/rewrite-evidence/p04/2026-10-19-local-main-consolidation/inputs.sha256').read_bytes())
failed = STATE / 'attempts/workspace-main'
save('initial-btrfs-run/output.log.gz', (failed / 'output.log').read_bytes())
save('initial-btrfs-run/record.json.gz', (failed / 'record.json').read_bytes())
tools = {name: subprocess.check_output(argv, text=True).strip() for name, argv in {'rustc': ['rustc', '-Vv'], 'cargo': ['cargo', '-V'], 'kernel': ['uname', '-srmp']}.items()}
cleanup = json.loads((STATE / 'cleanup-result.json').read_text())
assert len(cleanup['removed_worktrees']) == 78 and len(cleanup['removed_branches']) == 87 and len(cleanup['retained']) == 8
manifest = {'source': source, 'assembled_product_source': 'b587621a2cde11442ce318728f57faad6fc384e7', 'workspace_components': components, 'workspace_passed': sum(components.values()), 'stream_b_harness_passed': leaf_count, 'harness_overlap': 'Root API cases overlap workspace checks; do not sum the two counts as unique tests', 'checks': checks, 'storage_runtime': runtime, 'tools': tools, 'cleanup': {'worktrees_removed': 78, 'branches_removed': 87, 'dirty_worktrees_retained': 8}, 'frontend': {'passed': 219, 'scope': 'Earlier unchanged final frontend source, TypeScript and production build; see 19 October packet'}, 'fresh_native_arm64_and_installed': 'UNRUN', 'remote_push': False, 'phase_completion': False}
(PACKET / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
(PACKET / 'artifacts.sha256').write_text(''.join(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(PACKET)}\n' for path in sorted(PACKET.rglob('*')) if path.is_file() and path.name != 'artifacts.sha256'))
print(json.dumps({'workspace_passed': sum(components.values()), 'stream_b_harness_passed': leaf_count, 'removed_worktrees': 78, 'removed_branches': 87, 'source': source}))

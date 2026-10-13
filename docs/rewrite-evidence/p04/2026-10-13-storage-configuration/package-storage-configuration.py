# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind local retained-configuration checks to committed source and raw logs."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/storage-configuration-evidence'
logs = root / '.cache/rewrite-storage-configuration/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-storage-configuration'
qualified = 'a303e72fe3484643f34bc4b394e13a507cc32a1f'
names = ('configuration-focused', 'test-rest', 'storage-compile-on-disk', 'storage-fixtures', 'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')
checks = []
for name in names:
    metadata = json.loads((logs / (name + '.json')).read_text())
    assert metadata['exit'] == 0 and metadata['qualified_source'] == qualified
    compressed = (logs / metadata['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == metadata['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == metadata['compressed_sha256']
    for path, digest in metadata['runtime_hashes'].items():
        committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
        assert hashlib.sha256(committed).hexdigest() == digest
    if name == 'arm64-harness':
        metadata['harness_cases'] = int(re.search(rb'Ran (\d+) tests', raw)[1])
        assert metadata['harness_cases'] == 61 and raw.rstrip().endswith(b'OK')
    checks.append(metadata)
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest', 'storage-fixtures'))
assert rust == 559, rust
assert checks[0]['rust_cases'] == 24
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-13-configured-graph-integration/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-storage/src/configuration.rs', 'crates/executor-storage/src/configuration/tests.rs'}
bindings = {}
for name in sorted(files):
    actual = (work / name).read_bytes()
    assert actual == subprocess.check_output(['git', 'show', qualified + ':' + name], cwd=work), name
    bindings[name] = hashlib.sha256(actual).hexdigest()
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust, 'unit_integration_cases': 549, 'compile_fail_doctests': 10, 'focused_configuration_cases': 24, 'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61, 'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified, 'files': bindings}, indent=2) + '\n')
scope = {
    'qualified_source': qualified, 'parent_main': '0ab4870d6b689cffd314241922feadd041d056cc',
    'production_surface': 'unregistered read-only library', 'public_root_success': 'unqualified',
    'local_filesystem_authentication': 'private scratch root and actual test UID; production traversal/read code',
    'configuration_roles': ['storage_contract', 'pools', 'snapraid', 'declared_samba_shares'],
    'resource_caps': {'per_file_bytes': 65536, 'total_raw_file_bytes': 131072, 'retained_fds': 65, 'additional_revalidation_fds': 4, 'report_bytes': 16384, 'path_bytes': 512, 'path_components': 16, 'original_lease_seconds': 4, 'cooperative_work_seconds': 2},
    'pi_accessed': False, 'installed_qualification': 'untested', 'cutover': 'deferred by operator',
    'backup_bound_recovery_excluded': True, 'new_dependencies': False, 'schema_or_api_change': False,
    'service_or_capability_change': False, 'frozen_arm64_candidate_source': 'af94f19ac21f9da14b207439808ffb33c87e8e10',
    'frozen_arm64_candidate_contains_configuration_owner': False,
    'pending': ['actual Samba global/account/include state', 'retained configuration and physical/namespace composition', 'whole-worker admission and independently killable supervision', 'complete affected-resource claims', 'storage locks, effect-time revalidation and durable recovery', 'current native CI and installed ARM64 qualification', 'minimum container/stack UI and migration/return evidence'],
    'inventory_performance_gate': 'historical p95 61.401 ms against 20 ms remains failed', 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
diagnostics = out / 'diagnostics'
diagnostics.mkdir()
for suffix in ('json', 'log.gz'):
    shutil.copy2(logs / ('reentrant-before.' + suffix), diagnostics)
(diagnostics / 'README.md').write_text('The precommit reentrant-validation reproduction failed before the state-machine fix. Its metadata names the committed design baseline, not the uncommitted implementation; it is diagnostic evidence, not an exact-source qualification. The final focused and full tests above qualify the committed fix. The compressed transcript retains every original byte.\n')
for name in ('run-storage-configuration-check.py', 'package-storage-configuration.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f'''# Retained storage configuration: local acceptance

Qualified source `{qualified}`, based on local main `0ab4870`. The additive
`executor_storage::configuration` library retains four protected typed JSON files
and their root/ancestor/file descriptors. It validates the frozen planning rules,
compares held and freshly opened identities/bytes, preserves the original lease
and permanently refuses failed or reentrant validation. The design and API
boundary are in [the design](../../../plans/2026-10-13-storage-configuration-evidence-design.md)
and [the library guide](../../../p04-storage-configuration-evidence.md).

**{rust} Rust cases pass:** 549 unit/integration cases plus 10 compile-fail
doctests. The 24 focused configuration tests are already included in that total.
The four existing layouts feed the existing graph goldens. All-target strict
Clippy, fmt, generated contracts and repository checks pass. **61 ARM64 harness
tests pass locally.** [checks.json](checks.json) retains every command, runtime,
environment, source hash and original/compressed transcript hash.

Compilation, Cargo/UV caches, logs and general scratch stayed on disk. Existing
kernel storage fixtures were compiled on disk first, then used an owned tmpfs
directory that was removed after execution. Its sampled peak was
{peak['bytes']} bytes in {peak['files']} files. This exception preserves existing
filesystem guards; it does not put build outputs or caches on tmpfs.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files to the qualified commit. Logs use lossless
gzip with a fixed timestamp; decompress them to recover the exact combined
stdout/stderr bytes. The [precommit diagnostic](diagnostics/README.md) records the
reentrant regression before its fix and is excluded from qualification totals.
Checks were reviewed by the implementing agent; no independent review is claimed.

Qualification is local. Private scratch-root/UID tests exercise genuine descriptors
and production traversal/read logic. Public successful root collection, installed
confinement, native CI and current installed ARM64 remain unqualified. No Pi,
workstation Docker or raw workstation storage probe was used. The separate backup
recovery worktree stayed untouched. The signed ARM64 candidate remains bound to
its earlier source and contains neither this owner nor the configured graph.

The returned configuration describes declared planning state. Actual Samba
global/account/include state, physical and running-namespace composition,
whole-worker admission/supervision, complete claims, storage locks, approvals,
effects and recovery remain open. Separate files do not establish an atomic
configuration transaction. P04 and every defect/cutover/performance gate remain
open; the historical inventory p95 failure is unchanged. See [scope.json](scope.json).

To reproduce, use a fresh disk-backed output directory in the retained check
helper. Compile storage fixtures with `storage-compile-on-disk` before invoking
`storage-fixtures`; all other checks use disk-backed scratch. Current host
qualification requires separate authorization and exact-source evidence.
''')
entries = {str(path.relative_to(out)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(out.rglob('*')) if path.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified, 'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

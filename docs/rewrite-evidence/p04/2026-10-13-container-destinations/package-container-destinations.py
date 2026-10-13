# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package exact-source local destination checks without host qualification claims."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/container-destinations'
logs = root / '.cache/rewrite-container-destinations/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-container-destinations'
qualified = '93ec390fc8c5ebe3c896fb88a913f98e4d4e4a7d'
names = ('destinations-focused', 'test-rest', 'storage-compile-on-disk', 'storage-fixtures', 'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')
checks = []
for name in names:
    metadata = json.loads((logs / (name + '.json')).read_text())
    assert metadata['exit'] == 0 and metadata['qualified_source'] == qualified
    compressed = (logs / metadata['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == metadata['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == metadata['compressed_sha256']
    for path, digest in metadata['runtime_hashes'].items():
        assert hashlib.sha256(subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)).hexdigest() == digest
    if name == 'arm64-harness':
        metadata['harness_cases'] = int(re.search(rb'Ran (\d+) tests', raw)[1])
        assert metadata['harness_cases'] == 61 and raw.rstrip().endswith(b'OK')
    checks.append(metadata)
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest', 'storage-fixtures'))
assert rust == 589 and checks[0]['rust_cases'] == 27
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-13-storage-configuration/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-storage/src/destinations.rs', 'crates/executor-storage/src/destinations/tests.rs'}
bindings = {}
for name in sorted(files):
    actual = (work / name).read_bytes()
    assert actual == subprocess.check_output(['git', 'show', qualified + ':' + name], cwd=work), name
    bindings[name] = hashlib.sha256(actual).hexdigest()
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust, 'unit_integration_cases': 576, 'compile_fail_doctests': 13, 'focused_destination_cases': 27, 'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61, 'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified, 'files': bindings}, indent=2) + '\n')
scope = {
    'qualified_source': qualified, 'parent_main': '649dce8', 'production_surface': 'unregistered read-only library',
    'public_root_composition': 'unqualified', 'foreign_namespace_resolution': 'unqualified',
    'local_real_facts': ['scratch descriptors', 'O_PATH cached confined lookup', 'inode/type/mode/UID/GID/link checks', 'replacement/unlink refusal', 'file data read refusal'],
    'local_synthetic_facts': ['block devices and UUID graph inputs', 'host and process mount IDs/roots/tables', 'process membership and digests'],
    'resource_caps': {'destinations': 32, 'additional_retained_fds': 32, 'report_bytes': 32768, 'input_serialization_bytes': 65536, 'additional_local_lease_seconds': 4, 'cooperative_work_seconds': 4},
    'unsupported': ['nested host source mount candidates', 'process mounts below selected destinations', 'stacked selected mounts', 'destination symlinks', 'Btrfs', 'remote/FUSE/unknown filesystems'],
    'shared_source_changes': ['crate-private borrowed declarations/process roots', 'crate-private physical source owner', 'crate-private existing bounded digest and fdinfo helpers'],
    'existing_public_parent_collection_semantics_changed': False,
    'pi_accessed': False, 'installed_qualification': 'untested', 'cutover': 'deferred by operator',
    'backup_bound_recovery_excluded': True, 'new_dependencies': False, 'schema_or_api_change': False,
    'service_or_capability_change': False, 'frozen_arm64_candidate_source': 'af94f19ac21f9da14b207439808ffb33c87e8e10',
    'frozen_arm64_candidate_contains_destination_owner': False,
    'pending': ['fresh authenticated Engine reacquisition', 'retained planning file integration', 'physical consumers of unselected mounts/root filesystem backing', 'nested mount propagation', 'actual pool/protection/share state', 'whole-worker admission and independently killable supervision', 'complete affected-resource claims', 'storage locks, effect-time checks and durable recovery', 'current native CI and installed ARM64 qualification'],
    'inventory_performance_gate': 'historical p95 61.401 ms against 20 ms remains failed', 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-container-destinations-check.py', 'package-container-destinations.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f'''# Running container destinations: local acceptance

Qualified source `{qualified}`, based on local main `649dce8`.
The new `executor_storage::destinations` library borrows genuine process and
physical-dependency owners, verifies flat running destinations against retained
host sources, and retains O_PATH descriptors without reading file data.
Independent namespace mount IDs, device/inode/kind, full filesystem roots and
writable state must agree. The complete parent reports remain digest-bound;
their original leases remain unchanged. See [the design](../../../plans/2026-10-13-container-destination-evidence-design.md)
and [the interface guide](../../../p04-container-destination-evidence.md).

**{rust} Rust cases pass:** 576 unit/integration cases and 13 compile-fail
doctests. The 27 new focused cases are already included. **61 local ARM64
harness tests pass.** All-target strict Clippy, fmt, generated contracts and
repository checks pass. [checks.json](checks.json) binds every command,
environment, runtime, source hash and original/compressed transcript hash.

Local proof uses real scratch descriptors for confined cached O_PATH resolution,
inode/type/mode/UID/GID/link checks, replacement/unlink and symlink refusal,
and refusal to read file data. Device/UUID, process membership and mount-table
facts are explicitly synthetic private inputs. Snapshot/owner import and
parent-lifetime compile-fail tests preserve the public ownership boundary.
Successful public root composition, foreign namespace resolution and process
changes during the composed operation remain unqualified. Linux interface
references in the guide explain the chosen mechanism; they are not execution
evidence for the current installed target.

Source, compilation, caches, logs and general scratch stayed on disk. Existing
kernel storage fixtures were compiled on disk before using an owned, auto-cleaned
tmpfs directory; its sampled peak was {peak['bytes']} bytes in {peak['files']} files.
Existing kernel/service/capability guards were preserved. No Pi, Docker or raw
workstation storage probe was used, and the separate backup recovery worktree
stayed untouched. The earlier signed ARM64 candidate contains neither this owner
nor the subsequently integrated configuration/graph libraries.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files to the qualified commit. Lossless gzip retains
every raw stdout/stderr byte, with a fixed timestamp and both raw/compressed
SHA-256 values. [artifact-sha256.json](artifact-sha256.json) binds the evidence
packet. The implementing agent reviewed this slice; independent review is pending.

Initial success covers flat declared source/destination relationships. Nested
propagation, destination symlinks and unsupported filesystems refuse complete
collection. Physical consumers of unselected mounts/root filesystem backing,
fresh Engine reacquisition, retained configuration integration, actual pool/share
state, whole-worker admission, independent supervision, claims, storage locks,
approvals, effects and recovery remain open. P04 and every defect, performance
and cutover gate stay open; see [scope.json](scope.json).

Reproduction uses the retained check helper with fresh disk-backed log paths.
Compile storage fixtures with `storage-compile-on-disk` before `storage-fixtures`.
Other checks use disk scratch. Root/VM/installed qualification requires separate
authorization and exact-source evidence.
''')
entries = {str(path.relative_to(out)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(out.rglob('*')) if path.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified, 'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

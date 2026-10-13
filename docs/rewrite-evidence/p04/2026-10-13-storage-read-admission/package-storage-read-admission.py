# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package lossless local process admission checks bound to the committed source."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/storage-read-admission'
logs = root / '.cache/rewrite-storage-read-admission/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-storage-read-admission'
qualified = '6c4a5e4923f20df46d8dc8ff359d1f3898585832'
names = ('test-rest','storage-compile-on-disk','storage-fixtures','source-measurement','clippy-accepted','fmt-accepted','contracts-accepted','repository-accepted','arm64-harness-accepted')
checks, raws = [], {}
for name in names:
    metadata = json.loads((logs / (name + '.json')).read_text())
    assert metadata['exit'] == 0 and metadata['qualified_source'] == qualified
    compressed = (logs / metadata['log']).read_bytes()
    raw = gzip.decompress(compressed)
    raws[name] = raw
    assert hashlib.sha256(raw).hexdigest() == metadata['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == metadata['compressed_sha256']
    for path, digest in metadata['runtime_hashes'].items():
        assert hashlib.sha256(subprocess.check_output(['git','show',qualified+':'+path],cwd=work)).hexdigest() == digest
    checks.append(metadata)
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest','storage-fixtures'))
assert rust == 623
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert b'sources=32 retained=34' in raws['source-measurement']
previous = json.loads((work/'docs/rewrite-evidence/p04/2026-10-13-process-read-admission/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-storage/src/read_admission.rs','crates/executor-storage/src/read_admission/tests.rs','crates/executor-storage/src/sources/tests/admission_tests.rs'}
bindings = {}
for path in sorted(files):
    raw = (work/path).read_bytes()
    assert raw == subprocess.check_output(['git','show',qualified+':'+path],cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
out.mkdir()
for c in checks:
    shutil.copy2(logs/c['log'],out/c['log'])
(out/'checks.json').write_text(json.dumps({'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':609,'compile_fail_doctests':14,'new_regression_cases':21,'focused_and_measurement_cases_already_in_workspace_total':True,'arm64_harness_cases':61,'checks':checks},indent=2)+'\n')
(out/'source-manifest.json').write_text(json.dumps({'qualified_source':qualified,'files':bindings},indent=2)+'\n')
scope = {'qualified_source': '6c4a5e4923f20df46d8dc8ff359d1f3898585832', 'parent_main': '4b5744a', 'production_surface': 'closed trusted-library profile through retained physical parents; no worker entry point', 'public_root_block_collection': 'unqualified', 'foreign_namespace_collection': 'unqualified', 'installed_qualification': 'untested', 'real_local_facts': ['scratch source O_PATH descriptors', 'actual host/PID 1 mount tables for source fixtures', 'file-data read refusal and retained identity checks', '34 retained source descriptors above baseline and cleanup'], 'private_substitutions': ['nonroot source collection seam bypasses public root authorization', 'synthetic topology/probe counts and device/UUID facts in pure fixtures'], 'closed_profile': {'complete_declarations': 16, 'running': 8, 'declared_mounts_including_stopped': 32, 'table_bytes': 65536, 'table_rows': 512, 'table_vector_string_capacity_bytes': 131072, 'decoded_path_bytes': 512, 'filesystem_bytes': 64, 'optional_tokens': 32, 'topology_nodes': 48, 'parents_per_node': 8, 'total_parent_entries': 128, 'dag_height_edges': 16, 'block_probes': 40, 'storage_retained_fds': 41, 'source_retained_fds': 34, 'swap_entries': 8, 'swap_input_bytes': 65536, 'fstab_bytes': 65536, 'fstab_entries': 128, 'source_mount_references': 128, 'pending_components': 256, 'pending_string_capacity_bytes': 16384, 'digest_scratch_bytes': 262144, 'declaration_bytes': 65536, 'report_bytes': 49152}, 'payload_exclusions': ['raw buffers', 'other report/input copies', 'BTreeMap/BTreeSet nodes', 'allocator metadata', 'resolved path-walk scratch', 'probe process/runtime/transport/stacks'], 'retained_fd_exclusions': ['temporary descriptor walks', 'fresh fstab handle', 'probe subprocess and transport pipes', 'other owners and runtime'], 'whole_worker_resource_qualification': False, 'independent_supervision': False, 'new_universal_parent_poisoning_claim': False, 'standard_observation_behavior_preserved': True, 'fixed_fstab_swap_token_vectors_replaced': True, 'shared_change': 'crate-private existing bounded reader helper visibility', 'new_dependencies': False, 'wire_schema_or_route_change': False, 'service_capability_or_kernel_guard_change': False, 'pi_accessed': False, 'docker_accessed': False, 'raw_workstation_storage_probed': False, 'reserved_backup_recovery_worktree_untouched': True, 'cutover': 'deferred by operator', 'frozen_arm64_candidate_source': 'af94f19ac21f9da14b207439808ffb33c87e8e10', 'frozen_arm64_candidate_contains_this_profile': False, 'pending': ['concrete whole-worker budget', 'independent termination/single-flight/cancellation/parent-death/descendant reaping', 'Engine/configuration integration', 'nested/unselected physical consumers and complete claims', 'actual pool/share state', 'locks/effects/recovery', 'current native CI and installed ARM64 qualification'], 'p04_closed': False, 'defect_rows_closed': [], 'performance_gate_closed': False, 'independent_review': 'pending'}
(out/'scope.json').write_text(json.dumps(scope,indent=2)+'\n')
for name in ('run-storage-read-admission-check.py','package-storage-read-admission.py'):
    shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f"""# Storage read admission: local acceptance

Qualified source `{qualified}`, based on local main `4b5744a`.
The closed CombinedV1 storage profile propagates through inventory, source and
physical dependency collection. It bounds host parsing, topology nodes/edges,
probe candidates, swap input, source references and pending symlink expansion.
Retained parents preserve their profile and original expiry. Standard observations
and existing guards remain. See [the design](../../../plans/2026-10-13-storage-read-admission-design.md)
and [interface guide](../../../p04-storage-read-admission.md).

**623 Rust cases pass:** 609 unit/integration cases and 14 compile-fail doctests.
The 21 new regression cases are already included; the source measurement rerun
adds no cases to that total. **61 local ARM64 harness tests pass**, with strict
all-target workspace Clippy, fmt, contract and repository checks.
[checks.json](checks.json) binds successful final-source commands, environments,
runtime hashes and raw/compressed transcript hashes. A redundant allocation in
one pre-acceptance test fixture was removed after Clippy reported it.

Local source fixtures use real scratch O_PATH descriptors, actual host/PID 1
mount tables and retained identity/read-refusal checks through a private nonroot
collection seam. The exact 32-source fixture measured 34 retained descriptors
above baseline and verified cleanup plus whole-count refusal. Topology/probe
counts and device/UUID facts in pure tests are synthetic; no block device was
opened or probed for this evidence. The existing parent deadlines are cooperative for synchronous work; independent
termination remains unqualified. Public root/device collection, foreign namespaces and
installed permissions remain unqualified.

The combined profile admits at most 41 retained storage descriptors and 34
retained source descriptors, with a 128 KiB parsed-table capacity allowance.
Temporary descriptors, probe child/transport pipes and other owners are outside
those retention counts. Payload exclusions are explicit in [scope.json](scope.json).
No adapter count or private measurement proves whole-worker RSS, total heap,
descriptor headroom or elapsed performance. The implementing agent reviewed this
slice; independent review remains pending.

Sources, compilation, caches, logs and general scratch stayed on disk. Storage
fixtures were compiled on disk before owned, auto-cleaned tmpfs runtime checks.
Full-storage 50 ms samples observed lower-bound peaks of {peak['bytes']} bytes and
{peak['files']} files; these are not exact maxima. Source measurement also used
precompiled code and an owned cleaned fixture directory. No Pi, Docker or raw
workstation storage probe was used. The reserved backup recovery worktree and
installed service/kernel/capability guards were preserved.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files to the qualified commit. Fixed-timestamp gzip
preserves every stdout/stderr byte. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk log paths and the retained helper;
compile storage fixtures before their tmpfs runtime checks.

Next is the concrete budget across all retained owners, then independent worker
supervision. Engine/configuration integration, complete physical consumers and
claims, actual pool/share state, locks, effects, recovery, current native and
installed ARM64 qualification stay open. The frozen signed ARM64 candidate
contains no current storage profile. P04, defect, performance and cutover gates
remain open.
""")
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()}
(out/'artifact-sha256.json').write_text(json.dumps({'qualified_source':qualified,'files':entries},indent=2)+'\n')
print(json.dumps({'packet':str(out),'source_files':len(bindings),'rust_cases':rust,'harness_cases':61,'artifact_files':len(entries)}))

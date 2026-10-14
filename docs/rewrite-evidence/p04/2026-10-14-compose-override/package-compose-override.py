# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind lossless typed Compose compiler acceptance to exact committed source."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-override-compiler'
logs = root / '.cache/rewrite-compose-override/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-override'
qualified = '85e6bf2f3aca7dedb621036aca87ac4ea9ded1bd'
accepted = ('test-rest', 'storage-compile', 'storage-fixtures', 'compiler-focused',
            'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')
diagnostics = ('compiler-initial', 'compiler-reviewed')
checks, raws = [], {}
for name in accepted + diagnostics:
    check = json.loads((logs / (name + '.json')).read_text())
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    assert check['exit'] == 0
    check['accepted_exact_source'] = name in accepted
    if check['accepted_exact_source']:
        assert check['qualified_source'] == qualified
        for path, digest in check['runtime_hashes'].items():
            committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
            assert hashlib.sha256(committed).hexdigest() == digest
            assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    checks.append(check)
    raws[name] = raw
full_names = ('test-rest', 'storage-fixtures')
rust = sum(c['rust_cases'] for c in checks if c['name'] in full_names)
doctests = sum(int(n) for name in full_names for n in re.findall(
    rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed', raws[name], re.S))
assert rust == 813 and doctests == 33
focused = next(c['rust_cases'] for c in checks if c['name'] == 'compiler-focused')
assert focused == 18
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness'])[1]) == 61
assert raws['arm64-harness'].rstrip().endswith(b'OK')
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-compose-management/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/domain/src/compose_override.rs',
    'crates/domain/src/compose_override/tests.rs', 'tests/fixtures/compose-override/new-service.override.json',
    'tests/fixtures/compose-override/merge-delta.override.json'}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 340
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 20, 'focused_cases': focused, 'focused_cases_already_in_workspace_total': True,
    'focused_cases_kind': 'unit tests; two new compile-fail cases run in full workspace',
    'arm64_harness_cases': 61, 'opt_in_feature': 'limeos-executor/read-worker-probe', 'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified, 'files': bindings}, indent=2) + '\n')
scope = {'qualified_source': qualified, 'parent_main': '746eaec',
    'surface': 'pure unregistered conservative typed managed Compose override proposals',
    'inputs': 'bounded supplied P03 operator/current/desired declarations',
    'actual_protected_file_parsing_or_retained_owner_binding': False,
    'normalization_preceded_by_borrowed_collection_and_string_admission': True,
    'current_and_desired_must_be_representable_against_same_operator': True,
    'complete_replacement_managed_file': 'limeos.override.json',
    'operator_file_serialized_or_modified': False,
    'supported_merge_rules': ['service maps', 'ports IP/target/published/protocol',
        'mount target with retained kind', 'device sequence additions'],
    'refused_inherited_changes': ['service removal', 'port removal/remapping',
        'mount target removal', 'mount kind switch', 'device removal/remapping',
        'numeric user clearing', 'host network clearing'],
    'managed_only_additions_can_be_omitted_from_replacement': True,
    'numeric_uid_gid_and_literal_container_paths_preserved': True,
    'bind_create_host_path': False, 'device_permissions': 'rwm',
    'deduplicated_named_volume_definitions': 'empty standard definitions only',
    'service_limit': 16, 'per_service_port_mount_device_limit': 8,
    'named_volume_declaration_limit': 128, 'compact_artifact_bytes': 65536,
    'borrowed_compact_summary_bytes': 262144,
    'byte_ceiling_checked_before_extending_artifact': True,
    'peak_allocator_rss_and_installed_resource_qualification': 'pending',
    'shutdown_required_names_are_receipts_or_safe_delete_authority': False,
    'full_p03_service_impact_and_desired_elevation': True,
    'unresolved_gates': ['supported_source_admission', 'retained_file_binding',
        'engine_runtime_binding', 'physical_mounts_and_consumers', 'principal_and_fresh_approval',
        'stack_and_storage_locks', 'atomic_write_and_recovery', 'shutdown_and_effect_time_verification'],
    'plan_and_summary_deserialize': False,
    'new_local_cases': '18 unit regressions and two compile-fail import cases',
    'independent_test_side_merge_oracle_is_installed_compose_conformance': False,
    'actual_yaml_environment_includes_interpolation_and_pinned_compose_conformance': 'pending',
    'cli_rpc_routes_generated_wire_schemas_core_store_and_dependency_versions_changed': False,
    'claims_locks_writes_effects_recovery_or_worker_model_added': False,
    'service_guards_or_package_activation_changed': False,
    'independent_review': 'pending',
    'pi_ssh_docker_invocation_raw_block_probe_or_guest_used': False,
    'reserved_backup_and_external_qualification_paths_touched': False,
    'signed_arm64_candidate_contains_slice': False,
    'p04_defect_footprint_performance_or_cutover_gates_closed': False,
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-compose-override-check.py', 'package-compose-override.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Typed Compose override compiler: local acceptance

Qualified source `{qualified}`, based on local main `746eaec`.
The pure compiler produces conservative replacement managed JSON from supplied
typed operator/current/desired projects. Actual file admission, retained binding
and fresh executable approval remain unresolved. See the
[design](../../../plans/2026-10-14-compose-override-compiler-design.md) and
[interface guide](../../../p04-compose-override-compiler.md).

**813 Rust cases pass:** 780 unit/integration cases and 33 compile-fail doctests.
The 20 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, with strict feature all-target Clippy,
fmt, contracts and repository checks. [checks.json](checks.json) binds commands,
environments, source hashes and complete merged stdout/stderr. Accepted runtime
stays unchanged through documentation and evidence edits.

18 unit cases cover two byte-exact synthetic artifacts, numeric UID/GID including
zero and u32 maximum, immutable image digests, long-form ports/volumes/binds,
explicit false read-only/privileged values, host mode and explicit device rwm.
An independent test-side JSON merge oracle checks expected supported combinations
without using production representability/delta helpers. It qualifies no pinned
installed Compose binary or arbitrary YAML behavior.

Single managed-service removal preserves unrelated operator and managed rows;
last-managed-service removal retains shutdown requirements and every unresolved
gate. Inherited service/port/mount/device removal, port/device remapping, bind/
volume kind switches and clearing inherited numeric user/host mode refuse.
Managed-only additions can disappear when replacing the whole managed file.
The existing P03 catalog keeps complete impacts and elevated privileges, without
reusing its preview approval. Invalid/unrepresentable current projects refuse too.

Ordering/IP normalization preserve borrowed input. All collection/string ceilings
refuse before cloning. Invalid desired paths/images, wildcard-port conflicts and
mount/device collisions retain P03 validation. Maximum sixteen-service/eight-mount
fixtures cover shared-name deduplication and 128 distinct volume declarations.
Oversized compact artifacts refuse; capped writers accept exact limits and never
extend/count beyond them. Two compile-fail cases prevent plan/report import.

Bounds are sixteen services/eight ports/mounts/devices per service, at most 128
named volume declarations, 64 KiB artifacts and 256 KiB counted borrowed summaries.
These are admission ceilings, not peak allocator/RSS or installed headroom proof.
The operator document is never serialized or written. No YAML body, reset/override
tag, null deletion, environment, include or interpolation adapter is added.

The initial 136-case domain suite passed. Final review added the 128-distinct-volume
boundary and an invalid-desired regression; the reviewed 137-case precommit suite
passed. Both logs remain lossless diagnostics, distinct from exact committed-source
acceptance. No failed check was discarded. The implementing agent reviewed the
code; independent review remains pending.

Actual supported source admission, retained file/Engine mapping, complete physical
consumers, principal/fresh approval, locks, atomic writes/recovery and verified
shutdown/effect-time checks remain unresolved. Shutdown names are requirements,
not receipts or deletion authority. Supplied current facts are no retained recovery
before-image. No CLI/RPC/route/schema/Core/Store/service/package/worker activation
or dependency change is included. P04/defect/installed/performance/cutover gates
remain open; the frozen signed ARM64 candidate remains unchanged.

Build/cache/general scratch stayed on disk. Existing storage fixtures compiled
there before small owned auto-cleaned tmpfs runtime scratch. 50 ms samples observed
lower-bound peaks of {peak['bytes']} bytes/{peak['files']} files, not exact maxima.
No Pi, SSH, Docker invocation, raw block probe, guest, reserved backup or external
qualification path was used. Service guards remain 256 FDs/64 MiB/zero swap/16 tasks.
Historical inventory p95 61.401 ms still exceeds the 20 ms target.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source, fixture,
contract and script files. Fixed-timestamp gzip preserves every log byte with raw/
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce against the qualified source using the retained helpers and fresh work/
log/output paths. Compile storage fixtures before their small tmpfs runtime run.

Next: supported source admission and independent Compose conformance, followed by
a fresh approved lifecycle operation through the existing locks/recovery boundary.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified, 'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust,
                  'harness_cases': 61, 'artifact_files': len(entries)}))

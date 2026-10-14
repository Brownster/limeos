# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind strict JSON admission and offline parser conformance to committed source."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-json-source'
logs = root / '.cache/rewrite-compose-source/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-source'
qualified = '2c68557c112b4caf10fe0f502a73580edfd7f411'
accepted = tuple(name + '-final' for name in ('test-rest', 'storage-compile', 'storage-fixtures', 'source-focused',
            'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')) + ('compose-conformance-accepted',)
diagnostics = ('source-initial', 'source-reviewed', 'compose-conformance', 'object-shape-red',
               'object-shape-green', 'enum-shape-red', 'enum-shape-green', 'source-hardened',
               'test-rest', 'storage-compile', 'storage-fixtures', 'source-focused', 'clippy',
               'fmt', 'contracts', 'repository', 'arm64-harness', 'compose-conformance-final')
checks, raws = [], {}
for name in accepted + diagnostics:
    check = json.loads((logs / (name + '.json')).read_text())
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    assert check['exit'] == (101 if name in ('object-shape-red', 'enum-shape-red') else 0)
    check['accepted_exact_source'] = name in accepted
    if check['accepted_exact_source']:
        assert check['qualified_source'] == qualified
        for path, digest in check['runtime_hashes'].items():
            committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
            assert hashlib.sha256(committed).hexdigest() == digest
            assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    checks.append(check)
    raws[name] = raw
full_names = ('test-rest-final', 'storage-fixtures-final')
rust = sum(c['rust_cases'] for c in checks if c['name'] in full_names)
doctests = sum(int(n) for name in full_names for n in re.findall(
    rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed', raws[name], re.S))
assert rust == 836 and doctests == 35
focused = next(c['rust_cases'] for c in checks if c['name'] == 'source-focused-final')
assert focused == 21
for name, count in [('arm64-harness-final', 61), ('compose-conformance-accepted', 6)]:
    assert int(re.search(rb'Ran (\d+) tests', raws[name])[1]) == count
    assert b'skipped' not in raws[name] and b'OK' in raws[name]
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-compose-override/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-container/src/compose_source.rs',
    'crates/executor-container/src/compose_source/tests.rs', 'tests/fixtures/compose-source/operator.json',
    'tests/fixtures/compose-source/conformance-pin.json', 'tests/fixtures/compose-source/conformance-cases.json',
    'tests/qualification/compose/test_json_conformance.py'}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 346
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 23, 'focused_cases': focused, 'focused_cases_already_in_workspace_total': True,
    'focused_cases_kind': 'unit tests; two new compile-fail cases run in full workspace',
    'arm64_harness_cases': 61, 'native_compose_conformance_cases': 6,
    'opt_in_feature': 'limeos-executor/read-worker-probe', 'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified, 'files': bindings}, indent=2) + '\n')
pin = json.loads((work / 'tests/fixtures/compose-source/conformance-pin.json').read_text())
scope = {'qualified_source': qualified, 'parent_main': '23e5564',
    'surface': 'pure unregistered strict Compose JSON source admission and typed combination',
    'inputs': 'one actual operator byte slice and optional actual managed byte slice',
    'supported_format': 'closed JSON subset, not arbitrary YAML or canonical config output',
    'raw_document_bytes': 65536, 'service_limit': 16, 'per_service_port_mount_device_limit': 8,
    'combined_standard_named_volume_limit': 128, 'borrowed_compact_summary_bytes': 196608,
    'strings_maps_and_arrays_bounded_before_owned_clone_or_excess_value_decoding_growth': True,
    'all_declared_services_and_volumes_including_overridden_unused_validated': True,
    'all_struct_boundaries_require_object_shape_and_enum_fields_require_literal_strings': True,
    'unknown_duplicate_escaped_equivalent_null_wrong_missing_and_trailing_fields': 'whole-request refusal',
    'operator_independently_valid_and_combined_named_volume_references_declared': True,
    'supported_omitted_scalar_default_and_inheritance_with_explicit_false_override': True,
    'canonical_numeric_uid_gid_and_published_ports': True,
    'explicit_mount_read_only_and_bind_no_creation': True,
    'device_permissions': 'rwm', 'network_mode': 'host only',
    'inherited_mount_kind_switch_and_any_device_target_overlap': 'conservative refusal',
    'combined_counts_conflicts_and_resource_keys_validated_without_truncation': True,
    'actual_raw_byte_counts_and_sha256_retained_without_bodies': True,
    'plan_generated_artifact_admitted_and_combination_checked_against_expected_desired': True,
    'owner_and_summary_deserialize': False, 'existing_compiler_unresolved_gates_removed': False,
    'host_path_environment_command_or_engine_io_in_library': False,
    'retained_descriptor_root_namespace_lease_or_management_ownership': False,
    'physical_consumers_principal_permission_human_approval_claims_locks_writes_recovery': False,
    'shutdown_requirements_are_receipts_or_safe_delete_authority': False,
    'new_local_cases': '21 Rust unit cases and two compile-fail import cases',
    'independent_offline_parser_cases': 6, 'native_parser_pin': pin,
    'native_parser_path': '/usr/libexec/docker/cli-plugins/docker-compose',
    'native_parser_observed_owner_uid': 65534, 'native_parser_observed_mode': '0755',
    'native_parser_observed_bytes': 32521840, 'native_parser_publisher_or_protected_launch_trust': False,
    'native_commands': ['version --short', 'config --format json --no-env-resolution --no-interpolate --no-path-resolution'],
    'native_environment': 'cleared; fixed PATH, private DOCKER_CONFIG, COMPOSE_DISABLE_ENV_FILE=1, nonexistent Unix endpoint',
    'native_explicit_project_name_directory_and_scratch_file_list': True,
    'native_binary_hash_checked_before_and_after_each_case': True,
    'native_actual_canonical_outputs_retained': True,
    'native_ambient_dotenv_trap_files_credentials_and_input_files_unchanged': True,
    'native_daemon_up_pull_build_or_image_resolution': False,
    'other_compose_versions_arm64_install_and_yaml_environment_includes_interpolation': 'pending',
    'peak_allocator_rss_installed_resource_and_independent_review': 'pending',
    'cli_rpc_routes_generated_wire_schemas_core_store_and_dependency_versions_changed': False,
    'worker_model_service_guards_or_package_activation_changed': False,
    'pi_ssh_production_docker_raw_block_probe_or_guest_used': False,
    'reserved_backup_and_external_qualification_paths_touched': False,
    'signed_arm64_candidate_contains_slice': False,
    'p04_defect_footprint_performance_or_cutover_gates_closed': False,
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
(out / 'diagnosis.json').write_text(json.dumps({
    'passed_pre_hardening_source': 'dd284070f6600d4225e1e8882303596a3757729f',
    'qualified_hardened_source': qualified,
    'struct_shape_failure': {'log': 'object-shape-red.log.gz', 'exit': 101,
        'input': {'services': {}, 'volumes': {'cache': []}},
        'cause': 'serde_json deserialize_struct accepts array or map; derived empty struct supplied a sequence visitor',
        'fix': 'Object<T> requires deserialize_map then delegates through MapAccessDeserializer',
        'corrective_log': 'object-shape-green.log.gz'},
    'enum_shape_failure': {'log': 'enum-shape-red.log.gz', 'exit': 101,
        'input': {'services': {'api': {'restart': {'always': None}}}},
        'cause': 'serde_json deserialize_enum accepts string or externally tagged object for unit variants',
        'fix': 'EnumText<T> requires a bounded literal string before StrDeserializer',
        'corrective_log': 'enum-shape-green.log.gz'},
    'primary_local_sources': ['serde_json-1.0.145/src/de.rs deserialize_struct/map/enum',
        'serde_derive-1.0.228/src/de/struct_.rs visit_seq generation'],
    'hardened_focused_cases': 21, 'production_effects_added': False,
    'earlier_passed_checks_retained_as_diagnostics': True}, indent=2) + '\n')
for name in ('run-compose-source-check.py', 'package-compose-source.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures-final')
(out / 'README.md').write_text(f"""# Strict Compose JSON sources: local acceptance

Qualified source `{qualified}`, based on local main `23e5564`.
The pure adapter admits actual closed-subset JSON byte slices, combines operator/
managed declarations and re-admits compiled overrides against expected projects.
It acquires no file descriptor, lease, management authority or executable approval.
See the [design](../../../plans/2026-10-14-compose-json-source-design.md) and
[interface guide](../../../p04-compose-json-source.md).

**836 Rust cases pass:** 801 unit/integration cases and 35 compile-fail doctests.
The 23 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests and six independent offline parser cases pass**,
with strict feature all-target Clippy, fmt, contracts and repository checks.
[checks.json](checks.json) binds commands, environments, source hashes and complete
merged stdout/stderr. Accepted source stays unchanged through documentation edits.

21 unit cases cover compiler artifacts and actual operator bytes, scalar defaults/
inheritance/false overrides, unrelated services, single managed-service removal,
shutdown requirements, every unresolved gate, stable plans and whitespace-sensitive
raw hashes. Unknown/duplicate/escaped/null/wrong/missing fields refuse at all supported
levels. Canonical u32 users/published ports, explicit mount modes, literal bind/device
paths, no host-directory creation, closed permissions/network modes, volume references,
unused definitions, image pinning, target and wildcard-port conflicts pass/refuse as
specified. Invalid overridden operator data cannot disappear through a managed file.

Bounded visitors refuse excess maps/arrays before the next value is decoded and
strings before owned clones. Exact 64 KiB raw inputs pass; excess, trailing values,
invalid UTF-8 and general YAML refuse. Combined 16/17-service, 8/9-resource and
128/129-volume boundaries refuse excess without truncation; equal port keys,
same-kind mount replacement and repeated standard declarations fit full sets.
Borrowed summary counting accepts its exact boundary and refuses without increment.
Two compile-fail cases prevent owner/report import. Parsed facts remain declarations.

Six [shared fixtures](../../../../tests/fixtures/compose-source/conformance-cases.json)
check new service emission, mount/port/device merge deltas, false scalar override,
single managed-service removal, host mode and scalar updates preserving sequences.
Rust verifies each artifact against the compiler and source combination. The
independent standalone parser runs only `version`/`config`; actual canonical JSON,
commands, version output/stderr, fixed environment and binary hashes are lossless
in [compose-conformance-accepted.log.gz](compose-conformance-accepted.log.gz).

The observed AMD64 parser is version {pin['parser_version']}, SHA256
`{pin['parser_sha256']}`. The same hash is checked before and after each case.
Explicit scratch files/project, cleared environment, disabled default `.env`,
environment-file/interpolation/path resolution and a nonexistent Unix endpoint
keep daemon effects outside the test. Invalid ambient `.env` traps coexist with
passing checks; private credential directories stay empty and source files unchanged.
Publisher/protected production launch trust, other versions, ARM64 installation,
general YAML and complete frozen-layout support remain unqualified.

The initial 18-case focused run and six-parser-case run passed. Final review added
full-set key replacement and broader visitor boundary assertions; the reviewed
19-case focused run passed. Final review reproduced two Serde format loopholes:
derived structs accepted array representation (including an empty volume array),
and unit enums accepted tagged objects (including object-shaped restart). The two
failed targeted logs and passing corrective logs remain lossless. An object-only
map visitor now guards every Compose object boundary; a bounded string-only enum
visitor guards restart/network mode/port protocol/mount kind. The hardened 21-case
focused run passes. All earlier passed full-source checks remain diagnostics;
final acceptance runs against the exact hardened source. No failed check was
discarded. The implementing agent reviewed the code; independent review is pending.

Limits are 64 KiB per source, sixteen services/eight ports/mounts/devices and 128
standard volume declarations across the combination, with 192 KiB counted borrowed
summary. These are admission ceilings, not peak allocator/RSS or installed headroom
proof. Bodies are dropped; hashes establish neither freshness nor recovery custody.
The existing compiler's unresolved gates remain unchanged. Source-to-retained-file/
Engine binding, physical completeness, fresh approval, claims/locks/writes/recovery
and verified shutdown remain open. No CLI/RPC/route/schema/Core/Store/package/service/
worker activation or dependency change is included. The signed candidate is unchanged.

Build/cache/general scratch and offline parser fixtures stayed on disk. Existing
storage fixtures compiled there before small owned auto-cleaned tmpfs runtime scratch.
50 ms samples observed lower-bound peaks of {peak['bytes']} bytes/{peak['files']} files,
not exact maxima. No Pi, SSH, production Docker, raw block probe, guest, reserved
backup or external qualification path was used. Guards remain 256 FDs/64 MiB/zero
swap/16 tasks. Historical inventory p95 61.401 ms still exceeds 20 ms; no P04/defect/
installed/resource/performance/cutover gate closes.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source/fixture/
contract/script files. Fixed-timestamp gzip retains every check log byte with raw/
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce against the qualified source using retained helpers and fresh work/log/
output paths. Compile storage fixtures before their small tmpfs runtime run.

Next: integrate supported byte admission with genuine retained catalog/file/Engine
owners under original bounds, then issue a fresh approved lifecycle operation
through the existing locks/recovery boundary.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified, 'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust,
                  'harness_cases': 61, 'native_parser_cases': 6, 'artifact_files': len(entries)}))

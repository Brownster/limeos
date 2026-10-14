# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind lossless local declared Compose management acceptance to committed source."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-management-binding'
logs = root / '.cache/rewrite-compose-management/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-management'
qualified = '9504baf58800cf997c98283bbbe2efc421f284a7'
accepted_names = tuple(name + '-final' for name in ('test-rest', 'storage-compile', 'storage-fixtures', 'management-focused',
                  'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness'))
diagnostic_names = ('management-initial', 'catalog-diagnostic', 'stopped-diagnostic', 'management-reviewed',
                    'test-rest', 'storage-compile', 'storage-fixtures', 'clippy', 'fmt', 'repository', 'arm64-harness')
checks, raws = [], {}
for name in accepted_names + diagnostic_names:
    check = json.loads((logs / (name + '.json')).read_text())
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    check['accepted_exact_source'] = name in accepted_names
    if check['accepted_exact_source']:
        assert check['exit'] == 0 and check['qualified_source'] == qualified
        for path, digest in check['runtime_hashes'].items():
            committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
            assert hashlib.sha256(committed).hexdigest() == digest
            assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    elif name == 'management-initial':
        assert check['exit'] == 101 and b'137 passed; 2 failed' in raw
    else:
        assert check['exit'] == 0
    checks.append(check)
    raws[name] = raw
full_names = ('test-rest-final', 'storage-fixtures-final')
rust = sum(c['rust_cases'] for c in checks if c['name'] in full_names)
doctests = sum(int(n) for name in full_names for n in re.findall(
    rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed', raws[name], re.S))
assert rust == 793 and doctests == 31
focused = next(c['rust_cases'] for c in checks if c['name'] == 'management-focused-final')
assert focused in (30, 34)
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-final'])[1]) == 61
assert raws['arm64-harness-final'].rstrip().endswith(b'OK')
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-compose-files/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-container/src/compose_management.rs',
    'crates/executor-container/src/compose_management/catalog.rs', 'crates/executor-container/src/compose_management/tests.rs',
    'crates/executor-container/src/compose_management/catalog/tests.rs',
    'crates/executor-container/src/docker/process_evidence/tests/management_tests.rs', 'tests/fixtures/compose-management/catalog.json'}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 336
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 34, 'focused_cases': focused, 'focused_cases_already_in_workspace_total': True,
    'arm64_harness_cases': 61, 'opt_in_feature': 'limeos-executor/read-worker-probe', 'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified, 'files': bindings}, indent=2) + '\n')
scope = {'qualified_source': qualified, 'parent_main': 'cbae3a8',
    'surface': 'unregistered retained single selected declared Compose management binding',
    'management_configuration_distinct_from_p03_catalog_approval_and_executable_authority': True,
    'installed_default_catalog': False, 'selected_stack_only_actual_files_bound': True,
    'actual_effective_uid_0_before_callback_namespace_target_and_canonical_engine_access': True,
    'public_child_uid_endpoint_root_clock_digest_service_or_numeric_limit_overrides': False,
    'catalog_version': 1, 'registered_stacks': 8, 'services_per_stack': 16,
    'all_registrations_including_unselected_validated': True,
    'catalog_unknown_duplicate_missing_malformed_or_excess_fields': 'whole-request refusal',
    'managed_absent_variant_closed': 'empty struct, extra fields refuse',
    'catalog_raw_bytes': 16384, 'catalog_metadata_bytes': 4096, 'catalog_retained_fds': 18,
    'catalog_original_requested_read_bytes': 1048576,
    'component_disjoint_stack_directories_and_unique_stack_project_service_names': True,
    'genuine_catalog_file_engine_owners_retained': True,
    'expected_engine_root_mount_view_directory_ordered_files_hashes_and_services_match': True,
    'complete_original_full_container_replica_oneoff_stopped_mountless_and_unrelated_rows': True,
    'differently_named_project_overlap_with_selected_paths': 'whole-request refusal',
    'registered_zero_members': 'past observation, no safe absence/deletion authority',
    'original_composition_lifetime_seconds': 4, 'child_leases_and_spent_ledgers_renew': False,
    'catalog_file_bracket_genuine_engine_revalidation': True,
    'failure_cancel_drop_timeout_reentrancy_concurrency_unwind_permanently_invalidates': True,
    'child_owner_accessors': False, 'borrowed_compact_report_bytes': 81920,
    'retained_fd_preflight': 38, 'snapshot_clone_or_report_buffer_for_serialization': False,
    'existing_engine_closed_profile_and_old_public_file_interfaces_changed': False,
    'new_local_cases': ['15 genuine retained-owner composition/listener cases',
        '11 catalog parser/protected file cases', 'four management configuration/boundary cases',
        'four compile-fail owner/report cases'],
    'successful_fixture_policy': 'explicitly private scratch root/UID/clock and real owned authenticated Unix listeners',
    'public_root_positive_foreign_namespace_and_installed_qualification': 'pending',
    'independent_daemon_physical_host_user_pid_namespace_identity': False,
    'writer_transaction_principal_permission_human_approval_or_acquired_claims': False,
    'actual_yaml_environment_includes_interpolation_or_combined_compose_semantics': False,
    'claims_locks_effects_recovery_or_worker_model_added': False,
    'generated_wire_schema_core_store_route_command_service_guard_or_package_activation_changed': False,
    'dependency_versions_changed': False, 'independent_review': 'pending',
    'pi_ssh_real_docker_raw_block_probe_or_guest_used': False,
    'reserved_backup_and_external_qualification_paths_touched': False,
    'signed_arm64_candidate_contains_slice': False,
    'p04_defect_footprint_performance_or_cutover_gates_closed': False,
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
(out / 'diagnosis.json').write_text(json.dumps({'initial_log': 'management-initial.log.gz',
    'exit': 101, 'passed_before_failure': 137, 'failed': 2,
    'catalog_issue': 'Serde internally tagged unit absent accepted extra sha256; empty struct variant now rejects it',
    'fixture_issue': 'first stopped fixture container retained PID 1000; fixture now uses zero with production guard unchanged',
    'targeted_corrective_logs': ['catalog-diagnostic.log.gz', 'stopped-diagnostic.log.gz'],
    'reviewed_precommit_crate_cases': 149,
    'final_test_hardening': 'use sixty-second future wall time, avoiding a real second-boundary race in the rollback fixture',
    'passed_pre_hardening_source': '50e4b882fd4fbd50cc418382a42a8ef2a0732825',
    'production_runtime_changed_by_test_hardening': False,
    'accepted_runtime': qualified}, indent=2) + '\n')
for name in ('run-compose-management-check.py', 'package-compose-management-v1.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures-final')
(out / 'README.md').write_text(f"""# Declared Compose management binding: local acceptance

Qualified runtime `{qualified}`, based on local main `cbae3a8`.
The unregistered reader binds one selected protected management registration to
genuine retained catalog/file and authenticated Engine owners. This establishes
declared management intent, not principal permission or executable approval.
See the [design](../../../plans/2026-10-14-compose-management-binding-design.md)
and [interface guide](../../../p04-compose-management-binding.md).

**793 Rust cases pass:** 762 unit/integration cases and 31 compile-fail doctests.
The 34 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict feature all-target Clippy, fmt,
contracts and repository checks. [checks.json](checks.json) binds commands,
environments, source hashes and complete lossless merged stdout/stderr. Accepted
runtime remains unchanged through documentation and evidence edits.

15 owned authenticated-listener cases compose genuine owners for complete full
container identities, replicas, one-offs, stopped/mountless/unrelated/non-Compose
rows, zero observed registered members, exact expected Engine/path/file-order/
digest/service matching and differently named project overlap refusal. All four
operator basenames and explicit managed absence/presence bind. Different genuine
file roots refuse. Fresh catalog/file/Engine changes, late cancellation, dropped
validation connection EOF, original timeout, concurrency, reentrancy and unwind
permanently invalidate the binding; retries send no new HTTP. Original wall time,
borrowed report bytes, secret omission and GET-only requests pass. Public nonroot
binding refusal precedes callback, target and canonical Engine access.

11 catalog cases cover closed/missing/unknown/duplicate/escaped fields, bounded
strings/counts before excess value decoding/growth, invalid unselected entries,
literal/disjoint roots, names/digests/paths, explicit managed state, protected
mode/ownership/symlink/FIFO/directory/hardlink/missing-file refusal, raw whitespace
and named/ancestor replacements including late post-hash changes, exact raw/report/
descriptor limits, unrelated entries, original age/read budget, cancellation,
reentrancy/unwind and public nonroot refusal. Four configuration/boundary cases
cover selected identity/order, P03/secret separation, literal component overlaps
and unsupported override/approval fields. Four compile-fail cases prevent imported
catalog/binding reports or owners from reconstructing protected observations.

The failed first crate run (137 pass/two fail) remains lossless with source hashes
and [diagnosis.json](diagnosis.json). Internally tagged unit `absent` accepted an
extra digest; an empty struct variant now refuses it. The all-stopped fixture gave
its first container PID 1000; it now uses zero, with production PID guards intact.
Both targeted corrective runs and the reviewed 149-case precommit suite pass.
They remain diagnostics rather than exact-source acceptance. The passed first
full-source checks also remain lossless. Final review hardens the wall rollback
fixture from one to sixty seconds ahead, so a real second boundary cannot change
its expected refusal. Production timing rules are unchanged; final acceptance
reruns against the exact hardened source.

Successful fixtures use explicitly private scratch-root/UID/clock substitutions
through production traversal/hash code and real kernel-authenticated owned Unix
listeners. Public root-positive, foreign-namespace, installed/confinement and
independent daemon/physical-host qualification remain open. Management config is
separate from P03 preview grants/approvals. Unselected registrations are validated,
but their actual files are not bound. Unrelated Engine rows remain visible.

Fixed bounds are 16 KiB raw catalog/4 KiB metadata, eight registrations/sixteen
services, 18 catalog FDs, original 1 MiB requested-read ledger, four-second child/
composition lifetime and two-second cooperative catalog/file work passes. Child
ledgers and issue times never renew. Preflight 38 retained FDs and count one borrowed
80 KiB compact report without cloning snapshots or buffering its bytes. Existing
Engine/file caps stay unchanged. These are code/resource bounds, not an installed
whole-worker/peak allocator/RSS/cgroup observation. Kernel calls still need
independent supervision; multiple file/Engine checks are no writer transaction.

Actual YAML/.env/includes/interpolation/combined Compose semantics, complete
physical consumers, principal permissions, fresh executable approval, acquired
claims/locks/effects/recovery, worker integration and independent review remain
open. The implementing agent reviewed this slice; independent review is pending.
Generated/wire schemas, P03/Core/Store, service guards, dependency versions and
the frozen signed ARM64 candidate remain unchanged. No catalog or service is
installed. Root/installed/resource/performance qualification and cutover remain open.

Build/cache/new file fixtures and general scratch stayed on disk. Existing storage
fixtures compiled there before small owned auto-cleaned tmpfs runtime scratch.
50 ms samples observed lower-bound peaks of {peak['bytes']} bytes/{peak['files']} files,
not exact maxima. No Pi, SSH, real Docker, raw block probe, guest, reserved backup
or external qualification path was used. Service guards remain 256 FDs, 64 MiB,
zero swap and 16 tasks. Historical inventory p95 61.401 ms exceeds 20 ms; no P04,
defect, footprint, performance or cutover gate closes.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source, fixture,
contract and script files. Fixed-timestamp gzip preserves all log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce against the qualified source with fresh work/log/output paths using the
retained helpers; compile storage fixtures before their small tmpfs runtime run.

Next is actual supported combined Compose semantics and approved lifecycle planning;
declared management binding supplies no effect authority or safe-unmount result.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified, 'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust,
                  'harness_cases': 61, 'artifact_files': len(entries)}))

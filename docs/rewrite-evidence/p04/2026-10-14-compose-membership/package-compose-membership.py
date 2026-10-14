# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package lossless exact-source local Compose membership acceptance."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-membership'
logs = root / '.cache/rewrite-compose-membership/accepted-logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-membership'
qualified = 'a17d1f3bdd91b9cda72c8d0ff0b4f9c37bab15cf'
names = ('test-rest-accepted', 'storage-compile-on-disk', 'storage-fixtures',
         'compose-focused', 'clippy-accepted', 'fmt-accepted',
         'contracts-accepted', 'repository-accepted', 'arm64-harness-accepted')
checks, raws = [], {}
for name in names:
    check = json.loads((logs / (name + '.json')).read_text())
    assert check['exit'] == 0 and check['qualified_source'] == qualified
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    for path, digest in check['runtime_hashes'].items():
        committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
        assert hashlib.sha256(committed).hexdigest() == digest
        assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    checks.append(check)
    raws[name] = raw
accepted = ('test-rest-accepted', 'storage-fixtures')
rust = sum(c['rust_cases'] for c in checks if c['name'] in accepted)
doctests = sum(int(n) for name in accepted for n in re.findall(
    rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed', raws[name], re.S))
assert rust == 732 and doctests == 24
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert next(c['rust_cases'] for c in checks if c['name'] == 'compose-focused') == 24
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-storage-impact/source-manifest.json').read_text())
files = set(previous['files']) | {
    'crates/executor-container/src/docker/compose_evidence.rs',
    'crates/executor-container/src/docker/compose_evidence/tests.rs',
    'crates/executor-container/src/docker/process_evidence/tests/compose_tests.rs',
}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 326
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({
    'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 26, 'opt_in_feature': 'limeos-executor/read-worker-probe',
    'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61,
    'process_and_socket_fixtures_execution': 'approved local nonroot tests outside sandbox',
    'checks': checks,
}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': bindings,
}, indent=2) + '\n')
scope = {
    'qualified_source': qualified, 'parent_main': '4119b3c',
    'surface': 'unregistered fixed-endpoint retained Engine-declared Compose membership',
    'fixed_endpoint': '/run/docker.sock', 'actual_effective_uid_0_required_before_endpoint_access': True,
    'public_uid_endpoint_clock_profile_cap_selectors': False,
    'peer_policy': 'existing root socket and kernel peer UID policy, before HTTP on every connection',
    'independent_daemon_pid_or_host_namespace_identity': False,
    'metadata_reference': 'Docker Compose v2.29.7 label definitions and project-label construction',
    'selected_fields': ['full original container snapshot', 'project', 'service', 'one-off',
                        'working directory', 'ordered declared config-file names'],
    'complete_noncompose_stopped_mountless_membership': True,
    'unknown_labels_environment_and_credentials_reported': False,
    'names_bytes': 64, 'path_bytes': 512, 'path_components': 16, 'ordered_config_files': 4,
    'project_name_merges_different_directory_or_file_order': False,
    'label_names_establish_trusted_files_or_argv': False,
    'hook_relay_and_partial_metadata': 'whole-request refusal',
    'duplicate_json_keys': 'new Compose mode rejects before repeated value; old modes unchanged',
    'same_original_storage_pid_owner_and_socket_retained': True,
    'initial_double_inspection_and_fresh_full_comparison': True,
    'original_wall_monotonic_lifetime_seconds': 5,
    'new_owner_failure_cancel_reentrancy_permanently_invalidates': True,
    'base_accessor_bypasses_compose_validation': False,
    'existing_interfaces_and_reports_unchanged': True,
    'complete_consumers': 16, 'running_consumers': 8, 'selected_mounts': 32,
    'http_lifetime_bytes': 2097152, 'counted_live_payload_bytes': 2097152,
    'total_storage_compose_compact_report_bytes': 49152,
    'projection_data_drops_before_reservation': True,
    'new_local_facts': ['nine metadata/validation-state cases', 'two strict duplicate-key mode cases',
                        'thirteen async collection/refusal cases', 'two compile-fail snapshot/owner cases',
                        'complete replicas/one-offs/non-Compose/running/stopped/mountless membership',
                        'malformed/partial/ambiguous labels and selected label changes refuse',
                        'full base fact changes and original age refuse',
                        'cancelled validation closes its connection and poisons',
                        'simultaneous validations both refuse and cannot restore readiness',
                        'original counts/report/lifetime bytes remain closed',
                        'counted payload returns after owner drop',
                        'public nonroot refusal before canonical endpoint access',
                        'root policy and wrong real kernel peer refuse before HTTP'],
    'successful_fixture_authentication': 'explicitly private unit UID/clock substitutions on owned listeners',
    'public_root_positive_collection': 'unqualified', 'installed_version_and_confinement': 'unqualified',
    'current_pi_larger_running_membership_qualified': False, 'peak_rss_cgroup_headroom_qualified': False,
    'independent_review': 'pending',
    'actual_stack_files_management_identity_claims_locks_approvals_effects_recovery_added': False,
    'combined_worker_or_probe_integration_added': False, 'wire_schema_routes_commands_changed': False,
    'service_guards_changed': False, 'dependency_versions_changed': False,
    'pi_accessed': False, 'ssh_used': False, 'docker_accessed': False,
    'raw_block_probe_used': False, 'guest_launched': False,
    'reserved_backup_touched': False, 'external_qualification_paths_touched': False,
    'frozen_signed_arm64_candidate_contains_slice': False,
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20,
    'performance_gate_closed': False, 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-compose-membership-check.py', 'package-compose-membership.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Retained Compose membership: local acceptance

Qualified runtime `{qualified}`, based on local main `4119b3c`.
The unregistered fixed-endpoint UID-0 reader projects complete Engine-declared
project/service/one-off membership through the genuine authenticated collector.
It retains the same original socket, storage/PID facts, lifetime and spent ledgers.
See the [design](../../../plans/2026-10-14-compose-membership-evidence-design.md)
and [interface guide](../../../p04-compose-membership-evidence.md).

**732 Rust cases pass:** 708 unit/integration cases and 24 compile-fail doctests.
The 26 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds commands,
environments, runtime hashes and lossless merged stdout/stderr. Documentation
edits during validation changed no runtime source.

Nine private metadata/validation-state cases cover explicit non-Compose labels,
required fields, exact name/path/file limits, meaningful file order, project-name
ambiguity, full container identity, secret omission, dropped/failed validation
and reentrancy. Two decoder cases prove nested/escaped duplicate-key refusal in
Compose mode while preserving old behavior and allowing equal keys in distinct
objects. Two compile-fail cases prevent importing snapshots or opaque owners.

Thirteen async collection/refusal cases use owned listeners for complete running/stopped/mountless/
non-Compose membership, replicas and one-offs, project-directory/file ambiguity,
malformed/partial metadata, initial-pass and fresh selected-label changes, base
PID/mount/identity/membership/Engine changes, original wall/monotonic expiry,
connection closure on dropped work and permanent invalidation through either
owner accessor. Concurrent validations both refuse; a later success cannot undo
the refusal. GET-only requests and secret omission pass. Real public nonroot
refusal occurs before canonical endpoint access. Root-policy and wrong-kernel-peer
negative fixtures send no HTTP before authorization.

Successful fixture collection uses explicitly private UID/clock substitutions.
It qualifies no public UID-0/installed Engine, independent daemon PID or host
namespace. Project/service/path conventions follow pinned Docker Compose v2.29.7
references linked in the guide; no installed-version compatibility is claimed.
Config-file names are comma-delimited declarations, not trusted argv or files.
Unknown labels/environment are omitted. Hook/relay, partial/malformed metadata
and conflicting selections refuse instead of becoming unmanaged or empty facts.

The existing 16-complete/eight-running/32-selected-mount, JSON shape, 2 MiB
lifetime HTTP and 2 MiB counted live-payload ceilings remain closed. Base and
Compose compact reports together fit the unchanged 48 KiB limit; long valid
metadata plus otherwise qualifying storage facts refuses the total. Revalidation
never replenishes spent input bytes. String/vector/comparison overlap is charged;
projection data drops before its reservation, and final drop returns the ledger.
This is no peak allocator/RSS/cgroup proof or support for the Pi's larger running
count. No service ceiling is raised.

Existing advisory/storage interfaces, snapshots and JSON semantics stay unchanged.
The new reader remains unregistered and outside the combined worker/probe. Labels
do not establish actual protected stack files, management identity, Store claim
keys, complete physical consumers or lifecycle authority. Those prerequisites,
claims/locks/approvals/effects/recovery, independent review, public root/installed
qualification and cutover remain open. Synchronous kernel work still needs
independent supervision. The implementing agent reviewed this slice; independent
review is pending. The frozen signed ARM64 candidate excludes it.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before small owned, auto-cleaned tmpfs runtime scratch. Their 50 ms samples
observed lower-bound peaks of {peak['bytes']} bytes and {peak['files']} files, not exact
maxima. No Pi, SSH, real Docker, raw block probe or guest launch was used. Reserved
backup and external qualification paths stayed untouched. Service guards remain
256 FDs, 64 MiB, zero swap and 16 tasks. Historical inventory p95 61.401 ms remains
above 20 ms; no P04, defect, footprint, performance or cutover gate closes.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source, fixture,
contract and script files. Fixed-timestamp gzip preserves full log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with fresh disk logs using the retained helpers; compile storage
fixtures before their small tmpfs runtime run.

Next is protected managed-stack file/catalog binding before membership can support
complete stack claims or lifecycle planning. Public root/installed qualification
and physical consumer completeness remain separate prerequisites.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': entries,
}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings),
                  'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

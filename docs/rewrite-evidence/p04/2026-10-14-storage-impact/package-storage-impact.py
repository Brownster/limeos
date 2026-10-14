# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package exact-source storage impact acceptance with lossless fixture diagnostics."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/storage-impact-preview'
logs = root / '.cache/rewrite-storage-impact/accepted-logs'
diagnostics = root / '.cache/rewrite-storage-impact/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-storage-impact'
qualified = '4e7f73d71f8cdf84e14489277fa371941bb05332'
names = ('test-rest-accepted', 'storage-compile-on-disk', 'storage-fixtures',
         'impact-focused', 'clippy-accepted', 'fmt-accepted',
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
assert rust == 706 and doctests == 22
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert next(c['rust_cases'] for c in checks if c['name'] == 'impact-focused') == 15
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-combined-read-launch/source-manifest.json').read_text())
files = set(previous['files']) | {
    'crates/domain/src/storage_impact.rs', 'crates/domain/src/storage_impact/tests.rs',
    'tests/fixtures/storage-impact/expected.json',
}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 323
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({
    'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 16, 'opt_in_feature': 'limeos-executor/read-worker-probe',
    'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61,
    'process_and_socket_fixtures_execution': 'approved local nonroot tests outside sandbox',
    'checks': checks,
}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': bindings,
}, indent=2) + '\n')
(out / 'diagnostics').mkdir()
diagnostic = json.loads((diagnostics / 'impact-initial.json').read_text())
compressed = (diagnostics / diagnostic['log']).read_bytes()
assert diagnostic['exit'] != 0
assert hashlib.sha256(compressed).hexdigest() == diagnostic['compressed_sha256']
assert hashlib.sha256(gzip.decompress(compressed)).hexdigest() == diagnostic['raw_sha256']
shutil.copy2(diagnostics / diagnostic['log'], out / 'diagnostics' / diagnostic['log'])
(out / 'diagnostics' / 'impact-initial.json').write_text(json.dumps(diagnostic, indent=2) + '\n')
(out / 'diagnostics' / 'README.md').write_text(
    '# Regression fixture prerequisite refusal\n\n'
    'The first local working-source run passed 14 cases and failed one before\n'
    'impact selection: the added second pool copied the first pool name and\n'
    'contained only one branch. Frozen pool validation requires unique names\n'
    'and at least two branches. The corrected fixture uses a unique name and\n'
    'two disjoint branches on the related member device. It now reaches the\n'
    'intended test: related membership cannot import unrelated direct consumers\n'
    'or another pool into selected-device dependents. Selecting both devices\n'
    'includes those consumers. No frozen configuration rule was weakened.\n\n'
    'This failed transcript binds its uncommitted working-source hashes and\n'
    'design commit, not the accepted runtime. Its counts are excluded from\n'
    'acceptance totals. Full merged stdout/stderr bytes are retained with hashes.\n'
)
scope = {
    'qualified_source': qualified, 'parent_main': 'be705f2',
    'surface': 'unregistered pure domain dependency impact preview',
    'inputs': 'original contract, pools, protection, containers, shares, full expected devices and caller time',
    'imported_graph_trusted': False, 'complete_expected_device_matching': True,
    'all_configurations_validated_before_success': True, 'original_docker_age_preserved': True,
    'selection_max_devices': 32, 'resource_candidate_limit': 128,
    'existing_global_route_limit': 512, 'preview_bytes': 262144,
    'report_counting': 'compact JSON through borrowed serde_json counting writer; no second report allocation',
    'typed_graph_constructed_before_byte_check': True,
    'dependency_change': 'existing serde_json 1.0.145 pin moves from domain dev-only to runtime',
    'dependency_versions_changed': False,
    'selected_and_related_devices_distinct': True,
    'pool_membership_expands_resource_candidates': True,
    'affected_protection_uses_one_global_subject': True,
    'related_members_import_direct_consumers_or_other_pools': False,
    'stopped_disabled_read_only_resource_candidates_preserved': True,
    'running_candidates_keep_full_original_snapshots': True,
    'stack_ownership_inferred_from_container_id': False,
    'resource_candidates_are_store_keys': False, 'resource_candidates_are_acquired_claims': False,
    'empty_declared_dependents_establish_safe_absence': False,
    'fixed_evidence_needed': ['retained identity/freshness', 'complete physical consumers',
                            'actual pool state', 'actual protection state', 'actual share state',
                            'stack ownership', 'complete claims/approval', 'effect-time validation'],
    'pure_fixture_facts': ['four independent frozen-layout target/resource goldens',
                         'full configured graph and original time preserved',
                         'complete selected metadata and sanitized refusal',
                         'related members distinct from selected targets and dependents',
                         'data/parity/global protection and pool propagation',
                         'stopped/disabled/read-only inclusion', 'literal paths, ancestors and named volumes',
                         'sorting/deduplication and input immutability',
                         '128/129 subject and 512-route boundaries',
                         'valid oversized preview refuses without partial result',
                         'empty rows retain every missing evidence boundary'],
    'independent_review': 'pending', 'live_claims_locks_approvals_effects_recovery_added': False,
    'combined_collector_or_report_changed': False, 'wire_schema_routes_commands_changed': False,
    'service_guards_changed': False, 'installed_count_support_qualified': False,
    'domain_limits_qualify_closed_combined_worker_counts': False,
    'pi_accessed': False, 'ssh_used': False, 'docker_accessed': False,
    'raw_block_probe_used': False, 'guest_launched': False,
    'reserved_backup_touched': False, 'external_qualification_paths_touched': False,
    'frozen_signed_arm64_candidate_contains_slice': False,
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20,
    'performance_gate_closed': False, 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-storage-impact-check.py', 'package-storage-impact.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Declared storage impact: local acceptance

Qualified runtime `{qualified}`, based on local main `be705f2`.
The pure unregistered planner validates full expected-device selections against
all original typed configurations and the checked configured graph. It preserves
original Docker age and full relationships, then derives bounded typed resource
candidates and full running container snapshots for review. See the
[design](../../../plans/2026-10-14-storage-impact-preview-design.md) and
[interface guide](../../../p04-storage-impact-planning.md).

**706 Rust cases pass:** 684 unit/integration cases and 22 compile-fail doctests.
The 16 new cases and 15-case focused rerun are already in that total.
Full workspace checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds commands,
environments, runtime hashes and full merged stdout/stderr logs. Documentation
edits during validation changed no runtime source.

Four independent frozen-layout goldens cover selected/related devices, resource
subjects and running dependents. Tests preserve the full checked graph, original
time, complete expected metadata, numeric ownership, branch order, share settings,
source/destination/volume details and flags. Unknown, duplicate, missing, excessive
or changed targets refuse with sanitized fixed-field findings. Invalid unused
configuration and stale/future original Docker declarations also refuse.

Affected pools include all configured member device/mount candidates. Any affected
SnapRAID drive includes one global protection subject and all protection members.
Related members do not become unmount targets or import unrelated direct consumers
and other pools into selected-device dependents. Selecting those devices explicitly
does include their consumers. Stopped/read-only containers and disabled/read-only
configuration remain candidate resources; running snapshots deduplicate full IDs.
Literal case/component boundaries, ancestor sources, named volumes, ordering and
input immutability pass. No stack ownership is invented from container IDs.

Exactly 128 distinct subjects pass; the 129th refuses the whole preview. The graph's
existing global 512-route ceiling remains enforced before selection. A valid
490-route graph with long paths exceeds 256 KiB and refuses without a partial
preview. Borrowed compact JSON serialization counts bytes without a second report buffer.
The bounded typed graph is constructed before that check; peak allocation and
installed service headroom remain unqualified. The existing workspace serde_json
pin moves to domain runtime use for counting; no dependency version changes.
Compile-fail coverage prevents importing the serialize-only result.

Every result keeps eight missing evidence boundaries explicit: retained identity/
freshness, complete physical consumers, actual pool/protection/share state, stack
ownership, complete claims/approval and effect-time validation. Empty declared rows
and unmapped shares cannot prove safe unmounting. Typed resource subjects are
independent of Store keys and acquire no claims. [Diagnostics](diagnostics/README.md)
retain the first invalid pool fixture and its correction under unchanged frozen
rules. The implementing agent reviewed the slice; independent review is pending.

No combined collector/report, Store claim path, schema, route, command, service
guard or package executable list changes. Domain bounds do not qualify the smaller
CombinedV1 worker counts. Root/foreign-namespace and installed qualification,
complete physical consumers/claims, actual pool/share state, locks/effects/recovery,
resource headroom and cutover remain open. The frozen signed ARM64 candidate
excludes this slice. Service guards stay 256 FDs, 64 MiB, zero swap and 16 tasks.
Historical inventory p95 61.401 ms remains above the 20 ms target. No P04, defect,
footprint or performance gate closes.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before small owned, auto-cleaned tmpfs runtime scratch; 50 ms sampling
observed lower-bound peaks of {peak['bytes']} bytes and {peak['files']} files, not exact
maxima. No Pi, SSH, Docker, raw block probe or guest launch was used. Reserved
backup work and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source, fixture,
contract and script files. Fixed-timestamp gzip retains full log bytes with raw/
compressed hashes. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with fresh disk logs using the retained helpers; compile storage
fixtures before their small tmpfs runtime run.

Next local integration requires protected physical closure, actual runtime state
and stack identity before candidates can bind complete claims. Installed/root
qualification of the supplied combined probe remains in the separate workstream.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': entries,
}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings),
                  'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

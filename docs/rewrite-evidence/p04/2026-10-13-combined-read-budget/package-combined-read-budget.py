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
work = root / '.worktrees/combined-read-budget'
logs = root / '.cache/rewrite-combined-read-budget/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-combined-read-budget'
qualified = '94a2826861c94763306e91b23cf17642d5f3acf6'
names = ('test-rest','storage-compile-on-disk','storage-fixtures','budget-focused','budget-measurement','clippy-accepted','fmt-accepted','contracts-accepted','repository-accepted','arm64-harness-accepted')
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
assert rust == 644
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert b'soft_nofile=226' in raws['budget-measurement'] and b'real_owner_collection=false root_authorization=false' in raws['budget-measurement']
previous = json.loads((work/'docs/rewrite-evidence/p04/2026-10-13-storage-read-admission/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-storage/src/configuration/admission.rs','bins/executor/src/lib.rs','bins/executor/src/read_budget.rs','bins/executor/src/read_budget/tests.rs'}
bindings = {}
for path in sorted(files):
    raw = (work/path).read_bytes()
    assert raw == subprocess.check_output(['git','show',qualified+':'+path],cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
out.mkdir()
for c in checks:
    shutil.copy2(logs/c['log'],out/c['log'])
(out/'checks.json').write_text(json.dumps({'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':627,'compile_fail_doctests':17,'new_regression_cases':21,'focused_and_measurement_cases_already_in_workspace_total':True,'arm64_harness_cases':61,'checks':checks},indent=2)+'\n')
(out/'source-manifest.json').write_text(json.dumps({'qualified_source':qualified,'files':bindings},indent=2)+'\n')
scope = {'qualified_source': '94a2826861c94763306e91b23cf17642d5f3acf6', 'parent_main': '65f1f04', 'production_surface': 'unregistered bin-library admission and closed configuration profile; no collector launch', 'configuration_profile': {'file_bytes': 16384, 'total_bytes': 32768, 'retained_fds': 33, 'metadata_report_bytes': 12288, 'shared_json_nodes': 2048, 'json_depth': 16, 'sequence_items': 128, 'map_entries': 32, 'decoded_strings_bytes': 65536, 'one_string_bytes': 4096, 'typed_cell_size_audit_bytes': 1024}, 'resource_model': {'retained_fds': 178, 'baseline_fds': 16, 'transient_fds': 32, 'required_worker_soft_nofile': 226, 'service_nofile': 256, 'baseline_rss_bytes': 25165824, 'modeled_payload_envelope_bytes': 25165824, 'owner_envelopes_plus_report_bytes': 22282240, 'sampled_rss_refusal_bytes': 50331648, 'service_memory_bytes': 67108864, 'worker_threads': 4, 'service_tasks': 16, 'report_bytes': 262144}, 'model_exclusions': ['allocator metadata/abort', 'runtime/library growth', 'child memory', 'unmeasured implementation scratch', 'sampled RSS is not peak enforcement'], 'real_local_facts': ['private configuration scratch descriptors and guarded reads', 'all four frozen configuration layouts under closed profile', 'own-process status/fd/rlimit observations in isolated test child', 'child soft limit lowered to 226; parent limit preserved'], 'private_synthetic_facts': ['configuration root/UID authentication substitution', 'resource boundary samples in private tests', 'phase transitions without owner acquisition'], 'public_root_collection': 'unqualified', 'genuine_combined_owner_collection': False, 'independent_supervision': False, 'process_cleanup_proved_by_single_flight': False, 'installed_resource_qualification': 'untested', 'memory_fit_or_peak_claim': False, 'single_flight': 'nonblocking guard per manager; supervisor must retain through reaping', 'report_is_admission_or_effect_authority': False, 'phase_commit_proves_collection': False, 'standard_configuration_behavior_preserved': True, 'new_external_versions_or_dependencies': False, 'direct_existing_serde_dependency_added_to_bin': True, 'wire_schema_route_cli_service_or_capability_change': False, 'kernel_fd_limit_set_by_production_library': False, 'pi_accessed': False, 'docker_accessed': False, 'raw_workstation_storage_probed': False, 'reserved_backup_recovery_worktree_untouched': True, 'external_qualification_paths_untouched': True, 'cutover': 'deferred by operator', 'frozen_arm64_candidate_source': 'af94f19ac21f9da14b207439808ffb33c87e8e10', 'frozen_arm64_candidate_contains_this_slice': False, 'pending': ['genuine owner composition and original-age revalidation', 'independent termination/cancellation/parent-death/descendant reaping', 'installed and current native resource qualification', 'complete physical consumers/claims', 'actual pool/share state', 'locks/effects/recovery'], 'p04_closed': False, 'defect_rows_closed': [], 'performance_gate_closed': False, 'independent_review': 'pending'}
(out/'scope.json').write_text(json.dumps(scope,indent=2)+'\n')
for name in ('run-combined-read-budget-check.py','package-combined-read-budget.py'):
    shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f"""# Combined reader budget: local acceptance

Qualified source `{qualified}`, based on local main `65f1f04`.
The closed configuration profile preflights protected raw JSON shape before
materializing typed values and uses the smaller host parser through revalidation.
The new unregistered bin library supplies resource reservations, fresh worker
samples, lexical sequential phase guards, a single-use bounded report and
nonblocking single-flight ownership. It neither launches nor collects owners.
See [the design](../../../plans/2026-10-13-combined-read-budget-design.md)
and [interface guide](../../../p04-combined-read-budget.md).

**644 Rust cases pass:** 627 unit/integration cases and 17 compile-fail doctests.
The 21 new cases are already included; focused/measurement reruns add no cases.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds final-source
commands, environments, exact runtime hashes and lossless transcript hashes.

All four frozen layouts pass the closed configuration profile in the private
scratch root/UID seam. It preserves original expiry, retained handles and typed
contracts. Shape tests cover exact/excess collections, rejection before decoding
an excess malformed token, shared nodes/strings, depth and trailing syntax.
The pinned known configuration cell-size audit passes. Existing Standard and
storage/process/destination regressions remain in the workspace total.

The owned test child lowers only its own soft descriptor limit to 226 and uses
production own-process counter reads. Its parent's limit is verified unchanged.
Budget phases reserve capacity without collecting any Engine/storage/process
owner. Synthetic samples cover exact/excess resources, phase failure/poisoning,
contention and escaped report bounds. Compile-fail checks cover closed-profile
import, session/phase borrowing and single-use reports. This qualifies no public
root authentication, combined collection, independent termination or installed
resource behavior. The implementing agent reviewed this slice; independent
review remains pending.

The plan reserves 178 retained descriptors, 16 baseline and 32 transient, totaling
226 below the service's unchanged 256. The conditional payload model is 21.25 MiB
within 24 MiB, alongside a 24 MiB baseline and 48 MiB sampled-RSS refusal threshold.
The 64 MiB, zero-swap and 16-task service guards stay intact. These arithmetic and
sample checks prove no total heap, peak RSS, cgroup headroom or footprint gate;
exclusions are explicit in [scope.json](scope.json). Single-flight drop is no
child cleanup proof. Actual supervisor ownership and process reaping remain open.

Source/build/cache/log/general scratch stayed on disk. Existing storage fixtures
were compiled on disk before owned, auto-cleaned tmpfs runtime scratch. The full
storage 50 ms samples observed lower-bound peaks of {peak['bytes']} bytes and
{peak['files']} files, not exact maxima. No Pi, Docker or raw block probe was used;
reserved backup recovery and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files. Fixed-timestamp gzip preserves every original
stdout/stderr byte; both hashes are retained. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs and the retained helper; compile
storage fixtures before their tmpfs runtime run.

Next is genuine owner composition in an independently supervised worker,
original-age validation, cancellation, parent death and descendant reaping, then
installed resource qualification. Complete physical consumers/claims, actual
pool/share state, locks, effects and recovery remain open. The frozen signed
ARM64 candidate excludes this slice. P04, defect, performance and cutover gates
stay open.
""")
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()}
(out/'artifact-sha256.json').write_text(json.dumps({'qualified_source':qualified,'files':entries},indent=2)+'\n')
print(json.dumps({'packet':str(out),'source_files':len(bindings),'rust_cases':rust,'harness_cases':61,'artifact_files':len(entries)}))

# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind lossless local Compose file acceptance to committed source."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-file-evidence'
logs = root / '.cache/rewrite-compose-files/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-files'
qualified = '7359a97756c898aa0cc324c905cca9c9fedf4851'
accepted_names = ('test-rest', 'storage-compile', 'storage-fixtures', 'files-focused',
                  'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')
diagnostic_names = ('files-initial', 'files-reviewed')
checks, raws = [], {}
for name in accepted_names + diagnostic_names:
    check = json.loads((logs / (name + '.json')).read_text())
    compressed = (logs / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    check['accepted_exact_source'] = name in accepted_names
    assert check['exit'] == 0
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
assert rust == 759 and doctests == 27
assert next(c['rust_cases'] for c in checks if c['name'] == 'files-focused') == 24
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness'])[1]) == 61
assert raws['arm64-harness'].rstrip().endswith(b'OK')
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-14-compose-membership/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-container/src/compose_files.rs',
    'crates/executor-container/src/compose_files/tests.rs', 'tests/fixtures/compose-files/operator.yaml',
    'tests/fixtures/compose-files/limeos.override.json'}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
assert len(bindings) == 330
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({'qualified_source': qualified, 'rust_cases': rust,
    'unit_integration_cases': rust - doctests, 'compile_fail_doctests': doctests,
    'new_regression_cases': 27, 'focused_cases_already_in_workspace_total': True,
    'arm64_harness_cases': 61, 'opt_in_feature': 'limeos-executor/read-worker-probe',
    'checks': checks}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source': qualified,
    'files': bindings}, indent=2) + '\n')
scope = {'qualified_source': qualified, 'parent_main': '19beb67',
    'surface': 'unregistered retained protected single operator and optional fixed managed file facts',
    'selection': 'explicit checked names only; one existing ComposeFileName and managed-file inclusion',
    'actual_effective_uid_0_before_namespace_and_target_access': True,
    'public_root_uid_clock_or_numeric_limit_selectors': False,
    'mount_view': 'fixed nsfs references for self and PID 1 compare device/inode; one reference retained',
    'independent_physical_host_user_pid_or_docker_daemon_identity': False,
    'protected_no_follow_root_ancestors_regular_single_link_files': True,
    'alternate_basenames': 'any object refuses without content open',
    'managed_file_presence': 'must match explicit inclusion',
    'raw_sha256_arbitrary_bytes': True, 'full_file_bodies_retained_or_reported': False,
    'file_bytes': 65536, 'files': 2, 'stack_buffer_bytes': 4096,
    'lifetime_requested_read_bytes': 1048576, 'requested_bytes_refunded_or_renewed': False,
    'original_lifetime_seconds': 4, 'cooperative_work_seconds_per_pass': 2,
    'retained_descriptors': 19, 'conservative_additional_validation_descriptors': 6,
    'compact_report_bytes': 16384, 'derived_path_bytes': 512, 'derived_path_components': 16,
    'held_and_fresh_identity_hash_comparison': True,
    'post_content_absence_named_object_and_ancestry_checks': True,
    'directory_timestamps_in_identity': False,
    'failure_cancel_reentrancy_concurrency_unwind_permanently_invalidates': True,
    'writer_transaction_or_effect_authority': False,
    'new_local_cases': ['24 owned descriptor cases', 'three non-deserialization compile-fail cases',
        'all four recognized operator names and optional managed file', 'opaque binary/empty/exact file limits',
        'alternate object and changed absence refusal', 'selected symlink FIFO directory hardlink refusal',
        'root/ancestor/file mode and ownership checks', 'held/fresh replacement/content/metadata changes',
        'late post-content file/ancestor/absence changes', 'unchanged unrelated entries',
        'original wall/monotonic age and work deadline', 'cancelled chunked read, poisoned retry',
        'reentrant/concurrent/unwound validation invalidation', 'exact path/descriptor/report/requested-byte bounds',
        'actual nsfs primitive', 'public nonroot refusal before callback or namespace/target access'],
    'successful_fixture_root_uid_clock': 'explicitly private substitutions through production traversal/hash code',
    'public_root_positive_and_foreign_namespace_qualification': 'pending',
    'installed_confinement_resource_or_performance_qualification': 'pending',
    'catalog_management_identity_or_engine_file_binding': False,
    'yaml_environment_includes_interpolation_or_combined_semantics': False,
    'empty_stack_or_arbitrary_override_support': False,
    'claims_locks_approvals_effects_recovery_added': False,
    'combined_worker_probe_resource_model_changed': False,
    'dependency_change': 'existing SHA256/hex workspace pins added directly; no version/download change',
    'wire_schema_routes_commands_service_guards_or_package_activation_changed': False,
    'independent_review': 'pending', 'pi_accessed': False, 'ssh_used': False, 'docker_accessed': False,
    'raw_block_probe_used': False, 'guest_launched': False,
    'reserved_backup_or_external_qualification_paths_touched': False,
    'frozen_signed_arm64_candidate_contains_slice': False,
    'p04_or_cutover_closed': False, 'defect_rows_closed': [],
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20, 'performance_gate_closed': False}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-compose-files-check.py', 'package-compose-files.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Protected Compose files: local acceptance

Qualified runtime `{qualified}`, based on local main `19beb67`.
The unregistered fixed-bound reader retains protected root/ancestor/file facts
for one recognized operator file and an explicitly selected fixed managed file.
See the [design](../../../plans/2026-10-14-compose-file-evidence-design.md)
and [interface guide](../../../p04-compose-file-evidence.md).

**759 Rust cases pass:** 732 unit/integration cases and 27 compile-fail doctests.
The 27 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, plus strict feature all-target Clippy,
fmt, contracts and repository checks. [checks.json](checks.json) binds commands,
environments, source hashes and complete lossless merged stdout/stderr.
Initial/reviewed precommit checks also pass; their logs and dirty-source hashes
remain as diagnostics rather than exact-source acceptance. The last source review
adds a late post-content directory/absence regression. Documentation edits after
qualification change no runtime source.

24 owned-descriptor cases cover all four operator basenames and optional managed
selection, formatting/anchor preservation, arbitrary binary/empty contents and
secret omission, alternate files/dangling links/FIFOs/directories, mandatory file
and explicit managed presence, selected symlink/FIFO/directory/hardlink refusal,
root/ancestor/file modes and ownership, changed names/content/metadata/ancestors,
unchanged unrelated entries and late file/directory/absence changes after content
checks. Three compile-fail cases prevent importing selection/report/owner types.

Original wall/monotonic age, cooperative work deadline, cancelled chunked reads,
reentrancy/concurrent refusal/unwinding and poisoned retries pass. Every failed
validation invalidates the owner. Exact canonical derived path/component, file,
retained-descriptor, requested-byte and encoded-report bounds pass. Successful
checks preserve the original lease and spend the original ledger. A real nsfs
primitive test compares owned current-namespace references. Public nonroot refusal
precedes callback, namespace and target access.

Successful files use explicitly private scratch-root/UID/clock substitutions
through production traversal/hash code. The production maximum is 19 retained
descriptors, including a namespace reference; the maximum-depth scratch case
measures 18 without that reference. Six additional validation descriptors is a
conservative code bound, not an installed whole-worker observation. Public root-
positive/foreign namespace/PID-1 mount-view qualification stays open. No capability
or confinement guard is relaxed.

SHA256 streaming uses a fixed 4 KiB stack buffer and reports raw hashes/metadata,
never full bodies or environment. One operator and at most one managed file each
fit 64 KiB. The original four-second lease, two-second per-pass cooperative window,
1 MiB lifetime requested-read ledger and 16 KiB compact report remain fixed.
Sentinels and repeated fresh/held comparisons spend the ledger before I/O, with
no refund or renewal. Separate files and checks are no writer transaction. Kernel
calls still require independent supervision; peak allocator/RSS/cgroup headroom
is unqualified. The existing combined worker/probe/resource model is unchanged.

Actual files do not establish approved catalog digests, management identity,
genuine Engine-to-file binding or stack claim keys. YAML/.env/includes/interpolation,
combined Compose semantics, custom overrides, empty-stack discovery, complete
consumers, claims/locks/approvals/effects/recovery and independent review remain
open. The implementing agent reviewed this slice; independent review is pending.
Root/installed confinement, performance and cutover remain open. The frozen
signed ARM64 candidate excludes it. Existing SHA256/hex pins are reused directly,
without dependency version or registry download changes.

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
Reproduce with fresh disk logs using retained helpers; compile existing storage
fixtures before their small owned tmpfs runtime run.

Next is approved catalog/hash/management binding to the retained actual files and
genuine Engine membership. This slice supplies file facts, not stack ownership.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({'qualified_source': qualified,
    'files': entries}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings), 'rust_cases': rust,
                  'harness_cases': 61, 'artifact_files': len(entries)}))

# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package exact-source local supervision checks with lossless diagnostics."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/read-worker-supervision'
logs = root / '.cache/rewrite-read-worker-supervision/final-logs'
diagnostics = root / '.cache/rewrite-read-worker-supervision/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-read-worker-supervision'
qualified = '25c92bab2dc8c20c948b90a4327510eb3447295c'
names = ('test-rest-accepted', 'storage-compile-on-disk', 'storage-fixtures',
         'supervision-focused', 'clippy-accepted', 'fmt-accepted',
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
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest-accepted', 'storage-fixtures'))
assert rust == 675
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert next(c['rust_cases'] for c in checks if c['name'] == 'supervision-focused') == 10
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-13-combined-read-collection/source-manifest.json').read_text())
files = set(previous['files']) | {
    'bins/executor/src/read_supervision.rs', 'bins/executor/src/read_supervision/guardian.rs',
    'bins/executor/src/read_supervision/tests.rs', 'bins/executor/tests/read_worker_supervision.rs',
    'bins/executor/tests/support/read_worker_probe.rs',
}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
out.mkdir()
for check in checks:
    shutil.copy2(logs / check['log'], out / check['log'])
(out / 'checks.json').write_text(json.dumps({
    'qualified_source': qualified, 'rust_cases': rust, 'unit_integration_cases': 656,
    'compile_fail_doctests': 19, 'new_regression_cases': 17,
    'opt_in_feature': 'limeos-executor/read-worker-probe',
    'focused_cases_already_in_workspace_total': True, 'arm64_harness_cases': 61,
    'process_and_socket_fixtures_execution': 'approved local tests outside sandbox',
    'checks': checks,
}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': bindings,
}, indent=2) + '\n')
(out / 'diagnostics').mkdir()
for name in ('precommit-supervision', 'diagnostic-supervision'):
    check = json.loads((diagnostics / (name + '.json')).read_text())
    compressed = (diagnostics / check['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert check['exit'] != 0
    assert hashlib.sha256(raw).hexdigest() == check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == check['compressed_sha256']
    shutil.copy2(diagnostics / check['log'], out / 'diagnostics' / check['log'])
    (out / 'diagnostics' / (name + '.json')).write_text(json.dumps(check, indent=2) + '\n')
(out / 'diagnostics' / 'README.md').write_text(
    '# Cleanup selector regression\n\n'
    'These failed pre-acceptance logs bind uncommitted working-source hashes, not\n'
    'the final accepted runtime. Escaped-child and manager-death tests found live\n'
    'fixtures adopted by the external runner after premature cleanup acknowledgment.\n'
    'The pinned rustix 1.1.5 process/wait.rs documentation explains that waitpid(None)\n'
    'selects only children in the caller’s own process group. The dedicated guardian\n'
    'requires wait(NOHANG), which selects all its children across groups. That\n'
    'correction passes the retained escaped-session and orphan-observer regressions.\n'
    'The fixture observer was corrected to the same selector. Exact marker-line\n'
    'matching ensures a spawned-child record cannot substitute for a live leaf.\n\n'
    'Final cleanup also uses only parent-checked pidfds, never reusable numeric group\n'
    'signals. Final-source regressions and process inspection found no remaining\n'
    'owned fixtures. These diagnostic counts are excluded from acceptance totals.\n'
)
scope = {
    'qualified_source': qualified, 'parent_main': 'e1faf39',
    'surface': 'unregistered reusable supervisor/guardian/bootstrap and opt-in fixture binary',
    'production_launch_selection': 'not integrated; commands are trusted Rust-side selections',
    'test_feature': 'read-worker-probe; disabled by default; never packaged',
    'deadline_seconds': 4, 'original_ticket_deadline_preserved': True,
    'single_flight_until_cleanup': True, 'unconfirmed_completion_poisons_admission': True,
    'guardian': 'dedicated one-thread process with no pre-existing children; session/subreaper',
    'worker_soft_nofile': 226, 'parent_limit_changed': False,
    'cleanup': 'bounded own-child enumeration, parent-checked pidfds, all-child NOHANG waits',
    'numeric_process_group_signals': False, 'report_bytes': 262144,
    'complete_frame_and_successful_guardian_exit_required': True,
    'real_local_facts': ['blocked synchronous owned worker stopped independently',
                         'cancellation and dropped ticket clean owned children',
                         'manager death observed through exclusive pipe EOF',
                         'escaped-session descendants adopted and reaped',
                         'full 256 KiB report transported under backpressure',
                         'bootstrapped genuine public nonroot collection refusal',
                         'parent descriptor limit preserved'],
    'private_unit_facts': ['frame/buffer boundaries', 'queued-report expiry/cancellation',
                           'admission drop poisons unconfirmed completion'],
    'storage_authority_transferred': False, 'genuine_root_positive_collection': 'unqualified',
    'installed_confinement': 'unqualified', 'independent_review': 'pending',
    'pi_accessed': False, 'ssh_used': False, 'docker_accessed': False,
    'raw_block_probe_used': False, 'guest_launched': False,
    'reserved_backup_touched': False, 'external_qualification_paths_touched': False,
    'service_guards_changed': False, 'frozen_signed_arm64_candidate_contains_slice': False,
    'remaining': ['protected fixed launch/configuration policy and test-only combined probe integration',
                  'root/capability/foreign-namespace positives', 'installed syscall/resource compatibility',
                  'blocked exec/launch termination', 'guardian SIGKILL cgroup teardown',
                  'unkillable-task cleanup latency', 'strict scheduler latency',
                  'peak memory/FD/task headroom', 'complete physical consumers/claims',
                  'actual pool/share state', 'locks/effects/recovery'],
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20,
    'performance_gate_closed': False, 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-read-worker-supervision-check.py', 'package-read-worker-supervision.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Read-worker supervision: local acceptance

Qualified runtime `{qualified}`, based on local main `e1faf39`.
Unregistered manager/guardian mechanics independently supervise owned worker
processes. Admission stays occupied through confirmed cleanup; unknown completion
poisons it. A dedicated session/subreaper guardian observes manager pipe EOF and
the original deadline, signals only pinned child handles and reaps all children,
including adopted descendants in other sessions. Worker bootstrap establishes
parent-death signaling and inherited NOFILE=226 before genuine collection.
See the [design](../../../plans/2026-10-13-read-worker-supervision-design.md) and
[interface guide](../../../p04-read-worker-supervision.md).

**675 Rust cases pass:** 656 unit/integration cases and 19 compile-fail doctests.
The 17 new cases and focused rerun are already in that total. Workspace tests use
the opt-in `limeos-executor/read-worker-probe` feature; the fixture binary is
disabled by default and absent from the packaging executable list.
**61 local ARM64 harness tests pass**, plus strict all-target Clippy, fmt,
contract and repository checks. [checks.json](checks.json) binds final-source
commands, environments, runtime hashes and lossless transcripts. Documentation
edits during validation changed no runtime source.

Ten owned-process tests cover actual deadline termination of synchronous sleep,
cancellation/drop, contention, manager SIGKILL with orphan observer reaping,
escaped-session child cleanup, inherited output-pipe cleanup, complete JSON/frame
and successful exit requirements, malformed/partial/trailing/oversized output,
nonzero exit and guardian-failure poisoning. Both small and full 256 KiB reports
pass. Marker checks confirm recorded fixture PIDs have gone after cleanup. The
parent's descriptor limit stays unchanged. Bootstrap plus the genuine public
collector passes resource admission and refuses nonroot configuration access;
this qualifies no root-positive storage/Engine composition.

Private unit tests cover complete/escaped JSON bounds, missing EOF, restricted
status tags, buffer capacity, late/cancelled queued reports and poisoning before
permit drop. Compile-fail coverage prevents importing report bytes as authority.
[Diagnostics](diagnostics/README.md) retain the cleanup selector failure: the
original waitpid(None) overlooked children outside the guardian's group. The
correct all-child wait and pinned-handle cleanup pass the retained regressions.
The implementing agent reviewed this slice; independent review remains pending.

The ticket deadline can return while cleanup or launch remains pending, holding
admission. This is no guarantee of termination for blocked exec or an unkillable
kernel task, nor a strict scheduler-latency bound. Guardian SIGKILL cannot confirm
descendant cleanup; future service cgroup teardown remains the final guard.
Arbitrary commands/frame JSON do not authenticate storage facts: protected fixed
production launch/configuration selection is not integrated. No production
command/RPC/route/schema/service is added; the combined qualification probe stays
blocked. Installed capabilities, namespaces and syscall compatibility remain open.

Service guards remain 256 descriptors, 64 MiB, zero swap and 16 tasks. Whole
manager/guardian/worker peak resource headroom is unqualified. Historical inventory
p95 61.401 ms remains above the 20 ms target. No footprint or performance gate closes.
The frozen signed ARM64 candidate excludes this slice.

Build/cache/general scratch stayed on disk. Existing storage fixtures were compiled
there before owned, auto-cleaned tmpfs runtime scratch. Their 50 ms samples observed
lower-bound peaks of {peak['bytes']} bytes and {peak['files']} files, not exact maxima.
No Pi, SSH, Docker, raw block probe or guest launch was used. Reserved backup work
and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files. Fixed-timestamp gzip preserves complete merged
stdout/stderr bytes with raw/compressed hashes. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs using the retained helpers;
compile storage fixtures before their tmpfs runtime run.

Next is protected fixed launch/configuration selection and the test-only combined
qualification seam, followed by installed root/capability/namespace and resource
qualification. Complete physical consumers/claims, actual pool/share state, locks,
effects and recovery remain open. P04, defect and cutover gates stay open.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': entries,
}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings),
                  'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

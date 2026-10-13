# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind local collector checks to committed runtime and preserve lossless logs."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/combined-read-collection'
logs = root / '.cache/rewrite-combined-read-collection/final-logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-combined-read-collection'
qualified = 'ed6bd09c936fb281dc36b5e6eaba49c7a46f03e3'
names = ('test-rest-accepted', 'storage-compile-on-disk', 'storage-fixtures',
         'collection-focused', 'collection-measurement', 'clippy-accepted',
         'fmt-accepted', 'contracts-accepted', 'repository-accepted',
         'arm64-harness-accepted')
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
        committed = subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
        assert hashlib.sha256(committed).hexdigest() == digest
        assert hashlib.sha256((work / path).read_bytes()).hexdigest() == digest
    checks.append(metadata)
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest-accepted', 'storage-fixtures'))
assert rust == 658
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness-accepted'])[1]) == 61
assert raws['arm64-harness-accepted'].rstrip().endswith(b'OK')
assert b'RW040_NONROOT_COLLECTION_REFUSAL soft_nofile=226 uid=1000 stage=configuration' in raws['collection-measurement']
assert next(c['rust_cases'] for c in checks if c['name'] == 'collection-measurement') == 1
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-13-combined-read-budget/source-manifest.json').read_text())
files = set(previous['files']) | {'bins/executor/src/read_collection.rs', 'bins/executor/src/read_collection/tests.rs'}
bindings = {}
for path in sorted(files):
    raw = (work / path).read_bytes()
    assert raw == subprocess.check_output(['git', 'show', qualified + ':' + path], cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
out.mkdir()
for c in checks:
    shutil.copy2(logs / c['log'], out / c['log'])
diagnostic_logs = root / '.cache/rewrite-combined-read-collection/logs'
diagnostic = json.loads((diagnostic_logs / 'test-rest.json').read_text())
assert diagnostic['exit'] != 0 and diagnostic['qualified_source'] == '54f539a476fb27426110fc8feb426111fb243931'
assert not subprocess.check_output([
    'git', 'diff', '--name-only', diagnostic['qualified_source'], qualified, '--',
    'bins/core', 'crates/executor-container', 'crates/observations',
], cwd=work)
raw = gzip.decompress((diagnostic_logs / diagnostic['log']).read_bytes())
assert hashlib.sha256(raw).hexdigest() == diagnostic['raw_sha256']
assert hashlib.sha256((diagnostic_logs / diagnostic['log']).read_bytes()).hexdigest() == diagnostic['compressed_sha256']
assert b'Operation not permitted' in raw
(out / 'diagnostics').mkdir()
shutil.copy2(diagnostic_logs / diagnostic['log'], out / 'diagnostics' / diagnostic['log'])
(out / 'diagnostics' / 'test-rest.json').write_text(json.dumps(diagnostic, indent=2) + '\n')
(out / 'diagnostics' / 'README.md').write_text(
    '# Initial sandbox refusal\n\n'
    'The unchanged existing UnixListener fixture binds failed with EPERM in the\n'
    'sandbox in Core, container and observations tests. Their source did not change.\n'
    'The earlier collector source was 54f539a; final runtime acceptance binds ed6bd09.\n'
    'The approved final-runtime workspace rerun passed outside the sandbox using\n'
    'local-test permissions; it used only owned fixture sockets, never workstation\n'
    'Docker. No application guard or test expectation was weakened. This failed\n'
    'lossless transcript is diagnostic evidence, excluded from accepted case totals.\n'
)
(out / 'checks.json').write_text(json.dumps({
    'qualified_source': qualified, 'rust_cases': rust, 'unit_integration_cases': 640,
    'compile_fail_doctests': 18, 'new_regression_cases': 14,
    'focused_and_measurement_cases_already_in_workspace_total': True,
    'arm64_harness_cases': 61, 'checks': checks,
    'sandbox_retry': 'test-rest-accepted ran outside sandbox with approved local fixture permissions',
}, indent=2) + '\n')
(out / 'source-manifest.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': bindings,
}, indent=2) + '\n')
scope = {
    'qualified_source': qualified, 'parent_main': '56afdd6',
    'production_surface': 'unregistered bin-library genuine retained collection; no launch or runtime registration',
    'configuration': 'existing four trusted protected paths; every role required',
    'engine': 'fixed /run/docker.sock; EnginePeerPolicy::root(); CombinedV1',
    'profiles': 'fixed CombinedV1 configuration/Engine/process/storage and closed destination limits',
    'collection_phases': ['configuration', 'engine', 'processes', 'physical', 'destinations', 'graph'],
    'final_validation': ['configuration', 'engine', 'destinations with original physical/process parents', 'engine', 'configuration'],
    'window': {'monotonic_seconds': 4, 'starts_before_admission': True,
               'wall_high_water_checked': True, 'cooperative_only': True,
               'post_serialization_check': True},
    'original_leases_and_engine_ledger_preserved': True,
    'report_bytes': 262144, 'required_worker_soft_nofile': 226,
    'real_local_facts': ['owned test child UID 1000 lowers only its own soft limit',
                         'genuine public configuration root guard refuses collection',
                         'parent descriptor limit preserved',
                         'blocked synchronous fixture returns after window and its value is discarded'],
    'private_synthetic_facts': ['window clock boundary samples', 'operation/publication gate fixtures'],
    'genuine_owner_composition': 'implemented using existing public adapters; root-positive execution unqualified',
    'public_root_collection': 'unqualified', 'independent_supervision': False,
    'atomic_snapshot_claimed': False, 'independent_review': 'pending',
    'pi_accessed': False, 'docker_accessed': False, 'raw_block_probe_used': False,
    'guest_launched': False, 'reserved_backup_touched': False,
    'external_qualification_paths_touched': False, 'service_guards_changed': False,
    'installed_qualification': False, 'frozen_signed_arm64_candidate_contains_slice': False,
    'memory_model_exclusions': ['allocator metadata/abort', 'runtime/library growth', 'child memory',
                                'unmeasured implementation scratch', 'sampled RSS is not peak enforcement'],
    'remaining': ['single-flight supervisor through child cleanup', 'blocked-work termination/cancellation',
                  'parent death and descendant reaping', 'root/capability/foreign-namespace positives',
                  'installed memory/FD/task headroom', 'original-age race qualification',
                  'complete physical consumers/claims', 'actual pool/share state', 'locks/effects/recovery'],
    'historical_inventory_p95_ms': 61.401, 'inventory_p95_target_ms': 20,
    'performance_gate_closed': False, 'p04_closed': False, 'defect_rows_closed': [],
}
(out / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
for name in ('run-combined-read-collection-check.py', 'package-combined-read-collection.py'):
    shutil.copy2(root / '.cache' / name, out / name)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name'] == 'storage-fixtures')
(out / 'README.md').write_text(f"""# Retained combined collection: local acceptance

Qualified runtime `{qualified}`, based on local main `56afdd6`.
The unregistered collector calls genuine closed-profile configuration, Engine,
process, physical and destination adapters, retains their owners, and computes
the configured graph. Sequential budget phases precede acquisition. Final
configuration/Engine/destination/Engine/configuration comparisons preserve all
original leases and the Engine lifetime byte ledger. One borrowed 256 KiB body
is exposed only after a final original-window check. See the
[design](../../../plans/2026-10-13-combined-read-collection-design.md) and
[interface guide](../../../p04-combined-read-collection.md).

**658 Rust cases pass:** 640 unit/integration cases and 18 compile-fail doctests.
The 14 new cases are included; focused and measurement reruns add no cases.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds exact-source
commands, environments, runtime hashes and lossless transcripts. Documentation
edits during validation changed no runtime source.
The initial sandbox denied unchanged owned Unix-socket fixture binds; the full
approved local-test rerun passed. [Diagnostic evidence](diagnostics/README.md)
retains that failed transcript without adding its counts to acceptance.

Window tests cover exact monotonic expiry, independent wall expiry, rollback
above original issue time, negative/backward samples, permanent refusal, failure
stage/reason, cancellation before polling and after completed work, discarded
publication bytes and unchanged original start time. A real synchronous sleep
blocks the test runtime beyond four seconds; its completed value is refused and
dropped after return. This explicitly demonstrates the need for independent
supervision. The compile-fail check rejects report import as evidence.

An owned test child lowers only its own soft descriptor limit to 226, uses
production own-process admission and calls the genuine public collector as UID
1000. The configuration root guard refuses before Engine/storage acquisition;
the parent's limit stays unchanged. This qualifies public nonroot refusal,
not root-positive combined collection, installed capability behavior, process
termination or cleanup. Private gate/window fixtures do not construct fake public
owners. Existing frozen layouts and closed-profile regressions remain included.
The implementing agent reviewed the code; independent review is pending.

No launcher, command, RPC, route, schema or service is added. The standalone probe's
combined case remains blocked. The four-second window is cooperative and the
comparisons are sequential, not atomic. Resource reservations and sampled counters
prove no peak RSS/cgroup headroom; the unchanged 256-FD/64 MiB/zero-swap/16-task
service restrictions still need installed qualification. Historical inventory
p95 61.401 ms remains above the 20 ms target. No performance gate closes.

Source/build/cache/log/general scratch stayed on disk. Existing storage fixtures
were compiled there before owned, auto-cleaned tmpfs runtime scratch. Their 50 ms
samples observed lower-bound peaks of {peak['bytes']} bytes and {peak['files']}
files, not exact maxima. No Pi, SSH, Docker, raw block probe or guest launch was
used. Reserved backup recovery and external qualification paths stayed untouched.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files. Fixed-timestamp gzip preserves every original
stdout/stderr byte with raw/compressed hashes. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk logs and the retained helpers; compile
storage fixtures before their tmpfs runtime run.

Next is independent worker supervision: single-flight ownership until cleanup,
deadline/cancellation independent of blocked work, parent death, and descendant
termination/reaping. Root/capability/namespace positives, original-age races,
installed footprint/performance, complete physical consumers/claims, actual
pool/share state, locks, effects and recovery remain open. The frozen signed
ARM64 candidate excludes this slice. P04, defect and cutover gates stay open.
""")
entries = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
(out / 'artifact-sha256.json').write_text(json.dumps({
    'qualified_source': qualified, 'files': entries,
}, indent=2) + '\n')
print(json.dumps({'packet': str(out), 'source_files': len(bindings),
                  'rust_cases': rust, 'harness_cases': 61, 'artifact_files': len(entries)}))

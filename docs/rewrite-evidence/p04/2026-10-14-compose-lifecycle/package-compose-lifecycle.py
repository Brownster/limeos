# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package exact-source acceptance for read-only bound Compose lifecycle proposals."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/compose-lifecycle-proposal'
logs = root / '.cache/rewrite-compose-lifecycle/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-14-compose-lifecycle'
qualified = '3b0f78edb2584f95aa2c2da9b452e9d42e634991'
accepted = ['test-rest', 'storage-compile', 'storage-fixtures', 'clippy', 'lifecycle-focused',
            'fmt', 'contracts', 'repository', 'arm64-harness', 'compose-conformance']
diagnostics = ['lifecycle-initial', 'lifecycle-reviewed', 'lifecycle-final-review']
assert not out.exists()
assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=work, text=True).strip() == qualified
files = json.loads((logs/'test-rest.json').read_text())['runtime_hashes']
bindings = {path: hashlib.sha256((work/path).read_bytes()).hexdigest() for path in files}
assert files == bindings and len(bindings) == 352
checks, raws = [], {}
for name in accepted+diagnostics:
    c = json.loads((logs/(name+'.json')).read_text())
    compressed = (logs/c['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == c['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == c['compressed_sha256']
    assert c['exit'] == (101 if name in diagnostics[:2] else 0)
    c['accepted_exact_source'] = name in accepted
    if name in accepted:
        assert c['qualified_source'] == c['head_before'] == qualified
        assert c['runtime_hashes_before'] == c['runtime_hashes'] == bindings
    checks.append(c)
    raws[name] = raw
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest','storage-fixtures'))
doctests = sum(int(n) for name in ('test-rest','storage-fixtures') for n in
              re.findall(rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed',raws[name],re.S))
assert (rust,doctests) == (876,39)
for name,count in [('arm64-harness',61),('compose-conformance',6)]:
    assert int(re.search(rb'Ran (\d+) tests',raws[name])[1]) == count
    assert b'OK' in raws[name] and b'skipped' not in raws[name]
focused = next(c['rust_cases'] for c in checks if c['name']=='lifecycle-focused')
assert focused == 15
out.mkdir()
for c in checks: shutil.copy2(logs/c['log'],out/c['log'])
def write(name,value):
    (out/name).write_text(json.dumps(value,indent=2)+'\n')
write('source-manifest.json', {'qualified_source':qualified,'files':bindings})
write('checks.json', {'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':rust-doctests,
    'compile_fail_doctests':doctests,'new_regression_cases':16,'focused_cases':focused,
    'focused_filter':'lifecycle: fourteen new unit cases and one earlier case; two new import refusals run in full workspace only',
    'focused_cases_already_in_workspace_total':True,'arm64_harness_cases':61,
    'native_compose_conformance_cases':6,'opt_in_feature':'limeos-executor/read-worker-probe','checks':checks})
write('scope.json', {'qualified_source':qualified,'parent_main':'bb8d54a',
    'surface':'unregistered read-only Compose lifecycle proposal library',
    'inputs':['genuine retained JSON owner or fixed catalog path and stack ID','ordinary desired typed project','cancellation callback'],
    'fixed_factory_real_uid_0_before_callback_desired_target_namespace_and_engine':True,
    'caller_supplied_current_source_body_hash_child_policy_or_caps':False,
    'original_owner_and_complete_membership_retained':True,'original_parent_lifetime_seconds':4,
    'file_requested_read_ledger_bytes':1048576,'original_issue_times_and_spent_ledgers_renew':False,
    'compiled_against_actual_admitted_operator_and_current_sources':True,
    'every_desired_service_registered':True,'registration_is_principal_permission':False,
    'generated_json_re_admitted_and_supported_combination_verified':True,
    'artifact_bytes':65536,'artifact_sha256_is_approval':False,
    'source_changes':'add/change/remove with all original selected service member indices',
    'replica_oneoff_stopped_mountless_unchanged_unrelated_noncompose_rows_preserved':True,
    'observed_zero_or_removal_candidates_are_shutdown_receipts_safe_absence_or_deletion_authority':False,
    'source_changes_prove_deployed_runtime_equivalence':False,
    'whole_borrowed_compact_report_bytes':262144,'service_change_limit':16,
    'total_selected_member_reference_limit':16,'retained_fd_limit':38,'new_retained_fds':0,
    'failure_cancel_drop_timeout_reentrant_concurrent_unwind_permanently_poison':True,
    'successful_child_can_restore_failed_proposal':False,'child_owner_getters':False,
    'physical_host_daemon_identity_and_complete_consumers':'pending',
    'principal_fresh_approval_claims_locks_atomic_writes_recovery_shutdown_effect_checks':'pending',
    'worker_integration_independent_review_peak_rss_installed_root_namespace_performance':'pending',
    'core_store_contract_dependencies_services_activation_or_signed_candidate_changed':False,
    'successful_fixtures':'private protected scratch roots and authenticated owned Unix listeners',
    'local_native_parser':'six existing pinned AMD64 version/config-only fixtures without daemon effects',
    'pi_ssh_production_docker_raw_block_guest_reserved_backup_or_external_qualification_used':False,
    'p04_defect_performance_or_cutover_gates_closed':False,
    'historical_inventory_p95_ms':61.401,'inventory_p95_target_ms':20})
write('diagnosis.json', {
    'initial':{'log':'lifecycle-initial.log.gz','exit':101,
        'issue':'New removal assertion expected an empty volumes map; established renderer deliberately omits it',
        'correction':'Expect exact existing services-only empty override; production code unchanged',
        'source_tracking':'Diagnostic only: whole-report regression was added during this run; end-of-run dirty hashes do not identify its compiled binary'},
    'reviewed':{'log':'lifecycle-reviewed.log.gz','exit':101,
        'issue':'Oversized report fixture performed an extra full source pass first and exhausted the original 1 MiB ledger',
        'correction':'Use genuine initially admitted owner plus independently admitted valid child plan to test constructor report refusal before redundant full revalidation; guards/budgets unchanged'},
    'final':{'log':'lifecycle-final-review.log.gz','exit':0,'rust_cases':212,
        'source_tracking':'Before/after hashes agree; precommit diagnostic rather than exact committed-source acceptance'},
    'production_changed_by_test_corrections':False,'independent_review':'pending'})
for helper in ('run-compose-lifecycle-check.py','package-compose-lifecycle.py','verify-compose-lifecycle.py'):
    shutil.copy2(root/'.cache'/helper,out/helper)
peak = next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f"""# Bound Compose lifecycle proposals: local acceptance

Qualified source `{qualified}`, based on local main `bb8d54a`.
The read-only proposal consumes genuine retained JSON/catalog/file/Engine owners
and compiles exact managed candidate bytes against their original admitted sources.
See the [design](../../../plans/2026-10-14-compose-lifecycle-proposal-design.md)
and [interface guide](../../../p04-compose-lifecycle-proposals.md).

**876 Rust cases pass:** 837 unit/integration cases and 39 compile-fail doctests.
The 16 new cases and focused rerun are already included. Full workspace acceptance
enables the existing opt-in local worker fixture feature. **61 local ARM64 harness
tests and six existing pinned offline parser cases pass**, plus strict all-target
feature Clippy, fmt, contracts and repository checks. [checks.json](checks.json)
binds commands, environments and exact before/after source hashes to full merged logs.

Fourteen new authenticated owned-listener cases cover add/change/remove/no-op source
proposals, all selected replicas/one-offs/stopped/mountless members and unchanged/
unrelated/non-Compose rows, exact immutable artifact SHA256/length, numeric ownership,
registered desired services, bounded/unrepresentable typed input, failed source
construction, changed real catalog/file/Engine, original age/expiry/read allowance,
late/entry cancellation, dropped/timeout connection EOF, concurrency/reentrancy/unwind,
child-isolation and exact combined report accounting. An actual large admitted source
and independently admitted child plan each fit their limits, while the combined
proposal constructor refuses at 256 KiB. Two import-refusal doctests prevent JSON
owner/report reconstruction. Private root/UID fixtures exercise production traversal,
held-file admission and genuine kernel-authenticated owned Unix Engine listeners.
Public nonroot refusal precedes callback, desired processing, target and Engine access.
Root-positive, foreign namespace and installed qualification remain open.

Three precommit diagnostic runs remain lossless in [diagnosis.json](diagnosis.json).
The initial removal assertion expected an empty volumes object, which the established
renderer omits; only the expectation changed. A report-limit fixture then exhausted
the original read ledger during a redundant prior full pass. It now tests actual
initially admitted evidence and independently admitted child plan against the
constructor's whole-report counter. Production guards/limits remained unchanged.
The corrected complete 212-case container suite passed. Initial metadata captures
dirty source at completion; a regression was added during that run, so it does not
identify the compiled binary. Accepted checks record matching before/after hashes
and exact committed source. No diagnostic was discarded; independent review is open.

Every proposal preserves original four-second parent/file lifetimes, spent requested-
read ledgers and at most 38 retained FDs, adding no lease or FD. Exact candidate JSON
is at most 64 KiB; the whole borrowed compact summary is at most 256 KiB, preserving
all existing individual limits. At most 16 registered source changes and 16 total
member references. Construction validates genuine source evidence before return.
Failure/cancellation/drop/timeout/reentrancy/concurrency/unwind permanently invalidates
the new owner; a successful child cannot restore it. Reports/artifacts remain past
facts after failure. Bounded models do not prove allocator/RSS/whole-worker headroom,
and synchronous kernel work still requires independent supervision.

Source changes and observed members do not prove deployed image/mount/port/user/network
equivalence or independently identified daemon/physical-host identity. Registration
is no principal permission. Artifact hashes are no executable approval. Zero observed
members, removal candidates and compiler shutdown-required names grant no shutdown
receipt, safe absence or deletion authority. Both original compiler gates and explicit
lifecycle gates remain in every report. Physical consumers, principal/fresh approval,
acquired claims/locks, atomic writes/recovery, verified shutdown/effect-time checks,
worker integration, public root/namespace/installed/resource/performance qualification
and independent review remain open. No operation, CLI/RPC/route, Core/Store, contract,
dependency, service, activation or signed ARM64 candidate changes occur.

Build/cache/general scratch stayed on disk. Existing storage fixtures compiled there
before small owned auto-cleaned tmpfs runtime. 50 ms samples observed lower-bound
peaks of {peak['bytes']} bytes/{peak['files']} files, not exact maxima. The six parser
fixtures use the existing pinned AMD64 standalone config/version-only harness and
nonexistent daemon endpoint. No Pi, SSH, production Docker, raw block probe, guest,
reserved backup or external qualification path was used. Guards stay 256 FDs/64 MiB/
zero swap/16 tasks. Historical inventory p95 61.401 ms still exceeds the 20 ms target.
No P04/defect/performance/cutover gate closes.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source/fixture/
contract/script files. Fixed-timestamp gzip preserves every log byte with raw and
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce using retained helpers with fresh work/log/output paths; compile storage
fixtures before their small cleaned tmpfs runtime run. Durable principal/approval,
acquired claims/locks and effects through recovery remain subsequent work.
""")
entries = {str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest()
           for p in sorted(out.rglob('*')) if p.is_file()}
write('artifact-sha256.json',{'qualified_source':qualified,'files':entries})
print(json.dumps({'source_files':len(bindings),'artifact_files':len(entries),'rust_cases':rust,
                  'compile_fail_doctests':doctests,'focused_cases':focused,'harness_cases':61,'native_cases':6}))

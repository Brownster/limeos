# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind retained Compose source acceptance to exact committed runtime."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root=pathlib.Path('/home/marc/Documents/github/lime-os')
work=root/'.worktrees/compose-retained-json'
logs=root/'.cache/rewrite-retained-json/logs'
out=work/'docs/rewrite-evidence/p04/2026-10-14-compose-retained-json'
qualified='20f57bb1d6bf93ffb3f0f6cd41c3044ad5dd6467'
accepted=('test-rest','storage-compile','storage-fixtures','binding-focused','clippy','fmt','contracts','repository','arm64-harness','compose-conformance')
diagnostics=('binding-initial','binding-reviewed','binding-final-review')
previous=json.loads((work/'docs/rewrite-evidence/p04/2026-10-14-compose-source/source-manifest.json').read_text())
paths=set(previous['files'])|{'crates/executor-container/src/compose_management/sources.rs',
    'crates/executor-container/src/compose_management/sources/tests.rs',
    'crates/executor-container/src/docker/process_evidence/tests/source_binding_tests.rs'}
bindings={}
for path in sorted(paths):
    raw=(work/path).read_bytes()
    assert raw==subprocess.check_output(['git','show',qualified+':'+path],cwd=work)
    bindings[path]=hashlib.sha256(raw).hexdigest()
assert len(bindings)==349
checks,raws=[],{}
for name in accepted+diagnostics:
    check=json.loads((logs/(name+'.json')).read_text())
    compressed=(logs/check['log']).read_bytes()
    raw=gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest()==check['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest()==check['compressed_sha256']
    assert check['exit']==(101 if name=='binding-initial' else 0)
    check['accepted_exact_source']=name in accepted
    if check['accepted_exact_source']:
        assert check['qualified_source']==qualified and check['runtime_hashes']==bindings
    checks.append(check)
    raws[name]=raw
assert b'function `assemble_sources` is private' in raws['binding-initial']
assert b'function `check_source_report` is private' in raws['binding-initial']
rust=sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest','storage-fixtures'))
docs=sum(int(n) for name in ('test-rest','storage-fixtures') for n in re.findall(rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed',raws[name],re.S))
assert rust==860 and docs==37
focused=next(c['rust_cases'] for c in checks if c['name']=='binding-focused')
assert focused==39
for name,count in [('arm64-harness',61),('compose-conformance',6)]:
    assert int(re.search(rb'Ran (\d+) tests',raws[name])[1])==count
    assert b'OK' in raws[name] and b'skipped' not in raws[name]
out.mkdir()
for check in checks: shutil.copy2(logs/check['log'],out/check['log'])
(out/'source-manifest.json').write_text(json.dumps({'qualified_source':qualified,'files':bindings},indent=2)+'\n')
(out/'checks.json').write_text(json.dumps({'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':rust-docs,
    'compile_fail_doctests':docs,'new_regression_cases':24,'focused_cases':focused,'focused_cases_already_in_workspace_total':True,
    'focused_filter':'source, including earlier source fixtures; new file read cases also run in full crate/workspace',
    'arm64_harness_cases':61,'native_compose_conformance_cases':6,'opt_in_feature':'limeos-executor/read-worker-probe','checks':checks},indent=2)+'\n')
scope={'qualified_source':qualified,'parent_main':'d4f4c6e',
    'surface':'unregistered retained management/catalog/file/Engine plus strict JSON source composition',
    'public_inputs':['catalog path','stack ID','cancellation callback'],
    'real_uid_0_before_callback_target_namespace_and_canonical_engine_access':True,
    'public_body_hash_child_root_uid_clock_endpoint_or_limit_overrides':False,
    'genuine_original_owners_retained':True,'original_parent_lifetime_seconds':4,'file_cooperative_pass_seconds':2,
    'file_requested_read_ledger_bytes':1048576,'times_and_spent_ledgers_renew':False,
    'held_fd_body_identity_and_original_hash_bracketed_by_fresh_protected_validation':True,
    'body_request_per_file_bytes':65537,'aggregate_temporary_body_requests_bytes':131074,'pread_request_bytes':4096,
    'every_request_including_eof_charged':True,'body_contents_reported_or_retained':False,
    'typed_parse_limit_cancel_reentrant_concurrent_unwind_failure_poisons_file_owner':True,
    'source_fingerprints_match_original_ordered_size_hash_and_presence':True,
    'every_effective_service_registered_and_every_selected_member_service_defined':True,
    'observed_orphan_service': 'whole-request refusal',
    'original_complete_identity_replica_oneoff_stopped_mountless_unrelated_noncompose_rows_preserved':True,
    'zero_members_or_allowed_undefined_names_are_safe_absence_or_action_authority':False,
    'genuine_parent_revalidation_brackets_fresh_held_source_admission':True,
    'failure_cancel_drop_timeout_reentrant_concurrent_unwind_permanently_poisons_source_owner':True,
    'valid_child_binding_can_restore_source_readiness':False,'child_owner_getters':False,
    'borrowed_compact_combined_report_bytes':262144,'retained_fd_limit':38,
    'individual_binding_and_source_report_limits_unchanged':True,
    'source_and_deployed_image_mount_port_user_network_equivalence':False,
    'public_root_positive_foreign_namespace_independent_daemon_physical_host_and_installed_qualification':'pending',
    'successful_fixtures':'private scratch-root/UID/clock substitutions and genuine authenticated owned Unix listeners',
    'new_local_cases':'five file-body cases, four local-source cases, thirteen genuine Engine/composition cases, two import refusals',
    'native_conformance':'six existing pinned AMD64 standalone configuration-parser fixtures; no daemon effects',
    'yaml_environment_includes_interpolation_physical_completeness_and_executable_approval':'pending',
    'claims_locks_writes_recovery_verified_shutdown_or_worker_integration_added':False,
    'core_store_contracts_dependencies_service_guards_or_package_activation_changed':False,
    'peak_allocator_rss_whole_worker_installed_resource_and_independent_review':'pending',
    'pi_ssh_production_docker_raw_block_probe_or_guest_used':False,
    'reserved_backup_and_external_qualification_paths_touched':False,
    'signed_arm64_candidate_contains_slice':False,'p04_defect_performance_or_cutover_gates_closed':False,
    'historical_inventory_p95_ms':61.401,'inventory_p95_target_ms':20}
(out/'scope.json').write_text(json.dumps(scope,indent=2)+'\n')
(out/'diagnosis.json').write_text(json.dumps({'initial_log':'binding-initial.log.gz','exit':101,
    'issue':'Sibling Engine test module called private production constructor/report helpers and imported an unused self',
    'correction':'Add cfg(test)-only delegating wrappers; keep production helpers private and remove unused test import',
    'production_changed_by_compile_correction':False,'reviewed_crate_cases':195,
    'final_review':'Add proof that successful original child validation cannot restore cancelled source owner',
    'final_review_crate_cases':196},indent=2)+'\n')
for name in ('run-retained-json-check.py','package-retained-json.py'): shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f'''# Retained Compose JSON sources: local acceptance

Qualified source `{qualified}`, based on local main `d4f4c6e`.
The unregistered reader composes genuine retained management/catalog/file/Engine
owners with supported JSON admitted directly from their protected held FDs.
It verifies declared selected service association, not deployed configuration
equivalence or executable authority. See the [design](../../../plans/2026-10-14-compose-retained-json-design.md)
and [interface guide](../../../p04-compose-retained-json.md).

**860 Rust cases pass:** 823 unit/integration cases and 37 compile-fail doctests.
The 24 new cases and focused rerun are already included. Full workspace checks
enable the existing opt-in local worker fixture feature. **61 local ARM64 harness
tests and six existing independent offline parser cases pass**, with strict feature
all-target Clippy, fmt, contracts and repository checks. [checks.json](checks.json)
binds commands/environments/exact-source hashes and complete merged stdout/stderr.

Five file-body cases cover actual held bytes, original issue/FD/read ledger,
requested-byte/EOF accounting, exhausted budget, original age, parser failure,
cancellation, reentrancy/unwind, maximum admitted body, in-read growth/truncation
and late named replacement. Four source cases cover genuine fingerprints/numeric
ownership, protected unsupported JSON/YAML refusal and poisoning, different pure
source fingerprints and managed-presence changes without fresh evidence authority.

Thirteen authenticated owned-listener cases compose real original Engine owners:
full identities/replicas/one-offs/stopped/mountless/unrelated/non-Compose rows,
explicit managed absence, zero members, source/registered/observed service admission,
pure snapshot substitution refusal, unsupported operator/managed body refusal,
catalog/file/Engine changes, late cancellation, dropped connection EOF, original
timeout/wall rollback, concurrency/reentrancy/unwind and exact borrowed report limits.
Valid child binding checks cannot restore a source owner cancelled at entry.
Public nonroot refusal precedes callback/target/canonical Engine access. Two
compile-fail cases prevent imported owner/report reconstruction. Successful fixtures
substitute only private scratch root/UID/clock and use actual kernel peer credentials.

The initial compile failure remains lossless with [diagnosis.json](diagnosis.json).
Sibling tests needed cfg(test)-only wrappers around private constructor/report
helpers; production visibility/behavior stayed unchanged. The corrected 195-case
crate suite passed. Final child-isolation review added one test; 196 cases passed.
Both passing precommit runs remain diagnostics rather than exact-source acceptance.
No failed check was discarded; independent review remains pending.

Original limits remain: parent/file four-second lifetimes, two-second cooperative
file passes, 1 MiB requested file reads, 64 KiB per body plus one sentinel, 4 KiB
pread requests and 38 retained FDs. Temporary body buffer requests total at most
131074 bytes; bodies drop after parsing. Original metadata/parsed models remain
immutable. Count one borrowed compact report against 256 KiB without report clones,
keeping individual 80 KiB binding/192 KiB source caps. Excess or spent budget refuses;
large valid sources receive no additional ledger. Allocator/RSS/whole-worker headroom
and installed confinement/resource qualification remain open.

Collection revalidates before return; later checks bracket fresh typed held reads
with genuine parent checks and compare original sources/service association.
Every failure/cancellation/drop/timeout/reentrant/concurrent/unwind invalidates
the new owner permanently. Original observations and spent ledgers never renew;
child accessors are absent. Reports remain past facts. Kernel blocking work needs
independent supervision and these observations are no writer transaction.

General YAML/.env/includes/interpolation, deployed image/mount/port/user/network
equivalence, independent daemon/physical-host identity, complete physical consumers,
principal/fresh approval, acquired claims/locks, writes/recovery, verified shutdown,
worker integration and public root/namespace/installed qualification stay open.
No Core/Store/schema/dependency/service/package change or activation occurs. The
signed ARM64 candidate remains frozen. No P04/defect/performance/cutover gate closes.

Build/cache/general scratch stayed on disk. Existing storage fixtures compiled there
before small owned auto-cleaned tmpfs runtime. 50 ms samples observed lower-bound
peaks of {peak['bytes']} bytes/{peak['files']} files, not exact maxima. Offline parser
checks use the existing pinned AMD64 config/version-only harness and nonexistent
daemon endpoint. No Pi, SSH, production Docker, raw block probe, guest, reserved
backup or external qualification path was used. Guards stay 256 FDs/64 MiB/zero swap/
16 tasks. Historical inventory p95 61.401 ms still exceeds the 20 ms target.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source/fixture/
contract/script files. Fixed-timestamp gzip preserves every log byte with raw/
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce against qualified source using retained helpers and fresh work/log/output
paths; compile storage fixtures before their small cleaned tmpfs runtime run.

Next is bounded lifecycle proposals with fresh permissions/approval and effect-time
verification through locks/recovery; retained JSON binding grants no host action.
''')
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()}
(out/'artifact-sha256.json').write_text(json.dumps({'qualified_source':qualified,'files':entries},indent=2)+'\n')
print(json.dumps({'source_files':len(bindings),'artifact_files':len(entries),'rust_cases':rust,'harness_cases':61,'native_cases':6}))

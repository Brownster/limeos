# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package exact-source local acceptance for retained Compose review subjects."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root=pathlib.Path('/home/marc/Documents/github/lime-os')
work=root/'.worktrees/compose-review-subject'
logs=root/'.cache/rewrite-compose-review/logs'
out=work/'docs/rewrite-evidence/p04/2026-10-14-compose-review'
qualified='36d8d61b24808cdfda8254ec557f4d9baef4e397'
accepted=['test-rest','storage-compile','storage-fixtures','clippy','review-focused',
          'fmt','contracts','repository','arm64-harness','compose-conformance']
diagnostics=['review-initial','review-final-review']
assert not out.exists()
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=work,text=True).strip()==qualified
files=json.loads((logs/'test-rest.json').read_text())['runtime_hashes']
bindings={p:hashlib.sha256((work/p).read_bytes()).hexdigest() for p in files}
assert files==bindings and len(bindings)==355
checks,raws=[],{}
for name in accepted+diagnostics:
    c=json.loads((logs/(name+'.json')).read_text())
    compressed=(logs/c['log']).read_bytes()
    raw=gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest()==c['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest()==c['compressed_sha256']
    assert c['exit']==0
    c['accepted_exact_source']=name in accepted
    assert c['runtime_hashes_before']==c['runtime_hashes']
    if name in accepted:
        assert c['qualified_source']==c['head_before']==qualified
        assert c['runtime_hashes']==bindings
    checks.append(c);raws[name]=raw
rust=sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest','storage-fixtures'))
docs=sum(int(n) for name in ('test-rest','storage-fixtures') for n in
         re.findall(rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed',raws[name],re.S))
assert (rust,docs)==(894,41)
focused=next(c['rust_cases'] for c in checks if c['name']=='review-focused')
assert focused==16
for name,count in [('arm64-harness',61),('compose-conformance',6)]:
    assert int(re.search(rb'Ran (\d+) tests',raws[name])[1])==count
    assert b'OK' in raws[name] and b'skipped' not in raws[name]
out.mkdir()
for c in checks:shutil.copy2(logs/c['log'],out/c['log'])
def write(name,value): (out/name).write_text(json.dumps(value,indent=2)+'\n')
write('source-manifest.json',{'qualified_source':qualified,'files':bindings})
write('checks.json',{'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':rust-docs,
    'compile_fail_doctests':docs,'new_regression_cases':18,'focused_cases':focused,
    'focused_filter':'review_tests: sixteen new unit cases; two new import refusals run in full workspace',
    'focused_cases_already_in_workspace_total':True,'arm64_harness_cases':61,
    'native_compose_conformance_cases':6,'opt_in_feature':'limeos-executor/read-worker-probe','checks':checks})
write('scope.json',{'qualified_source':qualified,'parent_main':'613b990',
    'surface':'unregistered read-only P03 preview facts bound to genuine retained Compose review subject',
    'inputs':['actual retained lifecycle proposal','ordinary PlannedCompose','cancellation callback'],
    'canonical_before_desired_impact_stack_operator_hash_and_managed_name_match':True,
    'caller_models_compared_before_serialization_or_clone':True,'canonical_preview_byte_limit':49152,
    'original_p03_plan_digest_and_canonical_template_digest_checked':True,
    'required_scopes':'existing deployment_manage stack and exact deployment_elevated stack/template/desired digest when elevated',
    'scope_requirements_are_grants_or_authorization':False,'preview_input_authenticates_store_or_session':False,
    'p03_catalog_digest_equals_actual_management_catalog_hash':False,
    'review_digest_domain':'limeos.compose.review.v1','review_digest_is_preview_approval_or_execution_authority':False,
    'digest_includes_original_catalog_file_root_namespace_identity_hash_source_artifact_impact_changes_gates_and_complete_engine_members':True,
    'digest_includes_original_engine_socket_endpoint_peer_policy_running_pids_and_declared_mounts':True,
    'collection_observation_times_excluded_only_from_digest':True,
    'file_modified_changed_p03_created_expiry_and_container_incarnation_times_bound':True,
    'genuine_identical_recollection_new_engine_issue_time_matches':True,
    'genuine_pid_or_declared_mount_changes_require_distinct_digest':True,
    'original_parent_lifetime_seconds':4,'file_requested_read_ledger_bytes':1048576,
    'issue_times_leases_or_spent_ledgers_renew':False,'retained_fd_limit':38,'new_retained_fds':0,
    'whole_compact_borrowed_report_and_streaming_material_byte_limit':262144,
    'p03_expiry_before_and_after_genuine_child_validation_checked':True,
    'current_preview_comparison':'exact principal/revision/id/catalog/template/expiry and full stored preview facts',
    'failure_cancel_drop_timeout_concurrent_reentrant_unwind_permanently_poison':True,
    'successful_child_restores_failed_subject':False,'child_owner_getter_or_approval_token_input':False,
    'principal_current_grants_store_cancellation_catalog_authentication_and_fresh_executable_approval':'pending',
    'acquired_claims_locks_writes_recovery_shutdown_effect_time_or_worker_integration':'pending',
    'runtime_equivalence_independent_daemon_physical_identity_complete_consumers_root_namespace_installed_peak_rss_performance_review':'pending',
    'core_store_contract_dependencies_services_guards_signed_candidate_or_activation_changed':False,
    'successful_fixtures':'protected private scratch roots and genuine authenticated owned Unix Engine listeners',
    'native_parser':'six existing pinned AMD64 standalone config/version-only fixtures without daemon effects',
    'pi_ssh_production_docker_raw_block_guest_reserved_backup_external_qualification_used':False,
    'p04_defect_performance_or_cutover_gates_closed':False,'historical_inventory_p95_ms':61.401,'inventory_target_ms':20})
write('diagnosis.json',{'initial':{'log':'review-initial.log.gz','exit':0,'rust_cases':228},
    'final_review':{'log':'review-final-review.log.gz','exit':0,'rust_cases':230,
    'additional_proofs':'late cancellation after Engine bytes and P03 expiry during genuine child validation'},
    'precommit_runs_are_exact_committed_source_acceptance':False,
    'before_after_runtime_hashes_recorded':True,'failed_checks':[], 'independent_review':'pending'})
for name in ('run-compose-review-check.py','package-compose-review.py','verify-compose-review.py'):
    shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f"""# Retained Compose review subjects: local acceptance

Qualified source `{qualified}`, based on local main `613b990`.
The unregistered read-only subject binds ordinary P03 preview input to actual retained
lifecycle/catalog/file/source/Engine owners and exact candidate bytes. See the
[design](../../../plans/2026-10-14-compose-review-subject-design.md) and
[interface guide](../../../p04-compose-review-subjects.md).

**894 Rust cases pass:** 853 unit/integration cases and 41 compile-fail doctests.
The 18 new cases and 16-case focused rerun are already included. Full workspace
acceptance enables the existing opt-in local worker fixture feature. **61 local
ARM64 harness tests and six existing pinned offline parser cases pass**, with strict
all-target feature Clippy, fmt, contracts and repository checks. [checks.json](checks.json)
binds exact committed before/after source hashes, commands, environments and full logs.

Sixteen new unit cases cover exact canonical before/desired/impact/operator/stack/
digest/template matching, elevated and ordinary scope requirements, original numeric
ownership, immutable caller copies, malformed/expired/oversized input and failed
proposal construction. Current-preview comparisons bind principal, revision, ID,
catalog, template, expiry and review digest before further HTTP. Complete Engine
context binds socket identity, peer policy, endpoint, running PIDs and declared mounts.
Genuine recollection of the same named objects matches despite a new Engine issue
time; genuine changed PID or mounts yields a different review digest. File metadata,
incarnation and P03 creation/expiry times remain in the material. Real file/Engine
changes and spent original read allowance refuse. Entry/late cancellation, drop/
timeout connection EOF, concurrency/reentrancy/unwind, child-isolation, P03 expiry
during genuine child validation and exact whole-report/streaming-hash limits pass.
Two import-refusal doctests prevent JSON reconstruction of owner and summary.

Successful fixtures use protected private root/UID substitutions and authenticated
owned Unix Engine listeners. They exercise production traversal, held-byte admission,
strict parsing and genuine retained ownership. The initial complete 228-case suite
passed; final review added two proofs and the 230-case suite passed. Both precommit
runs remain full lossless diagnostics in [diagnosis.json](diagnosis.json), without
claiming committed-source acceptance. No failed check occurred in this slice, and
independent review remains pending.

The digest domain is `limeos.compose.review.v1`, separate from the P03 preview digest
and artifact SHA256. It includes exact preview/scope requirements and stable original
catalog/file/root/namespace/registration/source/artifact/impact/change/gate facts,
complete Engine membership and retained Engine socket/policy/PID/mount context.
Exclude only collection observation times; keep file modification/change, incarnation
and P03 creation/expiry times. The full borrowed summary preserves every original
observation time. Stream material through a 256 KiB hash writer without serializing
an allocated report; count the whole report against 256 KiB and the original preview
against 48 KiB before cloning. Child/artifact limits and at most 38 retained FDs stay
unchanged. No new FD, lease or read allowance; original four-second windows persist.
Construction revalidates original evidence. P03 expiry is checked before and after
genuine child checks. Failure/cancel/drop/timeout/concurrency/reentrancy/unwind poisons
the subject permanently; a valid child cannot restore it. Past reports/bytes remain
facts after failure. These checks provide no writer transaction or RSS/headroom proof;
synchronous kernel work still requires independent supervision.

Ordinary preview input, canonical hashes and required scopes do not authenticate
Store/session/principal/current grants, cancellation or P03 catalog provenance. The
future composition root must resolve those through authenticated Store/policy paths.
P03 catalog and actual management catalog digests remain separate facts. P03 approvals
remain preview-only; no token input, approval, receipt, executable request or effect is
added. Fresh executable approval, acquired claims/locks, writes/recovery, shutdown/
effect-time checks and worker integration remain open. Deployed configuration
equivalence, independent daemon/physical identity, complete consumers, public root/
namespace/installed/resource/performance qualification and independent review stay
open. No Core/Store/schema/dependency/service/guard/activation/signed-candidate change.
No P04/defect/performance/cutover gate closes.

Build/cache/general scratch stayed on disk. Storage fixtures compiled there before
small owned auto-cleaned tmpfs runtime. 50 ms samples observed lower-bound peaks of
{peak['bytes']} bytes/{peak['files']} files, not exact maxima. Existing pinned AMD64
parser checks use config/version only and a nonexistent daemon endpoint. No Pi, SSH,
production Docker, raw block probe, guest, reserved backup or external qualification
path was used. Guards stay 256 FDs/64 MiB/zero swap/16 tasks. Historical inventory p95
61.401 ms still exceeds the 20 ms target.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source/fixture/
contract/script files. Fixed-timestamp gzip preserves every log byte, with raw and
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce with retained helpers and fresh work/log/output paths, compiling storage
fixtures before their small cleaned tmpfs runtime run. Next is authenticated durable
review/fresh approval and acquired claim/lock integration through recovery.
""")
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest()
         for p in sorted(out.rglob('*')) if p.is_file()}
write('artifact-sha256.json',{'qualified_source':qualified,'files':entries})
print(json.dumps({'source_files':len(bindings),'artifact_files':len(entries),'rust_cases':rust,
    'compile_fail_doctests':docs,'focused_cases':focused,'harness_cases':61,'native_cases':6}))

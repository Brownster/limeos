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
work=root/'.worktrees/compose-authority-read'
logs=root/'.cache/rewrite-compose-authority/logs'
out=work/'docs/rewrite-evidence/p04/2026-10-14-compose-authority'
qualified='afc1514290f2bf65809eb430842ce910af04d933'
accepted=[n+'-final' for n in ['test-rest','storage-compile','storage-fixtures','clippy','authority-focused','fmt','contracts','repository','arm64-harness','compose-conformance']]
diagnostics=sorted(p.stem for p in logs.glob('*.json') if p.stem not in accepted)
assert not out.exists()
assert subprocess.check_output(['git','rev-parse','HEAD'],cwd=work,text=True).strip()==qualified
files=json.loads((logs/'test-rest-final.json').read_text())['runtime_hashes']
bindings={p:hashlib.sha256((work/p).read_bytes()).hexdigest() for p in files}
assert files==bindings and len(bindings)==359
checks,raws=[],{}
for name in accepted+diagnostics:
    c=json.loads((logs/(name+'.json')).read_text())
    compressed=(logs/c['log']).read_bytes()
    raw=gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest()==c['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest()==c['compressed_sha256']
    assert c['exit']==(101 if name in ('authority-initial','core-metadata-red','authority-reviewed') else 0)
    c['accepted_exact_source']=name in accepted
    assert c['runtime_hashes_before']==c['runtime_hashes']
    if name in accepted:
        assert c['qualified_source']==c['head_before']==qualified
        assert c['runtime_hashes']==bindings
    checks.append(c);raws[name]=raw
rust=sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest-final','storage-fixtures-final'))
docs=sum(int(n) for name in ('test-rest-final','storage-fixtures-final') for n in
         re.findall(rb'Doc-tests[^\n]+\n.*?test result: ok\. (\d+) passed',raws[name],re.S))
assert (rust,docs)==(927,43)
focused=next(c['rust_cases'] for c in checks if c['name']=='authority-focused-final')
assert focused==16
for name,count in [('arm64-harness-final',61),('compose-conformance-final',6)]:
    assert int(re.search(rb'Ran (\d+) tests',raws[name])[1])==count
    assert b'OK' in raws[name] and b'skipped' not in raws[name]
out.mkdir()
for c in checks:shutil.copy2(logs/c['log'],out/c['log'])
def write(name,value): (out/name).write_text(json.dumps(value,indent=2)+'\n')
write('source-manifest.json',{'qualified_source':qualified,'files':bindings})
write('checks.json',{'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':rust-docs,
    'compile_fail_doctests':docs,'new_regression_cases':19,'focused_cases':focused,
    'focused_filter':'authority::tests: sixteen new Store unit cases; one Core case and two import refusals run in full workspace',
    'focused_cases_already_in_workspace_total':True,'arm64_harness_cases':61,
    'native_compose_conformance_cases':6,'opt_in_feature':'limeos-executor/read-worker-probe','checks':checks})
write('scope.json',{'qualified_source':qualified,'parent_main':'e65a686','integrated_main':'4846a96',
    'surface':'authenticated read-only session/task resolution and existing Core preview-read integration',
    'public_clock_selector':False,'actual_session_or_task_database_identity_required':True,
    'transport_peer_uid_and_current_protected_catalog_resolved_by':'composition root, as for P03',
    'explicit_deferred_consistent_read_transaction':True,
    'stored_canonical_preview_before_caller_copy_or_serialization':True,
    'complete_canonical_expected_preview_and_exact_requirements_match':True,
    'current_role_grants_revision_catalog_template_source_impact_cancellation_and_body_digest_checked':True,
    'durable_row_revision_and_expiry_match_canonical_plan':True,
    'task_uid_id_generation_revision_expiry_revocation_and_each_exact_scope_checked':True,
    'fixed_start_end_clock_identity_preview_recheck_and_rollback_refusal':True,
    'actual_database_generation_matches_open_store':True,
    'private_owned_result_and_borrowed_serialize_only_report':True,'observation_byte_limit':65536,
    'preview_byte_limit_unchanged':49152,'authentication_secrets_retained_or_reported':False,
    'old_success_is_permission_lease_approval_or_effect_authority':False,
    'concurrent_revocation_seen_after_snapshot':'on next resolution; future broker must re-resolve before/after host checks',
    'query_only_purity_total_changes_audit_generation_approval_expiry_jobs_resources_locks_schema_preserved':True,
    'core_existing_preview_response_unchanged_but_inconsistent_metadata_refuses':True,
    'schema_version':9,'migrations_contracts_dependencies_or_executor_requests_changed':False,
    'fresh_approval_acquisition_consumption_claims_locks_writes_recovery_shutdown_or_effect_added':False,
    'root_broker_transport_host_runtime_physical_completeness_installed_performance_or_independent_review':'pending',
    'optional_pi_access_authorized_after':'2026-10-14 22:00 Europe/London',
    'earlier_healthy_host_return_boundary':'07:00 Europe/London',
    'pi_ssh_production_docker_raw_block_guest_or_cutover_used':False,
    'reserved_backup_or_external_qualification_paths_touched':False,
    'native_parser':'six existing pinned AMD64 config/version-only cases without daemon effects',
    'signed_arm64_candidate_activated_or_p04_defect_performance_cutover_gates_closed':False,
    'historical_inventory_p95_ms':61.401,'inventory_target_ms':20})
write('diagnosis.json',{'initial':{'log':'authority-initial.log.gz','exit':101,
    'issue':'Test passed an unsized Scope slice to existing Sized generic json helper',
    'correction':'Pass a reference to the slice reference; production serializer remains unchanged'},
    'core_regression':{'red_log':'core-metadata-red.log.gz','red_exit':101,
    'issue':'Existing preview read returned canonical plan despite inconsistent stored expiry column',
    'green_log':'core-metadata-green.log.gz','green_exit':0,'cases':1,
    'correction':'Use authenticated session resolver, including durable revision/expiry cross-checks'},
    'reviewed':{'log':'authority-reviewed.log.gz','exit':101,
    'issues':['Fixture changed already-privileged normalized elevated template instead of standard template',
              'Fixture used a time before actual session expiry and received preview-expired classification'],
    'correction':'Select standard template by ID and use actual stored session cutoff; production checks unchanged'},
    'final':{'log':'authority-final-review.log.gz','exit':0,'rust_cases':84},
    'precommit_diagnostics_are_exact_committed_source_acceptance':False,'all_logs_retained':True,
    'independent_review':'pending','integration':{'main':'4846a96','qualified_merge':qualified,'premerge_runtime':'f4a64f4f2445fc1f01e485f78a2cda1652b2fae5','premerge_rust_cases':913,'premerge_checks_retained':True}})
compat_paths=['crates/persistence/src/schema.sql']+[f'crates/persistence/src/migration-v{n}.sql' for n in range(2,10)]
compat_paths += [p for p in bindings if p.startswith('contracts/') or p in ('Cargo.toml','Cargo.lock') or p.endswith('/Cargo.toml')]
compat={}
for p in sorted(set(compat_paths)):
    old=subprocess.check_output(['git','show','e65a686:'+p],cwd=work)
    assert old==(work/p).read_bytes(),p
    compat[p]=hashlib.sha256(old).hexdigest()
write('compatibility.json',{'schema_version':9,'parent_main':'e65a686','integrated_main':'4846a96','files_unchanged':compat,
                          'ddl_contracts_dependency_manifests_and_lock_unchanged':True})
for name in ('run-compose-authority-check.py','package-compose-authority.py','verify-compose-authority.py'):
    shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures-final')
(out/'README.md').write_text(f"""# Authenticated Compose review reads: local acceptance

Qualified source `{qualified}`, based on local main `e65a686`.
Actual current sessions/tasks resolve owned stored previews and every exact requirement
in a deferred SQLite read snapshot. The existing Core preview-read API uses session
resolution and preserves its original response. See the
[design](../../../plans/2026-10-14-compose-authority-read-design.md) and
[interface guide](../../../p04-compose-authority-reads.md).

**927 Rust cases pass:** 884 unit/integration cases and 43 compile-fail doctests.
The 19 new cases and focused rerun are already included. Full workspace acceptance
enables the existing opt-in local worker fixture feature. **61 local ARM64 harness
tests and six existing pinned parser cases pass**, with strict all-target feature
Clippy, fmt, contracts and repository checks. [checks.json](checks.json) binds exact
committed before/after source hashes, commands, environments and full merged logs.

Sixteen new actual database cases cover session/task parity, ordinary/elevated scope,
query-only purity, unchanged approval/audit/generation/expiry/jobs/resources/locks,
unknown/revoked/expired/preview-approval credentials, task UID/ID/generation/revision/
expiry/revocation and missing elevated scope. Forged or oversized expected data never
replaces the bounded stored row. Exact requirements refuse omissions/extras/duplicates/
reordering/wildcards. Current catalog, cancellation, body/digest/row metadata, grants/
roles/revision, wrong owners and missing records refuse as specified. Old success
remains past data after revocation. Fixed end-time identity/plan expiry, rollback/
clock failure, consistent concurrent WAL revocation, public-clock reads/restart,
exact report bounds/redaction and nested transaction/generation mismatch pass.
One Core API case preserves successful read state and rejects inconsistent expiry
metadata. Two import-refusal doctests prevent owner/report reconstruction.

[diagnosis.json](diagnosis.json) retains all five precommit diagnostics and premerge passing checks. The initial
compile error was an unsized slice in a fixture's existing Sized serializer; the
fixture now passes a reference to the slice reference. The Core regression failed
against the old preview read, then passed with the session resolver. A later fixture
changed the wrong already-elevated template after normalization, and another used
a time before actual session expiry. Selecting the template by ID and reading the
actual cutoff corrected those expectations without relaxing production checks.
The complete 84-case Store suite passed. Another engineer then merged and pushed\nprivate backup reconstruction at `4846a96`; its code is preserved in the combined\nqualified source. The earlier 913-case workspace and passing checks remain\npremerge diagnostics. Full acceptance was repeated against the combined source. No diagnostic was discarded; these runs
are separate from exact committed-source acceptance and independent review is open.

Public resolution uses fixed wall time without clock overrides. Identity and stored
preview are rechecked at the end, including actual current policy and complete task
subsets. Exact caller preview comparison precedes caller serialization/copy. Stored
revision/expiry columns match the canonical plan. The owned result retains actual
principal/preview/requirements, kind, check time, generation and identity expiry;
no token, session digest, CSRF, password or approval is retained/reported. Count a
borrowed serialize-only observation against 64 KiB, keeping the 48 KiB preview cap.
It is a past observation, not a permission lease or executable approval. Concurrent
changes after its snapshot require fresh resolution. The future broker must resolve
before and after genuine host validation; writes still need human session/origin/CSRF
and new durable operation-bound approval. Tasks cannot approve. Transport peer UID
and trusted protected catalog provenance remain composition-root responsibilities.

Query-only tests and API read-state comparisons prove no logical Store mutation,
including approvals and claims. [compatibility.json](compatibility.json) binds
unchanged schema-9 DDL, contracts, dependency manifests and lockfile. No migration,
new route, executor request, job, approval acquisition/consumption, resource claim,
stack/storage lock, host write, recovery or shutdown is added. Root broker/transport,
fresh durable approval, runtime/physical completeness, acquired locks/recovery,
shutdown/effect-time checks, installed/resource/performance qualification and
independent review remain open. The signed ARM64 candidate stays frozen; no P04/
defect/performance/cutover gate closes. Historical inventory p95 61.401 ms still
exceeds the 20 ms target.

Build/cache/general scratch stayed on disk. Storage fixtures compiled there before
small owned auto-cleaned tmpfs runtime. 50 ms samples observed lower-bound peaks of
{peak['bytes']} bytes/{peak['files']} files, not exact maxima. Existing pinned AMD64
parser checks use config/version only and a nonexistent daemon endpoint. This
SQLite slice uses local tests and needs no Pi access. The operator permits needed
host tests after 22:00 Europe/London, preserving frozen app/media/mounts/health and
the earlier 07:00 return boundary; no replacement is authorized. No Pi, SSH,
production Docker, raw block probe, guest, reserved backup or external qualification
path was used. Guards remain 256 FDs/64 MiB/zero swap/16 tasks.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source/fixture/
contract/script files. Fixed-timestamp gzip retains every log byte with raw and
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce with retained helpers and fresh work/log/output paths, compiling storage
fixtures before their small cleaned tmpfs runtime. Next is root broker/transport
composition, followed by durable fresh approval and acquired locks/recovery.
""")
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest()
         for p in sorted(out.rglob('*')) if p.is_file()}
write('artifact-sha256.json',{'qualified_source':qualified,'files':entries})
print(json.dumps({'source_files':len(bindings),'artifact_files':len(entries),'rust_cases':rust,
    'compile_fail_doctests':docs,'focused_cases':focused,'harness_cases':61,'native_cases':6}))

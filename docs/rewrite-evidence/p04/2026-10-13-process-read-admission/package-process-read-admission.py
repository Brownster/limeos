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
work = root / '.worktrees/process-read-admission'
logs = root / '.cache/rewrite-process-read-admission/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-process-read-admission'
qualified = 'e0ee0f3243a5a225894afbc538191044715b7f7a'
names = ('profile-accepted', 'payload-accepted', 'process-payload-measurement', 'process-fd-measurement', 'test-rest', 'storage-compile-on-disk', 'storage-fixtures', 'clippy', 'fmt', 'contracts', 'repository', 'arm64-harness')
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
assert rust == 602
assert checks[0]['rust_cases'] == 11 and checks[1]['rust_cases'] == 2
assert int(re.search(rb'Ran (\d+) tests', raws['arm64-harness'])[1]) == 61
assert raws['arm64-harness'].rstrip().endswith(b'OK')
assert b'retained=37 peak=41 host_namespace_authentication=private_substitution' in raws['process-fd-measurement']
assert b'mount_vector_bytes=425984 mount_string_bytes=24576' in raws['process-payload-measurement']
previous = json.loads((work/'docs/rewrite-evidence/p04/2026-10-13-container-destinations/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/executor-storage/src/processes/admission.rs', 'crates/executor-storage/src/processes/tests/admission.rs'}
bindings = {}
for path in sorted(files):
    raw = (work/path).read_bytes()
    assert raw == subprocess.check_output(['git','show',qualified+':'+path],cwd=work)
    bindings[path] = hashlib.sha256(raw).hexdigest()
out.mkdir()
for c in checks:
    shutil.copy2(logs/c['log'],out/c['log'])
(out/'checks.json').write_text(json.dumps({'qualified_source':qualified,'rust_cases':rust,'unit_integration_cases':589,'compile_fail_doctests':13,'new_focused_cases':13,'focused_and_measurement_cases_already_in_workspace_total':True,'arm64_harness_cases':61,'checks':checks},indent=2)+'\n')
(out/'source-manifest.json').write_text(json.dumps({'qualified_source':qualified,'files':bindings},indent=2)+'\n')
scope = {
    'qualified_source':qualified, 'parent_main':'6791dd2',
    'production_surface':'additive trusted read-only library profile; no worker entry point',
    'public_root_collection':'unqualified', 'foreign_namespace_collection':'unqualified', 'installed_qualification':'untested',
    'real_local_facts':['owned child pidfd/proc/root/namespace handles','process mountinfo reads','37 retained/41 peak descriptors above baseline','descriptor cleanup'],
    'private_substitutions':['host namespace authentication in LocalChildren','frozen fixture clock and synthetic kernel facts for maximum-row admission'],
    'closed_profile':{'complete_declarations':16,'running_processes':8,'declared_mounts_including_stopped':32,'table_bytes':65536,'table_rows':512,'raw_bytes_per_running_pass':262144,'retained_mount_capacity_bytes':524288,'decoded_path_bytes':512,'filesystem_bytes':64,'retained_fds':37,'additional_temporary_fds':4,'report_bytes':49152,'declaration_binding_bytes':65536},
    'payload_exclusions':['allocator metadata','BTreeSet nodes','raw buffers','declaration/report copies','stacks','transport','runtime'],
    'whole_worker_memory_or_fd_qualification':False, 'independent_supervision':False,
    'standard_behavior_preserved':True, 'new_dependencies':False, 'wire_schema_or_route_change':False,
    'service_capability_or_kernel_guard_change':False,'pi_accessed':False,'docker_accessed':False,'raw_workstation_storage_probed':False,
    'reserved_backup_recovery_worktree_untouched':True,'cutover':'deferred by operator',
    'frozen_arm64_candidate_source':'af94f19ac21f9da14b207439808ffb33c87e8e10','frozen_arm64_candidate_contains_this_profile':False,
    'pending':['storage/topology/source admission','concrete whole-worker budget','independent termination/single-flight/cancellation/parent-death/descendant reaping','Engine/configuration integration','nested/unselected physical consumers and complete claims','actual pool/share state','locks/effects/recovery','current native CI and installed ARM64 qualification'],
    'p04_closed':False,'defect_rows_closed':[],'performance_gate_closed':False,'independent_review':'pending'
}
(out/'scope.json').write_text(json.dumps(scope,indent=2)+'\n')
for name in ('run-process-read-admission-check.py','package-process-read-admission.py'):
    shutil.copy2(root/'.cache'/name,out/name)
peak=next(c['tmpfs_fixture_peak'] for c in checks if c['name']=='storage-fixtures')
(out/'README.md').write_text(f"""# Process read admission: local acceptance

Qualified source `{qualified}`, based on local main `6791dd2`.
The additive CombinedV1 process profile admits complete Engine-sized membership,
bounds raw host/process tables and decoded fields before parsing allocations,
and shares a retained vector/string-capacity allowance across running processes.
The opaque owner retains the profile and original lease through validation.
Standard collection keeps its existing limits. See [the design](../../../plans/2026-10-13-process-read-admission-design.md)
and [interface guide](../../../p04-process-read-admission.md).

**602 Rust cases pass:** 589 unit/integration cases and 13 compile-fail doctests.
The 13 new focused cases and separate measurement reruns are already included.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds commands,
environments, exact runtime hashes and both transcript hashes. Only successful
checks on the frozen committed source are acceptance evidence. One precommit
boundary fixture had an extra separator; it was corrected without changing the
parser and both exact/excess optional-token cases pass.

The owned-child measurement uses real local kernel handles and mountinfo reads
with an explicit private host-authentication substitution. It measured 37
retained and 41 peak descriptors above baseline, then verified cleanup.
The synthetic maximum-row fixture retains 4096 rows in eight processes:
425984 vector bytes plus 24576 string-capacity bytes, from 122320 raw input bytes.
Its frozen clock proves admission, not timing. These measurements do not qualify
public root/PID 1 authentication, foreign namespaces, installed permissions,
whole-worker descriptors, total heap or RSS. Payload exclusions are explicit in
[scope.json](scope.json). The implementing agent reviewed this slice; independent
review remains pending.

Sources, compilation, caches, logs and general scratch stayed on disk. Storage
kernel fixtures were compiled on disk first, then ran in an owned automatically
cleaned tmpfs directory. Its 50 ms samples observed a lower-bound peak of
{peak['bytes']} bytes in {peak['files']} files, not an exact maximum. No Pi,
Docker or raw workstation storage probe was used. The reserved backup recovery
worktree stayed untouched. Existing service/kernel/capability guards remain.

[source-manifest.json](source-manifest.json) binds {len(bindings)} source,
fixture, contract and script files to the qualified commit. Gzip with a fixed
timestamp preserves every stdout/stderr byte; original and compressed hashes
are retained. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with the retained helper and fresh disk log paths; compile storage
fixtures before their tmpfs runtime check.

Next is storage/topology/source admission and a concrete whole-worker budget,
then independent worker supervision. Engine/configuration integration, complete
physical consumers/claims, actual pool/share state, locks, effects, recovery,
current native and installed ARM64 qualification remain open. The frozen signed
ARM64 candidate does not contain this profile. P04, defect, performance and
cutover gates stay open.
""")
entries={str(p.relative_to(out)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.rglob('*')) if p.is_file()}
(out/'artifact-sha256.json').write_text(json.dumps({'qualified_source':qualified,'files':entries},indent=2)+'\n')
print(json.dumps({'packet':str(out),'source_files':len(bindings),'rust_cases':rust,'harness_cases':61,'artifact_files':len(entries)}))

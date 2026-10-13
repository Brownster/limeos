# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Package only successful local integration checks with exact source bindings."""
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path('/home/marc/Documents/github/lime-os')
work = root / '.worktrees/storage-graph-integration'
logs = root / '.cache/rewrite-storage-graph-integration/logs'
out = work / 'docs/rewrite-evidence/p04/2026-10-13-configured-graph-integration'
out.mkdir()
qualified = 'bd87ee2b2a462cdf896c3ea4b68907cdc59d2210'
names = ('graph-focused','test-rest','storage-compile-on-disk','storage-fixtures','clippy','fmt','contracts','repository','arm64-harness')
checks = []
for name in names:
    metadata = json.loads((logs / (name + '.json')).read_text())
    assert metadata['exit'] == 0 and metadata['qualified_source'] == qualified
    raw = gzip.decompress((logs / metadata['log']).read_bytes())
    assert hashlib.sha256(raw).hexdigest() == metadata['raw_sha256']
    if name == 'arm64-harness':
        metadata['harness_cases'] = int(re.search(rb'Ran (\d+) tests',raw)[1])
        assert metadata['harness_cases'] == 61 and raw.rstrip().endswith(b'OK')
    shutil.copy2(logs / metadata['log'], out / metadata['log'])
    checks.append(metadata)
rust = sum(c['rust_cases'] for c in checks if c['name'] in ('test-rest','storage-fixtures'))
assert rust == 533
checks_doc = {'qualified_source':qualified,'integrated_branch':'engineer/storage-dependency-graph','engineer_tip':'f5d5f6adb5219ec4286e8c8a4adba0a1e8d81c57','parent_main':'7f5dca3','rust_cases':rust,'rust_unit_integration_cases':525,'compile_fail_doctests':8,'arm64_harness_cases':61,'focused_graph_cases':12,'focused_cases_already_in_workspace_total':True,'pi_accessed':False,'installed_qualification':'untested','cutover':'deferred by operator','checks':checks}
(out / 'checks.json').write_text(json.dumps(checks_doc,indent=2)+'\n')
previous = json.loads((work / 'docs/rewrite-evidence/p04/2026-10-12-completed-slices/source-manifest.json').read_text())
files = set(previous['files']) | {'crates/domain/src/storage_dependency_graph.rs','crates/domain/src/storage_dependency_graph/tests.rs','tests/fixtures/storage-dependency-graph/expected.json','tests/qualification/arm64/ci_provenance.py','tests/qualification/arm64/prepare_ci.py','tests/qualification/arm64/test_ci_provenance.py'}
bindings = {}
for name in sorted(files):
    actual = (work / name).read_bytes()
    committed = subprocess.check_output(['git','show',qualified+':'+name],cwd=work)
    assert actual == committed, name
    bindings[name] = hashlib.sha256(actual).hexdigest()
(out / 'source-manifest.json').write_text(json.dumps({'qualified_source':qualified,'files':bindings},indent=2)+'\n')
for name in ('run-storage-graph-integration-check.py','package-storage-graph-integration.py'):
    shutil.copy2(root / '.cache' / name, out / name)
summary = {'qualified_source':qualified,'new_runtime_files':['crates/domain/src/lib.rs','crates/domain/src/storage_dependency_graph.rs'],'new_test_files':['crates/domain/src/storage_dependency_graph/tests.rs','tests/fixtures/storage-dependency-graph/expected.json'],'backup_bound_recovery_excluded':True,'runtime_configuration_join_only':True,'schema_change':False,'api_or_ui_change':False,'new_dependencies_or_capabilities':False,'pending':['protected live configuration and physical/namespace composition','complete affected-resource claims','approved mount/fstab/unmount effects and runtime-loss shutdown','effect-time revalidation and durable recovery','current native CI and installed ARM64 qualification','minimum container/stack UI and migration/return evidence for cutover'],'frozen_arm64_candidate_source':'af94f19ac21f9da14b207439808ffb33c87e8e10','frozen_arm64_candidate_contains_graph':False}
(out / 'integration-status.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({'packet':str(out),'source_files_verified':len(bindings),'rust_cases':rust,'harness_cases':61}))

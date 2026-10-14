# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Verify source, full logs, artifacts and links for bound preview review acceptance."""
import gzip
import hashlib
import json
import pathlib
import re
import subprocess
import sys

work = pathlib.Path(sys.argv[1]).resolve()
packet = work/'docs/rewrite-evidence/p04/2026-10-14-compose-authority'
source = json.loads((packet/'source-manifest.json').read_text())
assert len(source['files']) == 359
for path,digest in source['files'].items():
    assert hashlib.sha256((work/path).read_bytes()).hexdigest() == digest,path
artifacts = json.loads((packet/'artifact-sha256.json').read_text())
assert artifacts['qualified_source'] == source['qualified_source']
for path,digest in artifacts['files'].items():
    assert hashlib.sha256((packet/path).read_bytes()).hexdigest() == digest,path
checks = json.loads((packet/'checks.json').read_text())
assert checks['qualified_source'] == source['qualified_source']
for c in checks['checks']:
    compressed = (packet/c['log']).read_bytes()
    raw = gzip.decompress(compressed)
    assert hashlib.sha256(raw).hexdigest() == c['raw_sha256']
    assert hashlib.sha256(compressed).hexdigest() == c['compressed_sha256']
    assert c['exit'] == (101 if c['name'] in ('authority-initial','core-metadata-red','authority-reviewed') else 0)
    if c['accepted_exact_source']:
        assert c['qualified_source'] == c['head_before'] == source['qualified_source']
        assert c['runtime_hashes_before'] == c['runtime_hashes'] == source['files']
assert (checks['rust_cases'],checks['unit_integration_cases'],checks['compile_fail_doctests']) == (927,884,43)
assert (checks['new_regression_cases'],checks['arm64_harness_cases'],checks['native_compose_conformance_cases']) == (19,61,6)
assert subprocess.run(['git','diff','--quiet',source['qualified_source'],'--',*source['files']],cwd=work).returncode == 0
compat = json.loads((packet/'compatibility.json').read_text())
assert compat['schema_version'] == 9
for path,digest in compat['files_unchanged'].items():
    assert hashlib.sha256((work/path).read_bytes()).hexdigest() == digest,path
for guide in ['docs/p04-compose-authority-reads.md','docs/p04-compose-review-subjects.md','docs/p03-compose-planning.md','docs/plans/2026-10-14-compose-authority-read-design.md','docs/plans/2026-10-04-rust-rewrite-roadmap.md','docs/plans/2026-10-12-basic-cutover-readiness.md','docs/rewrite-evidence/p04/2026-10-14-compose-authority/README.md']:
    page = work/guide
    for target in re.findall(r'\]\(([^)]+)\)',page.read_text()):
        if '://' in target or target.startswith('#'): continue
        assert (page.parent/target.split('#')[0]).exists(),(guide,target)
print(json.dumps({'source_files':len(source['files']),'artifact_files':len(artifacts['files']),
                  'rust_cases':927,'arm64_harness_cases':61,'native_parser_cases':6,
                  'source_artifacts_links_verified':True}))

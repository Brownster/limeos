# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
import hashlib
import json
import pathlib
import re

ROOT = pathlib.Path('/home/marc/Documents/github/lime-os')
OUT = ROOT/'docs/rewrite-evidence/arm64/2026-10-13-current-016ba49-memory-guard'
for path in OUT.rglob('*'):
    assert not path.is_symlink()
    if not path.is_file():
        continue
    assert path.name not in ('key','seed.iso','user-data','known_hosts')
    data=path.read_bytes()
    assert not re.search(rb'(?m)^-----BEGIN [A-Z ]*PRIVATE KEY-----$',data),path
    if path.suffix=='.json':
        json.loads(data)
    if path.suffix=='.jsonl':
        for line in data.splitlines():
            json.loads(line)
summary=json.loads((OUT/'qualification-status.json').read_text())
assert summary['identity']['source_commit']=='016ba49a27b228576db5d4ca28dcd122c60a6278'
assert summary['status'].startswith('incomplete') and summary['untested_gates']
assert not summary['native_workspace_tests_completed'] and not summary['native_packages_created']
assert summary['last_post_teardown_health']['test_processes']==[]
proof=json.loads((OUT/'inputs/deadline-proof-result.json').read_text())
assert proof['summary']=='pass' and proof['after']['actual_child'] is None
candidate=json.loads((OUT/'next-inputs/candidate/candidate-provenance.json').read_text())
assert candidate['source_commit']=='af94f19ac21f9da14b207439808ffb33c87e8e10'
assert candidate['package_version']=='0.4.8+arm64.3' and len(candidate['packages'])==2
assert not candidate['pi_accessed'] and not candidate['installed_behavior_qualified']
hashes={str(path.relative_to(OUT)):hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(OUT.rglob('*')) if path.is_file() and path.name!='evidence-sha256.json'}
(OUT/'evidence-sha256.json').write_text(json.dumps(hashes,indent=2)+'\n')
for path in [ROOT/'docs/plans/2026-10-12-basic-cutover-readiness.md',ROOT/'docs/plans/2026-10-13-arm64-ci-input-qualification-design.md',OUT/'README.md']:
    for target in re.findall(r'\]\(([^)]+)\)',path.read_text()):
        if not target.startswith(('http:','https:','#')):
            assert (path.parent/target.split('#')[0]).exists(),(path,target)
print(f'PASS: {len(hashes)} packet files hashed; JSON, links, refusal/health records and private-key exclusion validated')

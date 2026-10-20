# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Archive the completed CI outcome without relabelling the failed VM gate."""
from pathlib import Path
import gzip
import hashlib
import json
import re
import subprocess

root = Path(__file__).resolve().parents[2]
state = root / ".cache/main-consolidation"
source = "477b81f2ba78c9d100a6857950f50467079b406e"
out = root / "docs/rewrite-evidence/p04/2026-10-20-local-main-hosted"
out.mkdir()
run = json.loads((state / "hosted-run.json").read_text())
assert run["databaseId"] == 39429582703 and run["headSha"] == source
assert run["status"] == "completed" and run["conclusion"] == "failure"
jobs = {item["name"]: item for item in run["jobs"]}
assert jobs["check (ubuntu-24.04-arm)"]["conclusion"] == "success"
assert jobs["check (ubuntu-24.04)"]["conclusion"] == "success"
assert jobs["debian-vm"]["conclusion"] == "failure"

def save(relative, data):
    target = out / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)

summary = {}
for arch, filename in [("arm64", "hosted-native-arm.log"), ("amd64", "hosted-native-amd.log")]:
    raw = (state / filename).read_bytes()
    text = raw.decode("utf-8-sig")
    results = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out", text)
    assert results and all(all(int(value) == 0 for value in row[1:]) for row in results)
    count = sum(int(row[0]) for row in results)
    assert count == 1130, count
    assert "Generated contract drift" in text
    assert "# tests 219" in text
    if arch == "amd64":
        assert "Browser lifecycle, logs and focus regressions passed." in text
    save(f"native-{arch}.log.gz", gzip.compress(raw, mtime=0))
    summary[arch] = {"workspace_checks": count, "frontend_cases": 219, "job_conclusion": "success"}

vm_log = (state / "hosted-debian-vm.log").read_bytes()
assert b"scan.namespace.limit" in vm_log and b"expected 201" in vm_log
save("debian-vm.log.gz", gzip.compress(vm_log, mtime=0))
for filename in ["hosted-run.json", "hosted-artifacts.json"]:
    save(filename, (state / filename).read_bytes())
for relative in [
    "p01/ci-vm-result.json", "p02/ci-vm-result.json",
    "p03/ci-reference-vm-result.json", "p04/ci-approved-vm-failure.txt",
]:
    save("vm-artifact/" + relative + ".gz", gzip.compress((state / "hosted-vm" / relative).read_bytes(), mtime=0))
save("local-audit.json", (state / "final-audit/audit.json").read_bytes())

changes = subprocess.check_output(["git", "diff", "--name-only", source, "HEAD"], cwd=root, text=True).splitlines()
assert all(path.startswith("docs/") for path in changes)
audit = json.loads((state / "final-audit/audit.json").read_text())
cleanup = json.loads((state / "cleanup-result.json").read_text())
assert len(cleanup["removed_worktrees"]) == 78
assert len(cleanup["removed_branches"]) == 87
assert audit["all_local_tips_merged"] and audit["main_clean"] and len(audit["trees"]) == 9
assert all(t["dirty"] and t["merged_to_main"] for t in audit["trees"][1:])
save("manifest.json", (json.dumps({
    "source": source,
    "run": run["url"],
    "conclusion": "failure",
    "native": summary,
    "vm_failure": {
        "stage": "A1 plan creation before effect admission",
        "phase": "scan.namespace.limit",
        "observed": 9,
        "limit": 8,
        "http_status": 503,
        "production_limit_changed": False,
        "namespace_owners_not_recorded": True,
        "scope": "The first excess namespace is observed; total namespaces and ownership are unknown",
    },
    "later_committed_changes": changes,
    "later_product_source_changes": [],
    "removed_worktrees": 78,
    "removed_local_branches_original_cleanup": 87,
    "removed_local_branches_followup": [{"branch": "engineer/p04-stack-inventory-runtime", "head": "85d708f3092d0fca971a507c1876292ffe9f2393", "merged_to_main": True}],
    "retained_dirty_worktrees": 8,
    "draft_recovery": ".cache/main-consolidation/recovery/stack-inventory-start",
    "final_recovery_bundle": ".cache/main-consolidation/refs-final.bundle",
}, indent=2) + "\n").encode())
save("README.md", b"""# Merged-source hosted qualification, 20 October 2026

Run [39429582703](https://github.com/Brownster/limeos/actions/runs/39429582703)
binds source `477b81f2ba78c9d100a6857950f50467079b406e`. Its native ARM64 and
AMD64 jobs both passed. Each ran **1,130 Rust workspace checks and 219 frontend
cases**, strict Clippy, actual contract generation/drift, dependency/advisory
gates and native Debian package builds. AMD64 also passed the browser lifecycle,
logs and keyboard-focus regressions. These are repeated qualification runs,
not extra unique coverage to add to the local test counts.

The complete run **failed** in Debian VM acceptance. The genuine A1 host-consumer
scan encountered a ninth mount namespace against its documented limit of eight
(`scan.namespace.limit`, observed 9, limit 8). Plan creation returned HTTP 503
before effect admission. This is not the earlier optional-Kthread ABI failure
or generated-contract drift. The first excess observation does not establish
the total namespace count or identify its owners. The downloaded evidence does
not supply that census. No production limit, consumer check, fixture assertion
or service profile was relaxed to obtain a passing result.

The VM job first completed the P01, P02 and P03 acceptance wrappers. Only their
fresh `ci-*` outputs and the current P04 failure file are copied from its artifact;
the artifact also contains old tracked evidence which is not credited as new.
The complete VM log preserves the embedded A1 refusal and teardown report.
The later dependency VM wrapper was not reached. There is no full installed
ARM64, crash-recovery, performance, P04 completion or Pi cutover acceptance.

`hosted-run.json` preserves terminal job/step conclusions. `hosted-artifacts.json`
records the platform's artifact IDs/digests. Native and VM logs are byte-preserving
gzip copies of the respective job-log endpoints. `artifacts.sha256` covers this
packet; `manifest.json` distinguishes native success from the failed installed
gate. The source and the final local audit differ only in documentation.

All committed local tips are ancestors of actual local main. Cleanup removed
78 clean merged worktrees and 88 merged local branches in total. Eight dirty
worktrees remain, with current recovery patches/archives under
`.cache/main-consolidation/final-audit/recovery/`. Full history is retained in
the verified `.cache/main-consolidation/refs-final.bundle` and earlier bundles.
The unfinished stack-inventory start was saved separately under
`.cache/main-consolidation/recovery/stack-inventory-start/`; it is not accepted
runtime source. Its committed design is included in main. Remote branches and
production/Pi services were not modified by this consolidation.
""")
save("collect-hosted.py", Path(__file__).read_bytes())
hashes = []
for file in sorted(out.rglob("*")):
    if file.is_file() and file.name != "artifacts.sha256":
        hashes.append(f"{hashlib.sha256(file.read_bytes()).hexdigest()}  {file.relative_to(out)}\n")
save("artifacts.sha256", "".join(hashes).encode())
print(json.dumps({"packet": str(out), "native": summary, "vm": "failure: scan.namespace.limit observed=9 limit=8", "files": len(hashes)}))

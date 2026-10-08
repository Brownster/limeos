#!/usr/bin/env uv run python3
# /// script
# dependencies = []
# ///
"""Verify this local handoff's frozen source and raw check-output hashes."""
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tomllib

BASE = "d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4"
SOURCE = "ad5ff6e2ae75c98dc261c951b084a14f59c3b5e1"
EVIDENCE = Path(__file__).resolve().parent
ROOT = EVIDENCE.parents[3]
CHECKS = ["fmt-final", "clippy-final", "workspace-tests-final", "deny-cached-corrected",
          "audit-cached", "repository-final", "contracts-final", "resource-frozen-source",
          "real-fds-frozen-source", "diff-final"]


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> None:
    for name in CHECKS:
        metadata = json.loads((EVIDENCE / "logs" / f"{name}.json").read_text())
        assert metadata["exit_code"] == 0, name
        assert metadata["source_commit"] == SOURCE, name
        assert digest(EVIDENCE / "logs" / f"{name}.log") == metadata["sha256"], name
        for path, expected in metadata["source_sha256"].items():
            assert digest(ROOT / path) == expected, (name, path)
            frozen = subprocess.check_output(["git", "show", f"{SOURCE}:{path}"], cwd=ROOT)
            assert hashlib.sha256(frozen).hexdigest() == expected, ("commit", path)
    output = (EVIDENCE / "logs/workspace-tests-final.log").read_text()
    units, docs = output.split("   Doc-tests", 1)
    def count(text: str) -> int:
        return sum(map(int, re.findall(r"test result: ok\. (\d+) passed;", text)))
    assert (count(units), count(docs)) == (320, 2)
    assert len(re.findall(r"^test processes::tests::.* \.\.\. ok$", units, re.M)) == 29
    packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
    assert (len(packages), sum("source" in p for p in packages)) == (227, 211)
    changed = subprocess.check_output(["git", "diff", "--name-only", BASE, "HEAD"], cwd=ROOT, text=True).splitlines()
    exact = {"crates/executor-storage/Cargo.toml", "crates/executor-storage/src/lib.rs",
             "crates/executor-storage/src/processes.rs", "crates/executor-storage/src/processes/tests.rs",
             "docs/p04-container-process-evidence.md"}
    prefixes = ("tests/fixtures/container-process-evidence/", "docs/rewrite-evidence/p04/rw040-process-evidence/")
    assert all(path in exact or path.startswith(prefixes) for path in changed), changed
    print("Verified frozen source/output hashes, 320 unit/integration + 2 compile-fail tests, 29 process tests, 227 packages / 211 external, and assigned paths.")


if __name__ == "__main__":
    main()

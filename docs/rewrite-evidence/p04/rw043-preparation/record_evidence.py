#!/usr/bin/env -S uv run --script
# /// script
# dependencies = []
# requires-python = ">=3.11"
# ///
"""Record local preparation evidence without rerunning or adopting anything.

Run: uv run --offline --script docs/rewrite-evidence/p04/rw043-preparation/record_evidence.py
Only this assignment's evidence directory is written. Existing log bytes are
compressed after an exact round-trip check; initial tool excerpts stay labelled.
"""

import datetime
import gzip
import hashlib
import json
import re
import subprocess
from pathlib import Path

import tomllib

EVIDENCE = Path(__file__).resolve().parent
ROOT = EVIDENCE.parents[3]
BASE = "2fd74209e1238c1374837ab431ef670020726dc2"
SOURCE = "92a149dd24188912ce56581bc012e09f70c29f7d"


def git(*args: str) -> bytes:
    return subprocess.check_output(["git", *args], cwd=ROOT)


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def allowed(path: str) -> bool:
    return path in {
        "crates/backup-archive/src/lib.rs",
        "crates/backup-archive/src/staging.rs",  # separately isolated catalog seam
        "crates/backup-archive/src/preparation.rs",
        "crates/backup-archive/tests/preparation.rs",
        "docs/p04-backup-restore-preparation.md",
    } or path.startswith(
        (
            "crates/backup-archive/src/preparation/",
            "tests/fixtures/backup-preparation/",
            "docs/rewrite-evidence/p04/rw043-preparation/",
        )
    )


def main() -> None:
    changes = git("diff", "--name-only", BASE).decode().splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").decode().splitlines()
    unexpected = [p for p in changes + untracked if not allowed(p)]
    if unexpected:
        raise RuntimeError(f"unexpected assignment paths: {unexpected}")
    lock = (ROOT / "Cargo.lock").read_bytes()
    if lock != git("show", f"{BASE}:Cargo.lock"):
        raise RuntimeError("lockfile changed")
    packages = tomllib.loads(lock.decode())["package"]
    tracked = git("ls-files", "-z").decode().split("\0")
    inputs = []
    for path in sorted(p for p in tracked if p):
        if not (
            path.startswith(
                (
                    "crates/",
                    "bins/",
                    "spikes/",
                    "scripts/",
                    "contracts/",
                    "tests/fixtures/",
                    "packaging/",
                    ".cargo/",
                )
            )
            or path in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml"}
        ):
            continue
        data = (ROOT / path).read_bytes()
        if data != git("show", f"{SOURCE}:{path}"):
            raise RuntimeError(f"tested source changed: {path}")
        inputs.append({"path": path, "bytes": len(data), "sha256": digest(data)})
    (EVIDENCE / "source-manifest.json").write_text(
        json.dumps(
            {
                "qualified_source": SOURCE,
                "inputs": inputs,
            },
            indent=2,
        )
        + "\n"
    )

    for path in sorted(EVIDENCE.glob("*.txt")):
        if path.is_symlink():
            raise RuntimeError(f"refusing symlink log: {path}")
        data = path.read_bytes()
        compressed = gzip.compress(data, mtime=0)
        if gzip.decompress(compressed) != data:
            raise RuntimeError("log compression round-trip failed")
        path.with_suffix(".txt.gz").write_bytes(compressed)
        path.unlink()
    logs = []
    for path in sorted(EVIDENCE.glob("*.txt.gz")):
        data = gzip.decompress(path.read_bytes())
        logs.append(
            {
                "path": path.name,
                "uncompressed_bytes": len(data),
                "uncompressed_sha256": digest(data),
                "compressed_sha256": digest(path.read_bytes()),
                "scope": "tool-chunk excerpt"
                if path.name.startswith("initial-")
                else "complete redirected command output",
            }
        )

    text = gzip.decompress(
        (EVIDENCE / "workspace-92a149d.txt.gz").read_bytes()
    ).decode()
    if "test result: FAILED" in text:
        raise RuntimeError("final workspace tests failed")
    unit = docs = 0
    is_doc = False
    for line in text.splitlines():
        if line.lstrip().startswith("Doc-tests "):
            is_doc = True
        match = re.search(
            r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;", line
        )
        if match:
            count, failures, ignored = map(int, match.groups())
            if failures or ignored:
                raise RuntimeError("nonpassing final test summary")
            if is_doc:
                docs += count
            else:
                unit += count
    if (unit, docs) != (399, 4):
        raise RuntimeError(f"unexpected test counts: {unit}, {docs}")
    metadata = {
        "base": BASE,
        "qualified_source": SOURCE,
        "worktree": str(ROOT),
        "record_observed_at_utc": datetime.datetime.now(datetime.UTC).isoformat(),
        "unit_integration_passed": unit,
        "compile_fail_passed": docs,
        "total_passed": unit + docs,
        "failed": 0,
        "ignored": 0,
        "new_unit_integration": unit - 370,
        "new_compile_fail": docs - 2,
        "subprocess_helper_cases": 1,
        "packages": len(packages),
        "external_packages": sum("source" in p for p in packages),
        "lockfile_unchanged": True,
        "cached_advisories": 1290,
        "cached_advisory_commit": "ef6173cbc5c50ec8166f9a5b28f07834144373ee",
        "cached_advisory_date": "2026-10-03T07:49:26Z",
        "advisories_fetched": False,
        "changed_paths": changes,
        "logs": logs,
        "human_hours": None,
        "active_agent_time": "not independently instrumented",
        "waits": "approvals, compile/cache work, interface wait and daemon interruption; not human effort",
        "assignment_estimate_hours": 24,
        "assignment_review_hours": 36,
        "p04_estimate_hours": 320,
        "p04_review_hours": 480,
        "shared_exception": "Operator said continue after the concrete read-only catalog revalidation proposal; isolated seam awaits integrator review",
    }
    (EVIDENCE / "qualification.json").write_text(json.dumps(metadata, indent=2) + "\n")
    patch = git("diff", BASE, SOURCE, "--")
    (EVIDENCE / "allowed-path-diff.patch.gz").write_bytes(gzip.compress(patch, mtime=0))
    (EVIDENCE / "allowed-path-diff.patch").unlink(missing_ok=True)
    print(
        json.dumps(
            {
                "unit_integration": unit,
                "compile_fail": docs,
                "total": unit + docs,
                "bound_inputs": len(inputs),
                "logs": len(logs),
                "packages": len(packages),
            }
        )
    )


if __name__ == "__main__":
    main()

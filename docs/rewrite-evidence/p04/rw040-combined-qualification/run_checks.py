#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record this branch's local checks and ownership review with raw output.

uv run run_checks.py --output FRESH_DIR

No Rust, lockfile, workflow or production source changes on this branch, so
the Rust gates are not applicable; the review below proves that. Existing
output is refused so failed evidence is never overwritten.
"""

import argparse
import datetime
import hashlib
import json
import os
import platform
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
BASE = "2fd74209e1238c1374837ab431ef670020726dc2"
OWNED = (
    "tests/qualification/rw040-combined/",
    "tests/fixtures/rw040-combined/",
    "docs/rewrite-evidence/p04/rw040-combined-qualification/",
)
HARNESS = "tests/qualification/rw040-combined"


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    commands = [
        (
            "toolchain",
            ["sh", "-c", "python3 --version; uvx ruff --version; git --version"],
        ),
        (
            "harness-tests",
            [
                sys.executable,
                "-W",
                "error::ResourceWarning",
                "-m",
                "unittest",
                "discover",
                "-s",
                HARNESS,
                "-p",
                "test_*.py",
                "-v",
            ],
        ),
        (
            "ruff-check",
            [
                "uvx",
                "ruff",
                "check",
                HARNESS,
                "docs/rewrite-evidence/p04/rw040-combined-qualification",
            ],
        ),
        (
            "ruff-format",
            [
                "uvx",
                "ruff",
                "format",
                "--check",
                HARNESS,
                "docs/rewrite-evidence/p04/rw040-combined-qualification",
            ],
        ),
        ("repository", [sys.executable, "scripts/check_repository.py"]),
        ("contracts", [sys.executable, "scripts/check_contracts.py"]),
        ("diff-check", ["git", "diff", "--check", f"{BASE}..HEAD"]),
    ]
    changed = git("diff", "--name-status", f"{BASE}..HEAD").splitlines()
    outside = [line for line in changed if not line.split("\t")[-1].startswith(OWNED)]
    result = {
        "source_commit": git("rev-parse", "HEAD").strip(),
        "base_commit": BASE,
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "uid": os.geteuid(),
        "platform": platform.uname()._asdict(),
        "scope": "local workstation; harness checks only; no Rust/lockfile/workflow change to gate",
        "ownership_review": {
            "changed": changed,
            "outside_owned": outside,
            "passed": not outside,
        },
        "owned_sha256": {
            p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest()
            for p in git("ls-files", *OWNED).split()
        },
        "checks": [],
    }
    failed = bool(outside)
    for name, command in commands:
        start = time.monotonic()
        log = args.output / f"{name}.txt"
        with log.open("w") as stream:
            done = subprocess.run(
                command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, check=False
            )
        result["checks"].append(
            {
                "name": name,
                "command": command,
                "exit_code": done.returncode,
                "elapsed_seconds": round(time.monotonic() - start, 1),
                "log": log.name,
                "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest(),
            }
        )
        print(f"{name}: exit {done.returncode}", flush=True)
        failed |= done.returncode != 0
    result["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    result["passed"] = not failed
    (args.output / "checks.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n"
    )
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()

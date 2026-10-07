#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record local workspace checks without contacting remote test hosts.

uv run --offline run_checks.py --target-dir /scratch/target --output /fresh/logs
Existing output is refused so failed evidence is not overwritten.
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--tools", type=Path, default=Path("/tmp/limeos-validation-bin"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    env = {**os.environ, "CARGO_TARGET_DIR": str(args.target_dir), "CARGO_BUILD_JOBS": "2",
        "CARGO_NET_OFFLINE": "true", "PATH": str(args.tools) + os.pathsep + os.environ["PATH"]}
    commands = [
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--", "-D", "warnings"]),
        ("workspace-tests", ["cargo", "test", "--workspace", "--locked", "--offline"]),
        ("repository", [sys.executable, "scripts/check_repository.py"]),
        ("contracts", [sys.executable, "scripts/check_contracts.py"]),
        ("dependency-metadata", ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]),
        ("deny", ["cargo", "deny", "--offline", "check"]),
        ("audit", ["cargo", "audit", "--no-fetch", "--no-yanked", "--db", "/tmp/limeos-advisory-db"]),
        ("release-example", ["cargo", "build", "--release", "--locked", "--offline", "-p", "limeos-backup-archive", "--example", "stage"]),
    ]
    paths = ["Cargo.toml", "Cargo.lock", "crates/domain/src/backups.rs", "crates/backup-archive/Cargo.toml",
        "crates/backup-archive/src/lib.rs", "crates/backup-archive/src/staging.rs", "crates/backup-archive/src/staging/tests.rs",
        "crates/backup-archive/tests/archives.rs", "crates/backup-archive/examples/stage.rs",
        "tests/fixtures/backup-staging/policy.json", "docs/rewrite-evidence/p04/rw043-staging/measure_staging.py"]
    result = {"source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), "uid": os.geteuid(),
        "platform": platform.uname()._asdict(), "scope": "local workstation; cached dependencies/advisories; no native ARM/installed restore",
        "source_sha256": {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in paths},
        "checks": []}
    for name, command in commands:
        print(f"Running {name}", flush=True)
        start = time.monotonic()
        with (args.output / f"{name}.txt").open("w") as log:
            measured = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
        result["checks"].append({"name": name, "command": command, "exit_code": measured.returncode,
            "elapsed_seconds": time.monotonic() - start, "log": f"{name}.txt"})
        result["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        (args.output / "checks.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        print(f"{name}: exit {measured.returncode}", flush=True)
        if measured.returncode:
            raise SystemExit(measured.returncode)


if __name__ == "__main__":
    main()

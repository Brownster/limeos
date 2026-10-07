#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Record the custody branch's required local checks and ownership review.

uv run --offline run_checks.py --target-dir /scratch/target --output /fresh/logs

Runs every required check with its complete raw output, then reviews the
branch against its base: changed paths must be owned or the two allowed
shared edits, protected paths must be byte-identical, and every external
lockfile package must keep its version, source and checksum. Existing output
is refused so failed evidence is never overwritten. No remote hosts.
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
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
BASE = "d324c1b9f3ee8dcc46d7d5deb4cf00c25b1301f4"
OWNED = (
    "crates/backup-archive/src/custody.rs",
    "crates/backup-archive/src/custody/",
    "crates/backup-archive/tests/custody.rs",
    "crates/backup-archive/examples/custody.rs",
    "tests/fixtures/backup-custody/",
    "docs/p04-backup-archive-custody.md",
    "docs/rewrite-evidence/p04/rw043-custody/",
)
ALLOWED_SHARED = ("crates/backup-archive/src/lib.rs", "crates/backup-archive/Cargo.toml", "Cargo.lock")
PROTECTED = (
    "crates/backup-archive/src/staging.rs", "crates/backup-archive/src/staging/",
    "crates/backup-archive/src/tests.rs", "crates/backup-archive/tests/archives.rs",
    "crates/backup-archive/examples/inspect.rs", "crates/backup-archive/examples/stage.rs",
    "crates/domain/", "tests/fixtures/backup-archives/", "tests/fixtures/backup-staging/",
    "contracts/", "docs/rewrite-evidence/p04/rw043/", "docs/rewrite-evidence/p04/rw043-staging/",
    "docs/p04-backup-archive-admission.md", "docs/p04-backup-verified-staging.md",
    "Cargo.toml", "bins/", "frontend/", "packaging/", ".github/",
)


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True)


def lock_packages(text: str) -> dict:
    packages = tomllib.loads(text)["package"]
    return {(p["name"], p["version"]): (p.get("source"), p.get("checksum"), sorted(p.get("dependencies", [])))
            for p in packages}


def review() -> dict:
    changed = git("diff", "--name-status", f"{BASE}..HEAD").splitlines()
    paths = [line.split("\t")[-1] for line in changed]
    outside = [p for p in paths if not p.startswith(OWNED) and p not in ALLOWED_SHARED]
    protected = git("diff", "--name-only", f"{BASE}..HEAD", "--", *PROTECTED).split()
    lib = git("diff", f"{BASE}..HEAD", "--", "crates/backup-archive/src/lib.rs")
    lib_added = [l for l in lib.splitlines() if l.startswith("+") and not l.startswith("+++")]
    lib_removed = [l for l in lib.splitlines() if l.startswith("-") and not l.startswith("---")]
    before = lock_packages(git("show", f"{BASE}:Cargo.lock"))
    after = lock_packages((ROOT / "Cargo.lock").read_text())
    external_before = {k: v[:2] for k, v in before.items() if v[0]}
    external_after = {k: v[:2] for k, v in after.items() if v[0]}
    dependency_changes = {f"{k[0]} {k[1]}": {"added": sorted(set(after[k][2]) - set(before[k][2])),
                                             "removed": sorted(set(before[k][2]) - set(after[k][2]))}
                          for k in after if k in before and after[k][2] != before[k][2]}
    return {
        "base": BASE,
        "changed": changed,
        "outside_owned_or_allowed": outside,
        "protected_paths_changed": protected,
        "lib_rs_added_lines": lib_added,
        "lib_rs_removed_lines": lib_removed,
        "lock_packages_before": len(before),
        "lock_packages_after": len(after),
        "external_packages_identical": external_before == external_after,
        "external_package_count": len(external_after),
        "lock_dependency_list_changes": dependency_changes,
        "passed": not outside and not protected and lib_added == ["+pub mod custody;"] and not lib_removed
        and before.keys() == after.keys() and external_before == external_after
        and dependency_changes == {"limeos-backup-archive 0.2.0": {"added": ["serde"], "removed": []}},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--tools", type=Path, default=Path("/tmp/limeos-validation-bin"))
    parser.add_argument("--advisory-db", type=Path, default=Path("/tmp/limeos-advisory-db"))
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    env = {**os.environ, "CARGO_TARGET_DIR": str(args.target_dir), "CARGO_BUILD_JOBS": "4",
           "CARGO_NET_OFFLINE": "true", "PATH": str(args.tools) + os.pathsep + os.environ["PATH"]}
    commands = [
        ("toolchain", ["sh", "-c", "rustc -V; cargo -V; cargo deny --version; cargo audit --version; "
                       f"git -C {args.advisory_db} log -1 --format='advisory-db %H %cI'"]),
        ("crate-tests", ["cargo", "test", "-p", "limeos-backup-archive", "--locked", "--offline"]),
        ("fmt", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--offline", "--", "-D", "warnings"]),
        ("workspace-tests", ["cargo", "test", "--workspace", "--locked", "--offline"]),
        ("deny", ["cargo", "deny", "--offline", "check"]),
        ("audit", ["cargo", "audit", "--no-fetch", "--db", str(args.advisory_db)]),
        ("repository", [sys.executable, "scripts/check_repository.py"]),
        ("contracts", [sys.executable, "scripts/check_contracts.py"]),
        ("diff-check", ["git", "diff", "--check", f"{BASE}..HEAD"]),
        ("release-example", ["cargo", "build", "--release", "--locked", "--offline", "-p",
                             "limeos-backup-archive", "--example", "custody"]),
    ]
    owned_files = sorted(p for p in git("ls-files", *OWNED, *ALLOWED_SHARED).split())
    result = {
        "source_commit": git("rev-parse", "HEAD").strip(),
        "base_commit": BASE,
        "started_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "uid": os.geteuid(),
        "platform": platform.uname()._asdict(),
        "scope": "local workstation x86-64; cached dependencies and cached advisory DB; no native ARM, VM or installed restore",
        "source_sha256": {p: hashlib.sha256((ROOT / p).read_bytes()).hexdigest() for p in owned_files},
        "checks": [],
    }
    failed = False
    for name, command in commands:
        print(f"Running {name}", flush=True)
        start = time.monotonic()
        with (args.output / f"{name}.txt").open("w") as log:
            measured = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=False)
        log_path = args.output / f"{name}.txt"
        result["checks"].append({"name": name, "command": command, "exit_code": measured.returncode,
                                 "elapsed_seconds": round(time.monotonic() - start, 1), "log": log_path.name,
                                 "log_sha256": hashlib.sha256(log_path.read_bytes()).hexdigest()})
        print(f"{name}: exit {measured.returncode}", flush=True)
        failed |= measured.returncode != 0
    result["ownership_review"] = review()
    failed |= not result["ownership_review"]["passed"]
    result["finished_utc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    result["passed"] = not failed
    (args.output / "checks.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()

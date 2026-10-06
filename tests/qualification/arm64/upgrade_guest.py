#!/usr/bin/env python3
"""Qualify a genuine schema-6/7 ARM64 upgrade using the existing authority fixtures.

The previous repository must contain the recorded original artifact. Provenance
binds its SHA-256 and original source; changing the package version is no proof.
No schema-7 claim is emitted for a schema-6 artifact.
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

from qualification import guest_guard, installed_binaries, load_build, sha, verify_files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--expected", type=Path, required=True)
    parser.add_argument("--previous-repo", type=Path, required=True)
    parser.add_argument("--previous-provenance", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    guest_guard()
    if args.output.exists():
        parser.error("output exists; select a new result file")
    build = load_build(args.expected, args.repo)
    if build["identity"]["authority_schema"] != 8:
        parser.error("current approved-operation fixtures require schema 8")
    fixtures = Path(__file__).resolve().parents[3]
    verify_files(fixtures, build["fixtures_sha256"])
    previous = json.loads(args.previous_provenance.read_text())
    if not re.fullmatch(r"[0-9a-f]{40}", previous["source_commit"]):
        parser.error("previous original source commit required")
    if previous["source_commit"] == build["identity"]["source_commit"]:
        parser.error("previous artifact must come from a different original source")
    if (
        previous["authority_schema"] not in (6, 7)
        or previous["architecture"] != "arm64"
    ):
        parser.error(
            "authority preservation fixture supports genuine schema 6 or 7 ARM64 artifacts"
        )
    package = next(
        args.previous_repo.rglob(f"limeos_{previous['package_version']}_arm64.deb")
    )
    if sha(package) != previous["package_sha256"]:
        parser.error("previous package differs from original recorded SHA-256")
    control = subprocess.check_output(
        ["dpkg-deb", "-f", str(package), "Package", "Version", "Architecture"],
        text=True,
    )
    if (
        control
        != f"Package: limeos\nVersion: {previous['package_version']}\nArchitecture: arm64\n"
    ):
        parser.error("previous package control identity differs from provenance")
    if args.previous_repo.resolve() != Path("/opt/limeos-previous-repo"):
        parser.error(
            "shared upgrade fixture requires --previous-repo /opt/limeos-previous-repo"
        )
    sys.path.insert(0, str(fixtures / "tests/privileged_vm"))
    import p04_locks_guest as locks

    result = {
        "suite": "upgrade",
        "identity": build["identity"],
        "build_result_sha256": sha(args.expected),
        "previous": previous,
        "previous_provenance_sha256": sha(args.previous_provenance),
        "started": time.time(),
        "summary": "incomplete",
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    try:
        locks.VERSION = build["identity"]["package_version"]
        result["upgrade"] = locks.upgrade(
            args.repo,
            previous_version=previous["package_version"],
            previous_schema=previous["authority_schema"],
            authority_schema=8,
            previous_package_sha256=previous["package_sha256"],
            previous_core_sha256=previous["core_sha256"],
        )
        assert result["upgrade"] is not None, "upgrade was not exercised"
        result["installed_sha256"] = installed_binaries(build)
        result["summary"] = "pass"
    except Exception as error:
        result.update(summary="fail", error=repr(error))
        raise
    finally:
        result["finished"] = time.time()
        args.output.write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()

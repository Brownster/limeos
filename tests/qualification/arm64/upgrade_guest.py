#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Qualify a genuine schema-6/7/8 ARM64 upgrade with the existing authority fixtures.

The previous repository must contain the recorded original artifact. Provenance
binds its SHA-256 and original source; changing the package version is no proof.
Each result records the actual historical schema; labels confer no provenance.
"""

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

from qualification import guest_guard, installed_binaries, load_build, sha, verify_files


def validate_previous(previous: dict, current: dict) -> None:
    """Refuse incomplete, relabeled or unsupported provenance before guest writes."""
    if not isinstance(previous, dict):
        raise TypeError("previous provenance must be an object")
    for field, pattern in (
        ("source_commit", r"[0-9a-f]{40}"),
        ("package_sha256", r"[0-9a-f]{64}"),
        ("core_sha256", r"[0-9a-f]{64}"),
        ("package_version", r"[0-9][A-Za-z0-9.+~-]*"),
    ):
        value = previous.get(field)
        if not isinstance(value, str) or not re.fullmatch(pattern, value):
            raise ValueError(f"valid previous {field} required")
    if previous["source_commit"] == current["source_commit"]:
        raise ValueError("previous artifact must come from a different original source")
    if (
        type(previous.get("authority_schema")) is not int
        or previous["authority_schema"] not in (6, 7, 8)
        or previous.get("architecture") != "arm64"
    ):
        raise ValueError("genuine schema 6, 7 or 8 ARM64 provenance required")


def verify_previous_package(repository: Path, previous: dict) -> Path:
    """Select one contained original artifact and verify actual bytes/controls."""
    candidates = list(
        repository.rglob(f"limeos_{previous['package_version']}_arm64.deb")
    )
    if len(candidates) != 1:
        raise ValueError("exactly one previous ARM64 standard package required")
    package = candidates[0]
    if (
        package.is_symlink()
        or not package.is_file()
        or not package.resolve().is_relative_to(repository.resolve())
    ):
        raise ValueError("previous package must be a contained regular file")
    if sha(package) != previous["package_sha256"]:
        raise ValueError("previous package differs from original recorded SHA-256")
    control = subprocess.check_output(
        ["dpkg-deb", "-f", str(package), "Package", "Version", "Architecture"],
        text=True,
    )
    if (
        control
        != f"Package: limeos\nVersion: {previous['package_version']}\nArchitecture: arm64\n"
    ):
        raise ValueError("previous package control identity differs from provenance")
    return package


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
    if build["identity"]["authority_schema"] != 9:
        parser.error("current approved-operation fixtures require schema 9")
    fixtures = Path(__file__).resolve().parents[3]
    verify_files(fixtures, build["fixtures_sha256"])
    previous = json.loads(args.previous_provenance.read_text())
    try:
        validate_previous(previous, build["identity"])
        verify_previous_package(args.previous_repo, previous)
    except (TypeError, ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
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
            authority_schema=9,
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

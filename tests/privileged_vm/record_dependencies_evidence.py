#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind installed container-dependency acceptance to frozen source and packages."""

import argparse
import hashlib
import io
import json
import re
import subprocess
import tarfile
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BINARIES = ["limeos-core", "limeos-executor", "limeosctl", "limeos-password-worker"]


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    source = subprocess.check_output(
        ["git", "rev-parse", args.source_commit], cwd=ROOT, text=True
    ).strip()
    assert re.fullmatch(r"[a-f0-9]{40}", source)
    vm_path = args.evidence / "dependencies-vm-result.json"
    vm = json.loads(vm_path.read_text())
    assert vm["package_version"] == "0.4.6"
    assert vm["schemas"]["authority"] == 8 and vm["schemas"]["container_receipts"] == 3
    assert vm["dependency_cases"] == 11 and len(vm["checks"]) >= 49
    assert set(vm["operations"]) == {"restart", "start", "stop", "logs"}
    assert vm["upgrade"] is None
    for phrase in [
        "revocation during",
        "named volume",
        "change_list",
        "no intent",
        "shadow reads",
    ]:
        assert phrase in " ".join(vm["checks"]), phrase
    transcript = args.evidence / "rust-tests.txt"
    tests = re.findall(
        r"test result: (\w+)\. (\d+) passed; (\d+) failed", transcript.read_text()
    )
    assert tests and all(
        status == "ok" and failed == "0" for status, _, failed in tests
    )
    count = sum(int(passed) for _, passed, _ in tests)
    assert count >= 199
    source_hashes = {}
    assets = {}
    with tarfile.open(args.bundle, "r:gz") as bundle:
        for member in bundle:
            if not member.isfile() or not member.name.startswith("source/"):
                continue
            path = member.name.removeprefix("source/")
            data = bundle.extractfile(member).read()
            digest = hashlib.sha256(data).hexdigest()
            if path.startswith("frontend/dist/"):
                assets[path] = digest
                continue
            expected = subprocess.check_output(
                ["git", "show", f"{source}:{path}"], cwd=ROOT
            )
            assert hashlib.sha256(expected).hexdigest() == digest, path
            source_hashes[path] = digest
    for path in [
        "Cargo.lock",
        "bins/core/src/storage.rs",
        "crates/executor-container/src/docker/storage.rs",
        "tests/privileged_vm/p04_dependencies_guest.py",
    ]:
        assert path in source_hashes, path
    binary_hashes = {name: sha(args.artifacts / name) for name in BINARIES}
    assert binary_hashes == vm["binary_sha256"]
    packages = {}
    for profile, package in [("standard", "limeos"), ("shadow", "limeos-shadow")]:
        path = args.artifacts / "packages" / f"{package}_0.4.6_amd64.deb"
        payload = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(path)])
        with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
            for name, digest in binary_hashes.items():
                entry = archive.getmember(f"./usr/lib/{package}/{name}")
                assert entry.uid == 0 and entry.gid == 0 and entry.mode & 0o022 == 0
                assert (
                    hashlib.sha256(archive.extractfile(entry).read()).hexdigest()
                    == digest
                )
        packages[profile] = {"path": str(path), "sha256": sha(path)}
    validation = {
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "source_commit": source,
        "fixture_commit": source,
        "bundle": {
            "path": str(args.bundle),
            "sha256": sha(args.bundle),
            "bytes": args.bundle.stat().st_size,
        },
        "package_version": "0.4.6",
        "architecture": "amd64",
        "schemas": vm["schemas"],
        "packages": packages,
        "binary_sha256": binary_hashes,
        "source_files": len(source_hashes),
        "source_sha256_manifest": "source-sha256.json",
        "browser_asset_sha256": assets,
        "rust_tests": {
            "passed": count,
            "failed": 0,
            "transcript_sha256": sha(transcript),
        },
        "installed": {
            "groups": len(vm["checks"]),
            "dependency_groups": vm["dependency_cases"],
            "result_sha256": sha(vm_path),
        },
        "footprint": vm["footprint"],
        "limits": vm["limitations"]
        + [
            "Native ARM64 and comparable Pi/Python measurements remain separate",
            "No mount/fstab effect or complete physical/non-container consumer proof is claimed",
            "Historical package upgrades are independently qualified; this run installs the fresh payload",
        ],
    }
    (args.evidence / "source-sha256.json").write_text(
        json.dumps(source_hashes, indent=2, sort_keys=True) + "\n"
    )
    (args.evidence / "validation.json").write_text(
        json.dumps(validation, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                "source": source,
                "rust_tests": count,
                "installed_groups": len(vm["checks"]),
                "source_files": len(source_hashes),
            }
        )
    )


if __name__ == "__main__":
    main()

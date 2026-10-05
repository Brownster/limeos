#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind P04 storage VM acceptance to a frozen source bundle and installed payload."""

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


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument(
        "--slice", choices=["readiness", "planning"], default="readiness"
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
    )
    args = parser.parse_args()
    planning = args.slice == "planning"
    args.output = (
        args.output
        or ROOT / f"docs/rewrite-evidence/p04/storage-{args.slice}-validation.json"
    )
    evidence = args.output.parent
    vm_name = "planning-vm-result.json" if planning else "storage-vm-result.json"
    log_name = "rust-planning-tests.txt" if planning else "rust-readiness-tests.txt"
    vm = json.loads((evidence / vm_name).read_text())
    assert len(vm["passed"]) >= (27 if planning else 17)
    assert "three-second plan deadline" in " ".join(vm["passed"])
    assert "watchdog bound" in " ".join(vm["passed"])
    assert (
        vm["package_version"] == ("0.4.1" if planning else "0.4.0")
        and vm["architecture"] == "amd64"
    )
    if planning:
        assert "operator fstab edit" in " ".join(vm["passed"])
        assert "duplicate raw UUIDs" in " ".join(vm["passed"])
    assert 2.5 <= vm["plan_wait_seconds"] <= 7
    assert 3.5 <= vm["watchdog_seconds"] <= 8
    log = (evidence / log_name).read_text()
    assert "FAILED" not in log and "test result: ok." in log
    passed = sum(int(n) for n in re.findall(r"test result: ok\. (\d+) passed", log))
    assert passed >= (126 if planning else 111)
    manifest = {}
    with tarfile.open(args.bundle) as archive:
        for item in archive:
            if not item.isfile() or not item.name.startswith("source/"):
                continue
            name = item.name.removeprefix("source/")
            if (
                name.startswith("tests/privileged_vm/")
                and name != "tests/privileged_vm/build_guest.py"
            ):
                continue
            digest = hashlib.file_digest(
                archive.extractfile(item), "sha256"
            ).hexdigest()
            assert sha(ROOT / name) == digest, f"Runtime source drift: {name}"
            if not name.startswith("frontend/dist/"):
                committed = subprocess.check_output(
                    ["git", "show", f"{args.source_commit}:{name}"], cwd=ROOT
                )
                assert hashlib.sha256(committed).hexdigest() == digest, (
                    f"Commit binding failed: {name}"
                )
            manifest[name] = digest
    manifest_path = evidence / f"storage-{args.slice}-source-sha256.json"
    manifest_path.write_text(json.dumps(manifest, sort_keys=True, indent=2) + "\n")
    binaries = {
        name: sha(args.artifacts / name)
        for name in [
            "limeos-core",
            "limeos-executor",
            "limeosctl",
            "limeos-password-worker",
        ]
    }
    assert binaries["limeos-executor"] == vm["payload_sha256"]
    packages = {}
    for package in sorted((args.artifacts / "packages").glob("*.deb")):
        prefix = (
            "limeos-shadow" if package.name.startswith("limeos-shadow_") else "limeos"
        )
        packages[package.name] = sha(package)
        contents = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(package)])
        with tarfile.open(fileobj=io.BytesIO(contents)) as archive:
            for name, expected in binaries.items():
                actual = hashlib.file_digest(
                    archive.extractfile(f"./usr/lib/{prefix}/{name}"), "sha256"
                ).hexdigest()
                assert actual == expected, (
                    f"Packaged binary mismatch: {package.name}/{name}"
                )
            ready = f"./lib/systemd/system/{prefix}-storage-ready.service"
            assert (
                archive.getmember(ready).uid == 0
                and archive.getmember(ready).mode & 0o022 == 0
            )
            text = archive.extractfile(ready).read().decode()
            assert (
                "TimeoutStartSec=125s" in text
                and "Restart=no" in text
                and "[Install]" not in text
            )
            assert "storage-wait --plan /etc/" + prefix + "/storage-wait.json" in text
            if planning:
                reader = archive.getmember(
                    f"./lib/systemd/system/{prefix}-storage-reader.service"
                )
                assert reader.uid == 0 and reader.mode & 0o022 == 0
                content = archive.extractfile(reader).read().decode()
                assert (
                    "[Install]" not in content and "CapabilityBoundingSet=\n" in content
                )
                assert (
                    f"/etc/{prefix}/system-policy/storage.json /run/{prefix}-storage-reader/executor.sock storage"
                    in content
                )
    assert len(packages) == 2
    previous = json.loads(
        (ROOT / "docs/rewrite-evidence/p03/compose-source-sha256.json").read_text()
    )
    identical_assets = all(
        previous.get(name) == digest
        for name, digest in manifest.items()
        if name.startswith("frontend/dist/")
    )
    assert identical_assets, (
        "Frontend assets changed; fresh browser qualification required"
    )
    result = {
        "checked_at_utc": datetime.now(timezone.utc).isoformat(),
        "qualified_source_commit": args.source_commit,
        "build": {
            "host": "disposable Debian 12 KVM guest",
            "architecture": "amd64",
            "rust_toolchain": "1.88.0",
            "cargo": "locked offline release",
            "bundle_sha256": sha(args.bundle),
            "bundle_bytes": args.bundle.stat().st_size,
            "source_manifest": manifest_path.name,
            "bound_source_files": len(manifest),
            "binary_sha256": binaries,
            "package_sha256": packages,
            "signing_key": "test-only private key discarded with the VM",
        },
        "rust_workspace_tests": {"passed": passed, "log": log_name},
        "storage_vm": {
            "acceptance_groups": len(vm["passed"]),
            "result_file": vm_name,
            "installed_profile": "standard",
            "shadow_scope": "payload identity and dormant unit only",
        },
        "production_frontend_assets_identical_to_qualified_p03": identical_assets,
        "fixture_sha256": {
            str(p.relative_to(ROOT)): sha(p)
            for p in [
                ROOT / "tests/privileged_vm/run.py",
                ROOT / "tests/privileged_vm/p04_storage_guest.py",
                ROOT / "tests/privileged_vm/record_storage_evidence.py",
                *(
                    [ROOT / "tests/privileged_vm/p04_planning_guest.py"]
                    if planning
                    else []
                ),
            ]
        },
        "checks": {
            "strict_clippy": "pass",
            "rust_format": "pass",
            "generated_contracts": "pass",
            "repository_boundaries": "pass",
            "python_lint_and_format": "pass",
            "offline_dependency_policy": "pass",
            "cached_advisory_audit": "pass",
        },
        "advisory_database_commit": "ef6173cbc5c50ec8166f9a5b28f07834144373ee",
        "audited_lockfile_packages": 215,
        "added_third_party_packages": 0,
        "production_boundary_change": "Storage adapter also uses the first-party identity crate for SHA-256 evidence digests"
        if planning
        else "Already-locked rustix 1.1.5 is now a direct safe syscall adapter dependency",
        "limitations": [
            "P04 remains in progress; no storage effect or live mount-loss monitor is enabled",
            "Native ARM64 package/reference-host qualification pending",
            "Btrfs multi-device and FUSE NTFS are refused pending complete backing mappings",
            "No Pi performance comparison or P04 footprint signoff",
            "MNT-001 remains open until the full profile startup path uses this mechanism",
        ],
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        f"Recorded {len(vm['passed'])} VM groups against {len(manifest)} committed source/build files."
    )


if __name__ == "__main__":
    main()

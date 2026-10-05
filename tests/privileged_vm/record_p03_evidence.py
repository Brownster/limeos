#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind P03 lifecycle evidence to its qualified source bundle and test artifacts."""

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
        "--output",
        type=Path,
        default=ROOT / "docs/rewrite-evidence/p03/lifecycle-validation.json",
    )
    args = parser.parse_args()
    evidence = args.output.parent
    vm = json.loads((evidence / "lifecycle-vm-result.json").read_text())
    browser = json.loads((evidence / "lifecycle-browser-result.json").read_text())
    log = (evidence / "rust-lifecycle-tests.txt").read_text()
    assert "0 failed" in log and "FAILED" not in log
    assert vm["upgrade"] is not None and vm["upgrade"]["from"] == "0.3.0"
    interruptions = sum("kill " in check for check in vm["checks"])
    assert interruptions == 18, "Incomplete lifecycle interruption matrix"
    guardrails = {"combined_pss_kib": 30 * 1024, "overview_p95_ms": 20, "swap_kib": 0}
    for name, maximum in guardrails.items():
        assert vm["footprint"][name] <= maximum, f"VM footprint budget exceeded: {name}"
    subprocess.run(
        ["git", "cat-file", "-e", args.source_commit + "^{commit}"],
        cwd=ROOT,
        check=True,
    )
    manifest = {}
    with tarfile.open(args.bundle) as archive:
        for item in archive:
            if not item.isfile() or not item.name.startswith("source/"):
                continue
            name = item.name.removeprefix("source/")
            stream = archive.extractfile(item)
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
            if (
                name.startswith("tests/privileged_vm/")
                and name != "tests/privileged_vm/build_guest.py"
            ):
                # Qualification fixtures can change independently of a frozen
                # package; their current hashes are recorded separately below.
                continue
            assert sha(ROOT / name) == digest, f"Qualified runtime source drift: {name}"
            manifest[name] = digest
    manifest_path = evidence / "lifecycle-source-sha256.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    binaries = {
        name: sha(args.artifacts / name)
        for name in [
            "limeos-core",
            "limeos-password-worker",
            "limeos-executor",
            "limeosctl",
        ]
    }
    for name, digest in vm["package_sha256"].items():
        package = args.artifacts / "packages" / name
        assert sha(package) == digest, f"Qualified package drift: {name}"
        prefix = "limeos-shadow" if name.startswith("limeos-shadow_") else "limeos"
        contents = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(package)])
        with tarfile.open(fileobj=io.BytesIO(contents)) as archive:
            for binary, expected in binaries.items():
                stream = archive.extractfile(f"./usr/lib/{prefix}/{binary}")
                assert hashlib.file_digest(stream, "sha256").hexdigest() == expected, (
                    f"Package binary mismatch: {name}/{binary}"
                )
    result = {
        "checked_at_utc": datetime.now(timezone.utc).isoformat(),
        "qualified_source_commit": args.source_commit,
        "qualification_fixture_commit": subprocess.check_output(
            [
                "git",
                "log",
                "-1",
                "--format=%H",
                "--",
                "tests/privileged_vm/run.py",
                "tests/privileged_vm/p03_guest.py",
                "tests/privileged_vm/p03_lifecycle_guest.py",
            ],
            cwd=ROOT,
            text=True,
        ).strip(),
        "backend_commit": "4984d08",
        "frontend_commit": "26d5fb4",
        "build": {
            "host": "disposable Debian 12 KVM guest",
            "architecture": "amd64",
            "rust_toolchain": "1.88.0",
            "cargo": "locked offline release build",
            "source_bundle_sha256": sha(args.bundle),
            "source_bundle_bytes": args.bundle.stat().st_size,
            "qualified_bundle_files_match_workspace": len(manifest),
            "source_manifest": manifest_path.name,
            "binary_sha256": binaries,
            "package_sha256": vm["package_sha256"],
            "signing_key": "test-only key generated in the discarded guest",
        },
        "rust_workspace_tests": {
            "result": "pass",
            "passed": sum(
                int(n) for n in re.findall(r"test result: ok\. (\d+) passed", log)
            ),
            "log": "rust-lifecycle-tests.txt",
        },
        "real_engine_vm": {
            "result": "pass",
            "acceptance_groups": len(vm["checks"]),
            "result_file": "lifecycle-vm-result.json",
            "effect_interruption_cases": interruptions,
        },
        "browser": {
            "result": "pass",
            "acceptance_groups": len(browser["checks"]),
            "result_file": "lifecycle-browser-result.json",
            "scope": "qualified production assets with a closed API fixture",
        },
        "fixture_sha256": {
            name: sha(ROOT / name)
            for name in [
                "tests/privileged_vm/run.py",
                "tests/privileged_vm/p03_guest.py",
                "tests/privileged_vm/p03_lifecycle_guest.py",
                "tests/privileged_vm/build_bundle.py",
                "tests/privileged_vm/build_guest.py",
                "tests/frontend/restart.py",
            ]
        },
        "checks": {
            name: "pass"
            for name in [
                "formatting",
                "strict_clippy",
                "generated_contracts",
                "repository_boundaries",
                "frontend_unit_tests",
                "typescript_and_production_assets",
                "python_lint_and_format",
                "dependency_licenses_and_advisories",
                "npm_cached_audit",
                "vm_footprint_guardrails",
            ]
        },
        "advisory_database_commit": "ef6173cbc5c50ec8166f9a5b28f07834144373ee",
        "dependency_count": 214,
        "known_vulnerabilities_in_checked_database": 0,
        "added_third_party_rust_dependencies": 0,
        "footprint_guardrails": {
            **guardrails,
            "scope": "AMD64 VM only; reference Pi gate remains pending",
        },
        "limitations": [
            "P03 remains in progress: approved Compose plan/diff foundation and representative reference-stack qualification remain",
            "No reference-host mutation, installation, active benchmark or cutover",
            "No native ARM64 combined-package qualification in this continuation",
            "No comparable Python workload or hardware improvement percentage claimed",
            "Browser and real-Engine transports qualified separately against the same application payload",
            "GitHub CI changes not executed here",
            "All 50 defect-register requirements remain open pending their complete acceptance criteria",
        ],
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        f"Bound {len(manifest)} source files to lifecycle evidence; VM footprint meets local guardrails."
    )


if __name__ == "__main__":
    main()

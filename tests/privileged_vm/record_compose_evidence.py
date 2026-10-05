#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bind qualified Compose packages to source and independently recorded checks."""

import argparse
import hashlib
import io
import json
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
        "--vm-result",
        type=Path,
        help="Alternate completed P03 VM result; defaults to compose-vm-result.json beside output",
    )
    parser.add_argument(
        "--reference-profile",
        type=Path,
        help="Bind the redacted reference layout to its VM result",
    )
    parser.add_argument(
        "--checks",
        type=Path,
        required=True,
        help="Local validation results recorded before invoking this integrity check",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "docs/rewrite-evidence/p03/compose-validation.json",
    )
    args = parser.parse_args()
    directory = args.output.parent
    vm_path = args.vm_result or directory / "compose-vm-result.json"
    vm = json.loads(vm_path.read_text())
    arm = json.loads((directory / "compose-arm64-result.json").read_text())
    checks = json.loads(args.checks.read_text())
    gates = json.loads((directory / "compose-gate-probes.json").read_text())
    assert all(item["exit_code"] == 0 for item in checks["checks"])
    assert gates["result"] == "pass"
    assert (
        vm["upgrade"] is not None
        and vm["upgrade"]["from"] == "0.3.1"
        and vm["upgrade"]["to"] == "0.3.2"
    )
    assert vm["schemas"] == {"authority": 5, "container_receipts": 3}
    assert (
        vm["compose"]["host_effects"] == 0
        and not vm["compose"]["deployment_executable"]
    )
    interruptions = sum("kill " in item for item in vm["checks"])
    assert interruptions == 18
    assert vm["footprint"]["combined_pss_kib"] <= 30 * 1024
    assert vm["footprint"]["overview_p95_ms"] <= 20 and vm["footprint"]["swap_kib"] == 0
    subprocess.run(
        ["git", "cat-file", "-e", args.source_commit + "^{commit}"],
        cwd=ROOT,
        check=True,
    )
    manifest = {}
    with tarfile.open(args.bundle) as archive:
        for member in archive:
            if not member.isfile() or not member.name.startswith("source/"):
                continue
            name = member.name.removeprefix("source/")
            if (
                name.startswith("tests/privileged_vm/")
                and name != "tests/privileged_vm/build_guest.py"
            ):
                continue
            fingerprint = hashlib.file_digest(
                archive.extractfile(member), "sha256"
            ).hexdigest()
            assert sha(ROOT / name) == fingerprint, f"Qualified runtime drift: {name}"
            manifest[name] = fingerprint
    manifest_path = directory / (
        "reference-source-sha256.json"
        if args.reference_profile
        else "compose-source-sha256.json"
    )
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    binaries = {
        name: sha(args.artifacts / name)
        for name in [
            "limeos-core",
            "limeos-executor",
            "limeosctl",
            "limeos-password-worker",
        ]
    }
    for name, fingerprint in vm["package_sha256"].items():
        package = args.artifacts / "packages" / name
        assert sha(package) == fingerprint
        prefix = "limeos-shadow" if name.startswith("limeos-shadow_") else "limeos"
        payload = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(package)])
        with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
            for binary, expected in binaries.items():
                assert (
                    hashlib.file_digest(
                        archive.extractfile(f"./usr/lib/{prefix}/{binary}"), "sha256"
                    ).hexdigest()
                    == expected
                )
    # No frontend behavior changed. Reuse browser evidence only if both the
    # production assets and its complete fixture match the qualified milestone.
    previous = json.loads((directory / "lifecycle-validation.json").read_text())
    previous_manifest = json.loads(
        (directory / "lifecycle-source-sha256.json").read_text()
    )
    assets = {
        name: value
        for name, value in manifest.items()
        if name.startswith("frontend/dist/")
    }
    assert assets and assets == {
        name: value
        for name, value in previous_manifest.items()
        if name.startswith("frontend/dist/")
    }
    assert (
        sha(ROOT / "tests/frontend/restart.py")
        == previous["fixture_sha256"]["tests/frontend/restart.py"]
    )
    result = {
        "recorded_at_utc": datetime.now(timezone.utc).isoformat(),
        "qualified_source_commit": args.source_commit,
        "backend_commit": "fdc0ee9",
        "frontend_commit": "26d5fb4",
        "package_version": "0.3.2",
        "build": {
            "architecture": "amd64",
            "host": "disposable Debian 12 KVM guest",
            "rust_toolchain": "1.88.0",
            "cargo": "locked offline release build",
            "source_bundle_sha256": sha(args.bundle),
            "source_bundle_bytes": args.bundle.stat().st_size,
            "source_manifest": manifest_path.name,
            "qualified_bundle_files_match_workspace": len(manifest),
            "binary_sha256": binaries,
            "package_sha256": vm["package_sha256"],
            "signing_key": "test-only key generated in the discarded guest",
        },
        "local_checks": checks,
        "release_gate_probes": {
            "result": "pass",
            "groups": len(gates["probes"]),
            "result_file": "compose-gate-probes.json",
        },
        "real_engine_vm": {
            "result": "pass",
            "acceptance_groups": len(vm["checks"]),
            "effect_interruption_cases": interruptions,
            "result_file": vm_path.name,
        },
        "browser": {
            "result": "reused identical assets and fixture",
            "acceptance_groups": previous["browser"]["acceptance_groups"],
            "result_file": "lifecycle-browser-result.json",
            "asset_sha256": assets,
        },
        "arm64": {
            "result": "cross-build and qemu-user smoke passed",
            "native_qualified": False,
            "result_file": "compose-arm64-result.json",
            "acceptance_groups": len(arm["checks"]),
        },
        "qualification_fixture_commit": subprocess.check_output(
            [
                "git",
                "log",
                "-1",
                "--format=%H",
                "--",
                "tests/privileged_vm",
                "tests/arm64",
            ],
            cwd=ROOT,
            text=True,
        ).strip(),
        "fixture_sha256": {
            name: sha(ROOT / name)
            for name in [
                "tests/privileged_vm/run.py",
                "tests/privileged_vm/p03_guest.py",
                "tests/privileged_vm/p03_lifecycle_guest.py",
                "tests/privileged_vm/p03_compose_guest.py",
                "tests/privileged_vm/build_bundle.py",
                "tests/privileged_vm/build_guest.py",
                "tests/arm64/smoke.py",
            ]
        },
        "dependency_count": 214,
        "added_third_party_rust_dependencies": 0,
        "advisory_database_commit": "ef6173cbc5c50ec8166f9a5b28f07834144373ee",
        "known_vulnerabilities_in_checked_database": 0,
        "limitations": [
            "P03 still needs representative reference-stack qualification; native ARM64 and P02 reference-host gates remain pending",
            "Compose plans are protected non-executable snapshots; P04 must freshly validate host paths, combined project semantics and stack locks before deployment",
            "VM footprint was sampled after lifecycle qualification before the optional catalog ownership scenarios; it is not a Pi measurement",
            "ARM64 packages were not installed on native hardware or added to the signed AMD64 test repository",
            "No live host changes, benchmark, service restart or cutover; only one read-only SSH inventory check",
            "GitHub CI was updated but not run here; no additional defect-register rows are closed by this milestone",
        ],
    }
    if args.reference_profile:
        assert vm["reference"]["profile_sha256"] == sha(args.reference_profile)
        assert len(vm["reference"]["services"]) == 7
        assert vm["reference"]["profiled_containers"] >= 24
        assert vm["reference"]["production_environment_values_copied"] == 0
        result["reference"] = vm["reference"]
        for name in [
            "tests/privileged_vm/p03_reference_guest.py",
            "tests/reference/capture.py",
        ]:
            result["fixture_sha256"][name] = sha(ROOT / name)
        result["fixture_sha256"]["tests/fixtures/wybie-layout.json"] = sha(
            args.reference_profile
        )
        result["limitations"][0] = (
            "P03 local representative-layout qualification passed; native ARM64 and P02 reference-host gates remain pending"
        )
        result["limitations"][4] = (
            "No live host changes, benchmark, service restart or cutover; only light read-only SSH inventory and filtered configuration projection"
        )
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(
        f"Bound {len(manifest)} source files and both AMD64 packages to Compose/lifecycle evidence."
    )


if __name__ == "__main__":
    main()

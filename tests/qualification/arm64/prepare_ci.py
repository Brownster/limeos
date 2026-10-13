#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Prepare offline CI provenance around an existing signed ARM64 test repository.

Rebuild package payloads locally with the frozen builder and unchanged native
CI binaries, compare every file/mode/owner/control, then validate the resulting
record. Does not install packages, access a test host or execute ARM64 code.
Keep output/inputs beside build-result.json when copying it to a guest.
"""

import argparse
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

from ci_provenance import (
    BUILD_KIND,
    file_map,
    inspect_ci,
    package_payload,
    require,
    verify_bundle,
    verify_repository,
)
from qualification import BINARIES, load_build, load_manifest, sha


def prepare(bundle, manifest_path, ci_inputs, repository, fingerprint, output):
    require(not output.exists(), "output exists; historical inputs are immutable")
    manifest = load_manifest(manifest_path)
    require(
        manifest["identity"].get("qualification_kind") == "native-ci-build-kvm-install",
        "CI bundle required",
    )
    verify_bundle(bundle, manifest)
    ci, runtime = inspect_ci(ci_inputs, manifest["identity"]["source_commit"])
    repo_files = verify_repository(repository, fingerprint)
    version = manifest["identity"]["package_version"]
    candidates = {p.name: p for p in repository.rglob("*.deb")}
    names = {f"{p}_{version}_arm64.deb" for p in ("limeos", "limeos-shadow")}
    require(
        set(candidates) == names, "exact candidate standard/shadow repository required"
    )
    output.mkdir(parents=True)
    inputs = output / "inputs"
    (inputs / "ci").mkdir(parents=True)
    (inputs / "packaging").mkdir()
    for name in ("run.json", "job.json", "artifact.json", "artifact.zip"):
        shutil.copy2(ci_inputs / name, inputs / "ci" / name)
    shutil.copy2(bundle, inputs / "source.tar.gz")
    shutil.copy2(manifest_path, inputs / "source-manifest.json")
    shutil.copytree(repository, inputs / "repo")
    payloads, controls, steps = {}, {}, []
    # verify_bundle checked every member before extraction. No Git checkout,
    # dependency installation, Cargo invocation or executable ARM64 input.
    with tempfile.TemporaryDirectory(prefix="limeos-ci-comparison-") as directory:
        work = Path(directory)
        with tarfile.open(bundle) as archive:
            archive.extractall(work, filter="data")
        binaries = work / "binaries"
        binaries.mkdir()
        for name in BINARIES:
            target = binaries / name
            target.write_bytes(runtime[name])
            target.chmod(0o755)
        for profile, package in (("standard", "limeos"), ("shadow", "limeos-shadow")):
            log = inputs / "packaging" / f"{profile}.txt"
            with log.open("w") as stream:
                result = subprocess.run(
                    [
                        sys.executable,
                        str(work / "source/packaging/build.py"),
                        "--arch",
                        "arm64",
                        "--version",
                        version,
                        "--profile",
                        profile,
                        "--binaries",
                        str(binaries),
                        "--output",
                        str(work / "packages"),
                    ],
                    stdout=stream,
                    stderr=subprocess.STDOUT,
                    check=False,
                )
            require(
                result.returncode == 0, f"frozen package comparison build failed: {log}"
            )
            name = f"{package}_{version}_arm64.deb"
            expected, _ = package_payload(work / "packages" / name)
            actual, _ = package_payload(candidates[name])
            require(
                actual == expected,
                f"candidate payload differs from frozen builder: {name}",
            )
            payloads[name] = expected["payload"]
            controls[name] = actual["control"]
            steps.append(
                {"profile": profile, "exit": result.returncode, "log_sha256": sha(log)}
            )
    build = {
        "build_kind": BUILD_KIND,
        "provenance_version": 1,
        **{
            k: manifest[k]
            for k in (
                "identity",
                "runtime_source_sha256",
                "frontend_dist_sha256",
                "fixtures_sha256",
            )
        },
        "source_manifest_sha256": sha(manifest_path),
        "ci": ci,
        "binaries": ci["packages"]["limeos_0.4.4_arm64.deb"]["binaries"],
        "packages": {n: sha(p) for n, p in candidates.items()},
        "package_control": controls,
        "package_payloads": payloads,
        "packaging": {
            "method": "frozen-builder-payload-comparison",
            "builder_sha256": manifest["runtime_source_sha256"]["packaging/build.py"],
            "steps": steps,
        },
        "signing_fingerprint": fingerprint,
        "repository_sha256": repo_files,
        "input_sha256": file_map(inputs),
    }
    pending = output / "build-result.pending.json"
    pending.write_text(json.dumps(build, indent=2, sort_keys=True) + "\n")
    load_build(pending, repository)
    path = output / "build-result.json"
    pending.rename(path)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--ci-inputs", type=Path, required=True)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--signing-fingerprint", required=True)
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="New directory for build-result.json and adjacent inputs/",
    )
    args = parser.parse_args()
    try:
        path = prepare(
            args.bundle,
            args.manifest,
            args.ci_inputs,
            args.repository,
            args.signing_fingerprint,
            args.output,
        )
    except (ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    except (OSError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"CI preparation failed: {error}") from error
    print(
        json.dumps(
            {
                "build_result": str(path),
                "sha256": sha(path),
                "installed_qualification": "untested",
            }
        )
    )


if __name__ == "__main__":
    main()

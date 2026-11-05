#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Provenance for a locally cross-built ARM64 candidate (nothing compiled on the Pi).

The workstation cross-compiles the release binaries offline in a pinned Debian
build image, packages them from the extracted qualification bundle with the
frozen `packaging/build.py`, and signs a disposable repository. This module
records that as `build_kind: local-cross-build` and validates it again, offline
and without executing any ARM64 code, in the guest wrappers.

It is weaker evidence than a native build: there are no native test totals and
no native toolchain steps. The record says exactly that. Executable bytes are
bound by SHA-256 to the signed packages, to the bundle manifest (frontend,
contracts, fixtures) and to a build record naming the source commit.

    cross_build.py prepare --bundle B --manifest M --binaries DIR \
        --build-record R --repository REPO --signing-fingerprint FPR --output OUT
"""

import argparse
import hashlib
import json
import shutil
import sys
from pathlib import Path

from ci_provenance import (
    file_map,
    package_payload,
    require,
    verify_bundle,
    verify_repository,
)
from qualification import BINARIES, load_manifest, sha

BUILD_KIND = "local-cross-build"
QUALIFICATION_KIND = "local-cross-build-kvm-install"
# The stack guests also run the stack read worker; it is packaged but is not
# one of the four binaries the shared installed-hash checks compare.
PACKAGED = (*BINARIES, "limeos-stack-read-worker")


def check_record(record, source_commit):
    require(
        record.get("exit_code") == 0
        and record.get("source") == source_commit
        and record.get("target") == "aarch64-unknown-linux-gnu"
        and record.get("offline") is True
        and record.get("locked") is True
        and isinstance(record.get("image"), str)
        and set(record.get("binaries", {})) == set(PACKAGED),
        "cross-build record differs from candidate source",
    )


def inspect_packages(repository, manifest, record):
    """Return (binaries, controls, payloads) after cross-checking every package."""
    contracts = {
        n: d
        for n, d in manifest["runtime_source_sha256"].items()
        if n.startswith("contracts/generated/")
    }
    version = manifest["identity"]["package_version"]
    names = {f"{p}_{version}_arm64.deb" for p in ("limeos", "limeos-shadow")}
    packages = {p.name: p for p in Path(repository).rglob("*.deb")}
    require(set(packages) == names, "exact standard/shadow repository required")
    binaries = None
    controls, payloads = {}, {}
    for name, package in sorted(packages.items()):
        payload, runtime = package_payload(package)
        if binaries is None:
            binaries = payload["binaries"]
        require(payload["binaries"] == binaries, "mixed binaries between packages")
        for binary in PACKAGED:
            require(
                runtime.get(binary, b"")[:4] == b"\x7fELF"
                and hashlib.sha256(runtime[binary]).hexdigest()
                == record["binaries"][binary]["sha256"],
                f"packaged {binary} differs from the cross-build record",
            )
        require(
            payload["frontend"] == manifest["frontend_dist_sha256"]
            and payload["contracts"] == contracts
            and bool(contracts),
            "package UI/contracts differ from the bundle",
        )
        controls[name] = payload["control"]
        payloads[name] = payload["payload"]
    return binaries, controls, payloads


def prepare(
    bundle, manifest_path, binaries_dir, record_path, repository, fingerprint, output
):
    require(not output.exists(), "output exists; historical inputs are immutable")
    manifest = load_manifest(manifest_path)
    require(
        manifest["identity"].get("qualification_kind") == QUALIFICATION_KIND,
        "local cross-build bundle required",
    )
    verify_bundle(bundle, manifest, ci_pipeline=False)
    record = json.loads(record_path.read_text())
    check_record(record, manifest["identity"]["source_commit"])
    for name in PACKAGED:
        require(
            sha(binaries_dir / name) == record["binaries"][name]["sha256"],
            f"binary differs from the build record: {name}",
        )
    repo_files = verify_repository(repository, fingerprint)
    binaries, controls, payloads = inspect_packages(repository, manifest, record)
    output.mkdir(parents=True)
    inputs = output / "inputs"
    inputs.mkdir()
    shutil.copy2(bundle, inputs / "source.tar.gz")
    shutil.copy2(manifest_path, inputs / "source-manifest.json")
    shutil.copy2(record_path, inputs / "cross-build-record.json")
    shutil.copytree(repository, inputs / "repo")
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
        "cross_build": record,
        "limitations": [
            "cross-compiled on the workstation; no native toolchain, test or lint gates",
            "installed behaviour is untested until a native KVM guest run",
        ],
        "binaries": binaries,
        "packages": {p.name: sha(p) for p in Path(repository).rglob("*.deb")},
        "package_control": controls,
        "package_payloads": payloads,
        "signing_fingerprint": fingerprint,
        "repository_sha256": repo_files,
        "input_sha256": file_map(inputs),
    }
    pending = output / "build-result.pending.json"
    pending.write_text(json.dumps(build, indent=2, sort_keys=True) + "\n")
    from qualification import load_build

    load_build(pending, repository)
    path = output / "build-result.json"
    pending.rename(path)
    return path


def validate_build(path, build, repo=None):
    """Require adjacent inputs/ and re-derive every claim from those bytes."""
    require(
        build.get("provenance_version") == 1
        and type(build["provenance_version"]) is int,
        "unknown cross-build provenance version",
    )
    require(
        "steps" not in build and "tests" not in build and "ci" not in build,
        "ambiguous cross-build and native provenance",
    )
    root = path.parent / "inputs"
    require(
        file_map(root) == build["input_sha256"], "cross-build input manifest differs"
    )
    require(
        set(build["input_sha256"])
        == {"source-manifest.json", "source.tar.gz", "cross-build-record.json"}
        | {"repo/" + n for n in build["repository_sha256"]},
        "exact cross-build input file set required",
    )
    manifest_path = root / "source-manifest.json"
    manifest = load_manifest(manifest_path)
    require(
        manifest["identity"] == build["identity"]
        and sha(manifest_path) == build["source_manifest_sha256"]
        and manifest["identity"].get("qualification_kind") == QUALIFICATION_KIND,
        "cross-build source manifest identity/bytes differ",
    )
    for field in ("runtime_source_sha256", "frontend_dist_sha256", "fixtures_sha256"):
        require(build[field] == manifest[field], f"cross-build {field} differs")
    verify_bundle(root / "source.tar.gz", manifest, ci_pipeline=False)
    record = json.loads((root / "cross-build-record.json").read_text())
    require(record == build["cross_build"], "cross-build record differs")
    check_record(record, build["identity"]["source_commit"])
    signed = root / "repo"
    require(
        verify_repository(signed, build["signing_fingerprint"])
        == build["repository_sha256"],
        "signed repository input differs",
    )
    if repo is not None:
        require(
            file_map(repo) == build["repository_sha256"],
            "installed repository differs from cross-build input",
        )
    binaries, controls, payloads = inspect_packages(signed, manifest, record)
    require(
        binaries == build["binaries"]
        and controls == build["package_control"]
        and payloads == build["package_payloads"],
        "package contents differ from the cross-build record",
    )
    require(
        {p.name: sha(p) for p in signed.rglob("*.deb")} == build["packages"],
        "package hashes differ",
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("prepare")
    p.add_argument("--bundle", type=Path, required=True)
    p.add_argument("--manifest", type=Path, required=True)
    p.add_argument("--binaries", type=Path, required=True)
    p.add_argument("--build-record", type=Path, required=True)
    p.add_argument("--repository", type=Path, required=True)
    p.add_argument("--signing-fingerprint", required=True)
    p.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        path = prepare(
            args.bundle,
            args.manifest,
            args.binaries,
            args.build_record,
            args.repository,
            args.signing_fingerprint,
            args.output,
        )
    except (ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
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
    sys.exit(main())

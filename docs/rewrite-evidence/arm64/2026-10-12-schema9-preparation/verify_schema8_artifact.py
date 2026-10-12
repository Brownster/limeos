#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Reverify retained original schema-8 artifacts; never execute their binaries.

Run from this worktree with uv. All historical inputs are read-only. New proof,
provenance and the unchanged standard package go to exclusive new output files.
"""

import hashlib
import io
import json
import re
import subprocess
import tarfile
from datetime import UTC, datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
OLD = Path(
    "/home/marc/Documents/github/lime-os-arm64-overnight/.cache/arm64-qual/overnight"
)
EVIDENCE = (
    ROOT / "docs/rewrite-evidence/arm64/2026-10-07-current-8acac40-overnight-recorded"
)
OUTPUT = ROOT / ".cache/arm64-qual/schema9-2026-10-12/upgrade-inputs/schema8"
SOURCE = "8acac403e32d3692b028a036a11f4fead2050580"


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    if OUTPUT.exists() or (HERE / "schema8-verification.json").exists():
        raise SystemExit("select fresh output paths; prior proof is immutable")
    index = json.loads((EVIDENCE / "evidence-sha256.json").read_text())
    for name, digest in index.items():
        assert sha(EVIDENCE / name) == digest, name
    recorded = EVIDENCE / "build/build-result.json"
    local = OLD / "build-result.json"
    assert sha(recorded) == sha(local), "retained and reviewed build records differ"
    build = json.loads(recorded.read_text())
    assert build["identity"]["source_commit"] == SOURCE
    assert build["identity"]["authority_schema"] == 8
    assert build["identity"]["architecture"] == "arm64"
    assert build["identity"]["package_version"] == "0.4.7+arm64.1"
    assert set(build["packages"]) == {
        "limeos_0.4.7+arm64.1_arm64.deb",
        "limeos-shadow_0.4.7+arm64.1_arm64.deb",
    }
    assert all(step["exit"] == 0 for step in build["steps"])
    manifest = EVIDENCE / "source-manifest.json"
    assert sha(manifest) == build["source_manifest_sha256"]
    assert json.loads(manifest.read_text())["identity"] == build["identity"]
    archive = subprocess.check_output(["git", "-C", str(ROOT), "archive", SOURCE])
    with tarfile.open(fileobj=io.BytesIO(archive)) as source:
        for name, digest in build["runtime_source_sha256"].items():
            assert hashlib.sha256(source.extractfile(name).read()).hexdigest() == digest
        persistence = (
            source.extractfile("crates/persistence/src/lib.rs").read().decode()
        )
        assert (
            re.search(r"pub const SCHEMA_VERSION: u32 = (\d+);", persistence)[1] == "8"
        )
    packages = {}
    for name, digest in build["packages"].items():
        package = OLD / "packages" / name
        assert package.is_file() and not package.is_symlink()
        assert sha(package) == digest, name
        assert sha(OLD / "repo/pool/main" / name) == digest, name
        control = subprocess.check_output(
            ["dpkg-deb", "-f", str(package), "Package", "Version", "Architecture"],
            text=True,
        )
        assert control == build["package_control"][name]
        profile = "limeos-shadow" if name.startswith("limeos-shadow_") else "limeos"
        payload = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(package)])
        binaries = {}
        with tarfile.open(fileobj=io.BytesIO(payload)) as contents:
            for binary, expected in build["binaries"].items():
                members = [
                    member
                    for member in contents
                    if member.name.removeprefix("./") == f"usr/lib/{profile}/{binary}"
                ]
                assert len(members) == 1 and members[0].isfile(), binary
                data = contents.extractfile(members[0]).read()
                assert hashlib.sha256(data).hexdigest() == expected["sha256"], binary
                assert len(data) == expected["bytes"], binary
                assert data[:6] == b"\x7fELF\x02\x01", binary
                assert int.from_bytes(data[18:20], "little") == 183, binary
                binaries[binary] = {**expected, "elf_class": 64, "elf_machine": 183}
        packages[name] = {
            "sha256": digest,
            "bytes": package.stat().st_size,
            "control": control,
            "binaries": binaries,
        }
    name = "limeos_0.4.7+arm64.1_arm64.deb"
    provenance = {
        "source_commit": SOURCE,
        "authority_schema": 8,
        "architecture": "arm64",
        "package_version": "0.4.7+arm64.1",
        "package_sha256": build["packages"][name],
        "core_sha256": build["binaries"]["limeos-core"]["sha256"],
        "original_evidence": str(EVIDENCE.relative_to(ROOT)),
        "original_build_sha256": sha(recorded),
        "original_source_manifest_sha256": sha(manifest),
        "original_evidence_index_sha256": sha(EVIDENCE / "evidence-sha256.json"),
        "retained_package": str(OLD / "packages" / name),
    }
    OUTPUT.mkdir(parents=True)
    with (OUTPUT / name).open("xb") as stream:
        stream.write((OLD / "packages" / name).read_bytes())
    assert sha(OUTPUT / name) == provenance["package_sha256"]
    serialized = json.dumps(provenance, indent=2, sort_keys=True) + "\n"
    for destination in (
        HERE / "previous-schema8-artifact.json",
        OUTPUT / "previous-artifact.json",
    ):
        with destination.open("x") as stream:
            stream.write(serialized)
    proof = {
        "summary": "pass",
        "verified_at": datetime.now(UTC).isoformat(),
        "verification_kind": "local retained artifact bytes; no installation or execution",
        "source_commit": SOURCE,
        "source_files_verified": len(build["runtime_source_sha256"]),
        "historical_evidence_files_verified": len(index),
        "original_build_sha256": sha(recorded),
        "previous_provenance_sha256": sha(HERE / "previous-schema8-artifact.json"),
        "packages": packages,
        "copied_package": str((OUTPUT / name).relative_to(ROOT)),
    }
    with (HERE / "schema8-verification.json").open("x") as stream:
        stream.write(json.dumps(proof, indent=2, sort_keys=True) + "\n")
    print(
        json.dumps(
            {
                "summary": "pass",
                "packages": len(packages),
                "source_files": proof["source_files_verified"],
            }
        )
    )


if __name__ == "__main__":
    main()

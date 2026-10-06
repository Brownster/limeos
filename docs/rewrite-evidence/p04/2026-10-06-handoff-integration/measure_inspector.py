#!/usr/bin/env python3
"""Measure the frozen inspector example locally with synthetic archive data.

Build the release example first. Run from any directory on the workstation;
all generated archives live in a temporary directory. This does not measure
installed services, Pi performance or restore execution.
"""

import gzip
import hashlib
import json
import os
import platform
import re
import subprocess
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]


def sha(path):
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1 << 20), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def main():
    output = HERE / "peak-memory-integrated.json"
    if output.exists():
        raise SystemExit("historical result exists; select a fresh evidence directory")
    manifest = json.loads((HERE / "source-manifest.json").read_text())
    for name in (
        "Cargo.toml",
        "Cargo.lock",
        "crates/domain/src/backups.rs",
        "crates/backup-archive/Cargo.toml",
        "crates/backup-archive/src/lib.rs",
        "crates/backup-archive/examples/inspect.rs",
    ):
        assert sha(ROOT / name) == manifest["runtime_source_sha256"][name], name
    binary = ROOT / "target/release/examples/inspect"
    policy = ROOT / "docs/rewrite-evidence/p04/rw043/measurement-policy.json"
    fixtures = ROOT / "tests/fixtures/backup-archives"
    result = {
        "source_commit": manifest["identity"]["source_commit"],
        "platform": platform.uname()._asdict(),
        "scope": "local release example VmHWM; no installed/Pi/restore measurement",
        "binary_sha256": sha(binary),
        "binary_bytes": binary.stat().st_size,
        "policy_sha256": sha(policy),
        "samples": [],
    }
    with tempfile.TemporaryDirectory(prefix="limeos-inspector-memory-") as directory:
        work = Path(directory)
        payload = work / "random.bin"
        with payload.open("wb") as data:
            for _ in range(64):
                data.write(os.urandom(1 << 20))
        plain = work / "random.tar"
        with tarfile.open(plain, "w", format=tarfile.USTAR_FORMAT) as archive:
            info = tarfile.TarInfo("opt/stacks/random.bin")
            info.size = payload.stat().st_size
            info.mode = 0o644
            with payload.open("rb") as data:
                archive.addfile(info, data)
        compressed = work / "random.tar.gz"
        with (
            plain.open("rb") as source,
            compressed.open("wb") as destination,
            gzip.GzipFile(
                filename="", fileobj=destination, mode="wb", mtime=0
            ) as writer,
        ):
            for chunk in iter(lambda: source.read(1 << 20), b""):
                writer.write(chunk)
        subprocess.run(
            ["zstd", "-q", str(plain), "-o", str(work / "random.tar.zst")], check=True
        )
        cases = [
            fixtures / name
            for name in (
                "legacy-valid.tar.gz",
                "legacy-valid.tar.zst",
                "legacy-primary-overlap.tar.zst",
                "zeros-64m.tar.zst",
            )
        ] + [compressed, work / "random.tar.zst"]
        for case in cases:
            for sample in range(3):
                command = [str(binary), str(policy), str(case)]
                measured = subprocess.run(
                    command,
                    capture_output=True,
                    text=True,
                    check=True,
                    env={**os.environ, "LIMEOS_REPORT_HWM": "1"},
                )
                admitted = json.loads(measured.stdout)
                assert admitted["manifest_version"] == 2
                assert admitted["archive_sha256"] == sha(case)
                if case.name.startswith("random."):
                    assert admitted["file_bytes"] == 64 << 20
                    assert admitted["entries"][0]["sha256"] == sha(payload)
                if case.name == "legacy-primary-overlap.tar.zst":
                    assert admitted["coalesced_self_hardlinks"] == 2
                hwm = int(re.search(r"VmHWM:\s+(\d+)\s+kB", measured.stderr)[1])
                result["samples"].append(
                    {
                        "archive": case.name,
                        "sample": sample + 1,
                        "compressed_bytes": case.stat().st_size,
                        "peak_rss_kib": hwm,
                        "manifest": admitted,
                        "stderr": measured.stderr,
                    }
                )
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    for case in cases:
        values = [
            s["peak_rss_kib"] for s in result["samples"] if s["archive"] == case.name
        ]
        print(f"{case.name}: {min(values)}–{max(values)} KiB")


if __name__ == "__main__":
    main()

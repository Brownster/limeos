#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Generate synthetic fixtures and measure a release staging example locally.

uv run --offline measure_staging.py --binary /path/to/stage --workdir /scratch \
    --output /fresh/evidence.json
No installed services, remote hosts or managed destinations are used.
"""

import argparse
import gzip
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]


def sha(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def generate(work: Path, name: str, size: int, count: int = 1) -> list[Path]:
    payload = work / f"{name}.bin"
    with payload.open("wb") as output:
        counter = 0
        remaining = size
        while remaining:
            chunk = bytearray()
            for _ in range(min(remaining, 65536) // 32):
                chunk.extend(hashlib.sha256(counter.to_bytes(8, "little")).digest())
                counter += 1
            output.write(chunk)
            remaining -= len(chunk)
    plain = work / f"{name}.tar"
    with tarfile.open(plain, "w", format=tarfile.USTAR_FORMAT) as archive:
        for index in range(count):
            info = tarfile.TarInfo(f"opt/stacks/MixedCase-{index:04}.bin")
            info.size, info.mode, info.uid, info.gid, info.mtime = size, 0o644, 0, 0, 0
            with payload.open("rb") as source:
                archive.addfile(info, source)
    compressed = work / f"{name}.tar.gz"
    with plain.open("rb") as source, compressed.open("wb") as output:
        with gzip.GzipFile(filename="", fileobj=output, mode="wb", mtime=0) as encoder:
            shutil.copyfileobj(source, encoder, 65536)
    zstd = work / f"{name}.tar.zst"
    subprocess.run(["zstd", "-q", "-3", "-T1", str(plain), "-o", str(zstd)], check=True)
    return [compressed, zstd]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--workdir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; choose a fresh evidence path")
    policy = ROOT / "tests/fixtures/backup-staging/policy.json"
    result = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "uid": os.geteuid(),
        "platform": platform.uname()._asdict(),
        "scope": "local release example: admission, replay, independent catalog reads, discard; no installed restore",
        "binary_sha256": sha(args.binary),
        "binary_bytes": args.binary.stat().st_size,
        "policy_sha256": sha(policy),
        "generator_sha256": sha(Path(__file__)),
        "zstd_version": subprocess.check_output(["zstd", "--version"], text=True).strip(),
        "samples": [],
    }
    with tempfile.TemporaryDirectory(prefix="staging-footprint-", dir=args.workdir) as directory:
        work = Path(directory)
        cases = generate(work, "small", 128 << 10) + generate(work, "large", 64 << 20)
        cases += generate(work, "metadata-1000", 0, 1000)[1:]
        cases += [ROOT / "tests/fixtures/backup-archives/zeros-64m.tar.zst"]
        for case in cases:
            archive_sha = sha(case)
            payload = work / (case.name.split(".")[0] + ".bin")
            payload_sha = sha(payload) if payload.exists() else None
            for sample in range(3):
                with tempfile.TemporaryDirectory(prefix="private-root-", dir=work) as root:
                    command = [str(args.binary), str(policy), str(case), root]
                    start = time.monotonic()
                    measured = subprocess.run(command, capture_output=True, text=True,
                        env={**os.environ, "LIMEOS_REPORT_HWM": "1"}, check=False)
                    elapsed = time.monotonic() - start
                    record = {"archive": case.name, "archive_sha256": archive_sha,
                        "sample": sample + 1, "command": command, "exit_code": measured.returncode,
                        "elapsed_seconds": elapsed, "stdout": measured.stdout, "stderr": measured.stderr,
                        "root_empty_after_discard": not any(Path(root).iterdir())}
                    result["samples"].append(record)
                    if measured.returncode:
                        args.output.write_text(json.dumps(result, indent=2) + "\n")
                        raise SystemExit(f"staging failed: {case.name}: {measured.stderr}")
                    output = json.loads(measured.stdout)
                    manifest = output["manifest"]
                    if payload_sha is not None:
                        assert all(e["sha256"] == payload_sha for e in manifest["entries"])
                        record["payload_sha256"] = payload_sha
                    assert manifest["manifest_version"] == 2
                    assert manifest["archive_sha256"] == archive_sha
                    assert output["reader_verified_bytes"] == manifest["file_bytes"]
                    assert record["root_empty_after_discard"]
                    if case.name.startswith(("large.", "zeros-64m.")):
                        assert manifest["file_bytes"] == 64 << 20
                    if case.name.startswith("metadata-"):
                        assert manifest["file_count"] == 1000 and manifest["file_bytes"] == 0
                    record["peak_rss_kib"] = int(re.search(r"VmHWM:\s+(\d+)\s+kB", measured.stderr)[1])
                    record["file_count"] = manifest["file_count"]
                    record["file_bytes"] = manifest["file_bytes"]
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    for name in sorted({s["archive"] for s in result["samples"]}):
        values = [s["peak_rss_kib"] for s in result["samples"] if s["archive"] == name]
        print(f"{name}: {min(values)}–{max(values)} KiB")


if __name__ == "__main__":
    main()

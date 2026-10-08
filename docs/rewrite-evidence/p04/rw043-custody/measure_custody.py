#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Measure the release custody example on synthetic archives, locally.

uv run --offline measure_custody.py --binary /path/to/custody --workdir /scratch \
    --output /fresh/evidence.json

Each sample uses fresh private roots and two fresh processes, as a restart
would: `retain` (copy, seal, admit, publish) and `recover --discard` (verify,
re-admit, replay into staging, independent reader hashes, discard both).
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


def generate(work: Path, name: str, size: int) -> list[Path]:
    """SHA-256 counter payload (incompressible), USTAR, gzip and zstd -3."""
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
        info = tarfile.TarInfo("opt/stacks/MixedCase.bin")
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


def private(path: Path) -> Path:
    path.mkdir(mode=0o700)
    path.chmod(0o700)
    return path


def run(command: list[str]) -> tuple[subprocess.CompletedProcess, float, int]:
    start = time.monotonic()
    done = subprocess.run(command, capture_output=True, text=True,
                          env={**os.environ, "LIMEOS_REPORT_HWM": "1"}, check=False)
    elapsed = time.monotonic() - start
    hwm = re.search(r"VmHWM:\s+(\d+)\s+kB", done.stderr)
    return done, elapsed, int(hwm[1]) if hwm else -1


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--workdir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; choose a fresh evidence path")
    fixtures = ROOT / "tests/fixtures/backup-custody"
    policy, limits = fixtures / "policy.json", fixtures / "limits.json"
    result = {
        "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "uid": os.geteuid(),
        "platform": platform.uname()._asdict(),
        "scope": "local release example in fresh processes; workstation page cache; no installed restore, Pi or service claim",
        "binary_sha256": sha(args.binary),
        "binary_bytes": args.binary.stat().st_size,
        "policy_sha256": sha(policy),
        "limits_sha256": sha(limits),
        "generator_sha256": sha(Path(__file__)),
        "zstd_version": subprocess.check_output(["zstd", "--version"], text=True).strip(),
        "samples": [],
    }
    with tempfile.TemporaryDirectory(prefix="custody-footprint-", dir=args.workdir) as directory:
        work = Path(directory)
        cases = generate(work, "small", 128 << 10) + generate(work, "large", 64 << 20)
        cases += [ROOT / "tests/fixtures/backup-archives/zeros-64m.tar.zst"]
        for case in cases:
            archive_sha = sha(case)
            payload = work / (case.name.split(".")[0] + ".bin")
            payload_sha = sha(payload) if payload.exists() else None
            for sample in range(3):
                with tempfile.TemporaryDirectory(prefix="sample-", dir=work) as scratch:
                    custody = private(Path(scratch) / "custody")
                    staging = private(Path(scratch) / "staging")
                    record = {"archive": case.name, "archive_sha256": archive_sha,
                              "compressed_bytes": case.stat().st_size, "sample": sample + 1}
                    result["samples"].append(record)
                    command = [str(args.binary), "retain", str(policy), str(limits), str(case), str(custody)]
                    done, elapsed, hwm = run(command)
                    record["retain"] = {"command": command, "exit_code": done.returncode, "stdout": done.stdout,
                                        "stderr": done.stderr, "elapsed_seconds": elapsed, "peak_rss_kib": hwm}
                    if done.returncode:
                        args.output.write_text(json.dumps(result, indent=2) + "\n")
                        raise SystemExit(f"retain failed: {case.name}: {done.stderr}")
                    retained = json.loads(done.stdout)
                    binding = retained["binding"]
                    assert binding["archive_sha256"] == archive_sha
                    assert binding["archive_bytes"] == case.stat().st_size
                    (records,) = list(custody.iterdir())
                    record["custody_bytes"] = {p.name: p.stat().st_size for p in records.iterdir()}
                    binding_path = Path(scratch) / "binding.json"
                    binding_path.write_text(json.dumps(binding))
                    command = [str(args.binary), "recover", str(policy), str(limits), str(binding_path),
                               str(custody), str(staging), "--discard"]
                    done, elapsed, hwm = run(command)
                    record["recover"] = {"command": command, "exit_code": done.returncode, "stdout": done.stdout,
                                         "stderr": done.stderr, "elapsed_seconds": elapsed, "peak_rss_kib": hwm}
                    if done.returncode:
                        args.output.write_text(json.dumps(result, indent=2) + "\n")
                        raise SystemExit(f"recover failed: {case.name}: {done.stderr}")
                    recovered = json.loads(done.stdout)
                    assert recovered["reader_verified_bytes"] == recovered["manifest_file_bytes"]
                    if case.name.startswith(("large.", "zeros-64m.")):
                        assert recovered["manifest_file_bytes"] == 64 << 20
                    record["payload_sha256"] = payload_sha
                    record["file_bytes"] = recovered["manifest_file_bytes"]
                    record["roots_empty_after_discard"] = not any(custody.iterdir()) and not any(staging.iterdir())
                    assert record["roots_empty_after_discard"]
    args.output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    for name in sorted({s["archive"] for s in result["samples"]}):
        rows = [s for s in result["samples"] if s["archive"] == name]
        retain_kib = [s["retain"]["peak_rss_kib"] for s in rows]
        recover_kib = [s["recover"]["peak_rss_kib"] for s in rows]
        print(f"{name}: retain {min(retain_kib)}-{max(retain_kib)} KiB, "
              f"recover+stage {min(recover_kib)}-{max(recover_kib)} KiB")


if __name__ == "__main__":
    main()

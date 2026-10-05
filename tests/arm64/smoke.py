#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Check cross-built ARM64 artifacts locally with qemu-user; never contact a Pi."""

import argparse
import hashlib
import io
import json
import re
import subprocess
import tarfile
from datetime import datetime, timezone
from pathlib import Path


def run(*args, **kwargs):
    return subprocess.run(
        args, check=True, capture_output=True, text=True, timeout=20, **kwargs
    )


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--sysroot", type=Path, required=True)
    parser.add_argument("--packages", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    checks = []
    binaries = {}
    for name in [
        "limeos-core",
        "limeos-executor",
        "limeosctl",
        "limeos-password-worker",
    ]:
        path = args.binaries / name
        assert "AArch64" in run("readelf", "-h", str(path)).stdout
        versions = re.findall(
            r"GLIBC_(\d+)\.(\d+)", run("readelf", "--version-info", str(path)).stdout
        )
        highest = max(tuple(map(int, v)) for v in versions)
        assert highest <= (2, 36), (name, highest)
        binaries[name] = {
            "sha256": sha(path),
            "bytes": path.stat().st_size,
            "highest_glibc": ".".join(map(str, highest)),
        }
    checks.append(
        "all four release executables are AArch64 and require no glibc newer than Debian 12"
    )
    qemu = ["qemu-aarch64-static", "-L", str(args.sysroot)]
    ctl = str(args.binaries / "limeosctl")
    assert run(*qemu, ctl, "--version").stdout.startswith("limeosctl ")
    assert "Usage: limeosctl" in run(*qemu, ctl, "--help").stdout
    checks.append("ARM64 CLI starts and serves version/help under qemu-user")
    worker = str(args.binaries / "limeos-password-worker")
    password = "synthetic-arm64-smoke-password"
    result = json.loads(
        run(
            *qemu, worker, input=json.dumps({"mode": "hash", "password": password})
        ).stdout
    )
    encoded = result["upgraded"]
    assert encoded.startswith("$argon2id$")
    for candidate, valid in [(password, True), ("incorrect-fixture-password", False)]:
        result = json.loads(
            run(
                *qemu,
                worker,
                input=json.dumps(
                    {"mode": "verify", "password": candidate, "encoded": encoded}
                ),
            ).stdout
        )
        assert result["valid"] is valid
    checks.append(
        "ARM64 password worker hashes and verifies Argon2id and rejects the wrong password under qemu-user"
    )
    packages = {}
    for package in sorted(args.packages.glob("*.deb")):
        assert (
            run("dpkg-deb", "-f", str(package), "Architecture").stdout.strip()
            == "arm64"
        )
        payload = subprocess.check_output(
            ["dpkg-deb", "--fsys-tarfile", str(package)], timeout=20
        )
        prefix = (
            "limeos-shadow" if package.name.startswith("limeos-shadow_") else "limeos"
        )
        with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
            for binary, metadata in binaries.items():
                member = archive.getmember(f"./usr/lib/{prefix}/{binary}")
                assert member.uid == 0 and member.gid == 0 and member.mode & 0o022 == 0
                assert (
                    hashlib.sha256(archive.extractfile(member).read()).hexdigest()
                    == metadata["sha256"]
                )
        packages[package.name] = {
            "sha256": sha(package),
            "bytes": package.stat().st_size,
        }
    assert len(packages) == 2
    checks.append(
        "standard and shadow ARM64 packages contain the exact cross-built executables with root ownership"
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "checked_at_utc": datetime.now(timezone.utc).isoformat(),
                "checks": checks,
                "binary_sha256": binaries,
                "package_sha256": packages,
                "execution": "qemu-user on AMD64 workstation",
                "native_arm64_qualified": False,
                "pi_contacted": False,
            },
            indent=2,
        )
        + "\n"
    )
    print(
        f"Passed {len(checks)} ARM64 artifact smoke groups; native Pi qualification remains pending."
    )


if __name__ == "__main__":
    main()

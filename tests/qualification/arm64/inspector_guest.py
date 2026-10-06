#!/usr/bin/env python3
"""Build the frozen backup-archive inspector example natively and measure it.

Runs only inside a disposable native ARM64 guest, as root, against the bundle's
frozen source. Builds `cargo build --release --locked --example inspect`,
generates synthetic archives with GNU tar, gzip and zstd, and records each
run's VmHWM as reported by the example itself. This is standalone inspector
RSS, not installed-service PSS and not a restore measurement.
"""

import argparse
import hashlib
import json
import os
import platform
import socket
import subprocess
import sys
import time
from pathlib import Path


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source", type=Path, required=True, help="Extracted frozen source"
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    if (
        os.getuid() != 0
        or socket.gethostname() != "limeos-p01-test"
        or platform.machine() != "aarch64"
    ):
        raise SystemExit("Disposable native aarch64 guest required")
    env = dict(
        os.environ, PATH="/root/.cargo/bin:" + os.environ["PATH"], CARGO_INCREMENTAL="0"
    )
    result = {
        "scope": "native standalone inspector example VmHWM; not service PSS, not restore",
        "steps": [],
    }

    def step(name, command, **kwargs):
        started = time.monotonic()
        done = subprocess.run(
            command,
            shell=True,
            text=True,
            capture_output=True,
            env=env,
            check=False,
            **kwargs,
        )
        result["steps"].append(
            {
                "step": name,
                "exit": done.returncode,
                "seconds": round(time.monotonic() - started, 1),
                "tail": done.stdout[-2000:] + done.stderr[-2000:],
            }
        )
        if done.returncode:
            args.output.write_text(json.dumps(result, indent=2) + "\n")
            raise SystemExit(f"{name} failed")
        return done

    step(
        "apt",
        "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends build-essential pkg-config ca-certificates curl zstd > /dev/null",
    )
    step(
        "rustup",
        "curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.88.0 --no-modify-path",
    )
    step(
        "build",
        "cargo build --release --locked -p limeos-backup-archive --example inspect",
        cwd=args.source,
    )
    binary = args.source / "target/release/examples/inspect"
    result["binary"] = {
        "sha256": sha(binary),
        "bytes": binary.stat().st_size,
        "elf_machine": int.from_bytes(binary.read_bytes()[18:20], "little"),
    }
    result["rustc"] = run("/root/.cargo/bin/rustc", "-V", env=env).stdout.strip()
    policy = args.source / "docs/rewrite-evidence/p04/rw043/measurement-policy.json"
    result["policy_sha256"] = sha(policy)
    work = Path("/root/qual/inspector-cases")
    (work / "tree/opt/stacks").mkdir(parents=True, exist_ok=True)
    zeros = work / "tree/opt/stacks/zeros.bin"
    random_data = work / "tree/opt/stacks/random.bin"
    step(
        "generate",
        f"head -c 67108864 /dev/zero > {zeros} && head -c 67108864 /dev/urandom > {random_data}",
    )
    cases = {}
    fixtures = args.source / "tests/fixtures/backup-archives"
    for name in ["legacy-valid.tar.zst", "legacy-valid.tar.gz", "zeros-64m.tar.zst"]:
        cases[name] = fixtures / name
    for label, member, flag in [
        ("zeros-64m-full.tar.zst", "opt/stacks/zeros.bin", "-I zstd"),
        ("random-64m.tar.zst", "opt/stacks/random.bin", "-I zstd"),
        ("random-64m.tar.gz", "opt/stacks/random.bin", "-z"),
    ]:
        path = work / label
        step(
            f"archive-{label}",
            f"tar --format=gnu -C {work / 'tree'} {flag} -cf {path} {member}",
        )
        cases[label] = path
    samples = []
    for label, path in cases.items():
        for attempt in range(args.runs):
            started = time.monotonic()
            done = subprocess.run(
                [str(binary), str(policy), str(path)],
                capture_output=True,
                text=True,
                env=dict(env, LIMEOS_REPORT_HWM="1"),
                check=False,
            )
            line = next(
                (l for l in done.stderr.splitlines() if l.startswith("VmHWM:")), ""
            )
            outcome = json.loads(done.stdout) if done.stdout.strip() else {}
            samples.append(
                {
                    "archive": label,
                    "attempt": attempt,
                    "compressed_bytes": path.stat().st_size,
                    "archive_sha256": sha(path),
                    "exit": done.returncode,
                    "admitted": done.returncode == 0,
                    "finding": (outcome.get("findings") or [{}])[0].get("code"),
                    "vmhwm_kib": int(line.split()[1]) if line else None,
                    "seconds": round(time.monotonic() - started, 3),
                }
            )
    result["samples"] = samples
    result["summary"] = {
        label: {
            "max_vmhwm_kib": max(
                s["vmhwm_kib"] or 0 for s in samples if s["archive"] == label
            ),
            "exits": sorted({s["exit"] for s in samples if s["archive"] == label}),
        }
        for label in cases
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=1))


if __name__ == "__main__":
    sys.exit(main())

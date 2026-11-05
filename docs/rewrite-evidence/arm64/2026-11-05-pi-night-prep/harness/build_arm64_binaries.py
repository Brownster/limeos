#!/usr/bin/env python3
"""Cross-compile the five release binaries for aarch64 offline in the recorded Debian build image.

Compilation happens here, on the workstation (CARGO_BUILD_JOBS=4); nothing is built on the Pi. The image has
the aarch64 Rust std and aarch64-linux-gnu-gcc (glibc 2.36 sysroot, so Debian 12 compatible). Product inputs
must equal the commit given. Writes .cache/q/build/{binaries,output.log,record.json} in this worktree.

    build_arm64_binaries.py <commit>
"""

import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
COMMIT = sys.argv[1]
IMAGE = "sha256:af684f63adae359e95f9082cc3235bcc76208baaa9cd1a48b553784e226d5ba5"  # limeos-footprint:rw005
CARGO_HOME = Path("/home/marc/Documents/github/lime-os/.cache/p04-a1-integration/cargo-home")  # vendored, offline
STATE = ROOT / ".cache/q"
OUT = STATE / "build"
PRODUCT = ["crates", "bins", "Cargo.toml", "Cargo.lock", "packaging", "frontend/src", "contracts"]
NAMES = ["limeos-core", "limeos-password-worker", "limeos-executor", "limeos-stack-read-worker", "limeosctl"]
subprocess.run(["git", "diff", "--quiet", COMMIT, "--", *PRODUCT], cwd=ROOT, check=True)
head = subprocess.check_output(["git", "rev-parse", COMMIT + "^{commit}"], cwd=ROOT, text=True).strip()
available = next(int(line.split()[1]) * 1024 for line in open("/proc/meminfo") if line.startswith("MemAvailable"))
assert available >= 6 * 1024**3, available
shutil.rmtree(OUT, ignore_errors=True)
(OUT / "binaries").mkdir(parents=True)
(STATE / "target").mkdir(exist_ok=True)
(STATE / "tmp").mkdir(exist_ok=True)
inner = (
    "set -e; rustc -V; cargo -V; aarch64-linux-gnu-gcc --version | head -1; "
    "cargo build --offline --locked --release --target aarch64-unknown-linux-gnu "
    "-p limeos-core -p limeos-executor -p limeosctl; "
    "for n in " + " ".join(NAMES) + "; do cp /target/aarch64-unknown-linux-gnu/release/$n /out/$n; done"
)
command = [
    "docker", "run", "--rm", "--name", "limeos-arm64-xbuild", "--network=none", "--memory=6g", "--memory-swap=6g",
    "--pids-limit=256", "--cpus=4", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--security-opt=label=disable",
    "--user", "1000:1000", "--volume", f"{ROOT}:/build:ro", "--volume", f"{CARGO_HOME}:/cargo-home",
    "--volume", f"{STATE}/target:/target", "--volume", f"{OUT}/binaries:/out", "--workdir", "/build",
    "--env", "CARGO_HOME=/cargo-home", "--env", "CARGO_TARGET_DIR=/target", "--env", "CARGO_BUILD_JOBS=4",
    "--env", "CARGO_INCREMENTAL=0", "--env", "TMPDIR=/target",
    "--env", "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc",
    "--env", "CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc",
    "--entrypoint", "bash", IMAGE, "-c", inner,
]
with (OUT / "output.log").open("wb") as log:
    code = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=False).returncode
record = {
    "source": head, "target": "aarch64-unknown-linux-gnu", "offline": True, "locked": True, "image": IMAGE,
    "cargo_build_jobs": 4, "exit_code": code, "command": " ".join(command[:3]) + " ... cargo build --offline --locked --release --target aarch64-unknown-linux-gnu",
}
if code == 0:
    record["binaries"] = {
        name: {"sha256": hashlib.sha256((OUT / "binaries" / name).read_bytes()).hexdigest(), "bytes": (OUT / "binaries" / name).stat().st_size}
        for name in NAMES
    }
(OUT / "record.json").write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps(record))
sys.exit(code)

#!/usr/bin/env python3
"""Package the cross-built binaries from the extracted bundle and sign a disposable offline repository.

Packaging uses the bundle's own packaging/build.py, UI and contracts (the bytes the guests re-verify). The
repository is signed in the recorded offline gpg/apt-ftparchive image with a fresh 30-day ed25519 key whose
private half stays under .cache/q/signing and is never committed. Then prepares the candidate directory.

    package_arm64_candidate.py <bundle-dir> <version> <candidate-output-dir>
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[5]
BUNDLE, VERSION, CANDIDATE = Path(sys.argv[1]).resolve(), sys.argv[2], Path(sys.argv[3]).resolve()
STATE = ROOT / ".cache/q"
BINARIES = STATE / "build/binaries"
RECORD = STATE / "build/record.json"
KEYS = STATE / "signing"
WORK = STATE / "candidate-work"
TOOLS = "sha256:e0df67a00f0f3080a0d01274c4c492c1173c6e39853913864d2cf596ad726fab"  # limeos-arm64-repo-tools:local, no network
HARNESS = ROOT / "tests/qualification/arm64"

for path in (KEYS, WORK):
    shutil.rmtree(path, ignore_errors=True)
KEYS.mkdir(mode=0o700)
WORK.mkdir()
with tarfile.open(BUNDLE / "source.tar.gz") as archive:
    archive.extractall(WORK / "bundle", filter="data")
source = WORK / "bundle/source"
packages = WORK / "packages"
environment = {**os.environ, "TMPDIR": str(STATE / "tmp")}
for profile in ("standard", "shadow"):
    subprocess.run(
        [sys.executable, str(source / "packaging/build.py"), "--arch", "arm64", "--version", VERSION, "--profile", profile,
         "--binaries", str(BINARIES), "--output", str(packages)],
        check=True, env=environment,
    )


def inside(path):
    return "/q/" + str(Path(path).relative_to(STATE))


base = [
    "docker", "run", "--rm", "--network=none", "--read-only", "--memory=128m", "--memory-swap=128m", "--pids-limit=32",
    "--cpus=1", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--security-opt=label=disable", "--user", "1000:1000",
    "--env", "GNUPGHOME=" + inside(KEYS), "--env", "TMPDIR=/q/tmp", "--volume", f"{source}:/build:ro",
    "--volume", f"{STATE}:/q", "--workdir", "/build", TOOLS,
]
subprocess.run([*base, "gpg", "--batch", "--pinentry-mode", "loopback", "--passphrase", "", "--quick-generate-key",
                "LimeOS disposable arm64 night 2026-11-07", "ed25519", "sign", "30d"], check=True)
listing = subprocess.check_output([*base, "gpg", "--batch", "--with-colons", "--list-keys"], text=True)
fingerprints = [line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:")]
assert len(fingerprints) == 1 and re.fullmatch("[A-F0-9]{40}", fingerprints[0])
files = sorted(packages.glob("*.deb"))
assert len(files) == 2, files
repository = WORK / "repository"
subprocess.run([*base, "packaging/repository.sh", inside(repository), fingerprints[0], *map(inside, files)], check=True)
subprocess.run([*base, "gpgv", "--keyring", inside(repository / "limeos-archive-keyring.gpg"), inside(repository / "dists/stable/InRelease")], check=True)
subprocess.run(
    [sys.executable, str(HARNESS / "cross_build.py"), "prepare", "--bundle", str(BUNDLE / "source.tar.gz"),
     "--manifest", str(BUNDLE / "source-manifest.json"), "--binaries", str(BINARIES), "--build-record", str(RECORD),
     "--repository", str(repository), "--signing-fingerprint", fingerprints[0], "--output", str(CANDIDATE)],
    check=True, cwd=HARNESS,
)
digest = lambda p: hashlib.sha256(Path(p).read_bytes()).hexdigest()  # noqa: E731
summary = {
    "version": VERSION, "signing_fingerprint": fingerprints[0],
    "packages": {p.name: digest(p) for p in files},
    "build_result_sha256": digest(CANDIDATE / "build-result.json"),
    "repository_files": {str(p.relative_to(repository)): digest(p) for p in sorted(repository.rglob("*")) if p.is_file()},
}
(CANDIDATE.parent / (CANDIDATE.name + "-summary.json")).write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in ("version", "signing_fingerprint", "packages", "build_result_sha256")}))

#!/usr/bin/env python3
"""Build and sign test-only packages. Never run outside the disposable guest."""

import os
import socket
import subprocess
from pathlib import Path


def run(*args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def main():
    if os.getuid() != 0 or socket.gethostname() != "limeos-p01-test":
        raise SystemExit("Disposable guest required")
    source = Path("/root/build/source")
    run("apt-get", "update", "-qq")
    run(
        "apt-get",
        "install",
        "--no-install-recommends",
        "-y",
        "build-essential",
        "ca-certificates",
        "apt-utils",
        "gnupg",
        "docker.io",
        "busybox-static",
        "pkg-config",
    )
    environment = dict(
        os.environ,
        PATH="/root/build/toolchain/bin:" + os.environ["PATH"],
        CARGO_HOME="/root/build/cargo",
        CARGO_BUILD_JOBS="2",
        CARGO_INCREMENTAL="0",
    )
    run(
        "/root/build/toolchain/bin/cargo",
        "build",
        "--release",
        "--locked",
        "--offline",
        "--bin",
        "limeos-core",
        "--bin",
        "limeos-password-worker",
        "--bin",
        "limeos-executor",
        "--bin",
        "limeosctl",
        cwd=source,
        env=environment,
    )
    packages = Path("/root/build/packages")
    for profile in ["standard", "shadow"]:
        run(
            "python3",
            str(source / "packaging/build.py"),
            "--arch",
            "amd64",
            "--binaries",
            str(source / "target/release"),
            "--version",
            "0.3.1",
            "--profile",
            profile,
            "--output",
            str(packages),
        )
    # This key lives only in a throwaway guest and is never a release signing key.
    run(
        "gpg",
        "--batch",
        "--pinentry-mode",
        "loopback",
        "--passphrase",
        "",
        "--quick-generate-key",
        "Disposable P03 test signing key <p03@test.invalid>",
        "rsa2048",
        "sign",
        "1d",
    )
    listing = subprocess.check_output(
        ["gpg", "--batch", "--with-colons", "--list-secret-keys"], text=True
    )
    key = next(
        line.split(":")[9] for line in listing.splitlines() if line.startswith("fpr:")
    )
    run(
        "bash",
        str(source / "packaging/repository.sh"),
        "/opt/limeos-repo",
        key,
        *map(str, sorted(packages.glob("*.deb"))),
    )
    print("Fresh Debian release build and signed test repository ready.", flush=True)


if __name__ == "__main__":
    main()

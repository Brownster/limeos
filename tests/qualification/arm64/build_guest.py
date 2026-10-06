#!/usr/bin/env python3
"""Native ARM64 locked build, gates and package build. Runs only inside the disposable guest.

Every step's command, exit code, duration and full log are recorded. A failing
gate is reported and the remaining steps still run, so failures stay explicit
instead of hiding later results.
"""

import argparse
import json
import os
import platform
import re
import subprocess
import sys
import time
from pathlib import Path

from qualification import BINARIES, guest_guard, load_manifest, sha, verify_source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--work",
        type=Path,
        required=True,
        help="Extracted bundle and new build output directory",
    )
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    guest_guard()
    WORK = args.work.resolve()
    SOURCE = WORK / "source"
    LOGS = WORK / "logs"
    if LOGS.exists() or (WORK / "build-result.json").exists():
        parser.error("build output already exists; use a fresh guest/run directory")
    manifest = load_manifest(args.manifest)
    verify_source(WORK, manifest)
    VERSION = manifest["identity"]["package_version"]
    LOGS.mkdir(parents=True, exist_ok=True)
    env = dict(
        os.environ,
        PATH="/root/.cargo/bin:" + os.environ["PATH"],
        CARGO_TERM_COLOR="never",
        CARGO_INCREMENTAL="0",
    )
    steps = []

    def step(name, *command, cwd=SOURCE, jobs=None, gate=True):
        local = dict(env)
        if jobs:
            local["CARGO_BUILD_JOBS"] = str(jobs)
        started = time.monotonic()
        with (LOGS / f"{name}.txt").open("w") as log:
            log.write("$ " + " ".join(command) + "\n")
            log.flush()
            result = subprocess.run(
                command,
                cwd=cwd,
                env=local,
                stdout=log,
                stderr=subprocess.STDOUT,
                check=False,
            )
        record = {
            "step": name,
            "command": " ".join(command),
            "jobs": jobs,
            "exit": result.returncode,
            "seconds": round(time.monotonic() - started, 1),
            "gate": gate,
            "log": f"logs/{name}.txt",
        }
        steps.append(record)
        print(
            f"{'PASS' if result.returncode == 0 else 'FAIL'} {name} {record['seconds']}s",
            flush=True,
        )
        write(steps)
        return result.returncode == 0

    def write(steps):
        (WORK / "build-result.json").write_text(
            json.dumps(result_doc(steps), indent=2) + "\n"
        )

    def result_doc(steps):
        return {"version": VERSION, "steps": steps, **extra}

    extra = {
        "identity": manifest["identity"],
        "source_manifest_sha256": sha(args.manifest),
        "runtime_source_sha256": manifest["runtime_source_sha256"],
        "frontend_dist_sha256": manifest["frontend_dist_sha256"],
        "fixtures_sha256": manifest["fixtures_sha256"],
    }
    step(
        "apt",
        "sh",
        "-c",
        "apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "
        "build-essential pkg-config ca-certificates curl gnupg apt-utils dpkg-dev binutils file",
        cwd=WORK,
    )
    step(
        "rustup",
        "sh",
        "-c",
        "curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal "
        "--default-toolchain 1.88.0 -c clippy -c rustfmt --no-modify-path",
        cwd=WORK,
    )
    toolchain = subprocess.run(
        ["/root/.cargo/bin/rustc", "-vV"],
        capture_output=True,
        text=True,
        env=env,
        cwd=SOURCE,
        check=False,
    ).stdout
    linker = subprocess.run(
        [
            "sh",
            "-c",
            "command -v aarch64-linux-gnu-gcc && aarch64-linux-gnu-gcc --version | head -1",
        ],
        capture_output=True,
        text=True,
        check=False,
    ).stdout
    extra["platform"] = {
        "uname": " ".join(platform.uname()),
        "cpu": sorted(
            set(re.findall(r"CPU part\s*:\s*(\S+)", Path("/proc/cpuinfo").read_text()))
        ),
        "nproc": os.cpu_count(),
        "memory_kib": int(
            re.search(r"MemTotal:\s+(\d+)", Path("/proc/meminfo").read_text()).group(1)
        ),
        "os_release": Path("/etc/os-release").read_text(),
        "dpkg_architecture": subprocess.check_output(
            ["dpkg", "--print-architecture"], text=True
        ).strip(),
        "page_size": os.sysconf("SC_PAGE_SIZE"),
        "rustc": toolchain,
        "cargo": subprocess.run(
            ["/root/.cargo/bin/cargo", "-V"],
            capture_output=True,
            text=True,
            env=env,
            cwd=SOURCE,
            check=False,
        ).stdout.strip(),
        "native_linker": linker,
        "loadavg": Path("/proc/loadavg").read_text().strip(),
        "meminfo": Path("/proc/meminfo").read_text(),
    }
    if (
        "release: 1.88.0\n" not in toolchain
        or "host: aarch64-unknown-linux-gnu\n" not in toolchain
    ):
        write(steps)
        raise SystemExit("Rust 1.88.0 with native aarch64 host required")
    write(steps)
    step("fetch", "cargo", "fetch", "--locked")
    step("fmt", "cargo", "fmt", "--all", "--", "--check")
    step(
        "clippy",
        "cargo",
        "clippy",
        "--workspace",
        "--all-targets",
        "--locked",
        "--",
        "-D",
        "warnings",
        jobs=3,
    )
    step("test", "cargo", "test", "--workspace", "--locked", "--no-fail-fast", jobs=3)
    step("contracts", "python3", "scripts/check_contracts.py", jobs=3)
    step("dependency-direction", "python3", "scripts/check_repository.py")
    verify_source(WORK, manifest)
    built = step(
        "release",
        "cargo",
        "build",
        "--release",
        "--locked",
        *[arg for name in BINARIES for arg in ("--bin", name)],
        jobs=2,
    )
    log = (LOGS / "test.txt").read_text() if (LOGS / "test.txt").exists() else ""
    totals = [
        tuple(map(int, m))
        for m in re.findall(
            r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", log
        )
    ]
    extra["tests"] = {
        "passed": sum(t[0] for t in totals),
        "failed": sum(t[1] for t in totals),
        "ignored": sum(t[2] for t in totals),
        "result_lines": len(totals),
    }
    if built:
        release = SOURCE / "target/release"
        extra["binaries"] = {}
        for name in BINARIES:
            header = subprocess.check_output(
                ["readelf", "-h", str(release / name)], text=True
            )
            extra["binaries"][name] = {
                "sha256": sha(release / name),
                "bytes": (release / name).stat().st_size,
                "machine": re.search(r"Machine:\s+(.*)", header).group(1).strip(),
            }
        packages = WORK / "packages"
        for profile in ["standard", "shadow"]:
            step(
                f"package-{profile}",
                "python3",
                "packaging/build.py",
                "--arch",
                "arm64",
                "--binaries",
                str(release),
                "--version",
                VERSION,
                "--profile",
                profile,
                "--output",
                str(packages),
            )
        # Disposable key: lives only in this guest, never a release signing key.
        step(
            "signing-key",
            "gpg",
            "--batch",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
            "--quick-generate-key",
            "Disposable ARM64 qualification key <rewrite@test.invalid>",
            "rsa2048",
            "sign",
            "2d",
        )
        listing = subprocess.check_output(
            ["gpg", "--batch", "--with-colons", "--list-secret-keys"], text=True
        )
        key = next(
            line.split(":")[9]
            for line in listing.splitlines()
            if line.startswith("fpr:")
        )
        step(
            "repository",
            "bash",
            "packaging/repository.sh",
            str(WORK / "repo"),
            key,
            *map(str, sorted(packages.glob("*.deb"))),
        )
        extra["signing_fingerprint"] = key
        extra["packages"] = {p.name: sha(p) for p in sorted(packages.glob("*.deb"))}
        extra["package_control"] = {
            p.name: subprocess.check_output(
                ["dpkg-deb", "-f", str(p), "Package", "Version", "Architecture"],
                text=True,
            )
            for p in sorted(packages.glob("*.deb"))
        }
    write(steps)
    failed = [s["step"] for s in steps if s["gate"] and s["exit"] != 0]
    print(
        "FAILED GATES: " + ", ".join(failed) if failed else "ALL GATES PASSED",
        flush=True,
    )
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

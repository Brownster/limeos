#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Verify release checks reject deliberate violations in a disposable source copy."""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
import time
import tomllib
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--container",
        help="Use this existing /build-mounted container for native tools",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "docs/rewrite-evidence/p01/gate-probes.json",
    )
    args = parser.parse_args()
    cache = ROOT / ".cache"
    cache.mkdir(exist_ok=True)
    results = []
    with tempfile.TemporaryDirectory(prefix="gate-probes-", dir=cache) as directory:
        clone = Path(directory)
        ignored = shutil.ignore_patterns(
            "target", "dist", "node_modules", "__pycache__", ".git", ".cache"
        )
        for name in [
            "crates",
            "bins",
            "spikes",
            "contracts",
            "tests",
            "scripts",
            "packaging",
            "frontend",
            ".cargo",
        ]:
            shutil.copytree(ROOT / name, clone / name, ignore=ignored)
        for name in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "deny.toml"]:
            shutil.copyfile(ROOT / name, clone / name)
        (clone / "frontend/node_modules").symlink_to(
            ROOT / "frontend/node_modules", target_is_directory=True
        )
        target = ROOT / "target/gate-probes"

        def command(argv):
            env = {**os.environ, "CARGO_TARGET_DIR": str(target)}
            if args.container and argv[0] != "npm":
                cwd = "/build/" + str(clone.relative_to(ROOT))
                argv = [
                    "docker",
                    "exec",
                    "--workdir",
                    cwd,
                    "-e",
                    "CARGO_TARGET_DIR=/build/target/gate-probes",
                    args.container,
                    *argv,
                ]
            return subprocess.run(
                argv,
                cwd=clone,
                env=env,
                capture_output=True,
                text=True,
                timeout=180,
                check=False,
            )

        def rejects(name, argv, expected):
            started = time.monotonic()
            result = command(argv)
            diagnostic = result.stdout + result.stderr
            if result.returncode == 0 or expected not in diagnostic:
                raise RuntimeError(
                    f"{name}: gate did not reject the intended violation: {diagnostic[-4096:]}"
                )
            results.append(
                {
                    "gate": name,
                    "command": argv,
                    "exit_code": result.returncode,
                    "seconds": round(time.monotonic() - started, 3),
                    "result": "rejected",
                }
            )
            print("PASS rejection: " + name, flush=True)

        def mutate(path, value, name, argv, expected):
            file = clone / path
            previous = file.read_bytes()
            try:
                file.write_text(value)
                rejects(name, argv, expected)
            finally:
                file.write_bytes(previous)

        domain = (clone / "crates/domain/src/lib.rs").read_text()
        mutate(
            "crates/domain/src/lib.rs",
            domain + "\npub fn gate_probe( ){ }\n",
            "Rust formatting",
            ["cargo", "fmt", "--all", "--", "--check"],
            "Diff in",
        )
        mutate(
            "crates/domain/src/lib.rs",
            domain + "\npub fn gate_probe() { let gate_unused = 1; }\n",
            "Clippy warnings denied",
            [
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            "unused variable",
        )
        mutate(
            "crates/domain/src/lib.rs",
            domain + "\npub unsafe fn gate_probe() {}\n",
            "First-party unsafe forbidden",
            ["cargo", "check", "--locked", "-p", "limeos-domain"],
            "declaration of an `unsafe` function",
        )
        lock = (clone / "Cargo.lock").read_text()
        version = tomllib.loads((clone / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        stale = lock.replace(
            f'name = "limeos-domain"\nversion = "{version}"',
            'name = "limeos-domain"\nversion = "0.0.0"',
        )
        assert stale != lock
        mutate(
            "Cargo.lock",
            stale,
            "Locked graph",
            ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
            "needs to be updated",
        )
        vulnerable = lock.replace(
            'name = "libsqlite3-sys"\nversion = "0.38.2"',
            'name = "libsqlite3-sys"\nversion = "0.24.1"',
        )
        assert vulnerable != lock
        mutate(
            "Cargo.lock",
            vulnerable,
            "Known advisory",
            ["cargo", "audit", "--no-fetch", "--no-yanked"],
            "RUSTSEC-2022-0090",
        )
        deny = (clone / "deny.toml").read_text()
        disallowed = deny.replace(
            'allow = ["MIT", "Apache-2.0", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib", "CDLA-Permissive-2.0"]',
            'allow = ["Zlib"]',
        )
        assert disallowed != deny
        mutate(
            "deny.toml",
            disallowed,
            "Dependency licenses",
            ["cargo", "deny", "check", "licenses"],
            "license",
        )
        shell = (clone / "packaging/debian/postinst").read_text()
        mutate(
            "packaging/debian/postinst",
            shell + "\necho $gate_unquoted\n",
            "ShellCheck",
            ["shellcheck", "packaging/debian/postinst"],
            "SC2086",
        )
        unit = (clone / "packaging/systemd/limeos-core.service").read_text()
        mutate(
            "packaging/systemd/limeos-core.service",
            unit.replace(
                "ExecStart=/usr/lib/limeos/limeos-core",
                "ExecStart=/limeos-gate-nonexistent/program",
            ),
            "systemd units",
            ["systemd-analyze", "verify", "packaging/systemd/limeos-core.service"],
            "not executable",
        )
        api = (clone / "frontend/src/api.ts").read_text()
        mutate(
            "frontend/src/api.ts",
            api + "\nconst gate_probe: string = 1;\n",
            "Frontend type check",
            ["npm", "--prefix", "frontend", "run", "check"],
            "TS2322",
        )
        html = (clone / "frontend/index.html").read_text()
        mutate(
            "frontend/index.html",
            html.replace("/src/main.tsx", "/src/gate-missing.tsx"),
            "Frontend asset build",
            ["npm", "--prefix", "frontend", "run", "build"],
            "Failed to resolve",
        )
        dependency = (clone / "crates/domain/Cargo.toml").read_text()
        mutate(
            "crates/domain/Cargo.toml",
            dependency + "\naxum.workspace = true\n",
            "Dependency direction",
            ["python3", "scripts/check_repository.py"],
            "adapter dependency in domain",
        )
        schema = (clone / "contracts/generated/types.ts").read_text()
        mutate(
            "contracts/generated/types.ts",
            schema + "// deliberate drift\n",
            "Generated contract drift",
            ["python3", "scripts/check_contracts.py"],
            "contract drift",
        )
        for argv in [
            ["git", "init", "--quiet"],
            ["git", "add", "--force", "target/gate-output.txt"],
        ]:
            (clone / "target").mkdir(exist_ok=True)
            (clone / "target/gate-output.txt").write_text("fixture")
            result = command(argv)
            if result.returncode:
                raise RuntimeError(result.stderr)
        rejects(
            "Tracked build outputs",
            ["python3", "scripts/check_repository.py"],
            "tracked build output",
        )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(
            {
                "date_utc": datetime.now(timezone.utc).isoformat(),
                "probes": results,
                "result": "pass",
            },
            indent=2,
        )
        + "\n"
    )


if __name__ == "__main__":
    main()

#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Bundle source and an installed Rust toolchain for an offline disposable-VM build."""

import argparse
import hashlib
import json
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--toolchain",
        type=Path,
        default=Path.home() / ".rustup/toolchains/1.88.0-x86_64-unknown-linux-gnu",
    )
    parser.add_argument(
        "--registry", type=Path, default=Path.home() / ".cargo/registry"
    )
    args = parser.parse_args()
    if not (ROOT / "frontend/dist/index.html").is_file():
        parser.error("Build the frontend before bundling.")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(args.output, "w:gz", compresslevel=1) as archive:
        for name in [
            "Cargo.toml",
            "Cargo.lock",
            "bins",
            "crates",
            "spikes",
            "packaging",
            "contracts/generated",
            "frontend/dist",
            "docs/p01-operations.md",
            "docs/p02-operations.md",
            "docs/p03-operations.md",
            "docs/p03-compose-planning.md",
            "docs/p04-storage-readiness.md",
            "docs/p04-storage-planning.md",
            "tests/privileged_vm",
            "tests/fixtures",
        ]:
            archive.add(
                ROOT / name,
                arcname=f"source/{name}",
                filter=lambda info: (
                    None if "__pycache__" in Path(info.name).parts else info
                ),
            )
        archive.add(args.toolchain / "lib", arcname="toolchain/lib")
        for name in ["cargo", "rustc", "rustdoc"]:
            archive.add(args.toolchain / "bin" / name, arcname=f"toolchain/bin/{name}")
        archive.add(args.registry, arcname="cargo/registry")
    with args.output.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    print(
        json.dumps(
            {
                "bundle": str(args.output),
                "sha256": digest,
                "bytes": args.output.stat().st_size,
            }
        )
    )


if __name__ == "__main__":
    main()

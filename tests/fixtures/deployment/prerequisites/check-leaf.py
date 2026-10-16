#!/usr/bin/env uv run python3
# /// script
# dependencies = []
# ///
"""Prepare an isolated Cargo harness for the exact unregistered prerequisite leaf.

Usage: UV_CACHE_DIR=<on-disk-cache> uv run check-leaf.py
Then run the printed Cargo commands sequentially, with one build job.
"""

import hashlib
import json
import tomllib
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[4]
    harness = root / ".cache" / "prerequisites-leaf"
    harness.mkdir(parents=True, exist_ok=True)
    source = root / "bins/executor/src/updates/mod.rs"
    dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
        "dependencies"
    ]
    manifest = f'''[package]
name = "limeos-prerequisites-leaf-check"
version = "0.0.0"
edition = "2024"
rust-version = "1.88"

[workspace]

[lib]
path = "lib.rs"

[dependencies]
sha2 = "{dependencies["sha2"]}"

[dev-dependencies]
serde = {{ version = "{dependencies["serde"]["version"]}", features = ["derive"] }}
serde_json = "{dependencies["serde_json"]}"
'''
    (harness / "Cargo.toml").write_text(manifest)
    (harness / "lib.rs").write_text(
        f"#[path = {json.dumps(str(source))}]\npub mod updates;\n"
    )
    inputs = [
        source,
        source.with_name("prerequisites.rs"),
        source.with_name("prerequisites_tests.rs"),
        Path(__file__).resolve(),
        Path(__file__).with_name("cases.json"),
    ]
    (harness / "source-inputs.json").write_text(
        json.dumps(
            {
                str(path.relative_to(root)): hashlib.sha256(
                    path.read_bytes()
                ).hexdigest()
                for path in inputs
            },
            indent=2,
        )
        + "\n"
    )
    print(f"Prepared {harness}; shared manifests and exports unchanged")
    print(f"cargo test --offline --manifest-path {harness / 'Cargo.toml'}")
    print(
        f"cargo clippy --offline --manifest-path {harness / 'Cargo.toml'} --all-targets -- -D warnings"
    )


if __name__ == "__main__":
    main()

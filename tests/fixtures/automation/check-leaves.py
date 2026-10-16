#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare an isolated B1 leaf compilation harness; never edit shared crate roots.

Usage: uv run --offline tests/fixtures/automation/check-leaves.py PATH_ON_DISK
Then run cargo test --offline --manifest-path PATH_ON_DISK/Cargo.toml automation.
This is compilation with proposed module declarations, not installed registration.
"""
from pathlib import Path
import argparse
import json
import tomllib


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    destination = args.destination.resolve()
    if destination == Path("/tmp") or Path("/tmp") in destination.parents:
        parser.error("Use the project's on-disk cache, not /tmp")
    if root not in destination.parents:
        parser.error("Harness destination must be inside the worktree")
    dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["dependencies"]
    packages = {
        "domain": ("limeos-b1-domain", ["serde", "serde_json"],
            'pub use original_domain::*;\n#[path = ' + json.dumps(str(root / "crates/domain/src/automation/mod.rs")) + ']\npub mod automation;\n'),
        "api": ("limeos-b1-api", ["futures-util", "axum", "tokio", "serde", "serde_json", "subtle", "hyper", "hyper-util", "tower", "http-body-util"],
            'include!(' + json.dumps(str(root / "crates/api/src/lib.rs")) + ');\n#[path = ' + json.dumps(str(root / "crates/api/src/automation.rs")) + ']\npub mod automation;\n'),
        "runner": ("limeos-b1-runner", [],
            '#[path = ' + json.dumps(str(root / "bins/core/src/automation/mod.rs")) + ']\npub mod automation;\n'),
    }
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "Cargo.toml").write_text('[workspace]\nresolver = "3"\nmembers = ["domain", "api", "runner"]\n')
    for directory, (name, deps, source) in packages.items():
        package = destination / directory
        (package / "src").mkdir(parents=True, exist_ok=True)
        manifest = f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n[dependencies]\n'
        for dep in deps:
            value = dependencies[dep]
            if isinstance(value, str):
                manifest += f"{dep} = {json.dumps(value)}\n"
            else:
                if dep == "tokio":
                    value = dict(value, features=[*value["features"], "test-util"])
                fields = ', '.join(f'{key} = {json.dumps(val)}' for key, val in value.items())
                manifest += f"{dep} = {{ {fields} }}\n"
        if directory == "domain":
            manifest += 'original-domain = { package = "limeos-domain", path = ' + json.dumps(str(root / "crates/domain")) + ' }\n'
        else:
            manifest += 'limeos-domain = { package = "limeos-b1-domain", path = "../domain" }\n'
        if directory == "api":
            manifest += 'limeos-contracts = { path = ' + json.dumps(str(root / "crates/contracts")) + ' }\n'
        manifest += '[lints.rust]\nunsafe_code = "forbid"\n'
        (package / "Cargo.toml").write_text(manifest)
        (package / "src/lib.rs").write_text(source)
    print(f"Prepared isolated leaf harness: {destination}")


if __name__ == "__main__":
    main()

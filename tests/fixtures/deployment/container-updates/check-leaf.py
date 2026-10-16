#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare isolated container-update policy checks without shared root changes."""

import json
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[4]
destination = root / ".cache/container-update-leaf"
destination.mkdir(parents=True, exist_ok=True)
dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
    "dependencies"
]
(destination / "Cargo.toml").write_text(
    '[package]\nname = "limeos-container-update-leaf"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n'
    "[workspace]\n[dependencies]\n"
    f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
    f'serde = {{ version = "{dependencies["serde"]["version"]}", features = ["derive"] }}\n'
    f'serde_json = "{dependencies["serde_json"]}"\n'
    '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
)
(destination / "lib.rs").write_text(
    "pub use original_domain::*;\n"
    f"#[path = {json.dumps(str(root / 'crates/domain/src/deployment/mod.rs'))}]\npub mod deployment;\n"
)
print(f"Prepared {destination}; shared roots unchanged")

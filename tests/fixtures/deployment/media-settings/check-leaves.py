#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare Servarr settings/domain checks without shared root changes."""

import json
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[4]
destination = root / ".cache/media-settings-leaf"
deps = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["dependencies"]
common = '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
for name in ["domain", "adapter"]:
    (destination / name).mkdir(parents=True, exist_ok=True)
(destination / "Cargo.toml").write_text(
    '[workspace]\nmembers = ["domain", "adapter"]\nresolver = "2"\n'
)
for name in ["domain", "adapter"]:
    manifest = (
        f'[package]\nname = "limeos-settings-{name}-leaf"\nversion = "0.0.0"\n'
        'edition = "2024"\npublish = false\n[dependencies]\n'
        f'serde = {{ version = "{deps["serde"]["version"]}", features = ["derive"] }}\n'
        f'serde_json = "{deps["serde_json"]}"\n'
    )
    if name == "domain":
        manifest += f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
        source = (
            "pub use original_domain::*;\n"
            f"#[path = {json.dumps(str(root / 'crates/domain/src/deployment/mod.rs'))}]\npub mod deployment;\n"
        )
    else:
        manifest += 'limeos-domain = { package = "limeos-settings-domain-leaf", path = "../domain" }\n'
        source = f"#[path = {json.dumps(str(root / 'crates/executor-container/src/deployment'))}]\npub mod deployment {{ pub mod media; pub mod media_settings; }}\n"
    (destination / name / "Cargo.toml").write_text(manifest + common)
    (destination / name / "lib.rs").write_text(source)
print(f"Prepared {destination}; shared roots unchanged")

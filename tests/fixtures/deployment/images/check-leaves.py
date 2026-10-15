#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare isolated image content/domain checks without changing shared roots."""

import json
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[4]
destination = root / ".cache/image-content-leaf"
dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
    "dependencies"
]
for name in ["domain", "adapter"]:
    (destination / name).mkdir(parents=True, exist_ok=True)
(destination / "Cargo.toml").write_text(
    '[workspace]\nmembers = ["domain", "adapter"]\nresolver = "2"\n'
)
common = '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
(destination / "domain/Cargo.toml").write_text(
    '[package]\nname = "limeos-image-domain-leaf"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n[dependencies]\n'
    f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
    f'serde = {{ version = "{dependencies["serde"]["version"]}", features = ["derive"] }}\n'
    f'serde_json = "{dependencies["serde_json"]}"\n' + common
)
(destination / "domain/lib.rs").write_text(
    "pub use original_domain::*;\n"
    f"#[path = {json.dumps(str(root / 'crates/domain/src/deployment/mod.rs'))}]\npub mod deployment;\n"
)
(destination / "adapter/Cargo.toml").write_text(
    '[package]\nname = "limeos-image-adapter-leaf"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n[dependencies]\n'
    'limeos-domain = { package = "limeos-image-domain-leaf", path = "../domain" }\n'
    f'serde = {{ version = "{dependencies["serde"]["version"]}", features = ["derive"] }}\n'
    f'serde_json = "{dependencies["serde_json"]}"\n'
    f'sha2 = "{dependencies["sha2"]}"\nhex = "{dependencies["hex"]}"\n' + common
)
(destination / "adapter/lib.rs").write_text(
    f"#[path = {json.dumps(str(root / 'crates/executor-container/src/deployment'))}]\npub mod deployment {{ pub mod images; }}\n"
)
print(f"Prepared {destination}; shared roots unchanged")

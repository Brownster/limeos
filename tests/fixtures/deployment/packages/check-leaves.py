#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare isolated package domain/parser checks without shared root changes."""

import json
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[4]
destination = root / ".cache/package-leaf"
dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
    "dependencies"
]
for name in ["domain", "adapter"]:
    (destination / name).mkdir(parents=True, exist_ok=True)
(destination / "Cargo.toml").write_text(
    '[workspace]\nmembers = ["domain", "adapter"]\nresolver = "2"\n'
)
(destination / "domain/Cargo.toml").write_text(
    '[package]\nname = "limeos-package-domain-leaf"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n'
    "[dependencies]\n"
    f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
    f'serde = {{ version = "{dependencies["serde"]["version"]}", features = ["derive"] }}\n'
    f'serde_json = "{dependencies["serde_json"]}"\n'
    '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
)
(destination / "domain/lib.rs").write_text(
    "pub use original_domain::*;\n"
    f"#[path = {json.dumps(str(root / 'crates/domain/src/deployment/mod.rs'))}]\npub mod deployment;\n"
)
(destination / "adapter/Cargo.toml").write_text(
    '[package]\nname = "limeos-package-adapter-leaf"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n'
    "[dependencies]\n"
    'limeos-domain = { package = "limeos-package-domain-leaf", path = "../domain" }\n'
    '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
)
(destination / "adapter/lib.rs").write_text(
    f"pub mod updates {{ #[path = {json.dumps(str(root / 'bins/executor/src/updates'))}] pub mod leaves {{ pub mod packages; }} }}\n"
)
print(f"Prepared {destination}; shared roots unchanged")

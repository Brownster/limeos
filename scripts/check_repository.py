#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Check dependency direction and tracked build-output hygiene."

import subprocess
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[1]
ALLOWED = {
    "domain": set(),
    "contracts": {"domain"},
    "policy": {"domain"},
    "identity": set(),
    "persistence": {"domain", "identity", "policy"},
    "api": {"domain", "contracts"},
    "executor-protocol": {"contracts", "domain"},
    "observations": {"contracts", "domain", "executor-protocol"},
}
for crate, allowed in ALLOWED.items():
    config = tomllib.loads((ROOT / "crates" / crate / "Cargo.toml").read_text())
    deps = config.get("dependencies", {})
    actual = {dep.removeprefix("limeos-") for dep in deps if dep.startswith("limeos-")}
    assert actual <= allowed, (
        f"{crate}: dependency direction violation {actual - allowed}"
    )
    if crate in ["domain", "policy", "contracts", "identity"]:
        assert not {"axum", "rusqlite", "reqwest", "hyper"} & deps.keys(), (
            f"{crate}: adapter dependency in domain"
        )
    assert config["lints"]["workspace"], f"{crate}: unsafe-code policy not inherited"
listing = subprocess.run(
    ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=False
)
if listing.returncode == 0:
    for file in listing.stdout.splitlines():
        assert not any(
            part in ["target", "node_modules", "dist", "__pycache__", ".ruff_cache"]
            for part in Path(file).parts
        ), f"tracked build output: {file}"
assert (ROOT / "Cargo.lock").is_file(), "Cargo.lock is required"
assert not (ROOT / "Docs").exists(), "Only one docs root is allowed"
print("Repository dependency and hygiene checks passed.")

#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"Check dependency direction, tracked build-output hygiene and the evidence policy."

import os
import subprocess
from pathlib import Path

import tomllib

from evidence_bundles import tracked_violations

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
    "executor-container": {"domain", "identity"},
    "executor-storage": {"domain", "identity"},
    "backup-archive": {"domain"},
}
discovered = {
    manifest.parent.name for manifest in (ROOT / "crates").glob("*/Cargo.toml")
}
assert discovered == ALLOWED.keys(), (
    f"crate boundary registry mismatch: unregistered crates {sorted(discovered - ALLOWED.keys())}; "
    f"missing crates {sorted(ALLOWED.keys() - discovered)}"
)
for crate, allowed in ALLOWED.items():
    config = tomllib.loads((ROOT / "crates" / crate / "Cargo.toml").read_text())
    # Tests, build scripts, aliases and target-specific sections must preserve
    # the same boundary as the production dependency list.
    deps = {
        details.get("package", name) if isinstance(details, dict) else name
        for section in [config, *config.get("target", {}).values()]
        for kind in ["dependencies", "dev-dependencies", "build-dependencies"]
        for name, details in section.get(kind, {}).items()
    }
    actual = {dep.removeprefix("limeos-") for dep in deps if dep.startswith("limeos-")}
    assert actual <= allowed, (
        f"{crate}: dependency direction violation {actual - allowed}"
    )
    if crate in ["domain", "policy", "contracts", "identity"]:
        assert not {"axum", "rusqlite", "reqwest", "hyper"} & deps, (
            f"{crate}: adapter dependency in domain"
        )
    assert config["lints"]["workspace"], f"{crate}: unsafe-code policy not inherited"
listing = subprocess.run(
    ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=False
)
if listing.returncode == 0:
    for file in map(os.fsdecode, filter(None, listing.stdout.split(b"\0"))):
        assert not any(
            part in ["target", "node_modules", "dist", "__pycache__", ".ruff_cache"]
            for part in Path(file).parts
        ), f"tracked build output: {file}"
# Raw evidence lives in release bundles (docs/rewrite-evidence/README.md).
evidence = tracked_violations(ROOT)
assert not evidence, "evidence policy violation: " + "; ".join(evidence[:20])
assert (ROOT / "Cargo.lock").is_file(), "Cargo.lock is required"
assert not (ROOT / "Docs").exists(), "Only one docs root is allowed"
print("Repository dependency and hygiene checks passed.")

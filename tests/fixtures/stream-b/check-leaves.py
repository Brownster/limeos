#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare combined Stream B leaf checks inside this worktree's disk cache.

Run cargo test/clippy against .cache/stream-b-leaf/Cargo.toml, one job at a time.
This compiles proposed modules together; it does not register production routes,
Store methods, workers, authority or effects. Shared roots/manifests stay intact.
"""

import json
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[3]
destination = root / ".cache/stream-b-leaf"
dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
    "dependencies"
]


def module(name: str, relative: str) -> str:
    return f"#[path = {json.dumps(str(root / relative))}]\npub mod {name};\n"


packages = {
    "domain": (
        ["serde", "serde_json"],
        "pub use original_domain::*;\n"
        + module("automation", "crates/domain/src/automation/mod.rs")
        + module("deployment", "crates/domain/src/deployment/mod.rs"),
    ),
    "adapter": (
        ["serde", "serde_json", "sha2", "hex", "tokio"],
        "pub use original_executor::{compose_files, compose_management, docker};\n"
        + module("deployment", "crates/executor-container/src/deployment/mod.rs")
        + module("updates", "bins/executor/src/updates/mod.rs"),
    ),
    "api": (
        [
            "futures-util",
            "axum",
            "tokio",
            "serde",
            "serde_json",
            "subtle",
            "hyper",
            "hyper-util",
            "tower",
            "http-body-util",
        ],
        f"include!({json.dumps(str(root / 'crates/api/src/lib.rs'))});\n"
        + module("automation", "crates/api/src/automation.rs")
        + module("deployment", "crates/api/src/deployment.rs")
        + "pub fn proposed_routes<B: deployment::DeploymentBackend + automation::AutomationBackend>(backend: B, origin: String) -> Router {\n"
        "deployment::routes().merge(automation::routes()).with_state(App { backend, origin: origin.into(), limiter: Arc::new(Mutex::new(Limiter::default())), readers: Arc::new(Semaphore::new(2)) })\n}\n",
    ),
    "runner": (
        [],
        module("automation", "bins/core/src/automation/mod.rs"),
    ),
}
destination.mkdir(parents=True, exist_ok=True)
(destination / "Cargo.toml").write_text(
    '[workspace]\nresolver = "3"\nmembers = ["domain", "adapter", "api", "runner"]\n'
)
for name, (deps, source) in packages.items():
    package = destination / name
    package.mkdir(exist_ok=True)
    manifest = (
        f'[package]\nname = "limeos-stream-b-{name}-leaf"\nversion = "0.0.0"\n'
        'edition = "2024"\npublish = false\n[dependencies]\n'
    )
    for dep in deps:
        value = dependencies[dep]
        if isinstance(value, str):
            manifest += f"{dep} = {json.dumps(value)}\n"
        else:
            if dep == "tokio":
                value = dict(value, features=sorted({*value["features"], "test-util"}))
            fields = ", ".join(
                f"{key} = {json.dumps(val)}" for key, val in value.items()
            )
            manifest += f"{dep} = {{ {fields} }}\n"
    if name == "domain":
        manifest += f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
    else:
        manifest += 'limeos-domain = { package = "limeos-stream-b-domain-leaf", path = "../domain" }\n'
    if name == "adapter":
        manifest += f'original-executor = {{ package = "limeos-executor-container", path = {json.dumps(str(root / "crates/executor-container"))} }}\n'
    if name == "api":
        manifest += f"limeos-contracts = {{ path = {json.dumps(str(root / 'crates/contracts'))} }}\n"
    (package / "Cargo.toml").write_text(
        manifest + '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n'
    )
    (package / "lib.rs").write_text(source)
print(f"Prepared {destination}; shared production registrations unchanged")

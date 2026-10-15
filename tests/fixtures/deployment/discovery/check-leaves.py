#!/usr/bin/env -S uv run --offline
# /// script
# dependencies = []
# ///
"""Prepare exact B2 leaf compilation without editing shared crate roots."""

import json
import tomllib
from pathlib import Path


def main() -> None:
    root = Path(__file__).resolve().parents[4]
    destination = root / ".cache/b2-discovery-harness"
    dependencies = tomllib.loads((root / "Cargo.toml").read_text())["workspace"][
        "dependencies"
    ]
    packages = {
        "domain": (
            "limeos-b2-domain",
            ["serde", "serde_json"],
            f"pub use original_domain::*;\n#[path = {json.dumps(str(root / 'crates/domain/src/deployment/mod.rs'))}]\npub mod deployment;\n",
        ),
        "adapter": (
            "limeos-b2-adapter",
            ["tokio"],
            (
                "pub use original_executor::{compose_files, compose_management, docker};\n"
                f"#[path = {json.dumps(str(root / 'crates/executor-container/src/deployment/mod.rs'))}]\npub mod deployment;\n"
            ),
        ),
        "api": (
            "limeos-b2-api",
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
            (
                f"include!({json.dumps(str(root / 'crates/api/src/lib.rs'))});\n"
                f"#[path = {json.dumps(str(root / 'crates/api/src/deployment.rs'))}]\npub mod deployment;\n"
                "pub fn proposed_routes<B: deployment::DeploymentBackend>(backend: B, origin: String) -> Router {\n"
                "deployment::routes().with_state(App { backend, origin: origin.into(), limiter: Arc::new(Mutex::new(Limiter::default())), readers: Arc::new(Semaphore::new(2)) })\n}\n"
            ),
        ),
    }
    destination.mkdir(parents=True, exist_ok=True)
    (destination / "Cargo.toml").write_text(
        '[workspace]\nresolver = "3"\nmembers = ["domain", "adapter", "api"]\n'
    )
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
                fields = ", ".join(
                    f"{key} = {json.dumps(val)}" for key, val in value.items()
                )
                manifest += f"{dep} = {{ {fields} }}\n"
        if directory == "domain":
            manifest += f'original-domain = {{ package = "limeos-domain", path = {json.dumps(str(root / "crates/domain"))} }}\n'
        else:
            manifest += (
                'limeos-domain = { package = "limeos-b2-domain", path = "../domain" }\n'
            )
        if directory == "adapter":
            manifest += f'original-executor = {{ package = "limeos-executor-container", path = {json.dumps(str(root / "crates/executor-container"))} }}\n'
        if directory == "api":
            manifest += f"limeos-contracts = {{ path = {json.dumps(str(root / 'crates/contracts'))} }}\n"
        manifest += '[lints.rust]\nunsafe_code = "forbid"\n'
        (package / "Cargo.toml").write_text(manifest)
        (package / "src/lib.rs").write_text(source)
    print(f"Prepared {destination}; shared manifests/exports unchanged")


if __name__ == "__main__":
    main()

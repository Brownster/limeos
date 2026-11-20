# Deferred code

The production scope is the basic cutover surface: host metrics, container
status/logs/approved actions, stack status and whole-stack actions,
identity/sessions, job resolution and the migration tools. Code built ahead of
that scope, with no production caller, stays in the repository behind a cargo
feature named `deferred`. Without the feature it is not compiled, so it is not in
the shipped binaries or the Debian packages.

## What is deferred

| Crate | Gated code |
|---|---|
| `limeos-backup-archive` | The whole crate. Without the feature it is an empty library; its tests and examples set `required-features = ["deferred"]`. |
| `limeos-domain` | `deployment::{activity, catalog, container_updates, download_clients, egress, indexer_apps, local_config, media, media_settings, networks, notifications, packages, quickstart, tools}`; `backups`, `compose_override`, `storage_impact`, `pools::document`, `shares::document`, `protection::scrub`; the functions `plan_snapraid_sync`, `declared_container_storage_dependencies`, `smart_signal`, `required_mount_signal`, `protection_signal` (and `ProtectionHealth`), `Notification::recover_interrupted`, `HealthPolicyDefinition::from_planned`, and `automation::settings::plan_health_settings` with its private helpers. |
| `limeos-executor-container` | `deployment::{activity, download_clients, egress, images, indexer_apps, media, media_settings, networks, notifications, tools}`; `compose_source`; `compose_management::sources`; Compose JSON source admission in `compose_files`. |
| `limeos-executor-storage` | `protection_sources`. |
| `limeos-observations` | `pool_protection`, `shares`. |
| `limeos-executor` | `updates` (host prerequisites and dpkg projection). Before this gate it was never declared, so it was never compiled or tested. |
| `limeos-core` | No production code; the `backup_preparation_binding` integration test needs the feature. |

Each crate's `deferred` feature also enables `deferred` in the workspace crates it
depends on. A tests block or test function that uses deferred code carries the
same `#[cfg(feature = "deferred")]`.

## What stays live although dormant

Packaged services and registered routes are product decisions, not dead code. The
storage-mounts and storage-targets executor roles (installed, not enabled by
default), the storage, storage-mount, Compose-plan and automation routes, the
backup-preparation intent, and the RW-040 combined-read worker and its probes all
remain in the default build.

## Building and testing

```sh
# Default configuration, as shipped
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# With deferred code
cargo clippy --workspace --all-targets --locked --features deferred -- -D warnings
cargo test --workspace --locked --features deferred
```

With `--workspace`, `--features deferred` enables the feature on every member that
declares it. A single crate can be built with `-p <crate> --features deferred`.
CI runs both configurations, builds the Debian packages from the default
configuration and fails if `cargo tree` shows `deferred` enabled for
`limeos-core`, `limeos-executor` or `limeosctl`. `scripts/check_repository.py`
rejects a manifest that turns `deferred` on by default or through a dependency
entry.

Generated contracts come from the default configuration; no deferred type
appears in them.

To bring code back into scope, remove its `#[cfg(feature = "deferred")]` lines
together with the change that gives it a production caller.

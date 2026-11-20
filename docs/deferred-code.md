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
| `limeos-domain` | `deployment::{activity, catalog, container_updates, download_clients, egress, indexer_apps, local_config, media, media_settings, networks, notifications, packages, quickstart, tools}`; `backups`, `compose_override`, `storage_impact`, `pools::document`, `shares::document`, `protection::scrub`; the functions `plan_snapraid_sync`, `declared_container_storage_dependencies`, `smart_signal`, `required_mount_signal`, `protection_signal` (and `ProtectionHealth`), `Notification::recover_interrupted`, `HealthPolicyDefinition::from_planned`, and the legacy health settings planner `automation::settings::plan_health_settings` with its private helpers and the types only it produces (`PlannedHealthSettings`, `PlannedSilence`, `LegacyHealthEnvironment`, `LegacyPolicyRead`, `PolicyOrigin`, `PolicyFallback`, `SettingField`, `ReviewReason`, `SettingsReview`, `MAX_POLICY_BYTES`, `MAX_ENVIRONMENT_BYTES`). `PlannedTimestamp` and the timestamp parser stay live because health policy validation uses them. |
| `limeos-executor-container` | `deployment::{activity, download_clients, egress, images, indexer_apps, media, media_settings, networks, notifications, tools}`; `compose_source`; `compose_management::sources`; Compose JSON source admission in `compose_files` (`admit_json_sources`, `admit_json_with`, `read_body`); `docker::process_evidence::EngineReviewFacts` and `EngineStorageProcessEvidence::review_facts`, used only by the source review. |
| `limeos-executor-storage` | `protection_sources`. |
| `limeos-observations` | `pool_protection`, `shares`. |
| `limeos-executor` | `updates` (host prerequisites and dpkg projection). Before this gate it was never declared, so it was never compiled or tested. |
| `limeos-api` | `automation` and `deployment`: proposed routes that no router mounts. Previously undeclared; declared with `#[allow(dead_code)]`. |
| `limeos-core` | `automation`: the proposed observation runner, previously undeclared, also with `#[allow(dead_code)]`. The `backup_preparation_binding` integration test needs the feature: each of its six tests builds its input through the backup-archive custody, staging and preparation pipeline. The persistence `backup_preparation` store keeps its own ten tests in the default configuration. |

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
rejects any feature other than a crate's own `deferred` that reaches `deferred`
or `dep/deferred`, directly or transitively (so `default` cannot), and any
dependency entry that enables it, in every workspace manifest. It also fails on a
`.rs` file under a crate's `src/` that no `mod` declaration reaches.

The leaf harnesses under `tests/fixtures` run the gated modules in place with
the feature, through `tests/fixtures/leaf_runner.py`, which fails when no test
ran.

Generated contracts come from the default configuration; no deferred type
appears in them.

## Measurements

Recorded on 19 and 20 November 2026 for base `c49906d` (before) and the tree of
`911646b` (after the gating commits; later commits add tests and checks only). An
8-core aarch64 workstation with Rust 1.88.0, **shared with other engineers'
builds**, so the times are indicative only; sizes and counts are exact.

Each measured tree was a `git archive` export. Commands:

- Release build: a fresh, empty `CARGO_TARGET_DIR`, then
  `/usr/bin/time -v cargo build --locked --release -p limeos-core -p limeos-executor -p limeosctl`.
- Sizes: `stat -c %s` of the five packaged binaries in `target/release`.
- Dependencies: unique lines of
  `cargo tree --locked -e normal,build --prefix none --target all` for each shipped
  package and for `--workspace`, and of `-e all` for the whole workspace.
- Tests: in a target directory with dependencies already built, so only workspace
  crates recompile, `cargo test --workspace --locked --no-run` (build), then
  `cargo test --workspace --locked` (run), both under `/usr/bin/time -v`.

| Measure | Before | After |
|---|---|---|
| Clean release build | 3:35 | 2:20 |
| `limeos-core` | 8,458,304 B | 8,458,304 B |
| `limeos-password-worker` | 554,432 B | 554,432 B |
| `limeos-executor` | 6,438,720 B | 6,434,624 B |
| `limeos-stack-read-worker` | 1,275,392 B | 1,275,392 B |
| `limeosctl` | 1,291,776 B | 1,291,776 B |
| Dependencies: core / executor / ctl | 153 / 131 / 115 | 153 / 131 / 115 |
| Dependencies: workspace normal / all | 214 / 645 | 201 / 620 |
| Test build, default | 3:35 | 1:25 |
| Test run, default | 1:38 | 0:59 |
| Test build / run, `--features deferred` | n/a | 1:43 / 1:17 |
| Tests passed, default / deferred | 1976 / n/a | 1137 / 1995 |

The release binaries are effectively the same size: thin LTO and linker garbage
collection had already removed this code, which no shipped entry point reaches.
Before gating, the symbol tables of debug builds of every packaged binary showed no
symbol from any gated module. The gain is in build and test time, a smaller default
dependency graph (`tar`, `flate2`, `zstd` and their build dependencies leave it),
and an explicit boundary between shipped and deferred code.

To bring code back into scope, remove its `#[cfg(feature = "deferred")]` lines
together with the change that gives it a production caller.

# Catalog namespace planning evidence

Qualified product source `0ead2a40c8d123e792003d919e6e090d8beb0f9e`, based on
`0972b26`. See the [handoff](../../../p04-catalog-namespaces-handoff.md).

The unfiltered isolated domain harness passes **85 tests**: 17 new namespace
cases, 20 existing catalog, 21 media library, 15 activity and 12 B2 discovery.
Strict Clippy across all harness targets and Rustfmt pass. No product lint is
suppressed. The product commit contains the checked bytes with no intervening
product edits; `source-status.log.gz` records the clean state afterward.

- `metadata.json` records exact source identity, counts, commands and environment.
- `inputs.sha256` covers 59 tracked project inputs, including all domain sources,
  manifests/production lock and relevant deployment/storage-contract fixtures.
- `frozen-inputs.sha256` identifies four read-only frozen catalog inputs at
  `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. They were never executed or changed.
- `test.log.gz`, `clippy.log.gz`, `rustfmt.log.gz`, tool versions and
  `cargo-metadata.json.gz` record the final local run and dependency resolution.
- `harness/` retains the exact standalone Cargo manifest/lock and wrapper. The
  production crate root/manifests/lock were not changed. The wrapper compiles
  original domain code as a dependency; it does not run all original domain tests.
- `test-initial.log.gz` and initial Rust snapshots retain the 83/84 run. The new
  assertion expected numeric `published`; the established compiler emits a
  string. Only that expectation was corrected.
- `completed-condition-red.log.gz` and red source snapshots retain the reproduced
  contradictory namespace/completed-provider draft. The final graph validation
  refuses direct/transitive providers required to exit first, while allowing
  unrelated completed setup services.
- `package-evidence.py.gz` assembles the packet without rerunning passing suites.
  Ruff initially reported that this scratch helper had a shebang without its
  executable bit; that mode was corrected and Ruff passes. Both logs remain.
- `artifacts.sha256` covers packet artifacts except itself.

Reproduce from this worktree with the captured cached dependencies and one local
suite at a time. Keep scratch on disk under the repository:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/catalog/check-leaf.py
cp docs/rewrite-evidence/p04/2026-10-15-catalog-namespaces/harness/Cargo.lock .cache/catalog-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/catalog-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/catalog-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/catalog.rs crates/domain/src/deployment/catalog/tests.rs crates/domain/src/deployment/catalog/tests/namespaces.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-15-catalog-namespaces/inputs.sha256
```

These are pure typed declaration/compiler checks. The fixture is a synthetic
port of selected frozen namespace/healthy-dependency clauses, not a complete
gateway template, real VPN protection or actual healthcheck result. The frozen
gateway's nine ports exceed the unchanged base limit; capabilities, environment/
env_file and healthcheck source adapters remain absent. No source field is retired.

No executor/Engine/HTTP/provider/credential access, host write, API/UI/browser,
registered authority/recovery, package/native/ARM64/Pi, worker footprint,
performance or soak acceptance is claimed. Previous decoder/API/frontend packets
retain their own source/test identities. P04 completion remains pending central
registration and independent integrated qualification.

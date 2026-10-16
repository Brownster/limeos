# Prowlarr application registration evidence

Qualified product source `3c6669a8aa6a8f7669b3a4ee6654ccf8b6a3600b`, base `f0fd86c`.
See the [handoff](../../../p04-media-indexer-apps-handoff.md).

The unfiltered isolated harness passes **202 tests**: 163 domain (18 new indexer
application, 18 download-client, 18 settings, 24 local-config, 37 catalog/namespace,
21 library, 15 activity, 12 discovery) and 39 decoder (10 new indexer application,
9 download-client, 10 settings, 10 library). Strict all-target Clippy, Rustfmt and
helper Ruff pass. Generated manifest/lock/wrapper hashes are unchanged after the
helper format check. Product bytes were committed after checks without edits.

`metadata.json` records source, exact commands, counts, bounds and environment.
`inputs.sha256` identifies project sources, manifests/production lock and dedicated
fixtures/helpers; hashing does not mean every file's suite ran. Frozen client,
seed service/test and four catalog rows are identified by `frozen-inputs.sha256`
at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read/hashed only. The first test and
lint runs passed; no failed-run or live Python equivalence result is claimed.

Logs, Cargo metadata/tool versions, clean source status and `harness/` retain the
exact local check identity. The first offline test invocation generated the
standalone lock. Original domain dependency code compiled, but its entire unit
suite did not run. `package-evidence.py.gz` packages existing checks without
rerunning suites. `artifacts.sha256` covers packet artifacts except itself.

Reproduce with cached dependencies, disk scratch under the repository and one
suite at a time. The helper regenerates absolute paths for a new checkout:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/indexer-apps/check-leaves.py
cp docs/rewrite-evidence/p04/2026-10-16-indexer-apps/harness/Cargo.lock .cache/indexer-apps-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/indexer-apps-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/indexer-apps-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/indexer_apps.rs crates/domain/src/deployment/indexer_apps/tests.rs crates/domain/src/deployment/media.rs crates/domain/src/deployment/mod.rs crates/executor-container/src/deployment/indexer_apps.rs crates/executor-container/src/deployment/indexer_apps/tests.rs crates/executor-container/src/deployment/mod.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-16-indexer-apps/inputs.sha256
```

Fixtures port frozen full-sync/name intent and the TV-for-all defect, with
synthetic response IDs/keys and explicit example movie/music policy. The example
sets are neither installed defaults nor approved changes. Cases cover rerun and
rename behavior, duplicates/unknown metadata/policy conflicts, declaration/scope/
freshness failures, limits, private-field exclusion and deterministic set order.
Ordinary completeness/timestamps prove no protected acquisition. Matching
metadata proves no endpoint, credential or successful configuration.

No registered runtime/API/UI, actual schema/network/credential verification,
protected write/claims/locks/recovery, package/ARM64/Pi, performance, footprint,
soak or P04 completion is claimed.

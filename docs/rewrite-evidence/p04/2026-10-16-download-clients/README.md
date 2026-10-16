# Download-client registration evidence

Qualified product source `d457fc821a430b69dfdfeb0464a8ba29e7d376dc`, base `66241ef`.
See the [handoff](../../../p04-media-download-clients-handoff.md).

The unfiltered local harness passes **174 tests**: 145 domain (18 new registration,
18 settings, 24 local-config, 37 catalog/namespace, 21 library, 15 activity,
12 discovery) and 29 decoder (9 new registration, 10 settings, 10 library).
Strict all-target Clippy, Rustfmt and helper Ruff pass. Generated manifest/lock/
wrapper hashes are unchanged after the helper format check. Product source was
committed after checks without intervening product edits.

`metadata.json` records exact commands, source, counts, bounds and environment.
`inputs.sha256` identifies project source, manifests/production lock and dedicated
fixtures/helpers; hashing does not mean every file's suite ran. Frozen client,
seed service/test and five catalog rows are identified by `frozen-inputs.sha256`
at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read/hashed only. The test log is
the first passing run; the initial Cargo invocation generated the standalone lock
offline. No failed-run or live Python equivalence evidence is claimed.

Logs, Cargo metadata/tool versions, clean source status and `harness/` retain the
exact local check identity. Original domain dependency code compiled, but its
complete unit suite did not run. `package-evidence.py.gz` packages these results
without rerunning suites. `artifacts.sha256` covers packet artifacts except itself.

Reproduce with cached dependencies and disk scratch under the repository, one
suite at a time. The helper regenerates absolute paths for a new checkout:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/download-clients/check-leaves.py
cp docs/rewrite-evidence/p04/2026-10-16-download-clients/harness/Cargo.lock .cache/download-clients-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/download-clients-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/download-clients-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/download_clients.rs crates/domain/src/deployment/download_clients/tests.rs crates/domain/src/deployment/mod.rs crates/executor-container/src/deployment/download_clients.rs crates/executor-container/src/deployment/download_clients/tests.rs crates/executor-container/src/deployment/mod.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-16-download-clients/inputs.sha256
```

Fixtures are synthetic native resource rows with frozen category/removal intent
and fake credential sentinels. Cases cover rerun matching, generic-field defects,
unknown categories, conflicts, declaration/instance/freshness failures, limits and
selected-data privacy. Ordinary values and completeness/timestamps prove neither
protected origin nor actual acquisition completeness. Matching registration
metadata proves no endpoint/credential configuration or successful download.

No registered runtime/API/UI, protected acquisition, installed schema/network,
write/claims/locks/recovery, package/ARM64/Pi, performance, footprint, soak or P04
completion is claimed.

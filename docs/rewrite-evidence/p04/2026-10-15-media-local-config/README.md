# Local media configuration draft evidence

Qualified product source `ee83fefc99468000afd509d79b9e769fd187535a`, base
`25c9cbe`. See the [handoff](../../../p04-media-local-config-handoff.md).

The unfiltered isolated domain harness passes **109 tests**: 24 local-config,
37 catalog/namespace, 21 library, 15 activity and 12 discovery. Strict all-target
Clippy and Rustfmt pass. The product commit contains the checked bytes, with no
intervening product edits; clean post-commit status is captured separately.

- `metadata.json` records source, counts, exact commands, bounds and environment.
- `inputs.sha256` covers 65 tracked project inputs, including domain sources,
  manifests/production lock and deployment/storage-contract fixtures.
- `frozen-inputs.sha256` identifies the frozen seed service/tests and two catalog
  templates at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read and hashed only.
- Final test/lint/format logs and Cargo metadata/tool versions identify the local
  checks. `harness/` captures the exact standalone manifest/lock/wrapper. Original
  domain dependency code is compiled; its complete unit suite is not run here.
- `test-initial.log.gz` retains the initial passing 107-case run before two new
  collision regressions. `collision-red.log.gz` plus INI/test source snapshots
  retain those two reproduced failures; neither is credited as a final pass.
- `package-evidence.py.gz` assembles the packet without rerunning passing suites.
  `artifacts.sha256` covers packet artifacts except itself.

Reproduce with the captured cached dependencies, one local suite at a time, and
disk scratch under the repository:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/catalog/check-leaf.py
cp docs/rewrite-evidence/p04/2026-10-15-media-local-config/harness/Cargo.lock .cache/catalog-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/catalog-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/catalog-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/local_config.rs crates/domain/src/deployment/local_config/ini.rs crates/domain/src/deployment/local_config/json.rs crates/domain/src/deployment/local_config/tests.rs crates/domain/src/deployment/mod.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-15-media-local-config/inputs.sha256
```

These checks cover typed supplied-byte planning, preservation, fixed errors,
redacted summaries/debug, structural ambiguity and bounds. The fixtures contain
fake secret sentinels. The SABnzbd output is checked against fixed expected bytes
and rerun behavior; no independent ConfigObj interpreter or live application was
executed. Unknown source values remain uninterpreted. This is a targeted format
subset, not complete configuration compatibility or source/custody proof.

No runtime authority, actual source/file metadata, physical ownership, process,
network/credential access, file write/recovery, installed API/UI, package/native/
ARM64/Pi, performance, worker footprint, soak or P04 completion is claimed.

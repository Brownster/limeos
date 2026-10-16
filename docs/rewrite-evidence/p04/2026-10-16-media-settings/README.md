# Servarr settings planning evidence

Qualified product source `2e283f76a80ba47e3cfab4c495c0d629712d97ca`, base `1607167`.
See the [handoff](../../../p04-media-settings-handoff.md).

The unfiltered isolated harness passes **147 tests**: 127 domain (18 new settings,
24 local-config, 37 catalog/namespace, 21 library, 15 activity, 12 discovery) and
20 decoder (10 new settings, 10 library). Strict all-target Clippy and Rustfmt
pass. Ruff passes for the harness helper. Its generated manifests, lock and
wrappers are byte-identical before/after the helper's format check. Product
source was committed after the checks without intervening product edits.

- `metadata.json` records source, counts, commands, bounds and environment.
- `inputs.sha256` covers project sources, manifests/production lock and dedicated
  fixtures/helpers. Not every hashed file represents a test run by this harness.
- `frozen-inputs.sha256` identifies the frozen Arr client, seed service/test and
  three catalog rows at `80593b29443ea9b81eb60ed4e5c3c3ac7a236204`.
- Test/lint/format logs, Cargo metadata/tool versions and clean source status
  record the local checks. `harness/` retains the exact standalone workspace,
  member manifests, lock and wrappers. Original domain dependency code compiled;
  its complete unit suite did not run here.
- `test-initial.log.gz` records the failed first compile, where an old namespace
  fixture path depended on the harness manifest location. No test ran in that
  attempt. The final suite includes the corrected source-relative fixture test.
  The null-first duplicate rename-field case passed without a decoder fix.
- `package-evidence.py.gz` assembles this packet without rerunning passing suites.
  `artifacts.sha256` covers packet artifacts except itself.

Reproduce with cached dependencies, one suite at a time and disk scratch under
the repository. The helper regenerates absolute wrapper/source paths for a new
checkout; the captured files identify the original qualification paths.

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/media-settings/check-leaves.py
cp docs/rewrite-evidence/p04/2026-10-16-media-settings/harness/Cargo.lock .cache/media-settings-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/media-settings-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/media-settings-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/mod.rs crates/domain/src/deployment/catalog/tests/namespaces.rs crates/domain/src/deployment/media_settings.rs crates/domain/src/deployment/media_settings/tests.rs crates/executor-container/src/deployment/mod.rs crates/executor-container/src/deployment/media.rs crates/executor-container/src/deployment/media_settings.rs crates/executor-container/src/deployment/media_settings/tests.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-16-media-settings/inputs.sha256
```

These checks cover selected supplied-byte decoding, per-application mappings,
separate resource IDs, no-op observations, profile ambiguity/missing audio
profiles, custom script refusal, freshness and exact instance drift, structural
ambiguity and bounds. Synthetic credentials/script/naming-format fields are
excluded from selected DTOs and serialized previews. Selected paths/profile names
remain private data. Ordinary DTOs and caller-provided completeness/timestamps
prove neither protected source nor acquisition completeness.

No protected acquisition, HTTP status/source proof, write/field preservation,
authority/locks/recovery, registered API/UI, installed service/version,
package/ARM64/Pi, performance, footprint, soak or P04 completion is claimed.

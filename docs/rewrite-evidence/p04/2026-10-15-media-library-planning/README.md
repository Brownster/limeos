# Focused media library planning evidence

Tested code `5379d57bb2fd2b94e319ab17dc251405cc6fa0c1`, base catalog `c712486`.
[Handoff](../../../p04-media-library-planning-handoff.md) defines the initial
four-application subset and remaining parity/integration. `metadata.json` binds
source/frozen hashes and commands; `SHA256SUMS` covers packet artifacts, including
the exact standalone harness/lock and lossless diagnostics. Harness manifests use
this local absolute path: regenerate them for another checkout location.

**63 tests pass**, without filtering: 21 media domain + 10 decoder + 20 catalog +
12 B2 domain. Strict Clippy `--all-targets -- -D warnings`, Rustfmt, Ruff 0.17.0
and harness formatting pass. The production API/executor and original-domain unit
suite are not run by this two-crate leaf harness. Their acceptance is not implied.

Regenerate the harness from this worktree:

```sh
env UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp \
  uv run --offline --no-project tests/fixtures/deployment/media/check-leaves.py
```

Restore the packet's `harness/Cargo.lock` to `.cache/media-library-leaf/Cargo.lock`
in a fresh cache. Run one Cargo suite at a time:

```sh
env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp RUST_TEST_THREADS=2 \
  cargo test --offline --locked --manifest-path .cache/media-library-leaf/Cargo.toml
```

Run Clippy with that environment, replacing `test` with `clippy` and adding
`--all-targets -- -D warnings`. Metadata records the remaining exact check commands.

`template-red.log.gz` reproduces new media admission incorrectly accepting an
unresolved `${DOWNLOADS}` root; the final code checks both media/download roots
before deriving exclusions. Intermediate source/test snapshots are retained.
`test-initial.log.gz` and `test-bounds.log.gz` are earlier 60/62-test green runs,
not final acceptance. The final 63-test log includes the passing regression.

Frozen reference code is read/hashed only. Two response fixtures are synthetic;
three existing storage-profile fixtures supply the contract layouts. No live
credential, HTTP response, service/Engine/physical collection, host effect,
API/UI/browser, worker-resource, installed recovery, package/ARM64/Pi/performance
or cutover qualification is claimed. Storage/authority/registration and the full
media quickstart/service seeding inventory remain pending.

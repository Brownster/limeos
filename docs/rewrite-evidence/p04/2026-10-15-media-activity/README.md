# Focused media activity evidence

Tested source `873938a17b153415196874b8fa2d99a6c8d732cc`; Rust/API commit
`80cf363`, base media libraries `1f64979`. [Handoff](../../../p04-media-activity-handoff.md)
defines behavior, registration and remaining acceptance. Metadata binds source,
frozen reference hashes, exact commands/tools, harness/lock and lossless logs.
`SHA256SUMS` covers packet artifacts. Paths in generated harness manifests are
local absolute paths; regenerate for another checkout location.

Final results: **107 Rust tests**, no filtering; **10 frontend cases**; strict
TypeScript, Clippy, Rustfmt and Python harness checks pass. Rust counts include
33 new activity cases and 74 existing media-library/catalog/B2/API regressions.
The original-domain dependency's separate unit suite, B2 reader and frontend
controller suites are not rerun or requalified by these counts.

Regenerate the isolated three-crate harness:

```sh
env UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp \
  uv run --offline --no-project tests/fixtures/deployment/activity/check-leaves.py
```

Restore `harness/Cargo.lock` to `.cache/media-activity-leaf/Cargo.lock` in a new
cache. Run suites sequentially with one Cargo job:

```sh
env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp RUST_TEST_THREADS=2 \
  cargo test --offline --locked --manifest-path .cache/media-activity-leaf/Cargo.toml
```

Clippy uses the same environment with `clippy` in place of `test`, adding
`--all-targets -- -D warnings`. Frontend commands are `node
frontend/tests/media-activity.test.mjs` and `frontend/node_modules/.bin/tsc --noEmit
--project frontend/tsconfig.json`. Existing local frontend dependencies were
temporarily linked from the main checkout; the link is removed after validation.
No dependency or production lockfile was changed. The direct Node command records
all 10 node:test cases; `--test` wrapper diagnostics only show file-level results.

Retained diagnostics:

- `ui-partial-regression-red.log.gz`: 9 pass/1 fail with incorrect partial totals;
  `ui-final.log.gz`: all 10 pass after the explicit complete check.
- `ui-worker-initial.log.gz`: earlier file-level failure without case detail;
  the direct red run above supplies the actual assertion. Wrapper/direct green
  diagnostic logs are retained but not counted as extra acceptance.
- `wire-generation-initial.log.gz`: standalone harness omitted Tokio test-util
  used by existing API tests. Final helper adds it; production manifest unchanged.
- `clippy-harness-initial.log.gz`: non-test route wrapper missing in the harness.
  Final helper exports the same proposed route hook as B2; no dead-code suppression.
- Wire generation used an intermediate test-only write branch, retained as a
  lossless source snapshot. That branch is absent from final tests. Final tests
  compare committed JSON without writes. This fixture binds serialization, not
  an independent service-response oracle; per-provider assertions test the port.
- `harness-stable.log.gz` verifies that later Python formatting did not change
  any final manifest/wrapper/lock bytes used by passing Rust/Clippy runs.

Four synthetic provider fixtures reproduce selected frozen cases, with fake
private sentinels. No reference Python is imported/executed. No live endpoint,
credential/Engine/physical collection, network effect, browser/layout, installed
recovery, whole-worker resources, package/ARM64/Pi/performance or cutover acceptance
is claimed. Shared route/cache/Store/schema/UI registration remains pending.

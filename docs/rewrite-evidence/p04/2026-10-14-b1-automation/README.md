# B1 focused leaf evidence

Qualified leaf source: `45026ea36c3a3bfb7fdd824dab9370ac366a5b3e`, branch `engineer/p04-automation-b1`, starting main `0c7ec8c`. This packet qualifies the supplied unregistered leaves, not B1 completion. The [handoff](../../../b1-automation-handoff.md) specifies the shared transaction, authority and installed registration work.

| Check | Result |
| --- | --- |
| Isolated Rust leaf tests | 30 pass: 20 domain, 6 runner, 4 API. Existing API tests compile but are filtered out. |
| Strict Clippy on the isolated test crates | Pass, `-D warnings`. |
| Owned Rust leaf formatting | Pass. |
| Existing frontend TypeScript check with new leaves | Pass. |
| Focused frontend scenarios | 8 pass under `node tests/automation.test.mjs`. |

`manifest.json` records the exact commands, on-disk scratch/build configuration and SHA-256 for all 17 changed code/fixture files. Source hashes and HEAD agree before/after the checks. Shared root manifests/lock, domain/API/Store roots, SQL schema, Core root and dashboard/client are unchanged against the starting main. `harness-registration.json` retains the generated manifests/module declarations; `harness-Cargo.lock` retains the exact dependency resolution. Tool output is retained losslessly as fixed-mtime gzip logs, with raw and compressed hashes in the manifest. `SHA256SUMS` binds this packet's artifacts.

The compilation harness reexports the unchanged domain crate and adds the new automation module, includes the unchanged API crate root with the proposed API module, and compiles the Core runner separately. It never patches shared repository roots. Test Store/backend implementations are stubs. Serialization round-trips prove model behavior only. The simulated 24-hour alert case proves unchanged material emits no new transition, not idle database/service writes.

The initial diagnostic compile failed with `E0405` because the new API page helper referred to `Serialize` without importing that trait. The helper now uses `serde::Serialize`; the final captured compile, tests and Clippy pass. Early `node --test` invocation reported only a file-level aggregate; the accepted focused command above emits all eight named assertions and their totals. These diagnostic runs are not counted as additional acceptance cases.

## Reproduce

Use a checkout of the qualified source and an on-disk cache. The root uses existing pinned dependencies and no new package dependency.

```sh
mkdir -p .cache/tmp .cache/uv
UV_CACHE_DIR="$PWD/.cache/uv" uv run --offline tests/fixtures/automation/check-leaves.py .cache/b1-leaf-harness
cp docs/rewrite-evidence/p04/2026-10-14-b1-automation/harness-Cargo.lock .cache/b1-leaf-harness/Cargo.lock
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 TMPDIR="$PWD/.cache/tmp" cargo test --offline --locked --manifest-path .cache/b1-leaf-harness/Cargo.toml automation
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 TMPDIR="$PWD/.cache/tmp" cargo clippy --offline --locked --manifest-path .cache/b1-leaf-harness/Cargo.toml --tests -- -D warnings
rustfmt --check --edition 2024 crates/domain/src/automation/mod.rs crates/api/src/automation.rs bins/core/src/automation/mod.rs
```

Run `npm run check` and `node tests/automation.test.mjs` from `frontend` with the pinned frontend dependencies installed. The current packet used the existing local dependency directory through a temporary worktree symlink; that symlink is removed after checking and is not committed. These commands are focused checks; the qualification owner retains broad suites.

## Scope and remaining acceptance

RW-045: arithmetic fixtures cover due/grace/coalescing/skip/rollback and window boundaries, plus independent enumerated window candidates. Alert cases preserve long-running container policies, SMART/mount/protection severities, thresholds, missing/unavailable facts, silences and opening/recovery deduplication. Delivery cases expose retries, exhausted/ambiguous outcomes and bounded retention. Runner cases reject malformed pages and effect-job replies; all five unavailable effect adapters submit blocked records.

API-001: focused API scenarios reject mutation GETs, wrong origin/session/CSRF, extra input fields, oversized bodies and invalid/stale revisions. UI cases preserve partial/blocked/failure state, escaped summaries, read-only controls and current revision POSTs. This unregistered evidence closes no defect row.

Still required: real Store/schema atomicity and rollback, concurrent connections/processes, restart reconciliation, current-grant checks at dispatch, approved recurring effects, registered API/security middleware and dashboard, actual installed UI interaction, complete legacy schedule/channel inventory, 24-hour whole-service idle-write/resource measurement and independent qualification. No native ARM64, guest, Pi, production Docker, executor mutation, install or service activation ran. B2–B7 and RW-042/RW-044 remain open.

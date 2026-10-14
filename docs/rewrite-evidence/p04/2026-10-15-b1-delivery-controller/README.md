# B1 delivery and controller focused evidence

Qualified leaf source: `5a8a8cf0959d23ed520f9a99ec922c77d5bbf1a9` on `engineer/p04-automation-b1`, including the published main dependency fix `b37c718`. This continues the [October 14 packet](../2026-10-14-b1-automation/README.md). The [integration handoff](../../../b1-automation-handoff.md) supplies the current central requirements and single-component dashboard mounting proposal.

| Focused check | Result |
| --- | --- |
| Rust automation leaves | 38 pass: 28 domain, 6 runner, 4 API; 9 existing API cases compiled but filtered out. |
| Strict Clippy on the isolated test crates | Pass with `-D warnings`. |
| Owned Rust leaf formatting | Pass. |
| Frontend TypeScript | Pass. |
| Client/view scenarios | 10 pass. |
| Controller/reconnect/expiry scenarios | 14 pass. |

These 62 focused cases qualify unregistered leaves. They do not prove database transactions, installed runtime/API/UI, real restart deduplication, current recurring authority, native ARM64 or whole-service performance. The API and Core Store test doubles remain stubs. No production services, Pi, Docker, install or package activation ran. No enabled browser provider was available; frontend cases execute the real transpiled client/controller and React server rendering, with controlled test responses, rather than browser interaction.

## Changed behavior

A recovery waits for the same key/generation/kind/channel's verified incident delivery. Pending, exhausted, ambiguous or missing opening evidence cannot release it. Interrupted attempts preserve the original identity and become ambiguous; unavailable external adapters remain blocked. Acknowledging a failed notification is not receiver-delivery proof. The retention regression fixes early deletion when acknowledgment precedes a delayed successful delivery, preserving seven days after the later resolving timestamp.

The panel maintains one page per section, replaces pages instead of retaining history, and preserves partial/error state. Successful or failed mutations invalidate older in-flight reads before refreshing. Aborted or previous-lifetime responses cannot overwrite current data. Reconnect/focus refreshes coalesce; session expiry clears private pages and invokes the existing expiry callback once. Forbidden reads clear the affected page. Unavailable reads retain visibly stale data with mutation controls disabled. An initial error is distinct from an empty successful list. No failed POST is automatically retried.

`manifest.json` records commands/environment and before/after hashes for 22 actual source/fixture files. The tested file hashes are checked against the final commit's Git blobs; tests ran on those exact bytes before the UI commit. Shared roots, Store/schema, manifests/lock and dashboard/client match accepted main `b37c718`; no central registrations were applied. The generated wrapper manifests/module declarations and exact harness lockfile are retained. Tool output is losslessly retained as fixed-mtime gzip, with raw/compressed hashes. `SHA256SUMS` binds the packet.

## Reproduce

Use the qualified source plus this evidence packet and pinned frontend dependencies. Keep compilation/scratch on disk and run one local Cargo job.

```sh
mkdir -p .cache/tmp .cache/uv
UV_CACHE_DIR="$PWD/.cache/uv" uv run --offline tests/fixtures/automation/check-leaves.py .cache/b1-leaf-harness
cp docs/rewrite-evidence/p04/2026-10-15-b1-delivery-controller/harness-Cargo.lock .cache/b1-leaf-harness/Cargo.lock
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 TMPDIR="$PWD/.cache/tmp" cargo test --offline --locked --manifest-path .cache/b1-leaf-harness/Cargo.toml automation
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 TMPDIR="$PWD/.cache/tmp" cargo clippy --offline --locked --manifest-path .cache/b1-leaf-harness/Cargo.toml --tests -- -D warnings
rustfmt --check --edition 2024 crates/domain/src/automation/mod.rs crates/api/src/automation.rs bins/core/src/automation/mod.rs
```

From `frontend`, run `npm run check`, `node tests/automation.test.mjs` and `node tests/automation-controller.test.mjs`. This run reused the existing pinned dependency directory through a temporary worktree symlink, removed after checking. The harness compiles proposed module declarations without editing the actual domain/API/Core roots. Broad qualification remains with the existing test owner.

The initial TypeScript check reported `TS2345`/`TS2322`: a generic mapped page lookup lost the correlation between the selected key and its row type. The private typed getter now preserves that correlation; the final strict check passes. A browser-capability probe was unavailable and contributes no acceptance evidence. The earlier raw timing/diagnostic runs are not counted as additional cases.

B1 still needs the frozen Store contract, approved authority/schema/jobs, installed route/runtime/dashboard registration, concurrent real connections/processes, restart/commit-boundary failure injection and measured idle-write/resource behavior. B2–B7, P04 completion and Pi cutover remain open.

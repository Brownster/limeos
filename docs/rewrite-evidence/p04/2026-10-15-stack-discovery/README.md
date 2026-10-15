# Local B2 stack discovery evidence

15 October 2026. Exact tested code `c4b30f52e1d7f1cb318ff16e122a13f0cac219df`,
based on main `b37c718`. This independent branch adds unregistered stream B leaves.

Passing final checks: **16 focused Rust tests** (12 domain, 2 private adapter
projections, 2 GET handler tests), **18 frontend tests**, strict Clippy, TypeScript,
Rustfmt and Python harness lint/format. Ten runtime-layout cases include empty,
stopped, replicas, one-offs, missing services and registration drift. `wire.json`
is compared to actual Rust serialization and decoded/rendered in the UI tests.
The project-style Node test invocation also passes; it reports one aggregate file,
while the direct node:test invocation records all 18 subtests.

`metadata.json` records exact source hashes checked against committed Git blobs,
the frozen reference hashes, tool versions, commands, environment and limits.
The independent harness Cargo.lock/manifests/module registration are retained.
The API harness includes a proposed leaf mounting function so non-test Clippy
compiles the routes; it does not register or qualify shared production middleware.
`SHA256SUMS` binds the packet. Logs are preserved as lossless gzip files, including
development failures and final checks. Diagnostic intermediate sources were not
frozen as commits and confer no acceptance on the final source.

Development diagnostics include the initial compiler-cache sandbox failure, API
import/test-runtime and cookie-fixture corrections, unregistered API mount lint,
and frontend clock/freshness/coalescing/cancellation/raw-JSON error regressions.
Final checks pass after those corrections. The C compiler cache was relocated to
the repository's on-disk cache; scratch/build outputs also stay on disk.

Reproduce with the committed `tests/fixtures/deployment/discovery/check-leaves.py`,
restore the retained Cargo.lock into `.cache/b2-discovery-harness/` and run the
recorded commands sequentially with one Cargo build job. Frontend dependencies
use the unchanged package lock; install them in the test worktree before running
its focused test file and TypeScript check. Recompute source hashes before crediting
another checkout.

These checks qualify pure reconciliation, private projections, stub-backed HTTP
handlers, static React rendering and client/controller behavior on x86_64. They
qualify no genuine root-positive collection, installed cache/API/browser/UI,
worker budgets, ARM64/Pi, lifecycle effects, atomic writes, crash recovery,
performance or cutover. See the [handoff](../../../p04-stack-discovery-handoff.md)
for central registration and acceptance requirements.

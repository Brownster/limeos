# Local B4 catalog planning evidence

15 October 2026. Exact tested code:
`39039fd73496b25c9098659b6990953f94588320`, based on B2 `5e275ca`.
Branch `engineer/p04-catalog-planning` includes B2 and adds a dormant domain leaf.

Final checks pass: **20 focused Rust tests**, strict Clippy (`--all-targets`,
`-D warnings`), Rustfmt and Python harness lint/format. The inherited 12 B2 tests
compile but are filtered; this packet confers no new acceptance on those tests.
No frontend or broad host suite was rerun for this domain-only slice.

Coverage includes all four resource sections/grants, collisions, resource reuse,
same app in two stacks, one-service removal, dependencies, retained resources,
numeric ownership/case-sensitive paths, unsupported fields, unresolved references,
determinism, selected-input limits and output growth through JSON escaping.
The fixture adapts the frozen resource regression to typed supported definitions
and a pinned image. Test-side mapping union checks inherited resources; neither
test parses actual YAML nor invokes Compose.

`metadata.json` binds the exact committed source/fixtures, inherited domain build
inputs and frozen reference files by SHA-256. It records tool versions, environment,
commands and limits. `harness/` retains the independent manifest, complete lockfile
and leaf module wrapper; shared roots and the production lockfile are unchanged.
`SHA256SUMS` covers this packet. Retained development/final logs are stored
as lossless deterministic gzip, including the stale test-field compile error and
uv's read-only tool-directory failures. Intermediate sources were not committed;
their logs confer no acceptance on the final source. Escalated reruns of Ruff
passed after those sandbox failures.

Reproduce using `uv run --offline tests/fixtures/deployment/catalog/check-leaf.py`,
restore `harness/Cargo.lock` to `.cache/catalog-leaf/Cargo.lock`, then run the
recorded checks sequentially with one Cargo build job. Manifest/wrapper absolute
paths must point at the reproduced checkout. Keep scratch/build/cache on disk
under the repository; compare source hashes before crediting another checkout.

These x86_64 checks qualify pure typed proposals only. Source parsing/admission,
protected ownership, actual Compose conformance, template inventory parity,
authenticated API/UI, execution authority, verified shutdown, atomic writes,
recovery, ARM64/Pi, performance and cutover are unqualified. There are no host
effects, SQL migrations or runtime registrations. The
[handoff](../../../p04-catalog-planning-handoff.md) describes central integration
and the remaining installed defect acceptance.

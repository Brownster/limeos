# Local container-update policy evidence

15 October 2026. Exact tested code:
`46f63015d380f20ca31768d188c02e0f4e60e786`, based on B2 `5e275ca`.
Branch `engineer/p04-container-update-planning` includes B2 and adds a dormant
domain policy leaf; shared runtime registration remains absent.

Final checks pass: **24 focused Rust tests**, strict Clippy (`--all-targets`,
`-D warnings`), Rustfmt and Python harness lint/format. Twelve inherited B2 tests
are compiled but filtered and retain their earlier source/acceptance identity.
No frontend, broad host, package or hardware suite was rerun for this domain slice.

The default-settings fixture preserves the frozen configuration subset, including
preset spelling. History still needs central job migration. Four image/text cases
prove the deliberate replacement of download-text guesses: config-ID changes
matter, layer output and manifest digest changes alone do not prove recreation.
Other cases cover selection/exclusions, unreadable config, stopped stacks,
replicas/one-offs, engine/source/platform/scope drift, stale/future observations,
pre/post-dispatch boundaries, incomplete convergence, strict fields, selected-only
review data and count/aggregate bounds. There is no actual image resolver/parser,
Compose invocation or protected success receipt in these tests.

`metadata.json` records committed source/fixture hashes verified against Git blobs,
frozen reference hashes, tool versions, exact commands/environment and limits.
`harness/` retains the independent manifest, complete Cargo.lock and module wrapper.
Shared production manifests/lockfile, roots, Store/schema and protocol are unchanged.
`SHA256SUMS` binds the packet. Retained development/final logs use lossless
deterministic gzip, including the two fixture-initializer Clippy diagnostics.
The earlier 21/23/24-test development sources were not frozen as commits and confer
no acceptance on the final source. Final-source tests and strict checks pass.

Reproduce with `uv run --offline tests/fixtures/deployment/container-updates/check-leaf.py`,
restore `harness/Cargo.lock` to `.cache/container-update-leaf/Cargo.lock`, then run
the recorded checks sequentially with one Cargo build job. Manifest/wrapper
absolute paths must point at the reproduced checkout. Keep scratch/cache/builds
on disk under the repository; compare source hashes before crediting another run.

These x86_64 checks qualify pure selection drafts and diagnostic post-comparisons.
They qualify no trusted source/image owner, OCI content-chain/platform/origin,
registered API/UI, executable or recurring authority, due/window/timezone policy,
physical dependencies, lifecycle/atomic writes, health, receipt, recovery,
ARM64/Pi, performance or cutover. No Docker/registry command or host mutation
occurred. The [handoff](../../../p04-container-update-policy-handoff.md) defines
the remaining integration and installed acceptance requirements.

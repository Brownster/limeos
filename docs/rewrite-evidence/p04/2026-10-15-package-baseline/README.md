# Local B5 package baseline evidence

15 October 2026. Tested package source:
`94a6e33f28b270b2690cb6d4ff390243be1e63d3`, based on HOST-001 `65ed86e`.
Branch `engineer/p04-package-planning` includes the earlier prerequisite slice.

Final checks pass: **25 Rust tests** (19 domain, 6 parser), strict Clippy for all
harness targets with warnings denied, Rustfmt and Python harness lint/format.
Ten typed policy fixtures adapt the frozen baseline rules. A 35-version matrix
contains **1,225 comparisons** captured from local read-only dpkg 1.23.7; the pure
Rust comparator matches every recorded result. The matrix covers epochs, tildes,
revisions, leading zeros, punctuation/case and long digit runs. It is a finite
corpus, not qualification of all Debian syntax or package execution.

Cases also cover unknown vs absent, unavailable/unoffered candidates, exact pins,
holds, downgrade refusal, incomplete dpkg states, feature scope, unsupported
managers/controls, strict fields, identity/architecture/error refusal and bounds.
The initial regression exposed Serde's extra-field acceptance for internally
tagged unit variants. Empty struct variants now enforce strict fields, with a
passing regression for policies and unknown/absent/broken observation variants.

`metadata.json` records committed source/fixture hashes checked against Git blobs,
frozen reference hashes, exact commands/environment, tool versions and limits.
`harness/` retains the standalone workspace, manifests, wrappers and complete
Cargo.lock. The parser harness compiles just the new package leaf; HOST-001's
tests/collector/effects were not rerun or newly qualified. Shared production
manifests, roots, protocol, schema and runtime registration are unchanged.

Retained development/final logs use lossless deterministic gzip. They include the
initial standalone module-path compile error, the strict-field regression, its
empty-variant guard compile correction, and the harness executable-bit lint.
Intermediate sources were not frozen commits; their logs do not qualify the
final source. `SHA256SUMS` binds this packet. The oracle capture script is an exact
copy of the developer utility run at `.cache/prepare-package-oracle.py`; copy it
back there to regenerate the matrix with dpkg on another host. No reference
Python was imported or executed.

Reproduce with `uv run --offline tests/fixtures/deployment/packages/check-leaves.py`,
restore `harness/Cargo.lock` to `.cache/package-leaf/Cargo.lock`, then run the
recorded commands sequentially with one Cargo build job. The wrapper/manifests'
absolute paths must point at the reproduced checkout. Keep scratch/cache/builds
on disk under the repository and compare source hashes before crediting a rerun.

These x86_64 tests qualify pure typed assessment and byte parsing only. There is
no genuine package/source owner, APT repository/simulation, registered API/UI,
effect, upgrade/downgrade, migration/recovery, broad host suite, ARM64/Pi,
performance or cutover acceptance. No package or service changed. See the
[handoff](../../../p04-package-baseline-handoff.md) for integration requirements.

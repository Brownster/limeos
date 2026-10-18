# CopyParty tools local evidence

Tested product source: `b265690a4b8dc5f112389880fe1048b91122d5f7` on
`engineer/p04-tools-planning`, based on `53ec484`. Shared main remains at
`7df3f75`. See the [handoff](../../../p04-tools-planning-handoff.md) for behavior,
registration requirements and acceptance limits.

The combined Stream B harness passes **499 Rust tests**: 321 domain, 137 supplied
adapter/host-policy, 35 API and six runner. The 38 new tools tests exercise 27
synthetic config cases, 20 property captures, supplied outcome/bounds/privacy,
independent freshness, exact previews, unverified applied state and cached API
authentication/default/error/no-mutation behavior. Fixture rows are included
within test counts. Strict all-target Clippy, Rustfmt, diff checks and the cached
evidence-script lint pass. Harness regeneration preserves the tested generated
files and checked shared roots. Root domain/executor unit suites were not run;
their libraries compile as dependencies. There are no frontend changes or new
frontend test results in this packet.

`inputs.sha256` binds the Rust source/fixture superset; `frozen-inputs.sha256`
binds five read-only reference files at frozen `pi-health`
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`. Python reference code was not
executed. `harness/` records the generated standalone manifests, module roots and
the reused leaf lock. Production roots/manifests/lock are unchanged. Gzipped logs
and scripts record local commands, toolchain, failed attempts, final checks and
source status; `metadata.json` gives exact bounds, counts and scope.
`artifacts.sha256` covers the packet excluding that manifest itself.

The initial config decoder accepted an empty array because the defaulted Serde
struct also accepted positional input. The existing object-only wrapper fixes
the admission boundary; the failing fixture, original source and successful
focused regression are preserved. A subsequent maximum-path fixture constructed
511 bytes instead of 512; correcting its final component exercises the intended
inclusive bound. The failure log remains alongside the final passing suite.

Reproduce in the same disk-backed environment with the archived source:

```text
uv run --offline --no-project tests/fixtures/stream-b/check-leaves.py
cp docs/rewrite-evidence/p04/2026-10-16-network-egress/harness/Cargo.lock .cache/stream-b-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/stream-b-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/stream-b-leaf/Cargo.toml --all-targets -- -D warnings
```

Use `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `RUST_TEST_THREADS=2`, and repository
disk caches for `CARGO_TARGET_DIR`, `TMPDIR` and `CCACHE_DIR` as recorded in
metadata. Run suites sequentially. Generated path attributes contain this
worktree's absolute paths; regenerate them for another checkout rather than
copying the archived roots directly.

This is local pure-planner, supplied-decoder and leaf-router evidence. No actual
systemd collector, trusted installation, service mutation, protected cache/Core,
Store/schema/protocol, production route/middleware/contracts/UI registration or
loaded-result recovery is accepted here. Installed/Pi/ARM64, browser, performance,
soak, durable scheduling, defect closure, P04 completion and cutover remain open.

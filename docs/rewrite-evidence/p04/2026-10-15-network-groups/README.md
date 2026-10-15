# Focused network-group observation evidence

Tested source `c25dbe59a0c85d445061ad03b67629a7b28f9a90`, based on B2 `5e275ca`.
The [handoff](../../../p04-network-groups-handoff.md) defines behavior, bounds and
remaining integration. `metadata.json` binds committed inputs and frozen source
hashes. `SHA256SUMS` covers this packet, including the exact standalone harness,
lockfile, tool/command metadata and lossless logs. Harness paths are local absolute
paths: regenerate with the committed helper at another checkout path.

Final results: **54 tests pass** (23 topology + 19 inspect decoder + 12 B2 domain),
no filtering; strict Clippy `--all-targets -- -D warnings`, Rustfmt, Ruff 0.17.0 and
harness formatting pass. Original-domain code is a dependency in the harness;
its separate unit suite and production executor/API are not run by this packet.
Shared manifests/exports/production lockfile remain unchanged.

Reproduce the focused checks from this worktree, one Cargo suite at a time:

```sh
env UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp \
  uv run --offline --no-project tests/fixtures/deployment/networks/check-leaves.py
env CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
  CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target \
  TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp RUST_TEST_THREADS=2 \
  cargo test --offline --locked --manifest-path .cache/network-group-leaf/Cargo.toml
```

Restore the packet's `harness/Cargo.lock` to `.cache/network-group-leaf/Cargo.lock`
before the locked test in a fresh cache. Run Clippy with the same environment,
replacing `test` with `clippy` and adding `--all-targets -- -D warnings`.

Diagnostic history is retained without extending final acceptance:

- `health-red.log.gz`: configured check without a result incorrectly appeared
  unconfigured; final source returns unknown without retaining health commands.
- `prefix-red.log.gz`: short prefixes below 12 characters incorrectly appeared
  missing; all unresolved nonempty hexadecimal prefixes below 64 stay unsupported.
- `shape-red.log.gz`: an empty array could deserialize as a health-check struct;
  map-only admission now covers all consumed Engine objects.
- `*-red-source.rs.gz` preserves the corresponding intermediate implementation
  files only, not a complete independently qualified intermediate checkout.
- `test-initial.log.gz` includes an incorrect test assumption that deep valid
  ignored JSON should fail. The pinned serde_json uses iterative skipping; the
  corrected test checks non-retention and malformed-nesting refusal instead.
- `test-second.log.gz` is the earlier 51-test pass, before the later regressions.

Fixtures reproduce selected frozen rules with synthetic full IDs; no Python
reference code is run. Health commands/environment/log sentinel values are fake.
No Docker/registry/probe/host effect, live protected collection, API/UI/browser,
installed recovery, whole-worker budget, package/ARM64/Pi/performance or cutover
qualification is claimed. The registered Stream B milestone remains pending.

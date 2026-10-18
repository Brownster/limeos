# MergerFS document edit evidence, 18 October 2026

Tested product: `e43ac89560faadfb149d036ce5a6e636acb84cb5`, branch
`engineer/p04-pool-document-edits`, based on assistance candidate `6eb3b37`.
Later documentation commits preserve the tested product bytes. Only three
product paths change: `pools.rs`, `pools/document.rs` and its tests. The actual
production domain crate exports the leaf through its existing pools module;
there is no scratch wrapper or new runtime registration.

The [handoff](../../../p04-pool-document-edits.md) documents exact ownership,
input limits, unsupported forms and pending atomic publication/recovery work.
The [design](../../../plans/2026-10-18-pool-document-edits-design.md) records scope.
This is local pure-domain evidence. No installed fstab, helper, mount, balance,
systemd, API/UI, durable-job, package, migration, resource/performance, hardware,
crash-recovery, defect or phase acceptance is claimed.

| Actual check | Result |
| --- | --- |
| Baseline `pools` filter | Eight passed; 170 filtered; no whole-domain baseline credit |
| Final native `limeos-domain` unit suite | 207 passed; zero failed/ignored/filtered |
| Final compile-fail doctests | Seven passed, including two new borrowed-lifetime/deserialization checks |
| New cases in those totals | 29 unit and two compile-fail doctests; repetitions add no checks |
| Strict all-target Clippy | Passed, `-D warnings` |
| Rustfmt, three affected source paths | Passed |
| Frozen renderer regressions | Existing layouts, policies, presets and option precedence continue to pass in the actual domain suite |
| Collector Ruff 0.17.0 format/check | Passed |

`test-first.log.gz` records 205 unit/seven doctest passes before the explicit
shape preflight. `test-final.log.gz` records 206/seven after adding that guard.
Review then added the owned-target spelling regression; `quoted-target-red.log.gz`
reproduces the admission error (one failed, 206 filtered). The root cause was
applying the closed target spelling check only to external records. The final
code uses the same guard on typed previous/desired targets, without changing the
public configuration validator/renderer. `test-verified.log.gz` and
`clippy-verified.log.gz` bind the final passing behavior. No failed attempt has
been discarded and no old run is counted again.

`red-regression.patch` adds just that test against initial feature commit
`340d7e1`. In a separate checkout of that commit, apply the patch and run
`cargo test --offline --locked -p limeos-domain typed_owned_targets` to reproduce
the recorded failure. The final product includes the correction and the test.
Do not mutate the frozen source used for acceptance to reproduce a failed run.

## Reproduction and source binding

All commands ran sequentially with disk scratch/caches and one local Cargo job:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export CCACHE_DIR=/home/marc/Documents/github/lime-os/.cache/ccache
cargo test --offline --locked -p limeos-domain
cargo clippy --offline --locked -p limeos-domain --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/pools.rs crates/domain/src/pools/document.rs crates/domain/src/pools/document/tests.rs
```

Dependencies were reused offline at the root lock; no manifest/lock update or
dependency fetch ran. The packet includes the exact lock and tool/environment
metadata. `inputs.sha256` binds 195 domain source/manifest, root Cargo/toolchain
and tracked fixture files. Fixtures are a conservative superset of used inputs;
hashing them does not mean all their suites ran. Check this manifest from the
candidate worktree root. `reference-inputs.sha256` covers the two frozen MergerFS
source/test inputs at pi-health `80593b2`; verify from that repository. They were
read only, never imported or executed. Actual test inputs are the existing JSON
layouts and synthetic supplied byte buffers.

`artifacts.sha256` covers the logs, metadata, lock, collector, regression patch
and this README relative to this directory. The collector records the actual
capture procedure and asserts the final source shape/counts. Its source-bound
record is not runtime validation. Inherited assistance packets retain their own
source/checkout requirements; their counts are not added here. No frontend file
changed and no frontend or broader backup/workspace/guest suite was repeated.

The current reader's CRLF/semantic limitations and complete shared authority,
FUSE/claim, file identity, before-image, publication/reload/verification/recovery
and installed-return requirements remain in the handoff. No main merge or
production activation occurred. The historical inventory p95 61.401 ms against
20 ms remains failed, and full Stream A/P04/P05/P06 gates remain open.

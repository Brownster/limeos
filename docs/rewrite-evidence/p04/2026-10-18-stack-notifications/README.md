# Stack notification leaf evidence, 18 October 2026

Product source: `b72c14cfdf3ca904195ce0403f1f54f5c4cd6b7d`, branch
`engineer/p04-stack-notifications`, based on main
`7df3f7530865c31b7faefd1f7b12a25aa7655860`. Later documentation commits do not
change the tested product. Seven product paths add the B-owned domain and
supplied-body decoder, their module declarations, tests and fixture.

The [handoff](../../../p04-stack-notifications-handoff.md) describes behavior,
input replacements, preserved configuration/channel obligations and the central
ingestion/outbox work. The [design](../../../plans/2026-10-18-stack-notifications-design.md)
records scope. This packet gives local leaf evidence only. It grants no new
installed, browser/API/UI, token/authentication, durable publication/deduplication,
external delivery, ARM64, package, resource, performance, crash-recovery, defect
or P04/cutover acceptance.

## Actual local results

| Check | Result |
| --- | --- |
| Focused notification tests | 30 passed: 13 domain and 17 decoder; included in the combined count below |
| Combined B harness on this source | 454 passed: 294 domain, 135 supplied-reader/host-policy, 19 API harness and six observation runner; zero failed/ignored/filtered |
| Combined doc-tests | Zero cases; no extra credit |
| Strict all-target Clippy | Passed, `-D warnings` |
| Formatting, six affected Rust files | Passed |
| Central protected-path comparison against base | Passed for manifests/lock, production domain/container roots, persistence, API root, Core main and dashboard |
| Collector Ruff 0.17.0 format/check | Passed after correcting the collector's non-executable shebang mismatch |
| Frozen event-map AST comparison | All 16 aliases and their quiet-policy membership match the literal frozen table; zero functional-test credit |

The combined harness checks the B leaves present at the base plus this change.
It compiles production domain/container libraries as dependencies and proposed
API/runner wrappers. API cases use test backends; runner cases use local doubles.
No production route, Store transaction, supervised worker or UI mounting gains
this behavior from the harness. Other B branches do not inherit these results.
No frontend source changed and no frontend suite was repeated.

The frozen JSON fixture has eight representative payloads and all 16 aliases.
Loops exercise aliases under both modes. Fixture iterations, repeats and static
checks are not additional native cases. The original frozen Python code and
tests were read/AST-parsed only, never imported or executed.

`test-first.log.gz` retains three decoder failures: derived structs admitted
empty arrays, including a movie object, and ignored unknown values bypassed
Serde's normal nesting limit. `test-diagnosis.log.gz` repeats those with exact
synthetic inputs and adds a failing source-default preservation assertion.
The reference only lowercases supplied instance names, not the configured
fallback. Map-only record wrappers, a constant-memory 32-container preflight and
the corrected fallback now pass, including exact depth/escape boundaries.
Both failed attempts are retained; their overlapping cases are not new checks.
`collector-ruff.log.gz` retains the unrelated helper shebang lint failure.

## Reproduction and immutable inputs

Use disk caches, one local Cargo job and one suite at a time. From this worktree:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=1
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export CCACHE_DIR=/home/marc/Documents/github/lime-os/.cache/ccache
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/stream-b/check-leaves.py
```

The initial offline harness resolution created its own lock. For an exact repeat,
copy this packet's `harness/Cargo.lock` to `.cache/stream-b-leaf/Cargo.lock` after
generating the wrappers, then run the recorded checks:

```sh
cargo test --offline --locked --manifest-path .cache/stream-b-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/stream-b-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/mod.rs crates/domain/src/deployment/notifications.rs crates/domain/src/deployment/notifications/tests.rs crates/executor-container/src/deployment/mod.rs crates/executor-container/src/deployment/notifications.rs crates/executor-container/src/deployment/notifications/tests.rs
```

`harness/` preserves all ten generated manifest/source/lock files. Wrappers retain
the actual worktree's absolute source paths. Regenerate them for another checkout;
keep this packet's dependency lock. The production Cargo.lock was unchanged.
The manifest records compiler/tool versions, environment paths, original test
summaries and exact source identities. Dependencies were available offline; this
is not a dependency advisory or release qualification run.

`SOURCE_INPUTS.sha256` covers 418 tracked source/manifests/fixtures, including a
conservative superset of the actual harness inputs. It does not assert that every
included file's tests ran. Check it with `sha256sum -c` from the candidate worktree
root. `REFERENCE_INPUTS.sha256` covers the 12 read-only frozen inputs; check from
the frozen pi-health root. `SHA256SUMS` covers packet artifacts relative to this
directory. The archived collector records the actual capture procedure from
`.cache/stack-notifications/collect.py` at the tested product head.

No Pi, production Docker, service activation, migration, external message or
installed test-host mutation occurred. Central schedule registration and storage
worktrees were not edited, and main remains at the base source.

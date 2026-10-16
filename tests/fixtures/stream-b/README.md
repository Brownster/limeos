# Combined Stream B leaf checks

This entry point checks the committed automation, catalog/media/network/update
models, read adapters, host policy parsers, proposed API handlers and observation
runner together. It uses the current B-owned module lists, avoiding a new manual
list for each leaf. It leaves production root exports, routes, Store/schema,
protocol, workers, claims/authority, frontend shell and package activation intact.
Successful checks do not prove registered runtime behavior or P04 completion.

Use repository disk scratch and cached dependencies, one local Cargo job/suite at
a time. From the worktree root:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
export CCACHE_DIR=/home/marc/Documents/github/lime-os/.cache/ccache
uv run --offline --no-project tests/fixtures/stream-b/check-leaves.py
cargo test --offline --manifest-path .cache/stream-b-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/stream-b-leaf/Cargo.toml --all-targets -- -D warnings
```

The first Cargo invocation creates a separate harness lock; subsequent runs can
use `--locked`. The production lock is unchanged. Wrappers use absolute current
worktree paths, so regenerate them after moving the checkout. Root domain code
and contracts compile as dependencies; the full root-domain unit suite is not
implicitly included. API root unit tests are included alongside the B handlers.
No default production router or actual Core/Store backend gains these routes.
The wrapper joins both proposed route sets for compilation, so strict Clippy
checks their handlers without suppressing unused-code warnings.

Run frontend type and unit checks separately after the Rust suite finishes, using
the pinned project dependencies and one test worker. Installed host/hardware,
package, performance and soak qualification remain with the qualification owner.

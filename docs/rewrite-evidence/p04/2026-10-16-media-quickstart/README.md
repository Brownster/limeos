# Media quickstart preparation evidence

Qualified product source `684aa81a6a5a9117d34c07abcd057974e30ccfb1`, base `f4514d4`.
See the [handoff](../../../p04-media-quickstart-handoff.md).

The unfiltered isolated domain harness passes **186 tests**: 23 new quickstart,
18 indexer application, 18 download-client, 18 settings, 24 local-config,
37 catalog/namespace, 21 library, 15 activity and 12 discovery. Strict all-target
Clippy, Rustfmt and helper Ruff pass. The generated manifest/lock/wrapper hashes
are stable across regeneration. Product bytes were committed after checks without
intervening edits. Prior decoder/API/frontend packets keep separate source/results.

`metadata.json` records source, commands, counts, bounds and environment.
`inputs.sha256` identifies project domain source, manifests/production lock and
dedicated fixtures/helpers; hashing does not credit every input's suite. Frozen
quickstart/layout/profile services and tests, compatibility projection and bundle
are identified by `frozen-inputs.sha256` at
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read/hashed only.

The first `test-initial.log.gz` passes 185 cases before the capacity regression.
`capacity-red.log.gz` and source snapshots retain the focused failure: nine
unrelated services plus eight missing members were admitted despite the catalog
ceiling. `test.log.gz` is the final 186-case run after the fix; these counts are
not cumulative. Boundary coverage also includes eight other services plus eight
members, and eight other services plus one declared member and seven additions.

Logs, Cargo metadata/tool versions, clean source status and `harness/` retain the
exact local check identity. Original domain dependency code compiled, but its
entire unit suite did not run. `package-evidence.py.gz` packages recorded results
without rerunning suites. `artifacts.sha256` covers packet artifacts except itself.

Reproduce with cached dependencies, disk scratch under the repository and one
suite at a time. The helper regenerates absolute paths for a new checkout:

```sh
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 RUST_TEST_THREADS=2
export CARGO_TARGET_DIR=/home/marc/Documents/github/lime-os/.cache/rewrite-pool-dependencies/target
export TMPDIR=/home/marc/Documents/github/lime-os/.cache/tmp
export UV_CACHE_DIR=/home/marc/Documents/github/lime-os/.cache/uv
uv run --offline --no-project tests/fixtures/deployment/quickstart/check-leaf.py
cp docs/rewrite-evidence/p04/2026-10-16-media-quickstart/harness/Cargo.lock .cache/quickstart-leaf/Cargo.lock
cargo test --offline --locked --manifest-path .cache/quickstart-leaf/Cargo.toml
cargo clippy --offline --locked --manifest-path .cache/quickstart-leaf/Cargo.toml --all-targets -- -D warnings
rustfmt --edition 2024 --check crates/domain/src/deployment/quickstart.rs crates/domain/src/deployment/quickstart/tests.rs crates/domain/src/deployment/mod.rs
sha256sum -c docs/rewrite-evidence/p04/2026-10-16-media-quickstart/inputs.sha256
```

Cases cover frozen bundle/directory order, all three storage profiles, managed
numeric identity, unknown versus absent/existing targets, exact scope/freshness,
mapping/ownership/capacity conflicts, bounds, workflow dependency declarations and
the selected-field version-two profile draft. There is no runtime event consumer
or receipt verification test here. Ordinary catalog facts prove neither protected
origin, physical absence, installation equivalence nor execution authority.

No decoder/API/UI/browser, actual source/ownership/mount, directory/file/Compose/
seeding/profile effect, durable workflow/recovery, package/ARM64/Pi, performance,
footprint, soak or P04 completion is claimed.

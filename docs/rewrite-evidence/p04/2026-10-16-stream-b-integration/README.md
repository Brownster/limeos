# Combined Stream B source checks

This packet binds local checks to the combined source and new harness commit
identified in `metadata.json`. The candidate includes main's storage-mount
registration `0260538` and all five completed B heads listed in the
[handoff](../../../p04-stream-b-integration-handoff.md). Documentation and this
packet are committed afterward. No product source changed between final checks
and the harness commit.

| Local check | Result |
| --- | --- |
| Combined standalone Rust workspace | 424 passed: domain 281, adapter/host policy 118, API 19, runner 6 |
| Clippy | All harness targets passed with warnings denied |
| Frontend | Type-check passed; 89 cases passed across seven files |
| Helper checks | Ruff passed; generator preserved ten harness files and 14 shared source/manifest files |
| Merge module lists | Rustfmt check passed; both branch exports retained |

`inputs.sha256` covers a source/fixture superset, including compiled original
crates, proposed B leaves, frontend sources/tests and shared manifests/generated
contracts. It does not mean all those sources' unit tests were run. Root domain
and existing executor libraries compile as dependencies; their complete unit
suites are outside this packet. API root tests are included.

The generated workspace manifests, absolute source wrappers and separate lock
are under `harness/`; regenerate paths using the
[check instructions](../../../../tests/fixtures/stream-b/README.md) in another
checkout. Production workspace manifests and registrations remain intact.
`harness-regeneration.json` preserves before/after hashes. Toolchain, pinned
installed frontend package identities and commands are recorded in metadata.
`artifacts.sha256` binds the packet files.

The parent Node runner reported seven successful file-level entries. The direct
sequential rerun records each case in `frontend-direct-*.log.gz`, with an 89-case
summary in `frontend-direct-results.json`. Both results are preserved, without
counting the rerun as additional coverage.

Initial logs retain the new harness's missing discovery imports, a compiler-cache
read-only filesystem failure and a passing 422-case run before merging the newer
main. The final harness imports the existing evidence modules and exposes both
proposed API route sets to Clippy. Compiler cache and all scratch use repository
disk paths. Intermediate results do not qualify the final candidate separately.

This packet proves combined local compilation and leaf regressions. B's routes,
workers, Store transactions and UI panels remain unregistered in production.
It supplies no installed deduplication, effect recovery, package, ARM64,
performance, soak or Pi qualification. Existing focused packets retain their
separate identities; no P04 or cutover completion is claimed.

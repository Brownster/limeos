# Supplied egress comparison acceptance

Tested product source: `14427af0bb922646b10b73bc3054a69778790766`, based on
combined Stream B main `7df3f75`. The design precedes implementation; this packet
and the [handoff](../../../p04-network-egress-handoff.md) follow the product commit.
No product source changed between final checks and that commit.

The combined B harness passes **453 Rust cases**: 301 domain, 127 reader/host
policy, 19 API and six runner. The **29 new tests** cover 20 egress policy cases
and nine supplied-output decoder cases; their fixture tables contain ten address
comparisons and 16 response scenarios. Strict all-target Clippy and Rustfmt pass.
The initial passing log retains a test-name style warning; the final log and
Clippy use the corrected lowercase test name.

`metadata.json` records source, commands, toolchain, limits and scope. The
`inputs.sha256` source/fixture superset includes the original compiled crates and
shared generated contracts, without implying that their full unit suites ran.
Root domain/executor libraries compile as dependencies; API root tests run in
the harness. No frontend checks ran for this slice. The earlier 89 frontend cases
retain their separate combined-integration packet identity.

The harness uses the previously recorded integration lockfile, copied unchanged,
and regenerated absolute source wrappers under `harness/`. Reproduce using the
[combined check instructions](../../../../tests/fixtures/stream-b/README.md);
copy this packet's harness lock into the generated workspace before a locked
run. `harness-regeneration.json` records identical before/after hashes for ten
generated harness files and 14 shared source/manifest files.

`frozen-inputs.sha256` records three files under the frozen root named in
metadata; each worktree file was checked against its committed `80593b2` blob.
They were read/hashed, never imported or executed. Fixtures are synthetic ports,
not live endpoint/Engine captures. `artifacts.sha256` binds the archived files.

This proves pure local policy, supplied-response admission and regression
behavior. It does not establish endpoint/public routability, live process or
namespace identity, tunnel use or leak absence. Every report keeps VPN evidence
unestablished. No collector, HTTP/container-exec request, authority, Store,
protocol, production export/API/UI, package activation or host changed.
Protected collection, installed failure/restart behavior, resource budgets,
ARM64/performance/soak and Pi qualification remain open. No defect row, P04 phase
or cutover gate is closed by this packet.

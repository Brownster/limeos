# Cached network-group API/UI local acceptance

Tested product source: `5e107872595176d0296d157bec445012ba834d15`, based on
`31b3b68`. The design precedes implementation; this packet and the
[handoff](../../../p04-network-read-ui-handoff.md) follow the product commit.
No product source changed between final checks and that commit.

| Local check | Result |
| --- | --- |
| Combined B Rust harness | 461 passed: 301 domain, 127 reader/host policy, 27 API, six runner |
| New Rust coverage | Eight network API/wire cases |
| Strict Clippy and Rustfmt | Passed |
| Frontend type/unit checks | Passed; 123 cases across eight files, including 34 new network cases |
| Fixture preview build | Passed with pinned Vite/React and the existing stylesheet |
| Harness regeneration | Ten generated files and 14 shared source/manifest files preserved |
| Browser visual checks | Unavailable; no browser enabled and no screenshot acceptance |

`metadata.json` records the exact source, toolchain, dependencies, commands and
scope. `inputs.sha256` covers a source/fixture superset; it does not mean every
included crate's unit suite ran. The Rust API test binds `wire.json` to the
existing domain projection, and frontend tests consume the same synthetic file.
Original domain/executor libraries compile as dependencies; API root tests are
included. Root workspace, Store/protocol and shell registration remain intact.

The generated wrappers and unchanged prior integration lock are under `harness/`.
Regenerate absolute paths using the [combined instructions](../../../../tests/fixtures/stream-b/README.md)
before copying this packet's lock into the new generated workspace. The scripts
used for sequential frontend cases and regeneration are archived with the logs.
No frozen Python, live Docker/HTTP probe or production service is executed.

`preview/` preserves the fixture source/config and final self-contained built
assets. It is a standalone view preview, not the production dashboard or a browser
render. The temporary loopback server was stopped after the browser tool reported
no available browser. `browser-availability.json` records that limitation.
Scratch moved into the ignored repository cache; the final preview config uses
explicit pinned React aliases. The sandbox listener failure and moved-preview
dependency-resolution failure remain archived separately from the successful
final build. No visual, mobile or responsive acceptance is implied.

Initial logs/source snapshots preserve the nullable-count TypeScript diagnostic,
the overbroad frontend regex failure and the missing explicit Rust child-module
path. The final runs follow those fixes. `artifacts.sha256` binds the archived
files. Existing topology/egress packets retain their separate qualification
identities; duplicate runs are not additional coverage.

This packet proves local cached-handler, wire, controller and markup behavior.
Actual protected-cache integration, permission snapshots, production routes and
middleware, generated contracts, authenticated shell mounting, installed/browser
lifetime cases, worker budgets, recovery, ARM64/performance/soak and Pi acceptance
remain open. It closes no defect-register row, P04 phase or cutover gate.

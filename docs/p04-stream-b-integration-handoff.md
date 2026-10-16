# Stream B combined-source handoff

The completed Stream B branches are combined for landing on local `main`, as
requested on October 16. The integration preserves the separately landed storage
mount registration on main `02605383730fb56bc47035cafd34dc1e80cdf982`.
This lands B's proposed leaves; it does not register their runtime or finish P04.

| Completed head | Supplied behavior |
| --- | --- |
| `7fff043f697fef7a785709f97508743d11994ffc` | Stack discovery, catalog/namespaces, media library/activity/configuration/settings/client/indexer planning, quickstart preparation and frontend readers |
| `485206b0fe5675e438f81e45ca91fbb552349a50` | Observation schedule/alert/delivery models, runner, API handlers and frontend controller/panels |
| `a3e05e7ac87343addcd955427b90b38cc616883a` | Fixed package policy and host prerequisite parsers, including prerequisite head `65ed86e` |
| `224f12c31517d133c7528b3663111a39081befe4` | Bounded image-content decoding and container update policy, including policy head `d8f94de` |
| `f5fdffa0e53b2e823129183c6480a9480c179a86` | Network grouping policy and supplied Engine network evidence decoding |

The overlapping B-owned module lists retain every module from each branch. No
branch implementation was replaced to resolve those overlaps. Existing focused
handoffs and evidence retain their original source and qualification scope.

## Repeatable combined checks

The new [combined harness](../tests/fixtures/stream-b/README.md) compiles the actual
B module lists together: domain, container readers, host update policy, proposed
API handlers and observation runner. It imports the existing Compose/Docker
evidence libraries required by discovery, with a separate generated lockfile.
The API wrapper exposes the proposed routes only inside the check harness.
Frontend type and unit checks run separately using the pinned dependencies.

The combined checks passed 424 Rust cases (281 domain, 118 reader/host-policy,
19 API and six runner) and 89 frontend cases across seven files. Strict
all-target Clippy, frontend type-checking, helper Ruff and module-list formatting
passed. Regeneration preserved all ten harness files, including its lock, and
14 shared source/manifest files. These are local leaf checks.

The [integration evidence](rewrite-evidence/p04/2026-10-16-stream-b-integration/README.md)
records exact source/input hashes, generated wrappers and lock, local check
results and the failed initial harness/environment attempts. It gives no new
installed, ARM64, performance, package or crash-recovery qualification credit.

## Runtime handoff

B's automation/deployment root exports, production API routes, Core backend,
Store transactions/migrations, worker registration and frontend shell mounting
remain pending. The storage mount runtime landed by A remains separate.

The first milestone remains the registered observation-only schedule and in-app
notification path. The [B1 handoff](b1-automation-handoff.md) specifies the needed
central Store contract: atomic occurrence reservation/job creation, consumed
observation sequence, alert/outbox transition and publication, current grants,
revision checks, restart reconciliation and retention that preserves deduplication.
The existing tests exercise proposed traits and local doubles; they do not prove
durable deduplication through the shared Store.

After central contract acceptance, implement B's persistence leaf against that
contract, then register API/UI and the supervised observation runner. The existing
qualification owner runs the installed concurrent-tick/restart/failure scenarios.
Deployment/package effects still need the shared authority, resource claims,
locks, verified receipts and recovery path. Quickstart preparation still needs
its event consumer and verified stage execution; its profile draft is unsaved.
Production Pis remain untouched, and full cutover retains P05/P06 gates.

# Local main consolidation, validated 20 October 2026

This integration collects all committed local branch tips descended from or
reconciled with local main `7df3f7530865c31b7faefd1f7b12a25aa7655860`.
Original branch names and tips, dirty-file recovery copies and a complete Git
bundle are retained under `.cache/main-consolidation/`. No production/Pi service
or disk operation ran. Local integration does not close P04 or cutover gates.

## Integrated source

- Storage assistance: mount UI lifetime fixes, backup before-image preflight,
  numeric SnapRAID scrub planning, Samba and MergerFS document edit planners,
  and the frozen remote-mount preservation inventory
- Deployment/scheduling assistance: UI mutation lifetime fences, egress
  comparison, cached network/tools panels and normalized stack notifications
- Central scheduling: schema 11 reservations, fixed ObserveHealth
  self-enablement and durable schedule metadata routes with generated contracts
- Storage compatibility: use the real proc-stat PF_KTHREAD bit when the optional
  Kthread status field is absent; corroborate and validate any present field
- Original historical branches: retain their ancestry while preserving newer
  integrated archive, pool and ARM64 implementations; keep two distinct original
  symlink-lock regressions

The other engineer's source was in `engineer/p04-observe-health-schedules`, with
the ABI correction in `engineer/p04-kthread-abi-compatibility` and the earlier
hosted source in `qualification/p04-observe-health-schedules-bootstrap`.
The final schedule feature and contracts are committed as `ca13ffb`, its handoff
as `71df2a6`, and the ABI correction as `4baf338`.

Integration also corrects generated TypeScript trailing whitespace (`6759e62`),
aligns the existing UI with the generated timing input and truthful readiness
states (`3a9782c`), and makes the combined B harness reuse the registered domain
types (`e051394`). Archived preview paths move out of reserved `dist` directories
without changing payload bytes; original manifests remain compressed alongside
explicit relocation records.

Schedule creation sends only interval, grace, missed-run policy and an optional
UTC window. The server owns operation, owner and anchor. Saved schedules show
`awaiting_producer` or `authority_suspended`, not a running/healthy claim. Creating
or resuming metadata still starts no timer, producer or effect. The existing
automation panel remains a component awaiting authenticated shell integration;
this consolidation does not register the proposed alert/notification routes or
other B leaf adapters.

## Validation and limits

The [final local validation packet](rewrite-evidence/p04/2026-10-20-local-main-validation/README.md)
binds source `6466eb0` and records the completed checks. The
[initial assembly packet](rewrite-evidence/p04/2026-10-19-local-main-consolidation/README.md)
retains the earlier attempts and their original limits.

| Check | Result |
| --- | --- |
| Frontend tests | 219 cases passed across eleven files, sequential native case reporters |
| TypeScript | Strict project check passed |
| Production frontend build | Passed; 22 modules, no mounted browser/visual acceptance |
| Reference tests | 57 passed with local IPC outside the sandbox |
| ARM64 qualification harness | 63 pure harness cases passed; no ARM64 hardware execution |
| Schema generator and contract drift | Passed through the actual Rust generator; regression included in workspace results |
| Formatting and repository hygiene | Passed |
| Complete native Rust workspace | 1,130 unit/integration/doctest checks passed across disjoint workspace-rest, storage-unit and storage-doctest runs |
| Combined Stream B harness | 507 passed; overlaps root API cases, so its count stays separate |
| Strict all-target workspace and B Clippy | Both passed with `-D warnings` |
| Fresh native ARM64 and installed/package/VM checks | Not run for the combined source |

The reference/harness and schema records retain their earlier input snapshots;
later UI, leaf-harness and documentation corrections are identified separately.
Old source-bound leaf packets keep their original source/test claims. Counts
from old runs, harnesses or repeated checks are not added to the current totals.

After cleanup restored headroom, the full native run reached tests and exposed
17 existing Btrfs fixture failures. The documented storage-fixture procedure
resolved the test setup without changing production code or assertions:
compile on disk, then execute only the compiled storage tests in a private
runtime tmpfs directory. All 211 storage unit tests pass; sampled data peaked at
57,496 bytes and the directory was removed. Cargo, caches, logs and general
scratch stayed on disk. The remaining workspace and 13 storage compile-fail
doctests passed separately. Initial missing-cache, guard-stop and filesystem
failures remain archived; this does not qualify Btrfs source resolution.

The earlier [hosted bootstrap](https://github.com/Brownster/limeos/actions/runs/39155342674)
qualified the schedule Rust inputs and generated real contracts. The separate
[ordinary bootstrap CI](https://github.com/Brownster/limeos/actions/runs/39155342585)
failed at the expected tracked-contract drift check on both architectures. Those
runs predate this combined candidate and its ABI correction. The final local
checks qualify the assembled source independently. A fresh hosted ARM64/matrix
run requires publishing a qualification branch, which remains subject to the
user's optional pending permission. No push occurred during local consolidation.

The supplemental `qualification-stream-b-leaves.yml` workflow checks the existing
combined B harness on native AMD64 and ARM64. It supports explicit dispatch or
the temporary `qualification/local-main-2026-10-19` branch. Production root CI
remains unchanged. Source identity, exact production/harness locks, actual test
output and strict Clippy output are archived by that workflow.

## Cleanup boundary

Remove a worktree only after its HEAD is an ancestor of actual local main and its
tracked/untracked status is empty. Preserve ignored evidence caches separately;
delete only local branches with merged ancestry and no remaining worktree.
Cleanup removed **78 worktrees and 87 local branches** after these checks. The
machine-readable outcome is `.cache/main-consolidation/cleanup-result.json` and
is also archived in the final packet.

Eight worktrees contain uncommitted drafts and are retained, with recovery copies:

- `bound-private-reconstruction`
- `p04-a1-admission`
- `p04-a1-effects`
- `p04-a1-guest-qualification`
- `p04-a1-plan-diagnostics`
- `p04-a1-pre-effect-qualification`
- `p04-b1-store-integration`
- `p04-mount-subphase-diagnostics`

Their committed HEADs are included in the integration. Their uncommitted content
is not silently committed, discarded or credited as runtime acceptance.
Remote branches are unchanged. Full storage runtime, genuine automation
production, installed recovery, the failed inventory latency budget and P05/P06
migration/return qualification remain open.

# Local main consolidation, validated 20 October 2026

## Re-audit, 21 October 2026

A fresh `git fetch origin --prune` found no remote work outside local main.
The other engineer's schedules and verified contracts (`ca13ffb`), handoff
(`71df2a6`) and Linux ABI correction (`4baf338`) are all ancestors of main.
The [21 October packet](rewrite-evidence/p04/2026-10-21-namespace-roots-consolidation/README.md)
records the additional fast-forward from `d7f09e1` to `b28bdb1`, the branch
audit and current contract regeneration. No push occurred.

The new storage classifier retains genuine `nsfs` namespace roots across both
host and process read profiles. Such roots cannot become physical backing or
destination paths; an uninspected bound mount namespace still refuses storage
effect admission. Its committed source matches all 671 tracked inputs in each
successful compile, unit, Clippy and doctest record. The 225 storage unit tests,
13 compile-fail doctests, strict Clippy, formatting, actual contract drift and
repository checks pass. The packet preserves the pre-fix regressions and the
initial Btrfs fixture failure. This slice has no installed or ARM64 qualification.

The merged `engineer/p04-storage-namespace-roots` branch was deleted. There are
no clean secondary worktrees or unmerged local/fetched remote branch tips.
The eight dirty worktrees listed below remain intact, with fresh recovery copies
under `.cache/main-consolidation/request-2026-10-21/recovery/`. Their committed
tips are on main; their uncommitted drafts are not accepted source. A separate
unfinished VM probe from the root checkout is preserved byte-for-byte in
`request-2026-10-21/unfinished-namespace-probe/` under the same cache directory.
It passed Clippy but has no guest runtime result. A verified full Git bundle
was saved before deleting the clean branch. The checkout returns clean on main.

The earlier native hosted CI results retain their original source identity.
The installed A1 userspace-task conflict, broader A/B runtime work and P05/P06
cutover requirements remain open. No Pi connection or production mutation ran.

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

The subsequent [hosted qualification packet](rewrite-evidence/p04/2026-10-20-local-main-hosted/README.md)
records the completed main CI run on `477b81f`. Both native ARM64 and AMD64 jobs
passed: 1,130 Rust checks and 219 frontend cases per architecture, actual contract
drift, strict Clippy, dependency/advisory gates and Debian package builds. AMD64
also passed the browser lifecycle, logs and keyboard-focus regressions.

The full run failed in the Debian VM's A1 plan creation. The genuine host-consumer
scan encountered a ninth mount namespace against the documented limit of eight,
returning HTTP 503 before effect admission. This is distinct from the corrected
ABI and contract failures. The evidence does not identify the excess namespace's
owner or total namespace count. The descriptor reservation is already 225 of
226, so no production limit or safety check was relaxed. P01/P02/P03 VM wrappers
completed; the subsequent dependency VM wrapper was not reached. Full installed
acceptance, installed ARM64 and Pi qualification remain open.

The subsequent [A1 namespace-budget work](p04-a1-namespace-budget-handoff.md)
specializes fixed-path configuration admission and fits sixteen namespace
anchors within unchanged service limits. Its new executor passes the old
overflow but reaches a distinct userspace-task revalidation conflict. Native
checks pass; installed A1 remains open, with process-witness attempts and host
memory stops preserved separately from this consolidation's original CI result.

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
checks qualify the assembled source independently. The later main run
[39429582703](https://github.com/Brownster/limeos/actions/runs/39429582703)
supplies fresh native qualification with the installed failure described above.
No push occurred during local consolidation. The previously proposed temporary
qualification branch was not published.

The supplemental `qualification-stream-b-leaves.yml` workflow checks the existing
combined B harness on native AMD64 and ARM64. It supports explicit dispatch or
the temporary `qualification/local-main-2026-10-19` branch. Production root CI
remains unchanged. Source identity, exact production/harness locks, actual test
output and strict Clippy output are archived by that workflow.

## Cleanup boundary

Remove a worktree only after its HEAD is an ancestor of actual local main and its
tracked/untracked status is empty. Preserve ignored evidence caches separately;
delete only local branches with merged ancestry and no remaining worktree.
Cleanup removed **78 worktrees and 88 local branches** after these checks. The
original 78/87 machine-readable outcome is
`.cache/main-consolidation/cleanup-result.json` and is archived in the local
validation packet. One later committed design branch was fast-forwarded to main
and deleted; its unfinished code draft was preserved under
`.cache/main-consolidation/recovery/stack-inventory-start/`. No draft code was
merged or credited as stack runtime acceptance. The hosted packet records this
additional branch deletion.

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
The final audit confirms all local branch tips and live remote branch tips are
included in actual local main. The root checkout is clean on `main`. Fresh dirty
recovery copies are under `.cache/main-consolidation/final-audit/recovery/`, with
complete history in the verified `refs-final.bundle` alongside earlier bundles.
Remote branches are unchanged. Full storage runtime, genuine automation
production, installed recovery, the failed inventory latency budget and P05/P06
migration/return qualification remain open.

# B1 Store observation settlement

Source `11843c3076cb1918d533b5822af896565f948193` implements the
[completion design](plans/2026-10-21-b1-observation-completion-design.md) in the
existing sole Store. [Local evidence](rewrite-evidence/p04/2026-10-22-b1-observation-settlement/README.md)
binds six final checks to 685 recorded tracked inputs each. This qualifies the
Store boundary; the runtime producer is still unregistered.

## Committed behavior

Schema 12 adds owner sequence watermarks, compact immutable results, current
alert material and the in-app outbox. Each owner admits only its oldest queued
observation and one private, short-lived collection lease. The Store keeps at
most 64 leases. A nonce, generation, current grant revision, selected concrete
read resources and original monotonic deadline bind collection to completion.
Restart invalidates abandoned leases while retaining the original queued job.

Completion rechecks authority after the SQLite write lock, consumes an ordered
owner sequence and atomically commits alert transitions, unique incident/recovery
outbox entries, the compact result, terminal job and audit proofs. Failed SQL,
capacity or authority checks leave the entire transaction unconsumed. Missing
and unavailable signals preserve existing incidents. An unchanged alert does not
get rewritten merely because another observation completed. Exact lost-response
replay returns the existing receipt.

Startup checks compiled schema and triggers, reciprocal job/result/watermark
proofs, the full current alert-material digest and notification creation,
publication, acknowledgment and pruning proofs before generation or recovery
writes. Migration preserves existing schema-11 queued and canceled jobs.
Generic observation dispatch and running/verifying transitions remain refused.

The in-app publisher rechecks owner/resource authority and the original grant
revision. Recovery delivery uses its actual opening receipt. Publication and its
audit commit together. Bounded owner-scoped pages authenticate current sessions
and filter individual resource scopes; acknowledgment also requires CSRF and a
visible delivered row with revision compare-and-swap. Retention removes only
eligible delivered rows after seven days, retaining needed opening receipts and
auditing pruning. Pending rows remain pending.

The existing 256-alert, 1,024-notification and audit limits remain. Current alert
material cannot be silently discarded to bypass capacity; compact results and
owner sequence proofs follow retained job/audit lifetime. This change adds no
independent database, effect executor or external notification sender.

## Local qualification

| Check | Result |
| --- | --- |
| Entire persistence crate | 143 unit tests and two compile-fail doctests passed |
| New completion regressions | Ten test functions passed within that full unit run |
| Strict Clippy | All workspace targets passed with warnings denied |
| Formatting | Workspace check passed |
| Contracts | Actual Rust producer regenerated output; drift check passed |
| Repository | Dependency direction and tracked-output hygiene passed |
| Reference suite | All 84 tests passed, including the 11 notification inventory tests after final formatting |

The ten new functions cover owner ordering, restart and late leases, real SQLite
lock/deadline waits, changed grants/resources, partial data, capacity, rollback
at result/alert/outbox/audit/publication boundaries, startup corruption,
schema-11 migration and delivered retention. They overlap the 143-test total.

The first full run found seven legacy migration-fixture failures: the shared
downgrade helper left schema-12 tables behind. Correcting that test setup made
the full suite pass. Failed and preliminary records remain separate from final
source qualification. Earlier low-memory stops remain recorded. Final checks
used disk caches/scratch, one Cargo job and one test thread under the unchanged
3-GiB host floor.

## Next integration boundary

Use the accepted post-lock reservation clock and these Store methods through the
existing bounded Store worker. Supply a protected fresh collector with genuine
concrete resource bindings and the original read lifetime, then supervise its
bounded scheduler/publication work in Core. Register actual alert/notification
API/UI behavior and installed restart/concurrent-tick acceptance afterward.

Folding currently uses `AlertPolicy::default()`. Before configured-policy or
migration acceptance, bind current settings revisions and preserve the
[frozen notification rules](p04-frozen-notification-inventory.md), destination
references and silence metadata. Review how legacy name/path alert keys survive
replacement of a concrete read resource without silently rebinding an incident
or stranding it. This Store qualification does not establish those adapters.

Installed packages, ARM64/Pi behavior and whole-service resource, idle-write and
soak measurements remain separate gates. B1/P04 and cutover are not complete.

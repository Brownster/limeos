# B1 asynchronous Store bridge

Source `7b44a0158ffc5ec044121b4aa914786cf65d08e4` implements the
[bridge design](plans/2026-10-22-b1-async-store-bridge-design.md).
[Local checks from 26–27 October](rewrite-evidence/p04/2026-10-27-b1-async-store-bridge/README.md)
bind six final snapshots to 687 recorded tracked inputs each. This connects Core
orchestration to the accepted Store worker; fresh collection and a production
timer remain unregistered.

The later [owned-history integration](p04-b1-in-app-history-handoff.md) registers
the existing alert/notification reads and acknowledgment through API/UI. Its
source and qualification are separate from this bridge's original evidence.

## Runtime-facing contract

`bins/core/src/automation_store.rs` is compiled with Core. Its
`ObservationStoreBridge` holds the existing `Database` and a fixed production
clock. It creates no second database, thread, queue, sequence counter or recovery
ledger. Each scheduling pass reads and validates at most 32 schedules before
reserving rows through `automation_reserve_current`. Post-lock trusted time and
current stored authority decide admission. Unsupported effect adapters still
produce blocked occurrences without effect jobs.

The caller's original monotonic deadline applies before queueing, when queued
work reaches the Store, throughout the accepted transaction and when the reply
returns. Cancellation does not withdraw already queued work or prove rollback.
A request with a still-live deadline may commit after its caller abandons the
reply. Expired queued work refuses before invoking its Store operation. A late
committed reply is not reported as timely success.

A scheduling page uses separate atomic reservations. An earlier row can commit
before a later row refuses or times out. The returned job vector is advisory.
`queued_page` instead discovers original queued observation jobs from the sole
Store, using an opaque job-ID cursor and a 32-row limit. The query borrows and
bounds intent bytes before Rust decoding and performs no write. Canceled and
completed jobs stay out of that page. Restart never invents replacement work.

Queued hints grant no collection authority. `begin` transfers the Store's private
lease through the worker after current grants, selected resource scopes, owner
ordering and deadlines pass. `complete` uses that lease's original lifetime.
`discard` derives its reason from current stored state. Publication and retention
use the existing bounded in-app methods and return their cursors. A later
supervisor must carry those cursors fairly and wrap at the end of a scan; an
unadmittable job is not proof that another job may bypass owner ordering.

## Verification

| Check | Result |
| --- | --- |
| Core unit tests | 37 passed, including the six new bridge tests |
| Core integration tests | Seven passed: six backup-binding tests and one password-worker test |
| Persistence unit tests | 144 passed, including the new queued-page regression |
| Persistence compile-fail doctests | Two passed |
| Strict Clippy | All targets of Core and persistence passed with warnings denied |
| Formatting, actual contract drift, repository hygiene | Passed |

The six focused Core tests overlap the full Core run. They use actual file-backed
SQLite and the real database worker with controlled fixture wall clocks and
normalized fixture signals. Scenarios include concurrent passes and restart;
a reply held after an actual committed reservation until the original deadline;
an abandoned expired mutation queued behind a blocked worker; private-lease
completion, partial data, publication and retention; changed grants and pause;
and cursor coverage of 33 observation schedules plus five blocked effect kinds.

The Store pagination regression runs with SQLite `query_only=ON`, checks both
pages and invalid bounds, and verifies that completed/canceled jobs disappear
without recreating them. The full suites also retain actual authentication,
protected-reader peer checks, storage/backup binding and migration regressions.

Checks use one Cargo job and one test thread, repository-disk scratch and the
unchanged 3-GiB host floor. Before compilation, the unused backup-staging debug
cache was cleared through Cargo; release artifacts, source and evidence were
retained. No production Pi, package/service activation or push was performed.

## Remaining integration

Connect a genuine protected fresh health collector, including the frozen
restart-policy, healthcheck, SMART, required-mount and protection rules. The
overview cache lacks required facts and cannot stand in for that collection.
Bind current policy revisions and review stable alert keys against concrete
resource replacement, as recorded in the
[notification inventory](p04-frozen-notification-inventory.md). Default Store
policy does not establish migrated configuration parity.

Then supervise bounded reservation, queued-job collection, publication and
retention in Core; qualify the registered API/UI with genuine production
observations and installed restart/concurrent-tick/resource scenarios. Schedule
metadata continues to report awaiting producer. This bridge does not close B1,
P04 or the P05/P06 cutover gates.

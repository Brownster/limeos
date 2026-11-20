# Long-uptime soak (PR-41)

The acceptance test for the first readiness condition of the
[production readiness plan](plans/2026-11-19-production-readiness-plan.md): no
counter, table or cap turns normal long-running use into a refusal that needs
hand repair. It lives in `bins/core/tests/soak/` and runs months of a household
NAS in minutes against the real stores, plus the states normal traffic never
reaches, built directly.

## What it drives

| Component | How | Time |
|---|---|---|
| Core store (`limeos_persistence::Store`) | Real: opened with `Store::open`, every write through its public methods | `now` passed by the harness, as core passes it |
| Core retention | Real: the writer thread's own `maintain`, with its once-a-minute gate and per-rule error reporting, through `limeos_persistence::test_support::maintain` (the `test-support` feature, enabled only from limeos-core's dev-dependencies, so no shipped binary has it: the CI soak step checks `cargo tree -e normal,build`) | An `Instant` derived from the simulated clock |
| The writer thread's schedule | Real `Database`, writer thread and all (`writer.rs`): it must prune before serving its first call | Wall clock |
| Receipt store (`limeos_executor_container::ReceiptStore`) | Real: opened with `open_at`; every request answered by a handler that mirrors the container arms of `bins/executor/src/main.rs` | The store's own `clock`/`now` parameters |
| `metrics.sqlite` (`limeos_observations::telemetry`) | Real `Telemetry`, writer thread and all (`metrics.rs`) | Sample times from the simulated clock |
| Docker Engine | Simulated container state behind the receipt store's `Engine` trait (`engine.rs`): the reference host's 18 containers in four Compose stacks | Simulated clock |
| Core's dispatcher, recovery and resolution loops | Mirrored step for step in `dispatch.rs`, with core's `Rotation` on the simulated clock | `limeos-core` is a binary whose loops read `SystemTime` and `Instant`; calling them needs production seams planned for a later round |

The only production change is that seam: `Store::prune`, made public for the
first soak, is test-only again, and `retention.rs` exposes, behind
`test-support`, the writer's `maintain(now)` with the rows each rule's batch
removed, the rules' budgets and batch limits, and the per-event audit size.

## The tests

| Test | What | Run time (debug, x86-64) |
|---|---|---|
| `soak_smoke` | Three simulated days of the household; every `cargo test` | 2 s |
| `soak_ninety_days` | The acceptance run: 90 days; core restarts (nightly and at awkward moments) only in the first 30, then 60 days on one open store, so `Store::open` never recomputes a drifting counter; startup measured nightly on copies | 190 s |
| `soak_ninety_days_restarts` | Alongside it, the recovery coverage: the same household with another seed, both daemons restarting every night and at every awkward moment for all 90 days | 203 s |
| `soak_core_store_at_its_audit_cap` | The core store at, half-way into and at the end of the reserve past its 64 MiB audit cap, with and without prunable history, with and without a running job needing a recovery event (12 cases) | 15 s |
| `soak_core_retention_works_off_a_backlog` | Retention suspended during a burst of real logins, then resumed: twelve batches of backlog; every `cargo test` | 2 s |
| `soak_receipt_store_at_its_caps` | The receipt store at its 8,192-row cap and its 4,096-page cap, with prunable and unprunable rows, and past its 16 MiB file check; every `cargo test` | 2 s |
| `soak_occurrences_stay_at_their_budget` | One observe-health schedule at the 30 s minimum settled through the real reservation API for 30 days, then a day with retention suspended | 60 s |
| `soak_metrics_store_ninety_days` | `metrics.sqlite` at the collector's cadence for 90 days | 20 s |
| `writer_thread_runs_retention_unprompted` | The production retention schedule; every `cargo test` | 1 s |
| `automation_limit_at_minimum_interval` | The known automation limit, measured; not a gate (about 14 minutes) | — |

The CI step runs every ignored test named `soak_*` together (3 min 23 s on this
machine, under its own 10-minute limit); the others run with `cargo test`.

Why separate tests for the caps rather than phases of the 90-day run: each
state is built in one step (rows written with SQL where only their number
matters), so a failure names the one state that broke, and the 90-day run
keeps its household shape and its run time.

## Workload

Rates are in `workload.rs`, with their reasons. Normal days:

| What | Rate |
|---|---|
| Logins (2 administrators) | 8 a day; 40% log out, the rest expire after 8 h |
| Failed logins | 3 wrong passwords and 1 unknown name a day: the account read (`login_record`) only. The refusal is the password workers' (core's `login_checked`), not driven, so it is not counted as coverage |
| Dashboard and job-progress polls | every 30 s for 15 min after a login; every 2 s for 30 s after a job is queued |
| Container proposals | 10 a day (restart 85%, stop 15%, start a stopped one); 8% expire unapproved, 3% canceled, 2% approved too late, 2% never queued |
| Stack actions | 2 a day over four stacks |
| Faults per dispatched job | about 6%: execute answer lost; executor dead with the execute unread; executor stopped between `ReceiptEpoch` and the execute; executor killed inside the Engine call; Engine error before or after acting; core restart after the claim, after the execute answer, after the executor's verify (core restarts only while the scenario allows them); recovery lookup lost |
| Operator reviews | four a day; every `needs_intervention` job resolved on fresh evidence (10% of decisions interrupted by a core restart, while allowed) |
| Containers restarted outside LimeOS | 2 a day |
| Restarts | the executor every night at 04:00; core too while the scenario allows it |

Nights are quiet: the daily checkpoint runs at 04:00 and the weekly idle hour at
02:00. Two other writers are not quiet and are driven on their own:
`metrics.sqlite` writes a flush every five minutes, idle or not (one
transaction, with an unbatched `DELETE` of rows past its 31-day history), and
the core writer thread runs retention once a minute. The password workers and
the login limiter keep no state on disk.

At household rates no budget fills within 90 days, so two bursts, each within
what the code admits, saturate the tables early and the last two months run
against full tables:

- Day 20, 06:00 to 16:00: a runaway client logs in and out twice a second (the
  two password workers cap logins near four a second on a Pi): 144,000 events.
- Day 25, 08:00 to 20:00: a scripted client proposes a container action every
  40 s and a stack action every 45 s.

## Measuring startup

Every night, both stores are copied with SQLite's online backup API from a
read-only side connection (page for page, without writing or checkpointing the
live store) and each copy is opened by a child process of the test binary,
alone. The unit is SQLite page reads: one `pread` of one page per fetch that
misses the open's page cache, counted from the child's `/proc/thread-self/io`.
For a given file the count is the same on every run, on tmpfs or disk, in
debug or release. The child is needed because the bundled SQLite is built with
`SQLITE_ENABLE_MEMORY_MANAGEMENT`: every connection of a process shares one
page-cache LRU, so in the test process the live store's connection and the
parallel tests would decide what the open finds cached (measured that way,
two nights' nearly equal files differed by up to four times). Wall time and
bytes read are reported as information only.

Before the open, the child runs each of startup's known statements
(`startup::KNOWN`, copied verbatim from `crates/persistence/src`) cold, on its
own connection. A statement read cold reads at least as many pages as inside
the open, where the cache is shared, so the open reads more than their sum only
by running something else: an extra scan, or a query whose index went missing.

Rejected units: a copy whose header shrinks the page cache to its floor counts
every fetch, but the count jumps by up to four times between two nights; and
SQLite's own counters (`sqlite3_db_status`, `sqlite3_stmt_status`, a progress
handler) belong to a connection that `Store::open` keeps private. Reaching it
needs `unsafe` (forbidden in the workspace) or a production seam: a
`test-support` hook on the store's connection before startup runs. With it,
rescans of tables smaller than the 2 MiB cache, invisible to page reads, could
be counted too.

## Results

Measured on 20 November 2026 (debug build, x86-64, tmpfs, both scenarios side
by side). Job, fault and refusal counts vary by a few between runs (see
"Reproducibility").

| Over 90 days | `soak_ninety_days` | `soak_ninety_days_restarts` |
|---|---|---|
| Jobs | 2,328 | 2,132 |
| Events written, pruned, peak | about 159,500; 28,382 pruned; peak 131,313 against the 131,072 budget; largest batch 241 | 27,223 pruned; peak 131,319; largest batch 247 |
| Executor | 2,184 executes; 1,735 action and 256 stack rows kept | 1,997 executes; 1,879 and 256 kept |
| Faults | 126 injected (and 181 container restarts outside LimeOS) | 124 injected (and 185) |
| Restarts | core: 30 nightly and 30 at awkward moments, all before day 30; executor every night | core and executor every night, and 35 core restarts at awkward moments |
| Accounting | the counter equal to its definition at every checkpoint and restart; events never below their budget once reached | the same |
| Idle hours | 12 scheduled, 12 measured, no write | the same |

| Day | core.sqlite (WAL) | events | jobs | receipts.sqlite | core open on a copy: page reads (proof check) | receipts open |
|---:|---:|---:|---:|---:|---:|---:|
| 9 | 1.1 MiB (1.1) | 729 | 96 | 0.19 MiB | 285 | 57 |
| 27 | 31.2 MiB (1.1) | 131,072 | 1,664 | 2.06 MiB | 20,331 | 541 |
| 36 | 31.8 MiB (1.1) | 131,072 | 1,766 | 2.06 MiB | 60,088 (the proof check from here) | 549 |
| 63 | 33.4 MiB (1.1) | 131,072 | 2,038 | 2.09 MiB | 78,184 | 589 |
| 89 | 35.1 MiB (1.1) | 131,072 | 2,318 | 2.17 MiB | 91,087 (356 MiB read, 10 times the file) | 643 |

Every bounded table stayed flat over the last third; the open never read more
than its known statements cold plus 2.2%. Separately: the core store opens,
recovers, keeps revocation and prunes back to its budget in all 12 cap cases
(the one documented refusal leaves the store unchanged); a 12-batch backlog
goes in 12 passes of exactly 1,024; the receipt store at its row and page caps
prunes at most 64 rows an insert and refuses only while nothing is prunable;
occurrences stay at exactly 1,024 for 30 days and a backlog of 3,001 goes in 6
passes of at most 512; `metrics.sqlite` holds 44,641 rows in 0.78 MiB and writes
about 113 KiB an hour, flat.

### Defect: core.sqlite has no size bound

The events table is flat at its budget, but the per-job tables (`jobs`,
`job_resources`, `container_plans`, `container_results`, `stack_plans`,
`stack_results`, `job_resolutions`) are never pruned: 4.7 to 4.8 KiB per job,
50 KiB a day at household rates; core.sqlite passes its 80 MiB bound in about
2.5 years. Startup grows with them: every new per-job page is read by
`quick_check` and the foreign-key check, and six startup statements scan the
whole jobs table, which is now larger than SQLite's 2 MiB page cache, so the
known startup statements read 5.0 pages more for every page the per-job
tables gain (61 to 64 a night), reaching the startup bound of a store at its
file bound in about 2.1 years. No refusal follows (no cap
counts these rows), but neither size nor startup is bounded.

### Defect: the storage-mount proof check reads the events table once per job

Found by the stronger startup measurement. `storage_mounts::check` runs, on
every start:

```sql
SELECT EXISTS(SELECT 1 FROM jobs j WHERE json_extract(j.intent,'$.operation')='storage_mount'
  AND (j.state='succeeded' OR (j.state='precondition_changed' AND EXISTS(
    SELECT 1 FROM events e WHERE e.job=j.id AND json_extract(e.kind,'$.kind')='job_transition'
      AND json_extract(e.kind,'$.state')='running')))
  AND NOT EXISTS(SELECT 1 FROM storage_mount_results r WHERE r.job=j.id))
```

SQLite plans it as a multi-index OR over `jobs_state`, and for every
`precondition_changed` job evaluates the subquery before the storage-mount
filter, with the events looked up by kind (`SEARCH e USING INDEX events_kind`)
instead of by job (`events_job`): every `job_transition` event, once per such
job, on a store with no storage mount at all. While that index range fits in
the page cache it costs little; once retention has replaced the burst's session
events with job history it does not. Measured alone on copies: from
day 36 in the main scenario (day 72 in the restart scenario) this one statement
reads up to 65,499 pages, 7.3 files' worth, and the open 91,087 pages for an
8,990-page file on day 89; without it the open reads 2.4 times the file. Needs the subquery to search
`events_job` (or the operation filtered first). Pinned as an expected failure
by its plan.

### The automation limit, measured

`automation_limit_at_minimum_interval` observes every occurrence of one
observe-health schedule at the 30 s minimum through the store's real paths,
with the writer's retention pass once a simulated minute. Measured on 20
November 2026: the first refused write comes after **22.76 simulated days**
(65,535 observations): the reservation refused with `StateNotDurable` once the
events table reaches the 64 MiB audit cap (262,143 events, almost all of them
pinned automation proofs), with `automation_observation_results` one row short
of its 65,536 cap and core.sqlite at 202.6 MiB. The test now asserts exactly
that refusal (the reservation, `StateNotDurable`, at the audit cap, with the
results under their cap and the occurrences at their budget) between days 22
and 23.5, so a different limit reached earlier fails it. It takes about 14
minutes and up to 210 MiB of disk, so it is not part of CI.

## Assertions

| Check | What it catches |
|---|---|
| no lifetime or unexpected refusal | `StateNotDurable`, `Overloaded`, `BeyondReceiptHistory` from any store call or executor answer, a failed retention rule, and any refusal a call site does not list as legitimate |
| provoked refusals observed | an approval after expiry and a session used after expiry, each refused as such by the store |
| audit counter and retained rows match what retention promises | at every checkpoint and before every restart: `meta.audit_bytes` is exactly `EVENT_BYTES` (the store's constant) per event row; once the run has written as many events as the events rule keeps, at least that many remain; standalone and stack receipts never fall below what retention keeps once reached; the prune watermark never decreases and never passes the youngest expiry retention may prune |
| every retention batch within its rule's limit | every rule's batch, one transaction, removes at most its limit from the production rules |
| database files under their bounds | core file 80 MiB and WAL 8 MiB; receipt file 8 MiB and WAL 2 MiB, every checkpoint |
| every table within its budget | every table in both stores, found by walking `sqlite_master`; the events budget from the production rule |
| sizes and bounded tables flat across the last third | the bounded tables' bytes and rows, and the file size beyond what per-job tables added, over days 60 to 90: no table outside the per-job ones may grow |
| slow growth leaves at least five years | the per-job tables' growth, projected to the core file bound (expected failure, pinned) |
| startup under its bounds | every open's wall time (5 s core, 2 s receipts: the catastrophic only); on every copy, the page reads of core's known statements but the proof check at most 3.2 per page of file, the receipt store's open at most 2.0 |
| core startup reads nothing its checks do not explain | every core open on a copy reads at most the cold sum of its known statements, plus 256 pages and 1% of the file |
| the storage-mount proof check reads little without storage mounts | its plan does not look events up by kind, and it reads at most an eighth of the file (expected failure, pinned) |
| core startup work grows only with the per-job tables | over the last third, the known statements' page reads grow by at most 10 per page the per-job tables gain (measured 5.0) |
| core / receipt-store startup work leaves at least five years | the nightly page reads, projected to the bound of a store at its file bound (core: expected failure, pinned) |
| an idle hour writes nothing | once retention has nothing left, an hour of dispatcher and retention passes makes no write call; every scheduled idle hour is measured or skipped for a recorded reason, and at least three in four are measured |
| locks, ledger and retained history consistent after every restart | locks match job states; every job the workload queued exists and no other does; unsettled and contested jobs keep their history; the executor holds no lock for a job core has released |
| every job settled; none lost, duplicated or stranded | after a drain, every job is terminal and no resolution request is open |
| every Engine effect ran at most once and matches its job | per action |
| authority events all kept | every authority event the run wrote is still there |

## Expected failures, pinned

`EXPECTED_FAILURES` in `checks.rs` lists each known failure with the measures
it must show; a failure with any measure outside its range fails the run, and
so does a listed check that passes.

| Check | Pinned to | Measured (main / restarts) |
|---|---|---|
| slow growth leaves at least five years | per-job growth 25 to 100 KiB a day; 2.5 to 10 KiB per job; 1.25 to 5 years to the file bound | 50.5 / 49.7 KiB a day; 4.7 / 4.8 KiB; 2.50 / 2.56 years |
| core startup work leaves at least five years | the known statements but the proof check grow 32 to 128 page reads a night; 1 to 4.5 years to the bound | 64 / 61 a night; 2.02 / 2.12 years |
| the storage-mount proof check reads little without storage mounts | its plan looks events up by kind on every copy; its largest read 0 to 15 files' worth | by kind on every copy; 7.3 / 6.0 files' worth |

The sound parts are asserted without exemption: every table but the per-job
ones flat, the open explained by its known statements, the known statements'
growth explained by the per-job tables.

## Mutation checks

Each past lifetime defect, reintroduced in the production source (never
committed), and the check that now fails:

| Past defect | Mutation | Failing test and check |
|---|---|---|
| Audit counter that only grows | `prune_events` no longer calls `account(tx, -removed)` | `soak_ninety_days`, accounting: "day 21 04:00: meta.audit_bytes is 37288704, but 13 events take 3328: the audit counter drifted by 145646 events" and "13 events kept of 145659 written, below the 131072 the events rule keeps: retention over-pruned" |
| Receipt rows never pruned | the receipt store's `prune` returns at once | `soak_receipt_store_at_its_caps`: "row cap, prunable: job 0 ended needs_intervention with 8192 rows" (and the 90-day budgets and flatness, as before) |
| Occurrences capped with no pruner | `prune_occurrences` removes nothing | `soak_occurrences_stay_at_their_budget`: "day 0.36: a pass removed 0 of 1025 occurrences (budget 1024, batch 512)" |
| Missing index: one extra linear scan at startup | the audit reconciliation's count reads `events NOT INDEXED` | `soak_ninety_days`, core startup reads nothing its checks do not explain: "day 21.17: 13856 page reads, 891 more than its known startup statements read cold" |
| Prune batch with no limit | `run` passes the whole excess instead of `excess.min(rule.batch)` | `soak_core_retention_works_off_a_backlog`: "pass 1 removed 12290 events with 12290 over budget: each transaction removes one batch of at most 1024" (and the batch check of the 90-day runs) |
| Store refuses to open at its cap | `Store::open` refuses when `audit_bytes >= AUDIT_LIMIT` | `soak_core_store_at_its_audit_cap`: "AtCap, prunable history, no running job: the store refused to open: Error(StateNotDurable)", in all 11 cases that must open |
| The writer never calls maintenance | both `maintain(&mut store)` calls of the writer thread removed | `writer_thread_runs_retention_unprompted`: "the writer thread ran no retention pass on its own: 134146 events before it started, 134146 when it served its first call" |

## Reproducibility

The workload is fixed by a seeded SplitMix64 generator and nothing sleeps, but
a run is not bit-for-bit repeatable. The stores draw random identifiers
(`limeos_identity::opaque`) for jobs, plans, sessions and receipt-store epochs,
so B-tree layouts differ between runs, and the dispatcher's candidate listings
and startup recovery visit jobs in identifier order, so recovery order differs
too. Values still taken from the wall clock: `Store::open`'s timestamp on
recovery events, the receipt store's tokio timeouts around Engine calls (never
reached with the simulated Engine), the legacy-expiry back-fill of a receipt
store upgraded from schema 5 (never run here), `metrics.sqlite` query
timeouts, and the deadline (a minute ahead) each automation reservation must
meet. Counts of jobs, faults and refusals therefore vary by a few between
runs, and page counts by a few percent. The thresholds leave room for that:
budgets have a batch of headroom, flatness tolerates 2% or 256 KiB, the startup
factor 3.2 sits above the 2.4 measured, and the pins span a factor of two
around today's values.

## Running it

```sh
# Smoke and the fast cap tests: part of every cargo test
cargo test -p limeos-core --test soak

# Everything the CI soak step runs
cargo test -p limeos-core --test soak -- --ignored soak_ --nocapture

# One scenario
cargo test -p limeos-core --test soak -- --ignored --exact soak_ninety_days --nocapture

# The automation limit, measured (not a gate)
cargo test -p limeos-core --test soak -- --ignored --exact automation::automation_limit_at_minimum_interval --nocapture
```

The stores go in `LIMEOS_SOAK_DIR` when set; otherwise on `/dev/shm` when it
has at least 1 GiB free (a GitHub-hosted runner), else in the system temp
directory (a container's 64 MiB `/dev/shm`); the report names the place. Every
check counts work, not time, so the place changes the run time only. Peak disk
use is about 300 MiB with every test at once; the automation measurement needs
about 210 MiB. Per-thread I/O accounting (`/proc/thread-self/io`) is required:
startup page reads are counted with it.

## Reading a failure

A violation is printed when it happens, as `[profile] check: day D HH:MM:SS:
what`, and the run continues. At the end the report shows the job outcomes,
faults, restarts, legitimate refusals, retention removed and the largest batch
per rule, row counts and sizes every ten days, startup page reads on copies,
the measures the pins read, and one line per check: `PASS`, `FAIL` (with its
first 20 failures) or `XFAIL` (a known defect, within its pins).

A `has no budget` failure means a table was added: give it a budget in
`core_budget` or `receipt_budget`. A change to core's dispatcher, recovery or
resolution paths must be mirrored in `dispatch.rs`, and a statement added to
the startup checks must be added to `startup::KNOWN`, or the soak no longer
covers it (the explained-startup check fails until it is).

## What it does not cover

- Real hardware, fsync and SD-card latency.
- The real Engine and its HTTP client (covered by `docker/tests.rs`), and the
  executor daemon's socket, peer checks and store mutex.
- Core's own loops (`operations.rs`, `stack_operations.rs`,
  `job_resolution.rs`) and the executor's handlers are mirrored, not executed:
  calling them needs production seams planned for a later round.
- The idle writer's once-a-minute retention pass: the writer test proves the
  pass at startup only; proving the periodic one without a real minute's wait
  needs an injectable retention interval (or clock) for the writer thread.
- Rescans of tables smaller than SQLite's page cache at startup (see
  "Measuring startup"): needs a hook on the store's connection.
- The password check behind failed logins (core's password workers).
- A process killed inside a SQLite transaction: a simulated crash drops the
  process between two steps and its connections close cleanly.
- Automation producers (not in the shipped scope); see the measured limit above.
- Session revocation under a full queue, disk-full, schema upgrades: unit tests.

# Long-uptime soak (PR-41)

The acceptance test for the first readiness condition of the
[production readiness plan](plans/2026-11-19-production-readiness-plan.md): no
counter, table or cap turns normal long-running use into a refusal that needs
hand repair. It lives in `bins/core/src/soak/`, a unit-test module of the core
binary (so it can call core's own code), and runs months of a household NAS in
minutes through core's dispatcher, recovery, resolution and request handlers,
against the real stores and the container executor's own request handling,
plus the states normal traffic never reaches, built directly.

## What it drives

| Component | How | Time |
|---|---|---|
| Core (`Core`) | Real: the dispatcher tick is `Core::dispatcher_pass` (resolution retries, stack and container dispatch and recovery, storage listings); proposals, approvals, queueing, cancellation, progress and dashboard polls and logouts are core's handlers (`propose_action`, `approve_container`, `submit`, `cancel_restart`, `restart_progress`, `observations`, `logout`, and the stack ones); the operator decides through `rpc_resolution_view` and `rpc_resolve` | `now()` and `monotonic()` read the simulated clock on the soak's thread and the store's writer thread (`test_clock`, `cfg(test)` only) |
| Core store (`Database`, `Store`) | Real: opened with `test_support::open`, the writer thread, queues and all; core's code reaches it through `Database::call` | The writer's retention gate reads the simulated clock (`SimClock`, `test-support` only) |
| Core retention | Real: the writer thread's own schedule, a pass at startup and one whenever the clock has passed a minute since the last; the harness only advances the clock, at least once a simulated minute, and judges the passes the writer reports | Simulated |
| Container executor | Real request handling: `limeos_executor::container_requests::answer` behind the ceiling's `validate`, around the real `ReceiptStore` (opened with `open_at`), on a real Unix socket with the protocol's framing; the harness supplies the accept loop and the faults | The store's own `clock`/`now` parameters |
| Docker Engine | Simulated container state behind the `Engine` trait (`engine.rs`): the reference host's 18 containers in four Compose stacks | Simulated |
| `metrics.sqlite` (`limeos_observations::telemetry`) | Real `Telemetry`, writer thread and all (`metrics.rs`) | Sample times from the simulated clock |

One step is not core's own: a login is the session write `login_checked`
makes once the password workers have accepted the password (the password
workers are not driven, so neither are failed logins' refusals). Stack
proposals are core's `propose_stack`: the soak hands core's inventory cache
(`limeos_observations::stacks`) the cohort the storage reader would list from
the simulated Engine, as the reader's proxy does every few seconds, and the
cache judges its 90-second freshness on core's clock (`Cache::with_clock`),
so on simulated time.

### Production seams

None changes what a shipped binary does; every one is either compiled only
for tests or the same code with the system clock passed in.

| Where | What | Why it is neutral |
|---|---|---|
| `bins/core/src/main.rs` | `now()` and a new `monotonic()` read a clock a unit test installed on the calling thread (`test_clock`) | `cfg(test)` only; otherwise the system clocks as before |
| `bins/core/src/operations.rs` | `Rotation::advance`/`due` take the time; the dispatcher loop's body is `dispatcher_pass(&mut Dispatcher)` | Callers pass `monotonic()` where `Instant::now()`/`elapsed()` was read; the loop ticks and calls the same body |
| `crates/persistence/src/writer_clock.rs`, `lib.rs` | The writer thread reads its clock through `WriterTime`; `test_support::open` gives it a `SimClock`; `test_support::on_open` hands `Store::open`'s connection to a hook before startup runs | In a build without `test-support` only `Clock::System` exists: `Instant::now()` and the real `retention::INTERVAL` wait |
| `bins/executor/src/container_requests.rs`, `main.rs` | The container role's receipt-store and Engine arms extracted from the accept loop's closure into `answer`, with `refusal` | Same arms, guards, `try_lock` and `Forbidden` default; the closure keeps the peer check, framing, deadlines and its own reads |
| `crates/executor-container/src/lib.rs` | The row, page and file caps named, and every limit re-exported as `limits` | Same values where the literals were |
| `crates/observations/src/stacks.rs` | The inventory cache reads its freshness clock through `Cache::with_clock`; `Cache::new` passes the system clock | `Cache::new` and `start_proxy` read the system clocks as before |
| `bins/core/src/main.rs`, `job_resolution.rs` | A resolution's detached executor step is spawned through `detach`, which unit tests record so a simulated crash aborts it | `tokio::spawn` as before; the record is `cfg(test)` only |

`test-support` is enabled only from limeos-core's dev-dependencies, as is
rusqlite's `trace`. Three things keep them out of shipped binaries:

- The release step builds the shipped packages with the exact command it
  ships (`cargo build --locked --release -p limeos-core -p limeos-executor
  -p limeosctl`, inside the builder image), reads the features that build
  resolved from its own JSON messages, and fails if `limeos-persistence` has
  `test-support` or `rusqlite` has `trace` (or if either is missing from the
  report). `packaging/build.py` packages those binaries and builds nothing.
- The check job's `cargo tree -e normal,build` step fails if either is in a
  shipped binary's dependency graph, as it does for `deferred`.
- A runtime guard. When the workspace is built with dev targets (`cargo build
  --workspace --all-targets`, clippy, `cargo test`), feature unification does
  compile `test-support` into the debug `limeos-core` and `limeosctl`. Both
  binaries' `main` first calls `limeos_persistence::refuse_test_support()`,
  and from then on `test_support::open` (the simulated writer clock) and
  `test_support::on_open` (the open hook) panic. A unit test's process never
  runs a binary's `main` (the test harness supplies its own), so only tests
  can select them; no environment variable or configuration is involved.

## The tests

| Test | What | Run time (debug, x86-64) |
|---|---|---|
| `soak_smoke` | Three simulated days of the household, both receipt-store replacements and two crash images included; every `cargo test` | 9 s |
| `soak_ninety_days` | The acceptance run: 90 days; core restarts (nightly and at awkward moments) only in the first 30, then 60 days on one open store, so `Store::open` never recomputes a drifting counter; startup measured nightly on copies and every fifth day on crash images | 297 s alone, 448 s beside the others |
| `soak_ninety_days_restarts` | Alongside it, the recovery coverage: the same household with another seed, both daemons restarting every night and at every awkward moment for all 90 days | 288 s alone, 444 s beside the others |
| `soak_core_store_at_its_audit_cap` | The core store at, half-way into and at the end of the reserve past its 64 MiB audit cap, with and without prunable history, with and without a running job needing a recovery event (12 cases) | 15 s |
| `soak_core_retention_works_off_a_backlog` | Retention suspended during a burst of real logins, then resumed: twelve batches of backlog; every `cargo test` | 1 s |
| `soak_receipt_store_at_its_caps` | The receipt store at its row cap and its page cap (`limits`), with prunable and unprunable rows, and past its file check; executes sent by core's own `exchange`; every `cargo test` | 1 s |
| `soak_occurrences_stay_at_their_budget` | One observe-health schedule at the 30 s minimum settled through the real reservation API for 30 days, then a day with retention suspended | 60 s |
| `soak_metrics_store_ninety_days` | `metrics.sqlite` at the collector's cadence for 90 days | 25 s |
| `writer_thread_runs_retention_unprompted` | The writer thread's startup pass, before its first call; every `cargo test` | <1 s |
| `writer_thread_works_off_a_backlog_on_its_own` | The writer thread's periodic passes on simulated time, with nothing but the clock moving; every `cargo test` | <1 s |
| `walks_are_read_from_bytecode_under_any_alias` | The bytecode reader the startup checks rely on: a walk under an alias and `NOT INDEXED`, an index walk told from a table walk, no walk in a seek, an error (not "no walks") for an unreadable program; every `cargo test` | <1 s |
| `automation_limit_at_minimum_interval` | The known automation limit, measured; not a gate (about 14 minutes) | — |

The ordinary `cargo test` runs the smoke, the backlog, receipt-cap and writer
tests, the bytecode reader's and the generator's own tests: 11 to 14 s together (the smoke about 9 s
of it). Every other `soak_*` test is ignored there and run by CI's own `soak`
job, on both native architectures, under a 25-minute limit: 6 min 41 s for all
of them on this machine (8 cores, tmpfs) with the test binary built, the two
90-day runs taking 390 to 450 s each (13 minutes with four of them at once);
one at a time (`--test-threads=1`) they take 11 min 16 s, the 90-day runs
about 290 s each.
The job builds the test binary itself (the workflow keeps no build cache to
share). The packages and the probe are published (uploaded under their final
names, which the VM job downloads) only by the `publish` job, which needs both
the check job and the soak job.

Why separate tests for the caps rather than phases of the 90-day run: each
state is built in one step (rows written with SQL where only their number
matters), so a failure names the one state that broke, and the 90-day run
keeps its household shape and its run time. The cap, backlog, occurrence and
automation tests build a store's state directly and run its retention with
`test_support::maintain`, the writer's own entry point on the caller's clock;
the household runs and the writer tests leave retention to the writer thread.

## Workload

Rates are in `workload.rs`, with their reasons. Normal days:

| What | Rate |
|---|---|
| Logins (2 administrators) | 8 a day; 40% log out (core's `logout`), the rest expire after 8 h |
| Failed logins | 3 wrong passwords and 1 unknown name a day: the account read (`login_record`) only. The refusal is the password workers' (core's `login_checked`), not driven, so it is not counted as coverage |
| Dashboard and job-progress polls | every 30 s for 15 min after a login (`observations`); every 2 s for 30 s after a job is queued (`restart_progress`, `stack_progress`) |
| Container proposals | 10 a day (restart 85%, stop 15%, start a stopped one); 8% expire unapproved, 3% canceled, 2% approved too late, 2% never queued |
| Stack actions | 2 a day over four stacks |
| Faults per dispatched job | about 6%: execute answer lost; executor dead with the execute unread; executor stopped between `ReceiptEpoch` and the execute; executor killed inside the Engine call; Engine error before or after acting; core restart after the claim, after the execute answer, after the executor's verify (core restarts only while the scenario allows them); the execute answer and then the first recovery lookup's answer lost |
| Receipt store replaced | on six days (two in the smoke): the next container job's execute is handled, its answer lost, and the executor comes back with an empty receipt store (days 5 and 22, before the action storm refills it) or the copy of an earlier night (days 12, 33, 47, 55); none in the last third, whose flatness is asserted |
| A resolution held open | once (90-day runs): see below |
| Operator reviews | four a day; every `needs_intervention` job resolved on fresh evidence (10% of decisions interrupted by a core restart between the recorded request and the executor, while allowed) |
| Containers restarted outside LimeOS | 2 a day |
| Restarts | the executor every night at 04:00; core too while the scenario allows it |

Every fault is carried out where it happens. The executor decides per request
from the job it names: an answer it handled and never writes; a request it
reads and dies with; an Engine call it is killed inside (the Engine hangs, the
handler is dropped there, its `prepared` row committed); an epoch it answers
before its socket goes. Core's deaths are signalled by the executor at the
point only it sees (the execute arriving, its answer ready, the verify
answered, the resolution arriving); the harness polls that signal ahead of
core's work, so core's work is never polled again after it, and is dropped
where it stands, and core restarts with fresh state.

Core crashes are crashes. A daily restart is orderly (the writer checkpoints
and truncates its WAL as it stops); every other core restart (at an awkward
moment, or between a recorded resolution and the executor) reopens the store
from its files as they were at the crash point: the database and its WAL as
on disk, without the wal-index, so startup replays every frame committed
since the last checkpoint, as after a kill. Separately, every fifth day from
day 2 at 15:07, and in the middle of the session burst and of the action
storm, startup is measured on such a crash image in a child process (see
"Measuring startup").

A replaced receipt store loses receipts the jobs core holds were executed
under, and every executor start draws a new store epoch anyway. Every job
core holds when the executor starts (dispatched under an earlier epoch) is
recorded, and the executor records every job it answers a recovery lookup for
with a closure. A plan expiring within `PLAN_TTL_SECONDS` of a replacement may
be refused `BeyondReceiptHistory` by the new store (it cannot prove an older
plan's action never ran); only then is that refusal not a lifetime refusal.

The held-open resolution makes retention's rule for open resolution requests
observable, which ordinary traffic almost never does: a job is settled while
its request is open only between a recovery pass and the retry loop's next
supersession, a tick later. On day 19 the first container job queued after
08:00 has its execute answer lost; the executor then refuses its recovery
lookups (`Conflict`, its store busy) and its resolution (`Unavailable`), so
the operator decides it at the next review and the decision stays recorded
and open, while the day-20 session burst makes retention sweep every
prunable event older than it. On day 21 at 10:00 its lookups are answered
again, recovery settles it from its receipt with the request still open, and
core's dispatcher stalls (no pass, so no supersession) while the writer
thread keeps pruning: the soak writes login and logout events until every
event retention would prune before the job's first is gone, then once more.
Its history must still be there; then its resolution is answered and the next
pass supersedes the request.

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
parallel tests would decide what the open finds cached. Wall time and bytes
read are reported as information only.

The page reads are attributed to the statements the open itself executes.
`Store::open` hands its connection to `test_support::on_open` before any
startup statement runs, and the child installs rusqlite's `trace_v2` there:
the reads between two trace events (a statement starting, a statement
finishing) belong to the statement running, the innermost if one runs inside
another, or to none (preparing, the schema). Each statement is known by its
own text as production wrote it, with its VM steps and full-scan steps
(`SQLITE_STMTSTATUS_VM_STEP`, `SQLITE_STMTSTATUS_FULLSCAN_STEP`) summed over
its runs in the open as the difference of its counters between its start and
its end; no production SQL is copied into the soak. Each read-only statement
is then run again alone, cold, on its own connection to a second copy, and
its query plan read there. A statement read cold reads at least as many pages
as inside the open, where one statement's scan can evict another's pages, so
the trend checks fit the cold sums; the explained-startup check holds the
open to them.

Which b-trees a statement walks whole is read from its own bytecode, not from
its plan's text: on the second copy, each cursor its program opens on the
main database (`OpenRead`, `OpenWrite`, `ReopenIdx` with a literal root page)
is named by that root page through `sqlite_schema` (the table, or the index
and its table), and every `Rewind`, `Last` or `Count` on such a cursor is a
walk of that b-tree from one end. Aliases, `NOT INDEXED` and several nodes in
one statement change nothing: `SELECT sum(cursor) FROM events e NOT INDEXED`
walks `events` whatever its plan calls it, and an indexed search elsewhere in
the statement does not hide it. The counters are deterministic for a given
file, unlike page reads, which depend on what the cache holds.

A program that cannot be read fails the check rather than counting as one
that walks nothing. One statement runs inside another: the lock and
resource invariants read `pragma_foreign_key_check`, which prepares and
runs `PRAGMA foreign_key_check` as a statement of its own. SQLite reports a
nested statement's expanded text commented out line by line (`-- PRAGMA
foreign_key_check`), so its program is read from its own text; it is not
run cold, because its reads are already inside the invariants statement's
cold run. Until this round the reader explained that comment, found no
walks and said nothing: the foreign-key check walks every table with a
foreign key, the events table itself included, on every open. It now has
an allowance (`SCAN_ALLOWANCES`) like `quick_check`'s. A store holding a
table walked one step a row (64 rows or more) whose open counted no VM step
or no full-scan step at all fails as unmeasured: that is how a key clash
once read every full-scan count as 0.

A crash image is measured the same way, in a child process: the database and
its WAL copied as they are (no wal-index), opened and traced, with each
statement run cold on a consistent backup taken at the same moment. The WAL
frames it replays are counted from the WAL itself (the frames after its
header that carry its current salts, up to the last that commits a
transaction). Recovery reads them before the first statement runs, outside
every statement. The open must keep every committed transaction, held to
identity and content: before the open, a side connection on the live store
(the same moment, the writer idle) reads every job id and every event (its
cursor, principal, job, kind and time) and the jobs recovery must mark
(`running` or `verifying`); after it, the child reads the reopened store the
same way. Every committed job must be there and no other, every event
through the last committed cursor must be there unchanged (the same count
and digest of those rows), and after that cursor the open may have added
only one `job_recovery_required` event for each interrupted job. Counting
events would not do: those recovery events can make up for committed ones
lost. Nothing legitimately removes a row across this open: `Store::open`
runs no retention rule, job-row retention included (the writer thread runs
those after the open, and the child opens without one), except `make_room`
when the recovery events would not fit within the audit reserve past the
cap; the check refuses such an image outright rather than guess what it
pruned, and the soak never takes one (its events stay at their budget, half
the cap). A test (`a_crash_image_must_keep_every_committed_event_unchanged`)
holds the check to a dropped, a changed and an added event.

On the last night of the main run the open executed 50 statements: 82,213
page reads, 16 of them outside every statement (the same 16 on every copy of
every run: the schema's pages). The largest are the storage-mount proof
check (57,135), `quick_check` (13,703), the foreign-key check (4,938, with
871,762 VM steps, the second most after `quick_check`'s 1.5 million) and the
audit reconciliation's indexed count (988). Their VM steps came to 2,565,818
but the proof check's, 18.2 per retained row (141,339 rows).

## Results

Measured on 20 November 2026 (debug build, x86-64, tmpfs, both scenarios side
by side; three runs each with the same binary, the figures of the first).
Within a scenario the three runs ran the same jobs (2,058 and 1,697) and the
same faults; page counts differed by a few percent.

| Over 90 days | `soak_ninety_days` | `soak_ninety_days_restarts` |
|---|---|---|
| Jobs | 2,058 | 1,697 |
| Events pruned by the writer thread, peak | 26,789 pruned in 126,952 passes; peak 131,319 against the 131,072 budget; largest batch 247 | 24,523; peak 131,316; largest batch 244 |
| Executor | 1,483 container and 473 stack executes; 1,777 action and 256 stack rows kept; 24,276 receipt lookups, 5,791 answered with a closure | 1,172 and 417; 1,814 and 256 kept; 23,183 lookups, 8,779 closures |
| Faults | 14 executor deaths with an execute unread, 7 inside the Engine call, 7 between epoch and execute, 33 execute answers lost and 12 more with the first recovery lookup's, 28 Engine errors, 23 core crashes at awkward moments and 2 interrupting a resolution (2 detached resolution tasks ended with them), 2 receipt stores replaced by an empty one and 4 by an older copy, the held-open resolution (and 188 container restarts outside LimeOS) | 14, 6, 9, 29 and 13, 13, 23 and 5 (5), 2 and 4, the same (and 182) |
| Crashes | 25 core crashes reopened from their crash image, every one with committed WAL frames to replay | 28, likewise |
| Lost answers | 73 execute answers never written, each logged once as `Unavailable` at the execute site; 12 recovery-lookup answers lost, each logged once in its pass | 69; 13 |
| Receipt-store changes | 20 jobs held across an executor start or replacement answered with a closure by the new store; none released on it | 20 |
| Held-open resolution | the job settled with its request open; nothing prunable older than its history, the 3 events pruned after it were others'; its history kept | the same |
| Restarts | core: 30 nightly and 25 at awkward moments, all before day 30; executor every night | core and executor every night, and 28 core crashes |
| Accounting | the counter equal to its definition at every checkpoint and restart; events never below their budget once reached | the same |
| Idle hours | 12 scheduled, 12 measured, no store file written | the same |

| Day | core.sqlite (WAL) | events | jobs | receipts.sqlite | core open on a copy: page reads | receipts open |
|---:|---:|---:|---:|---:|---:|---:|
| 9 | 1.1 MiB (1.1) | 738 | 97 | 0.09 MiB | 286 | 33 |
| 27 | 30.7 MiB (1.0) | 131,072 | 1,423 | 2.28 MiB | 19,904 | 719 |
| 54 | 32.4 MiB (1.1) | 131,072 | 1,696 | 2.31 MiB | 24,016 | 748 |
| 63 | 32.8 MiB (1.1) | 131,072 | 1,777 | 2.34 MiB | 71,375 (the proof check from day 57) | 752 |
| 89 | 34.4 MiB (1.1) | 131,072 | 2,047 | 2.34 MiB | 82,213 (321 MiB read, 9.3 times the file) | 697 |

Startup on crash images: 20 per scenario, each replaying 7 to 262 committed
WAL frames (up to 1.1 MiB of WAL), every one opened and kept every committed
job and event (counted then; held to identity and content since); recovery read 1.0 to 1.6 pages per frame replayed beyond the
16 outside every statement of a clean open, and the opens read no more than
their statements did cold on a consistent copy plus that.

Every bounded table stayed flat over the last third. The cap, backlog,
occurrence and metrics tests are unchanged since last round, which measured:
the core store opens, recovers, keeps revocation and prunes back to its
budget in all 12 cap cases (the one documented refusal leaves the store
unchanged); a 12-batch backlog goes in 12 passes of exactly 1,024, by
`maintain` and by the writer thread on its own; the receipt store at its row
and page caps prunes at most 64 rows an insert and refuses only while
nothing is prunable; occurrences stay at their budget for 30 days and a
backlog of 3,001 goes in 6 passes of at most 512; `metrics.sqlite` holds
44,641 rows (31 days at one a minute, none missing, the oldest at the
retention boundary) in 0.78 MiB, answers every view with the averages of what
was written, and writes about 113 KiB an hour, flat.

### Defect: core.sqlite has no size bound

The events table is flat at its budget, but the per-job tables (`jobs`,
`job_resources`, `container_plans`, `container_results`, `stack_plans`,
`stack_results`, `job_resolutions`) are never pruned: 4.5 to 4.8 KiB per job,
46 to 49 KiB a day at household rates; core.sqlite passes its 80 MiB bound in
2.7 to 2.8 years. Startup grows with them: every new per-job page is read by
`quick_check` and the foreign-key check, and six startup statements scan the
whole jobs table, which is now larger than SQLite's 2 MiB page cache, so the
startup statements read 5.1 to 5.4 pages more for every page the per-job
tables gain (57 to 64 a night), reaching the startup bound of a store at its
file bound in 2.1 to 2.3 years. The bytecode shows the walks: `quick_check`
walks every table, the foreign-key check every table with a foreign key
(the events table and the seven per-job tables among them; neither makes
full-scan steps), and seven statements walk `jobs` (and two of them
`job_resources` or `job_resolutions`) with one full-scan step per row, 2,046
to 6,085 steps each on the last night. No refusal follows (no cap counts
these rows), but neither size nor startup is bounded.

### Defect: the storage-mount proof check reads the events table once per job

`storage_mounts::check`'s proof query (the startup statement that reads
`storage_mount_results`, found in the trace by that text) is planned as a
multi-index OR over `jobs_state`, and for every `precondition_changed` job
evaluates its subquery before the storage-mount filter, with the events looked
up by kind (`SEARCH e USING INDEX events_kind`) instead of by job
(`events_job`): every `job_transition` event, once per such job, on a store
with no storage mount at all. While that index range fits in the page cache it
costs little; once retention has replaced the burst's session events with job
history it does not. Measured in the open on the last night: 57,135 page reads
of 82,213 (3.2 million VM steps, against 2.6 million for every other startup
statement together), 6.5 files' worth cold (5.8 in the restart scenario),
from day 57 in the main scenario and day 89 in the other (days 72 and 72 to
81 last round). Needs the subquery to search `events_job` (or the
operation filtered first). Pinned as an expected failure by that statement's
plan and cost.

### Fixed: a lost executor answer read as the caller's bad input

Found by driving core's real `exchange` over the socket, pinned last round,
fixed now. `limeos_contracts::read_frame` mapped every failure to read a
frame, the end of the stream included, to `InvalidInput`: an executor that
closed a connection unanswered (it died, or the answer was lost) was logged
by core as "execute call failed" with the wrong code, and an HTTP caller on
the same path would have been told a transient failure was its own error
(`retry: false`). A connection that ends or fails before the whole frame
arrived is now `Unavailable`; a frame that arrived malformed (zero or
oversized length, a body that is not the expected JSON) is still
`InvalidInput`. With the fix the pin passed, which failed the run as
designed; it is gone, and the lost answers are now counted exactly
(`every lost executor answer is logged once, as unavailable`): 73 and 69
execute answers never written and 12 and 13 recovery lookups lost, each
logged once with `Unavailable`, and no `InvalidInput` anywhere.

### The automation limit, measured

`automation_limit_at_minimum_interval` observes every occurrence of one
observe-health schedule at the 30 s minimum through the store's real paths,
with the writer's retention pass once a simulated minute. Measured on 20
November 2026: the first refused write comes after **22.76 simulated days**
(65,535 observations): the reservation refused with `StateNotDurable` once the
events table reaches the 64 MiB audit cap (262,143 events, almost all of them
pinned automation proofs), with `automation_observation_results` one row short
of its 65,536 cap and core.sqlite at 202.6 MiB. The test asserts exactly that
refusal between days 22 and 23.5, so a different limit reached earlier fails
it. It takes about 14 minutes and up to 210 MiB of disk, so it is not part of
CI (not re-measured this round).

## Assertions

| Check | What it catches |
|---|---|
| no lifetime or unexpected refusal | `StateNotDurable`, `Overloaded`, `BeyondReceiptHistory` (but within a plan's lifetime after a receipt-store replacement) from any store call or executor answer or in anything core logs, a failed retention rule, and any refusal a call site does not list as legitimate |
| core logs only the warnings it is expected to | every warning or error core logs must match an entry of `EXPECTED_LOGS` (its exact message, level and codes, with the reason the soak provokes it): the dispatcher's two "remains pending" lines (`Conflict`, `Unavailable`), the execute that failed after it was written (`Unavailable` only), the execute never sent, the execute refused before any row after a replacement, and the held-open resolution that stays pending (`Unavailable`). Nothing may carry `InvalidInput` |
| every lost executor answer is logged once, as unavailable | every execute answer the executor never wrote (lost, or the process dead with the request unread or inside the Engine call) is logged by core exactly once, at its execute site, with `Unavailable`, and every other code there is a refusal the executor answered, one for one; every lost container recovery lookup is logged as that job's failure with `Unavailable` in the same dispatcher pass, one for one, whenever the executor ran throughout the pass (a lost stack lookup is `release_if_unsent`'s, which holds the job without a log, and the stack site logs none); every executes refused before any row matches its log line |
| no job dispatched before a receipt-store change is released on a closure alone | every job core held when the executor started (a new store epoch; with a replaced store, receipts lost) and that the new store answered with a closure ends in any state but `precondition_changed`; the replacements happened and such jobs were seen |
| a job's history stays while its resolution request is open | the held-open job, settled with its request open, keeps its `job_queued` event while retention prunes every older prunable event and once more; the window must be decisive (nothing prunable older than the job left, something pruned) |
| provoked refusals observed | an approval after expiry and a session used after expiry, each refused as such |
| audit counter and retained rows match what retention promises | at every checkpoint and before every restart: `meta.audit_bytes` is exactly `EVENT_BYTES` per event row; once the run has written as many events as the events rule keeps, at least that many remain; standalone and stack receipts never fall below what retention keeps once reached; the prune watermark never decreases and never passes the youngest expiry retention may prune |
| every retention batch within its rule's limit | every batch the writer thread reports, one transaction, removes at most its rule's limit |
| database files under their bounds | core file 80 MiB and WAL 8 MiB; receipt file half its page cap and WAL 2 MiB, every checkpoint |
| every table within its budget | every table in both stores, found by walking `sqlite_master`; the events budget from the production rule, the receipt budgets from `limits` |
| sizes and bounded tables flat across the last third | the bounded tables' bytes and rows, and the file size beyond what per-job tables added, over days 60 to 90 |
| slow growth leaves at least five years | the per-job tables' growth, projected to the core file bound (expected failure, pinned) |
| startup under its bounds | every open's wall time (5 s core, 2 s receipts: the catastrophic only); on every copy and crash image, the cold page reads of core's startup statements but the proof check at most 3.2 per page of file and WAL, the receipt store's open at most 2.0; every crash image opens and keeps every committed job and event unchanged (ids, and each event's cursor, principal, job, kind and time), adding only one recovery event per interrupted job |
| core startup reads nothing its checks do not explain | on every copy and crash image, from every statement's bytecode and counters: every statement's program read, nested ones included; no statement walks the events table itself (only `quick_check` and the foreign-key check may); only the statements in `SCAN_ALLOWANCES` walk a table growing with history (`events` or a per-job table, or an index of one), each only its listed tables, with a reason; each statement's full-scan steps at most its listed tables' rows times its steps per row (0 or 1), plus its walks of bounded tables (their rows, once per run), plus 16; the counters measured (some VM and full-scan steps once a table walked one step a row holds 64 rows); the open's VM steps but the proof check's at most 64 per retained row plus 200,000; at most 32 page reads outside every statement (measured 16) plus 2 per WAL frame replayed; the open reads at most what its statements read cold, plus 3%, 256 pages and 2 per WAL frame replayed |
| the storage-mount proof check reads little without storage mounts | the statement that reads `storage_mount_results`: its plan does not look events up by kind, and it reads at most an eighth of the file cold (expected failure, pinned) |
| core startup work grows only with the per-job tables | over the last third, the statements' cold reads (but the proof check's) grow by at most 10 per page the per-job tables gain |
| core / receipt-store startup work leaves at least five years | the nightly page reads, projected to the bound of a store at its file bound (core: expected failure, pinned); the receipt store's no further than the reads of a store with every table at its budget |
| an idle hour writes nothing | once retention has nothing left, an hour of dispatcher ticks and the writer's own passes changes no file of either store (database or WAL); every scheduled idle hour is measured or skipped for a recorded reason, and at least three in four are measured |
| locks, ledger and retained history consistent after every restart | locks match job states; every job the workload queued exists and no other does; unsettled and contested jobs keep their history; the executor holds no lock for a job core has released |
| every job settled; none lost, duplicated or stranded | after a drain, every job is terminal and no resolution request is open |
| every Engine effect ran at most once and matches its job | per action, counted by the executor |
| authority events all kept | retention never removed one, and their count never went down between checkpoints |
| recovery closes every absent action it asks about | every receipt lookup core sends asks to close an absent action, and every absence the executor answers is closed |

## Expected failures, pinned

`EXPECTED_FAILURES` in `checks.rs` lists each known failure with the measures
it must show; a failure with any measure outside its range fails the run, and
so does a listed check that passes.

Each pin is the range a measure took over three runs of each 90-day
scenario with the same binary (six values), widened by that range on each
side: a small multiple of the run-to-run spread, so a defect that grows
faster or slower than today's fails as not the known one. Doubled per-job
growth (about 95 KiB a day) is outside its pin; so is a startup series
growing 75 pages a night.

| Check | Pinned to | Measured, three runs (main; restarts) |
|---|---|---|
| slow growth leaves at least five years | per-job growth 42 to 53 KiB a day; 4.1 to 5.2 KiB per job; 2.6 to 2.9 years to the file bound | 47.2, 45.8, 46.5; 49.4, 49.0, 49.0 KiB a day (range 3.6); 4.59, 4.46, 4.52; 4.81, 4.77, 4.77 KiB; 2.71, 2.80, 2.76; 2.74, 2.74, 2.76 years (last round 48.0 / 48.7; 4.7 / 4.8; 2.77 / 2.75; before it 25 to 100, 2.5 to 10, 1.25 to 5) |
| core startup work leaves at least five years | the startup statements but the proof check grow 50 to 72 page reads a night; 1.9 to 2.45 years to the bound | 57.7, 57.5, 57.3; 63.5, 64.5, 63.6 a night (range 7.2); 2.24, 2.25, 2.26; 2.14, 2.10, 2.14 years (before: 32 to 128, 1 to 4.5) |
| the storage-mount proof check reads little without storage mounts | the statement reading `storage_mount_results` looks events up by kind on every copy; its largest cold read 4.5 to 8 files' worth | by kind on every copy; 6.49, 6.51, 6.51; 5.82, 5.75, 5.83 files' worth (before: 0 to 15) |

After the merge of the security batch (PR-32 to PR-35) and the bytecode
reader's fixes, one more run of each scenario, one at a time, ran the same
jobs (2,058 and 1,697) and measured 46.2 / 49.2 KiB a day, 4.50 / 4.79 KiB
a job, 2.77 / 2.74 years; 59.0 / 65.1 page reads a night, 2.19 / 2.09
years; 6.49 / 5.81 files' worth: all inside their pins, every other check
passing.

The proof check's pin is the measured range widened by itself (5.0 to 7.3)
and up to 8: its read jumps once retention has replaced the burst's session
events with job history, here on day 57 in one scenario and day 89 in the
other, and the largest of any earlier run was 7.3. One development run with
another schedule of receipt-store replacements had not reached the jump by
day 90 (0.27); with this schedule all six runs had, but a change to the
workload that delays it past day 90 fails this pin, and the pin must then be
measured again. The lost-answer pin is gone with its defect.

## Mutation checks

Each past lifetime defect, and a regression in each real path the soak now
runs, reintroduced in the production source (never committed; restored after
each run, `git status` clean), and the check that fails. This round re-ran
the first scan row (now an aliased `NOT INDEXED` walk) and ran the last two
rows; the others are last round's, against checks this round only made
stricter. After the merge of the security batch the scan row and the epoch
row were run again against `soak_smoke` and failed with the same checks
and messages (the scan row: "Rewind on events; 1285 full-scan steps", "85
full-scan steps, more than the 16 allowed it"; the epoch row: 3 jobs
released on a closure from the new store); the retention row was not run
again:

| Defect or regression | Mutation | Failing test and check |
|---|---|---|
| Audit counter that only grows | `prune_events` no longer calls `account(tx, -removed)` | `soak_ninety_days`, accounting: "day 21 04:00:00: daily checkpoint: meta.audit_bytes is 37288704, but 13 events take 3328 (256 each): the audit counter drifted by 145646 events" and "13 events kept of 145659 written, below the 131072 the events rule keeps: retention over-pruned by 131059" |
| Receipt rows never pruned | the receipt store's `prune` returns at once | `soak_receipt_store_at_its_caps`: "row cap, prunable: job 0 ended needs_intervention with 8192 rows" (and "row cap, aged rows: the execute was still refused: Overloaded", the file cap refusing every execute) |
| Occurrences capped with no pruner | `prune_occurrences` removes nothing | `soak_occurrences_stay_at_their_budget`: "day 0.36: a pass removed 0 of 1025 occurrences (budget 1024, batch 512)" |
| Missing index: one extra linear scan at startup | the audit reconciliation's count gains `+(SELECT 0*count(e.created) FROM events e NOT INDEXED)` (an aliased walk, whose plan reads `SCAN e`, in a statement that keeps its allowed indexed count) | `soak_smoke` (and every run), core startup reads nothing its checks do not explain: "a startup statement walks the events table itself (Rewind on events; 1285 full-scan steps, 0 page reads in the open): SELECT audit_bytes,(SELECT count(*) FROM events)+(SELECT 0*count(e.created) FROM events e NOT INDEXE…" and "a startup statement made 85 full-scan steps, more than the 16 allowed it (0 for tables growing with history, 0 for bounded tables)", on every copy and crash image |
| Prune batch with no limit | `run` passes the whole excess instead of `excess.min(rule.batch)` | `soak_core_retention_works_off_a_backlog`: "pass 1 removed 12290 events with 12290 over budget: each transaction removes one batch of at most 1024" |
| Store refuses to open at its cap | `Store::open` refuses when the reconciled `audit_bytes >= AUDIT_LIMIT` | `soak_core_store_at_its_audit_cap`: "AtCap, prunable history, no running job: the store refused to open: Error(StateNotDurable)", in every case that must open |
| The writer never prunes at startup | the writer thread's `maintain` before its loop removed (the loop's kept) | `writer_thread_runs_retention_unprompted`: "the writer thread ran no retention pass on its own before its first call: 134146 events before it started, 134146 when it served that call; one batch of 1024 was due" |
| The writer never prunes again | the `maintain` in the writer thread's loop removed (the startup one kept) | `writer_thread_works_off_a_backlog_on_its_own`: "after 1 simulated minutes the writer thread removed 0 events with 11266 over budget: it runs one pass of at most 1024 each minute on its own"; and `soak_ninety_days`, every table within its budget: "day 21 04:00:00: core.events: 145659 rows over its budget of 132096", and again from day 43, once core stops restarting (each restart's startup pass removes one batch) |
| Recovery releases a job without asking the executor (core's dispatcher) | `dispatch_one`'s recovery skips the `RestartReceipt` lookup and acts on a closure it makes up (a `precondition_changed` receipt, `closed`) | `soak_smoke`: locks, ledger and retained history: "executor crash with an execute unread: the executor holds a lock for job eb9f9e21d15b that core has as Some(\"precondition_changed\")"; every Engine effect: "job eb9f9e21d15b (Container Restart, fault Some(AnswerLost)) is precondition_changed after 1 Engine effects for 1 targets" |
| The executor skips the closure on an absent receipt (its request handling) | `container_requests::answer` passes `false` for `close_if_absent` to both receipt lookups | `soak_smoke`, recovery closes every absent action it asks about: "the lookup for job 753c9591dbb3 answered Some(NeverRecorded) without closing the action" (393 lookups) |
| Core records no executor epoch before acting on an answer (the epoch the dispatcher asks for before each claim) | `exchange` no longer calls `observe_executor_epoch` | `soak_smoke`, no job dispatched before a receipt-store change is released on a closure alone: "job b29be7ba8243 was held when the receipt store changed, and released on a closure from the new store" (3 jobs); every Engine effect: "job b29be7ba8243 (Container Restart, fault Some(ReceiptStoreLost { older: true })) is precondition_changed after 1 Engine effects for 1 targets" |
| Retention forgets open resolution requests | `retention::SETTLED` loses `AND NOT EXISTS(… job_resolutions … confirmation IS NULL)` | `soak_ninety_days`, a job's history stays while its resolution request is open: "day 21 10:01:30: the job settled (succeeded) with its resolution request open lost its history: retention pruned 3 events in 1 passes" |
| A crash open that loses a committed event (the harness's own check) | `core_crash` deletes one committed event from the image before the child opens it | `soak_smoke`, startup under its bounds: "day 1 06:05:00: a crash image of the core store lost committed work in its open (18 WAL frames to replay): 1285 events through cursor 1286 digest to 74746d82… (committed: 1286 events, d09ef811…)", on both crash images |

The stack-proposal shim and its mutation are gone with the shim: stack
proposals are core's own `propose_stack`.

Removing only the claim's own recording of the epoch
(`jobs::record_epoch` in `claim_container` and `claim_stack`) is not
caught by the soak (`soak_smoke` passes): every executor answer's epoch,
the `ReceiptEpoch` answer just before each claim included, is recorded by
`exchange` before core acts on it, so the claim's recording never changes
anything core does against a real executor. Core's unit tests that claim
through the store directly catch it (five of `operation_tests` and
`stack_operation_tests` fail).

## Reproducibility

The workload is fixed by a seeded SplitMix64 generator and nothing sleeps for
correctness: the writer thread's passes happen when the clock is advanced,
which returns once the writer has run its gate (or fails after two minutes of
real time, naming what the writer was doing), and core's work is polled
behind the executor's crash signal. A run is still not bit-for-bit
repeatable. The stores draw random identifiers (`limeos_identity::opaque`)
for jobs, plans, sessions and receipt-store epochs, so B-tree layouts differ
between runs, and the dispatcher's candidate listings and startup recovery
visit jobs in identifier order, so the order faults meet jobs differs too.
Values still taken from the wall clock: `Store::open`'s timestamp on recovery
events, tokio timeouts around executor calls, Engine calls and store calls
(never reached), the legacy-expiry back-fill of a receipt store upgraded from
schema 5 (never run here), `metrics.sqlite` query timeouts, and the deadline
(a minute ahead) each automation reservation must meet. Counts of jobs, faults
and refusals therefore vary between runs, and page counts by a few percent.
The thresholds leave room for that: budgets have a batch of headroom,
flatness tolerates 2% or 256 KiB, the startup factor 3.2 sits above the 2.5
measured, and each pin is the range measured over the runs listed with it,
widened by that range on each side. VM and full-scan steps are exact for a
given file, so their bounds are functions of the rows the copy holds, not
tolerances.

## Running it

```sh
# Smoke, the fast cap and writer tests: part of every cargo test
cargo test -p limeos-core --bin limeos-core soak::

# Everything CI's soak job runs
cargo test -p limeos-core --bin limeos-core -- --ignored soak_ --nocapture

# The same, one test at a time (a machine short of memory; about 11 minutes)
cargo test -p limeos-core --bin limeos-core -- --ignored soak_ --nocapture --test-threads=1

# One scenario
cargo test -p limeos-core --bin limeos-core -- --ignored --exact soak::soak_ninety_days --nocapture

# The automation limit, measured (not a gate)
cargo test -p limeos-core --bin limeos-core -- --ignored --exact soak::automation::automation_limit_at_minimum_interval --nocapture
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
faults, restarts, the executor's requests by kind, every warning core logged
by site, message and code with the reason it is expected, the executor's lost
answers and refusals by kind, legitimate refusals, retention removed and the
largest batch per rule, row counts and sizes every ten days, startup page
reads on copies, the largest startup statements on the last copy with their
reads in the open and cold and their VM and full-scan steps, every statement
that walks a b-tree whole with the reason it may, the retained rows and VM
steps per row, the opens on crash images with the frames they replayed, the
measures the pins read, and one line per check: `PASS`, `FAIL` (with its first
20 failures) or `XFAIL` (a known defect, within its pins).

A `walks … without an allowance` failure means a startup statement reads a
table that grows with history from one end: give it an index, or, if the
walk is meant, an entry in `SCAN_ALLOWANCES` with its reason. An `unexpected
warning` failure names the message and its site: either core logs something
it should not, or a new expected warning needs an `EXPECTED_LOGS` entry
with its reason.

A `has no budget` failure means a table was added: give it a budget in
`core_budget` or `receipt_budget`. A change to core's dispatcher, recovery,
resolution or handlers (`propose_stack` included), or to the executor's
container requests, is exercised as it is.

## What it does not cover

- Real hardware, fsync and SD-card latency (the stores are on tmpfs).
- The real Engine and its HTTP client (covered by `docker/tests.rs`).
- The executor daemon's accept loop, peer-credential check and connection
  permits, and its role's own reads (observations, logs, storage
  inventories); the soak serves the same request handling from its own
  accept loop.
- Core's HTTP layer (`limeos_api`: the route table and its gate, the
  reader-side session check `signed_in`, request framing, deadlines,
  limits): the soak calls core's handlers directly, each of which
  authenticates its session itself.
- The password workers: the password check behind logins and failed logins.
- The storage reader's inventory proxy (`start_proxy`, its RPC and its
  cohort's transport): the soak hands core's cache the cohort it would
  deliver.
- A process killed inside a SQLite transaction or with a torn write: a
  simulated core death takes its crash image with the writer idle (every
  call committed or rolled back), so the WAL holds whole frames; a simulated
  executor death drops the process between two steps (its handler at an
  await).
- Core's other loops: the observation runtime, reader health and the RPC
  server's own framing.
- Automation producers (not in the shipped scope); see the measured limit above.
- Session revocation under a full queue, disk-full, schema upgrades: unit tests.

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
| Container executor | Real request handling: `limeos_executor::container_requests::answer` behind the ceiling's `validate`, around the real `ReceiptStore` (opened with `open`, which reads no clock), on a real Unix socket with the protocol's framing; the harness supplies the accept loop and the faults | The store's own `clock`/`now` parameters |
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
| `crates/persistence/src/writer_clock.rs`, `lib.rs` | The writer thread reads its clock through `WriterTime`; `test_support::open` gives it a `SimClock`; `test_support::on_open` hands `Store::open`'s connection to a hook before startup runs; `Database::open` opens the store at its clock's time (`Clock::wall`), which dates startup recovery's events and any batch startup runs to make room | In a build without `test-support` only `Clock::System` exists: `Instant::now()`, the real `retention::INTERVAL` wait, and the system clock's time at open, as `Store::open` takes it |
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
| `soak_ninety_days` | The acceptance run: 90 days; core restarts (nightly and at awkward moments) only in the first 30, then 60 days on one open store, so `Store::open` never recomputes a drifting counter; startup measured nightly on copies and every fifth day on crash images | about 480 s alone (297 s before job-row retention) |
| `soak_ninety_days_restarts` | Alongside it, the recovery coverage: the same household with another seed, both daemons restarting every night and at every awkward moment for all 90 days | about 480 s alone (288 s before) |
| `soak_core_store_at_its_audit_cap` | The core store at, half-way into and at the end of the reserve past its 64 MiB audit cap, with and without prunable history, with and without a running job needing a recovery event (12 cases) | 15 s |
| `soak_core_retention_works_off_a_backlog` | Retention suspended during a burst of real logins, then resumed: twelve batches of backlog; every `cargo test` | 1 s |
| `soak_receipt_store_at_its_caps` | The receipt store at its row cap and its page cap (`limits`), with prunable and unprunable rows, and past its file check; executes sent by core's own `exchange`; every `cargo test` | 1 s |
| `soak_occurrences_stay_at_their_budget` | One observe-health schedule at the 30 s minimum settled through the real reservation API for 30 days, then a day with retention suspended | 60 s |
| `soak_metrics_store_ninety_days` | `metrics.sqlite` at the collector's cadence for 90 days | 25 s |
| `writer_thread_runs_retention_unprompted` | The writer thread's startup pass, before its first call; every `cargo test` | <1 s |
| `writer_thread_works_off_a_backlog_on_its_own` | The writer thread's periodic passes on simulated time, with nothing but the clock moving; every `cargo test` | <1 s |
| `walks_are_read_from_bytecode_under_any_alias` | The bytecode reader the startup checks rely on: a walk under an alias and `NOT INDEXED`, an index walk told from a table walk, no walk in a seek, an error (not "no walks") for an unreadable program; every `cargo test` | <1 s |
| `automation_limit_at_minimum_interval` | The known automation limit, measured; not a gate (about 14 minutes) | — |
| `startup_at_high_retained_job_count` | Core's open at several thousand retained jobs (a four-day hammering storm), measured; not a gate (about 8.5 minutes); see "Core startup at high job counts" | — |

The ordinary `cargo test` runs the smoke, the backlog, receipt-cap and writer
tests, the bytecode reader's and the generator's own tests: 11 to 14 s together (the smoke about 9 s
of it). Every other `soak_*` test is ignored there and run by CI's own `soak`
job, on both native architectures, under a 25-minute limit. One at a time
(`--test-threads=1`) they took 17 min 45 s on this machine (8 cores, tmpfs)
with the test binary built, the two 90-day runs about 480 s each: job-row
retention's passes add about 3 minutes to each (237 and 215 s of retention
passes), mostly the candidate query and the floor walks of a pass after day
30 in a debug build. Before it, all of them took 6 min 41 s side by side and
11 min 16 s one at a time; side by side was not measured again this round.
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
| Administrative cancels | 3.5% of household jobs queued without a fault (a few a week): the host operator looks the job up and cancels it on its owner's behalf before core's next pass, through `rpc_list_jobs` and `rpc_cancel_job` as `limeosctl job cancel --yes` does; the job must end `canceled` with no Engine effect |
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
with a closure. A replaced store has pruned nothing (its watermark is only
ever an expiry it pruned, never a clock reading), so it refuses no plan with
`BeyondReceiptHistory`; the soak never steps the clock back, so that refusal
is always a lifetime refusal.

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
- Day 25, 08:00 to 20:00: a scripted client proposes a container action
  every 40 s and a stack action every 45 s, on whatever the job list does
  not show locked. That is more than the container executor's effect budget
  admits (10 effects per container in ten minutes, 60 a day, a stack action
  charging each target; ADR 0013): a stack action every 45 s over four
  stacks charges each stacked container about 25 effects an hour. The
  script reads no job history, so it keeps hammering spent containers, and
  those jobs are refused `RateLimited` and settled as refused: the case the
  budget exists for (the run requires that the script's client see at least 100 of its own jobs settled as refused by the budget, from the jobs' progress; a refusal whose answer was lost settles otherwise and is not counted). People do
  read job histories: a container or stack whose history says its budget is
  spent until a time still to come counts as busy for the household until
  then (one time in twenty they act anyway, as they do on a lock). The
  storm's single-container actions go to the three standalone services
  only: on a stack member they would spend the budget its stack's actions
  need, and the storm would no longer fill the receipt store's 256 stack
  receipts in a day (with them spread over all 18 containers, 185 stack
  receipts were kept at day 63 and the receipt tables were still filling in
  the last third). A four-day storm that filled them by spreading out
  (hammering on its first day, backing off as people do on the other three)
  made 3,700 and 4,550 jobs, and while
  retention kept them core's open read up to 3.45 pages per page of file
  against the 3.2 its startup bound allows (in the restart scenario; a
  property of core's startup scans at that job count, retained 30 days, not
  of the budget; see "Core startup at high job counts"). The run asserts that only
  the storm and the day its charges still count meet the budget (`BUDGET`).

  Measured on 24 November with the budget (debug, x86-64, tmpfs, the
  ignored set together in 9 min 6 s): the budget refused 815 executes in
  the long profile and 943 with restarts; the storm's client saw 788 and 912
  of its own jobs settled as refused (the rest had their answers lost and
  settled by closure, or were not polled to the end); one household job the
  next morning met a container the storm had spent. The stack receipts
  fill on day 25, as before. At the end core.sqlite is 33.9 MiB (33.4 with
  restarts; 31.4 and 29.0 before), receipts.sqlite 2.91 MiB (2.86; 2.35
  before), the receipt store's open reads about 1,240 pages (697 before),
  1,623 jobs on day 54 (1,453 before) and 2,133 executes (1,928 before), and
  each 90-day run takes 545 s alone (437 s before). Over the per-job tail of
  the last third the per-job tables grow -41 KiB beyond the rows accounted
  for (-80 with restarts) against the 64 KiB bound, and their retained rows
  -24 rows beyond what the retained jobs explain (-7) against 64; counted by
  jobs alone they read +38 and +116 KiB. The composition explains that
  difference and the numbers close: over the tail the settled jobs without a
  result went from 63 to 24 (86 to 29), and of those, the jobs the budget
  refused from 37 to 0 (57 to 1), so the budget-refused jobs leaving the
  stack family's floor are 37 of the 39 resultless jobs that left (56 of
  57), the rest cancellations and stale-plan refusals.

## Core startup at high job counts (open, not fixed here)

Core's open reads more pages per page of its store as the retained jobs grow,
and past about 2,700 retained jobs its startup statements read more than the
3.2 pages per page of file and WAL the startup bound allows. That many jobs
is reachable in production: job-row retention keeps every settled job for 30
days (the floor of 128 newest per family is a minimum, not a cap), so 2,700
jobs is 90 a day and 5,100 is 170 a day, a busy or scripted household, and
jobs refused by the executor's effect budget count too. Found while trying a
four-day storm for PR-30; `startup_at_high_retained_job_count`
(`--ignored`, not part of CI's soak job, about 8.5 minutes) reproduces it:
the restart scenario with a four-day storm hammering throughout, 40 days.
Measured on 24 November (debug, x86-64, tmpfs), core's open on the nightly
copies:

| Retained jobs | Pages of file and WAL | Cold startup reads | Per page (bound 3.2) | Open, ms |
|---:|---:|---:|---:|---:|
| 257 | 5,872 | 12,906 | 2.20 | 70 |
| 1,073 | 7,228 | 18,961 | 2.62 | 90 |
| 2,191 | 9,007 | 27,513 | 3.05 | 131 |
| 2,718 | 10,322 | 33,200 | 3.22 | 139 |
| 3,852 | 11,683 | 41,291 | 3.53 | 173 |
| 5,113 | 13,890 | 53,384 | 3.84 | 210 |

(2,718 is from the same profile with the storm backing off after its first
day; the other rows are from the hammering one.) The statements whose reads
grow with the jobs, at 5,113 jobs, by cold page reads: the lock and resource
invariants (`SELECT EXISTS(SELECT 1 FROM jobs j WHERE NOT EXISTS(SELECT 1 FROM
job_resources ...`), 14,075, a walk of every job with a probe of its
resources; the automation, storage target and mount, and backup-preparation
checks that pick jobs by `json_extract(intent,'$.operation')`, about 2,050 to
2,690 each, eight of them, each a full walk of `jobs` (5,108 full-scan steps)
because the operation is inside the intent's JSON; and `PRAGMA quick_check`
(22,680) and `PRAGMA foreign_key_check` (8,944), which read every page and
grow with the file. The quick_check and foreign-key reads are a fixed
multiple of the file; the job walks are what push the ratio up, roughly 2,000
pages per walk at 5,100 jobs. The open still took a fifth of a second in a
debug build; on the Pi in release it was not measured. Likely remedies, for a
follow-up: an index on the operation (a generated column, or the operation
stored beside the intent), or one walk of `jobs` that serves every check;
or a bound stated in jobs rather than pages for these statements. The plan
row PR-41 records it as open.

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

On the last night of the main run the open executed 50 statements: 14,316
page reads, 16 of them outside every statement (the same 16 on every copy of
every run: the schema's pages). The largest are `quick_check` (9,830), the
foreign-key check (3,064, with 794,941 VM steps, the second most after
`quick_check`'s 1.2 million), the audit reconciliation's indexed count (970)
and the lock and resource invariants (243 in the open, 3,307 cold); the
storage-mount proof check read 0.004 files' worth at most. Their VM steps
came to 2,089,697, 16.0 per retained row (131,007 rows). Before job-row
retention and the proof check's fix the same night read 82,213 pages, 57,135
of them the proof check's.

## Results

Measured on 20 November 2026 (debug build, x86-64, tmpfs, one run of the
long profile, one test at a time), with job-row retention and the proof
check's fix. The workload is unchanged, and so are the jobs and faults (the
counts below are this run's; the round before measured the same jobs, 2,058
and 1,697).

| Over 90 days | `soak_ninety_days` | `soak_ninety_days_restarts` |
|---|---|---|
| Jobs | 2,058; 1,632 removed by job-row retention, 424 kept at the end (1,612 and 446 before startup recovery's events took the simulated clock, 23 November) | 1,697; 1,296 removed, 402 kept (1,284 and 413) |
| Events pruned by the writer thread, peak | 23,849 by the events rule, peak 131,319 against the 131,072 budget, largest batch 247; besides, the job events removed with their jobs | 21,646; largest batch 244 |
| Jobs removed | at most 5 a batch, from day 55 (the storm's jobs, 30 days on); every one settled when last read, none seen again | at most 6 a batch |
| Executor | 1,483 container and 473 stack executes; 1,782 action and 256 stack rows kept; 24,276 receipt lookups, 5,791 answered with a closure | 1,172 and 417; 1,815 and 256 kept; 23,183 lookups, 8,779 closures |
| Faults | 14 executor deaths with an execute unread, 7 inside the Engine call, 7 between epoch and execute, 33 execute answers lost and 12 more with the first recovery lookup's, 28 Engine errors, 23 core crashes at awkward moments and 2 interrupting a resolution (2 detached resolution tasks ended with them), 2 receipt stores replaced by an empty one and 4 by an older copy, the held-open resolution (and 188 container restarts outside LimeOS) | 14, 6, 9, 29 and 13, 13, 23 and 5 (5), 2 and 4, the same (and 182) |
| Crashes | 25 core crashes reopened from their crash image, every one with committed WAL frames to replay | 28, likewise |
| Lost answers | 73 execute answers never written, each logged once as `Unavailable` at the execute site; 12 recovery-lookup answers lost, each logged once in its pass | 69; 13 |
| Receipt-store changes | 20 jobs held across an executor start or replacement answered with a closure by the new store; none released on it | 20 |
| Held-open resolution | the job settled with its request open; nothing prunable older than its history, the 3 events pruned after it were others'; its history kept | the same |
| Restarts | core: 30 nightly and 25 at awkward moments, all before day 30; executor every night | core and executor every night, and 28 core crashes |
| Accounting | the counter equal to its definition at every checkpoint and restart; events never below their budget once reached, but for what job-row retention removed with its jobs (the table then refills: 127,410 on day 63, 128,527 on day 89) | the same (128,676; 129,782) |
| Idle hours | 12 scheduled, 12 measured, no store file written | the same |

| Day | core.sqlite (WAL) | events | jobs | receipts.sqlite | core open on a copy: page reads | receipts open |
|---:|---:|---:|---:|---:|---:|---:|
| 9 | 1.1 MiB (1.1) | 738 | 97 | 0.09 MiB | 286 | 33 |
| 27 | 30.6 MiB (1.1) | 131,072 | 1,423 | 2.28 MiB | 18,966 | 723 |
| 54 | 31.2 MiB (1.1) | 131,072 | 1,453 | 2.32 MiB | 19,340 | 757 |
| 63 | 31.2 MiB (1.2) | 127,410 | 419 | 2.36 MiB | 13,937 | 761 |
| 89 | 31.2 MiB (1.2) | 128,527 | 444 | 2.36 MiB | 14,316 (55.9 MiB read, 1.8 times the file) | 704 |

In the restart scenario core.sqlite holds at 29.0 MiB from day 54 to 89, the
jobs go from 1,093 to 396 between days 54 and 63 and end at 415, and the open
reads 17,128 pages on day 54 and 14,137 on day 89 (it read 82,213 and about
as many the round before, the proof check most of it).

Over the last third: the per-job tables grow 2.6 KiB a day (3.3 in the
restart scenario), core.sqlite would reach 80 MiB in 52 (43) years at that
rate (after the settlement-time and bounded-selection round, 22 November,
the long profile one test at a time in 938 s, most of it the two 90-day
runs, 164 and 155 s of them clock and retention passes: 2.1 KiB a day
(4.4 with restarts); beyond the jobs retention keeps, -57 KiB (36 KiB)
against 64; startup 15.1 (18.0) reads a night against 29.1 (37.4)
explained by 0.80 (1.47) per-job and 4.27 (4.67) events pages a night);
what still grows are the jobs an authority event keeps (resolved
jobs: 49 and 37 completed resolutions at the end), bounded by the authority
rule's budget. Core startup reads grow 16 pages a night, 3.2 (2.6) per page
the history tables gain: the events table refilling to its budget after the
storm's jobs took their history with them, which stops at the budget; at
that rate the bound is 10 years away. The proof check reads at most 0.004
files' worth on any copy.

Startup on crash images: 20 per scenario, each replaying 7 to 262 committed
WAL frames (up to 1.1 MiB of WAL), every one opened and kept every committed
job and event (counted then; held to identity and content since); recovery read 1.0 to 1.6 pages per frame replayed beyond the
16 outside every statement of a clean open, and the opens read no more than
their statements did cold on a consistent copy plus that.

Every table stayed flat over the last third, the per-job tables with them,
and no settled job was kept a day past job-row retention's horizon and
floor. The cap, backlog, occurrence and metrics tests passed again in the
same run, with the same figures as last round, which measured:
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

### Fixed: core.sqlite had no size bound

Pinned last round: the per-job tables (`jobs`, `job_resources`,
`container_plans`, `container_results`, `stack_plans`, `stack_results`,
`job_resolutions`) were never pruned, 4.5 to 4.8 KiB per job, 46 to 49 KiB a
day at household rates, core.sqlite past its 80 MiB bound in 2.7 to 2.8
years; and startup grew with them (`quick_check` and the foreign-key check
read every page, seven statements walk `jobs`), 57 to 64 page reads a night,
the startup bound in 2.1 to 2.3 years.

Job-row retention (`crates/persistence/src/retention/jobs.rs`, schema 19)
removes a settled container, stack or health-probe job 30 days after it
settled (`jobs.settled`, set with the state that settles it; its plan's
expiry, events, result and resolution all older too), with its remaining
events, resources, plan, result and completed resolution, at most 64 jobs
and 2,048 rows a batch. It keeps the newest 128 settled jobs of each family,
every unsettled job, every job with an open resolution request or an
authority event, and every job of another operation. The floor is per family
across every user, not per administrator: an administrator whose jobs all
settled before 128 newer ones of the others can find their listing empty
once the horizon has passed. The rule reads the writer's wall clock: a clock
set back keeps jobs longer, but one set far forward removes every settled
job past the floor early (no other retention rule ages rows by time, so
there is no guard to match, and refusing while the clock is far past the
newest stored time would last only until the first event written under it). With it the
per-job tables hold 444 jobs on the last night of the main run (0.8 MiB of
`jobs`, 2.7 MiB in all) instead of 2,047 and still growing, the file stays at
31.2 MiB from day 54, and startup reads 14,316 pages on the last night
instead of 82,213. Both pins are gone, and the checks they pinned pass
(`slow growth leaves at least five years`, `core startup work leaves at
least five years`). The flatness check now holds the per-job tables flat
with every other table, and the budget check counts any settled job kept a
day past the rule (none).

### Fixed: the storage-mount proof check read the events table once per job

Pinned last round: `storage_mounts::check`'s proof query was planned as a
multi-index OR over `jobs_state` that evaluated its subquery for every
`precondition_changed` job, of any operation, with the events looked up by
kind (`events_kind`): every `job_transition` event once per such job, on a
store with no storage mount at all; 57,135 page reads of 82,213 on the last
night, 6.5 files' worth cold. The query now starts from the mount plans
(every mount job has its plan naming it, checked just before), reaches each
mount job by its key and looks its `running` transition up among its own
events (`events_job`); a unit test reads its plan (`events_job`, never
`events_kind`, no scan of `events`) and holds its VM steps equal while the
other jobs grow fivefold and their events a hundredfold. Its pin is gone;
the check is tighter: no lookup by kind, and at most 64 pages cold on every
copy (`PROOF_READS`), where it measured at most 0.004 files' worth (about
30 pages).

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

### Fixed: the flatness check mis-modelled the events refill

`soak_ninety_days_restarts` failed once in CI (x86, commit `63ae4ea5`, a
workflow-only change from a commit that passed): "core: bounded tables
grew 0.51 MiB over the last third, beyond 0.21 MiB the events table
regained toward its budget". The check's model of the refill was wrong,
and it sat within a few percent of its threshold on every run, so the
run-to-run spread of page counts decided it. That nothing leaked rests on
two traced reproductions of the same code and seed: the failed run's
database was not available, only its log, whose per-day rows and counts
match the traces'.

The check took the events table's refill as the rows it regained (net)
at the table's average bytes a row. Two traced runs (every b-tree's
pages, cells and payload at every checkpoint, and the store copied on days
60 and 89) showed what the pages did instead. Over days 60 to 89 the table
received 2,247 rows (77.5 a day) and kept 1,213 (41.8 a day): job-row
retention removed the rest, the events of jobs settled 30 days before.
Those events sit between the job-less events of their days (logins,
plans), so their pages keep the neighbours and the space is not reused:
rows are appended at the end of the table and of each key range of its
scope and kind indexes (`events_job`, keyed by random job IDs, reuses its
holes and shrank). On day 89 the table's rows of days 30 to 59 stood 15 to
a page at 57% full, those of days 60 to 89 (their jobs' events still
there) 24 to a page at 97%, the login burst's 54 to a page at 98%. The
events b-trees gained 0.55 to 0.56 MiB over the third (4.8 to 5.0 pages a
night), 463 to 469 bytes a row kept, against an average of 178 bytes a
row, most of the table being the burst's 26-byte session rows; the check
credited 0.21 MiB. With the per-job tables' 0.12 to 0.13 MiB, the fitted
growth beyond that credit was 0.47 MiB against a threshold of 0.49 to 0.50
MiB (2% of the tables) in both traced runs, and 0.51 in the run that
failed.

The growth is bounded: the row budget caps the rows, and SQLite rebalances
a page that deletes leave less than about a third full (roots and the
newest pages aside). It does go on until the burst's rows are gone, about
eight years at these rates, at about 59 MiB of the household's rows
against 22 MiB of the burst's: well above the 40 MiB this document once
assumed for the events table at its budget, and within the 80 MiB bound.

The check now judges every b-tree on its own, so no table or index can
hide behind another shrinking:

- Every b-tree outside the events table (a table's own or an index):
  fitted growth under 2% of its size or a floor chosen from the traces:
  192 KiB for the per-job tables' (`stack_results` grew 142 KiB in both
  runs while the stack family refilled toward job-row retention's floor of
  128; every other per-job b-tree under 20 KiB), 32 KiB for every other
  core b-tree (all measured under 3 KiB), 256 KiB for the receipt store's
  (still filling toward their row budgets, 0.21 MiB in all).
- Each events b-tree against the cells of the rows appended to it, counted
  exactly from the rows at each checkpoint (`appended_cells`): its fitted
  growth at most those cells at 90% fill for the table (it appends at its
  right edge; measured 97 to 99%) or 75% for an index (`events_scope`
  measured 86%), plus 16 KiB. Nothing else is credited, so the holes
  job-row retention leaves count as never reused. Measured (restart
  scenario, then the main run): the table 364 KiB against a ceiling of
  424 (328 against 433), `events_scope` 167 against 225 (158, 230),
  `events_kind` 42 against 81 (31, 83), `events_job` -15 against 143
  (-37, 144).
- The cells of a row appended in the second half of the third at most 1.2
  times those of one appended in the first (measured 0.2% apart).
- While the events table regains rows (the events and authority rules
  removed nothing over the third, by the writer's passes, and its fitted
  rows rose by at least 256), where its growth ends: every other b-tree as
  it is at the end, with the events table at its row budget (132,096) at
  the bytes it gained per row kept, or its average a row if more, within
  the 80 MiB file bound: 62.3 MiB (473 bytes a row kept) with restarts,
  54.2 MiB (408) in the main run.
- The file's growth under 1 MiB beyond the events b-trees' ceilings.

What it catches, as a steady leak over the last third (`soak::checks::
flatness_model`, on the measured third): pages the rows do not explain,
more than about 60 KiB in the events table (2 KiB a day), 40 to 60 KiB
in `events_scope` or `events_kind`, 155 KiB in `events_job`, at any size of
the table and however many rows it regains (0.5 MiB fails with 1,213
rows regained or 3,000, and with the table at the steady state's 60 MiB);
in any other core b-tree, more than its floor less what it grew anyway
(about 50 KiB for `stack_results`, 32 KiB for a row-budgeted table), what
another b-tree releases notwithstanding; rows that grow by more than a
fifth within the third; and, while the table refills, a refill costing
about 30% more bytes a row kept than the measured 475 (the end then passes
the file bound). What it does not catch: while the events rule prunes, the
cell ceiling ignores the pages pruning frees, so a page leak smaller than
a b-tree's ceiling (426 KiB over a third for the table, 226 for
`events_scope`, at household rates) passes, and nothing projects the end;
rows made larger from the start of the third by less than about 30%; and
a leak in a b-tree that stays under its floor. Neither profile's last third reaches the budget (both
refill throughout), so the soak never runs where those gaps are.

The traces also showed 13 `job_recovery_required` events on day 60 and
18 on day 89 dated by the system clock, about 324 days after the simulated
one: `Store::open` took the time for startup recovery's events from
`SystemTime::now()`, past the writer's clock seam, so job-row retention
(which keeps a job while any of its rows is newer than its horizon) kept
every job startup recovery marked for the whole run, and the soak never
aged or removed one. `Database::open` now opens the store at its clock's
time (`Clock::wall`: the system clock in a shipped build, as before; the
simulated one in the soak), and a new check holds every stored time to the
simulated clock. With it, of the 23 jobs recovery marked in each run, the
main run removed all 23 and the restart scenario 18, keeping 5 settled
within the horizon; none was kept for an authority event, so it was the
clock alone that pinned them. Jobs removed went from 1,612 to 1,632 and
from 1,284 to 1,296, and every check passed as before, so nothing relied on
them staying.

The two traced runs had the same rows, cursors, cells and payload in every
b-tree at every checkpoint; only the page counts of indexes keyed by
random identifiers differed (`events_job` 504 and 457 pages on day 89,
the autoindexes of `jobs`, `job_resources` and the plan and result tables
by a page or two). See "Reproducibility".

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
| no lifetime or unexpected refusal | `StateNotDurable`, `Overloaded`, `BeyondReceiptHistory` from any store call or executor answer or in anything core logs, a failed retention rule, and any refusal a call site does not list as legitimate |
| core logs only the warnings it is expected to | every warning or error core logs must match an entry of `EXPECTED_LOGS` (its exact message, level and codes, with the reason the soak provokes it): the dispatcher's two "remains pending" lines (`Conflict`, `Unavailable`), the execute that failed after it was written (`Unavailable` only), the execute never sent, the execute refused before any row after a replacement, the execute refused by the executor's effect budget, the executor's own journal line for a refusal row of its action log, and the held-open resolution that stays pending (`Unavailable`). Nothing may carry `InvalidInput` |
| every lost executor answer is logged once, as unavailable | every execute answer the executor never wrote (lost, or the process dead with the request unread or inside the Engine call) is logged by core exactly once, at its execute site, with `Unavailable`, and every other code there is a refusal the executor answered, one for one; every lost container recovery lookup is logged as that job's failure with `Unavailable` in the same dispatcher pass, one for one, whenever the executor ran throughout the pass (a lost stack lookup is `release_if_unsent`'s, which holds the job without a log, and the stack site logs none); every executes refused before any row matches its log line |
| the executor's effect budget refuses only the action storm, which it does refuse | every execute the executor refused `RateLimited` was answered between the storm's first minute and a day and an hour after its last proposal (the storm's charges count for a day, so a household job on a container the storm spent is refused too); a storm of six hours or more saw at least 100 of its own jobs settled as refused by the budget, counted from their progress as its client sees them; core logs each heard refusal with its own line, one for one (`lost_answers`); a refusal whose answer is lost counts as a lost answer |
| no job dispatched before a receipt-store change is released on a closure alone | every job core held when the executor started (a new store epoch; with a replaced store, receipts lost) and that the new store answered with a closure ends in any state but `precondition_changed`; the replacements happened and such jobs were seen |
| every time core stores is on the simulated clock | at every checkpoint, no event's `created`, job's `settled`, result's `recorded` or resolution's `completed` after the simulated now: job-row retention ages jobs by them, on the writer's clock (`Store::open` once dated startup recovery's events by the system clock, months ahead, which kept every job they name for the whole run; it now takes the time `Database::open` opens at, the writer's clock) |
| a job's history stays while its resolution request is open | the held-open job, settled with its request open, keeps its `job_queued` event while retention prunes every older prunable event and once more; the window must be decisive (nothing prunable older than the job left, something pruned) |
| provoked refusals observed | an approval after expiry and a session used after expiry, each refused as such |
| audit counter and retained rows match what retention promises | at every checkpoint and before every restart: `meta.audit_bytes` is exactly `EVENT_BYTES` per event row; once the run has written as many events as the events rule keeps, at least that many remain, less what the authority rule and job-row retention (with its jobs) removed; standalone and stack receipts never fall below what retention keeps once reached; the prune watermark never decreases and never passes the youngest expiry retention may prune |
| every retention batch within its rule's limit | every batch the writer thread reports, one transaction, removes at most its rule's limit |
| database files under their bounds | core file 80 MiB and WAL 8 MiB; receipt file half its page cap and WAL 2 MiB, every checkpoint |
| every table within its budget | every table in both stores, found by walking `sqlite_master`; the events budget from the production rule, the receipt budgets from `limits`; the per-job tables at most 20 rows a job, and no settled container or stack job kept a day past job-row retention's horizon and floor (the newest 128 by settlement) with nothing that keeps it (no open request, no authority event, nothing newer, its settlement time included), counted by the soak's own query |
| sizes and bounded tables flat across the last third | over days 60 to 90, every b-tree on its own (each table's and each index's bytes) and each row-budgeted table's rows: every b-tree outside the events table, fitted growth under 2% of its size or its floor (192 KiB for the per-job tables', 32 KiB for core's others, 256 KiB for the receipt store's); each events b-tree, fitted growth under the leaf cells of the rows appended to it (counted from the rows) at 90% fill for the table and 75% for an index, plus 16 KiB; the cells of a row appended in the third's second half at most 1.2 times the first half's; while the events table regains rows (no events or authority rule removal in the third, fitted rows up by 256 or more), every other b-tree at the end with the events table at its row budget at the bytes it gained per row kept (or its average, if more) within the 80 MiB file bound; the file's growth under 1 MiB beyond the events b-trees' ceilings (see "Fixed: the flatness check mis-modelled the events refill" for what this catches and what not); and the per-job tables on their own, from the first checkpoint of the third with at most a tenth more jobs than the last (the storm's jobs gone), each table net of its rows that are accounted for (every row but those of the jobs the soak's own query finds removable at that moment and, in a plan table, unqueued plans already expired) at that table's bytes a row, fitted growth under 64 KiB; those rows in all may grow only as the retained jobs do, at the starting rows a job, plus one result row for each resultless job that left, fitted growth beyond that under 64 rows; no job owns more rows of a table than one job can (17 resources, one plan, result and resolution), and no unqueued plan outlives the next plan creation of its table, which removes every expired one (also asserted at every checkpoint, `BUDGETS`); unit tests show an accumulation of expired unqueued plans and growing rows per job both fail (`per_job_model_tests`); jobs are not all one size (a job refused before any effect has no result row), and the per-family floor of 128 newest jobs keeps the storm's refused stack jobs into the last third, where household jobs replace them one for one; counted by jobs alone at the average size, that replacement read as 117 KiB of growth (`PER_JOB_NET_JOBS`, reported, with the settled jobs without a result at both ends of the tail) |
| slow growth leaves at least five years | the per-job tables' growth, projected to the core file bound |
| startup under its bounds | every open's wall time (5 s core, 2 s receipts: the catastrophic only); on every copy and crash image, the cold page reads of core's startup statements (but a proof check that looks events up by kind, which its own check fails) at most 3.2 per page of file and WAL, the receipt store's open at most 2.2 (its `quick_check` walks every page once and every table's pages again for their constraints, through a cache smaller than the store: at most 2.0, measured 1.87; another full pass fails); every crash image opens and keeps every committed job and event unchanged (ids, and each event's cursor, principal, job, kind and time), adding only one recovery event per interrupted job |
| core startup reads nothing its checks do not explain | on every copy and crash image, from every statement's bytecode and counters: every statement's program read, nested ones included; no statement walks the events table itself (only `quick_check` and the foreign-key check may); only the statements in `SCAN_ALLOWANCES` walk a table growing with history (`events` or a per-job table, or an index of one), each only its listed tables, with a reason; each statement's full-scan steps at most its listed tables' rows times its steps per row (0 or 1), plus its walks of bounded tables (their rows, once per run), plus 16; the counters measured (some VM and full-scan steps once a table walked one step a row holds 64 rows); the open's VM steps (but such a proof check's) at most 64 per retained row plus 200,000; at most 32 page reads outside every statement (measured 16) plus 2 per WAL frame replayed; the open reads at most what its statements read cold, plus 3%, 256 pages and 2 per WAL frame replayed |
| the storage-mount proof check reads little without storage mounts | the statement that reads `storage_mount_results`: its plan does not look events up by kind, and it reads at most 64 pages cold (`PROOF_READS`) on every copy, whatever the jobs and events |
| core startup work grows only with the history tables (events and per-job) | over the last third, the statements' cold reads grow by at most 10 per page the per-job tables gain plus 4 per page the events table gains (while it refills to its budget), plus 4 a night |
| core / receipt-store startup work leaves at least five years | the nightly page reads, projected to the bound of a store at its file bound; the receipt store's no further than the reads of a store with every table at its budget |
| an idle hour writes nothing | once retention has nothing left, an hour of dispatcher ticks and the writer's own passes changes no file of either store (database or WAL); every scheduled idle hour is measured or skipped for a recorded reason, and at least three in four are measured |
| locks, ledger and retained history consistent after every restart | locks match job states; every job the workload queued exists, but those the writer thread's passes reported removing (each settled when last read, and never seen again), and no other does; unsettled and contested jobs keep their history; the executor holds no lock for a job core has released |
| every job settled; none lost, duplicated or stranded | after a drain, every job is terminal (a job retention removed, as last read) and no resolution request is open |
| every Engine effect ran at most once and matches its job | per action, counted by the executor, against the job's final state (a removed job's as last read) |
| authority events all kept | retention never removed one, and their count never went down between checkpoints |
| recovery closes every absent action it asks about | every receipt lookup core sends asks to close an absent action, and every absence the executor answers is closed |

## Expected failures, pinned

`EXPECTED_FAILURES` in `checks.rs` lists each known failure with the measures
it must show; a failure with any measure outside its range fails the run, and
so does a listed check that passes. Each pin is the range a measure took
over the runs of each 90-day scenario, widened by that range on each side.

The list is empty. Last round's three pins (the per-job tables' growth, the
startup reads it drove, and the storage-mount proof check) went with their
defects (see "Fixed" above); each pinned check now passes in both 90-day
scenarios. The mechanism stays, for the next defect found.

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
| Per-job tables never pruned (job-row retention removed) | `retention::jobs::prune` returns at once | `soak_ninety_days` (310 s): every table within its budget, "day 32 04:00:00: core: 10 settled jobs kept a day past job-row retention's horizon and floor, with nothing that keeps them", rising to 204 by day 51; flat: "core: bounded tables grew 1.70 MiB over the last third" and "file grew 1.73 MiB"; "core opens read 58 more pages each night over the last third (25157 now): 72345, the bound at the file bound, in 2.2 years"; "per-job tables grow 47 KiB a day (4.6 KiB a job): core.sqlite reaches 80 MiB in 2.7 years" |
| Per-job tables leak slowly (a few KiB a day) | job-row retention's window treats every job whose ID starts with `0` or `1` as kept (`j.id NOT GLOB '[01]*' AND …`): an eighth of the jobs stay | `soak_ninety_days` (438 s), flat: "day 90 00:00:00: core: per-job tables grew 99 KiB from day 60 to 89 beyond the jobs job-row retention keeps (at most 64 KiB)" (7.4 KiB a day against 1.1 to 1.9 unmutated; the whole store's 2% and the five-year projection both pass it); and every table within its budget, from "day 33 04:00:00: core: 1 settled jobs kept a day past job-row retention's horizon and floor" |
| Steady leaks over the last third, and its boundary cases | put into the measured third of `soak_ninety_days_restarts` (every b-tree's bytes and the events b-trees' appended cells, days 60 to 89) | `soak::checks::flatness_model`: `sessions` +300 KiB while `job_resources` releases 100, "core: sessions (of sessions) grew 300 KiB over the last third, more than 32 KiB"; `stack_results` +120 KiB; `sqlite_autoindex_jobs_1` +256 KiB while `jobs` releases as much; 72 KiB in `events`, `events_scope` or `events_kind` and 192 KiB in `events_job`, "core: events grew 436 KiB over the last third, more than the 426 KiB the rows appended to it explain"; 0.5 MiB with 3,000 rows regained, and at the steady state's 60 MiB; rows growing within the third, "rows appended to the events table in the second half of the last third take …"; an events index without a model. Boundaries that pass: a plateau then a gain (a finite end), falling rows (no projection), falling bytes (the end at the average row), a refill reaching the budget (projected until the events rule prunes) |
| Job-row retention ignores an open resolution request | the candidate query loses `r.confirmation IS NULL OR` | `retention::jobs::tests::every_reference_keeps_its_job`: the batch fails (the `jobs_delete` trigger refuses the job: `StateNotDurable`) |
| Job-row retention ignores authority events | the candidate query's authority-kind clause made false | `retention::jobs::tests::every_reference_keeps_its_job`: 9 jobs removed where 8 may be (the job with a withdrawal and its authority event gone) |
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
events in the crash-image and copy opens (children on copies, discarded),
tokio timeouts around executor calls, Engine calls and store calls
(never reached), the legacy-expiry back-fill of a receipt store upgraded from
schema 5 (never run here), `metrics.sqlite` query timeouts, and the deadline
(a minute ahead) each automation reservation must meet. Counts of jobs, faults
and refusals therefore vary between runs, and page counts by a few percent.
(Two traced runs of `soak_ninety_days_restarts` on 23 November had every
count, row, cell and byte of payload equal at every checkpoint; only the
pages of indexes keyed by random identifiers differed, `events_job` by
up to 10%.)
The thresholds leave room for that: budgets have a batch of headroom,
flatness judges the b-trees whose pages the random identifiers move
(`events_job` and the per-job autoindexes) against ceilings or floors that
measured at least 40 KiB above them, and the events table's end sits
about 17 MiB inside the file bound (the old refill model sat within 5% of
its threshold, which is how a page-count spread failed it), the startup factor 3.2 sits above the 2.5
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

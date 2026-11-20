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

Two steps are not core's own. A login is the session write
`login_checked` makes once the password workers have accepted the password
(the password workers are not driven, so neither are failed logins' refusals).
A stack proposal makes `propose_stack`'s calls in its order (session,
validation, authorization, fresh Engine listing through core's `inspect_stack`,
plan) with the members taken from the simulated Engine: core reads them from
the storage reader's inventory cache (`limeos_observations::stacks`), which
checks every cohort's freshness (90 s) against its own system clock, which the
simulated months cannot satisfy and which has no seam.
`stack_proposals_match_core` runs the real `propose_stack` beside the shim on
the same Engine at the system time and fails when the executor requests, the
events written, the plans kept or the plan itself differ.

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

`test-support` is enabled only from limeos-core's dev-dependencies, as is
rusqlite's `trace`; the CI soak step checks `cargo tree -e normal,build` for
both.

## The tests

| Test | What | Run time (debug, x86-64) |
|---|---|---|
| `soak_smoke` | Three simulated days of the household; every `cargo test` | 5 s |
| `soak_ninety_days` | The acceptance run: 90 days; core restarts (nightly and at awkward moments) only in the first 30, then 60 days on one open store, so `Store::open` never recomputes a drifting counter; startup measured nightly on copies | 289 to 377 s |
| `soak_ninety_days_restarts` | Alongside it, the recovery coverage: the same household with another seed, both daemons restarting every night and at every awkward moment for all 90 days | 264 to 352 s |
| `soak_core_store_at_its_audit_cap` | The core store at, half-way into and at the end of the reserve past its 64 MiB audit cap, with and without prunable history, with and without a running job needing a recovery event (12 cases) | 15 s |
| `soak_core_retention_works_off_a_backlog` | Retention suspended during a burst of real logins, then resumed: twelve batches of backlog; every `cargo test` | 1 s |
| `soak_receipt_store_at_its_caps` | The receipt store at its row cap and its page cap (`limits`), with prunable and unprunable rows, and past its file check; executes sent by core's own `exchange`; every `cargo test` | 1 s |
| `soak_occurrences_stay_at_their_budget` | One observe-health schedule at the 30 s minimum settled through the real reservation API for 30 days, then a day with retention suspended | 60 s |
| `soak_metrics_store_ninety_days` | `metrics.sqlite` at the collector's cadence for 90 days | 25 s |
| `writer_thread_runs_retention_unprompted` | The writer thread's startup pass, before its first call; every `cargo test` | <1 s |
| `writer_thread_works_off_a_backlog_on_its_own` | The writer thread's periodic passes on simulated time, with nothing but the clock moving; every `cargo test` | <1 s |
| `stack_proposals_match_core` | The stack-proposal shim against core's own `propose_stack`; every `cargo test` | <1 s |
| `automation_limit_at_minimum_interval` | The known automation limit, measured; not a gate (about 14 minutes) | — |

The CI step runs every ignored test named `soak_*` together: 4 min 49 s on
this machine (8 cores, tmpfs) when nothing else ran, 6 min 18 s while another
checkout's build and tests shared it, under its own 10-minute limit. The
mirror-based soak took 3 min 23 s; the difference is the real path's cost (the
writer thread's handoffs, the socket and framing for every executor request,
core's own handlers). The 90-day runs take 260 to 380 s each depending on
load. The others run with `cargo test`.

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
| Faults per dispatched job | about 6%: execute answer lost; executor dead with the execute unread; executor stopped between `ReceiptEpoch` and the execute; executor killed inside the Engine call; Engine error before or after acting; core restart after the claim, after the execute answer, after the executor's verify (core restarts only while the scenario allows them); recovery lookup lost |
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
own text as production wrote it, with its VM steps and full-scan steps; no
production SQL is copied into the soak. Each read-only statement is then run
again alone, cold, on its own connection to a second copy, and its query plan
read there. A statement read cold reads at least as many pages as inside the
open, where one statement's scan can evict another's pages, so the trend checks
fit the cold sums; the explained-startup check holds the open to them.

On the last night of the main run the open executed 50 statements: 74,029
page reads, 16 of them outside every statement. The largest are the
storage-mount proof check (51,088), `quick_check` (12,833), the foreign-key
check (4,531, inside the invariants statement) and the audit reconciliation's
count (1,014).

## Results

Measured on 20 November 2026 (debug build, x86-64, tmpfs, both scenarios side
by side). Job, fault and refusal counts vary between runs (see
"Reproducibility").

| Over 90 days | `soak_ninety_days` | `soak_ninety_days_restarts` |
|---|---|---|
| Jobs | 1,879 | 1,792 |
| Events pruned by the writer thread, peak | 25,783 pruned in 127,037 passes; peak 131,313 against the 131,072 budget; largest batch 241 | 25,173; peak 131,319; largest batch 247 |
| Executor | 1,465 container and 322 stack executes; 1,744 action and 256 stack rows kept; 44,407 receipt lookups | 1,403 and 291; 1,856 and 256 kept |
| Faults | 20 executor deaths with an execute unread, 10 inside the Engine call, 9 between epoch and execute, 33 answers lost, 30 Engine errors, 14 core restarts at awkward moments and 5 interrupting a resolution (and 185 container restarts outside LimeOS) | 8, 9, 8, 31, 20, 26 and 5 (and 188) |
| Restarts | core: 30 nightly and 19 at awkward moments, all before day 30; executor every night | core and executor every night, and 31 core restarts at awkward moments |
| Accounting | the counter equal to its definition at every checkpoint and restart; events never below their budget once reached | the same |
| Idle hours | 12 scheduled, 12 measured, no store file written | the same |

| Day | core.sqlite (WAL) | events | jobs | receipts.sqlite | core open on a copy: page reads | receipts open |
|---:|---:|---:|---:|---:|---:|---:|
| 9 | 1.1 MiB (1.0) | 729 | 96 | 0.19 MiB | 281 | 57 |
| 27 | 28.8 MiB (1.1) | 131,072 | 1,249 | 1.81 MiB | 17,604 | 472 |
| 63 | 31.0 MiB (1.1) | 131,072 | 1,612 | 2.27 MiB | 20,036 | 715 |
| 72 | 31.5 MiB (1.1) | 131,072 | 1,705 | 2.29 MiB | 52,366 (the proof check from here) | 732 |
| 89 | 32.6 MiB (1.1) | 131,072 | 1,879 | 2.31 MiB | 74,029 (289 MiB read, 8.9 times the file) | 703 |

Every bounded table stayed flat over the last third. Separately: the core
store opens, recovers, keeps revocation and prunes back to its budget in all
12 cap cases (the one documented refusal leaves the store unchanged); a
12-batch backlog goes in 12 passes of exactly 1,024, by `maintain` and by the
writer thread on its own; the receipt store at its row and page caps prunes at
most 64 rows an insert and refuses only while nothing is prunable;
occurrences stay at their budget for 30 days and a backlog of 3,001 goes in 6
passes of at most 512; `metrics.sqlite` holds 44,641 rows (31 days at one a
minute, none missing, the oldest at the retention boundary) in 0.78 MiB,
answers every view with the averages of what was written, and writes about
113 KiB an hour, flat.

The real paths ran fewer jobs than last round's mirror (1,879 and 1,792
against 2,328 and 2,132), most of the difference in the day-25 storm (1,249
jobs by day 27 against 1,664), when more of the scripted proposals found their
resource locked by a job held for the operator. Job counts are not asserted
and this was not investigated further; every check holds over the jobs run.

### Defect: core.sqlite has no size bound

The events table is flat at its budget, but the per-job tables (`jobs`,
`job_resources`, `container_plans`, `container_results`, `stack_plans`,
`stack_results`, `job_resolutions`) are never pruned: 4.7 to 4.8 KiB per job,
48 to 49 KiB a day at household rates; core.sqlite passes its 80 MiB bound in
about 2.8 years. Startup grows with them: every new per-job page is read by
`quick_check` and the foreign-key check, and six startup statements scan the
whole jobs table, which is now larger than SQLite's 2 MiB page cache, so the
startup statements read 5.4 pages more for every page the per-job tables gain
(66 to 68 a night), reaching the startup bound of a store at its file bound in
about 2.0 years. No refusal follows (no cap counts these rows), but neither
size nor startup is bounded.

### Defect: the storage-mount proof check reads the events table once per job

`storage_mounts::check`'s proof query (the startup statement that reads
`storage_mount_results`, found in the trace by that text) is planned as a
multi-index OR over `jobs_state`, and for every `precondition_changed` job
evaluates its subquery before the storage-mount filter, with the events looked
up by kind (`SEARCH e USING INDEX events_kind`) instead of by job
(`events_job`): every `job_transition` event, once per such job, on a store
with no storage mount at all. While that index range fits in the page cache it
costs little; once retention has replaced the burst's session events with job
history it does not. Measured in the open on the last night: 51,088 page reads
of 74,029, 6.1 files' worth cold (6.6 in the restart scenario), from day 72 in
the main scenario and between days 72 and 81 in the other this run (days 36
and 72 last round). Needs the subquery to search `events_job` (or the
operation filtered first). Pinned as an expected failure by that statement's
plan and cost.

### Defect: a lost executor answer reads as the caller's bad input

Found by driving core's real `exchange` over the socket.
`limeos_contracts::read_frame` maps every failure to read a frame, the end of
the stream included, to `InvalidInput`. When the executor closes a connection
unanswered (it died, or the answer was lost), core's dispatcher, recovery and
resolution paths get `InvalidInput`: core still holds the job for
reconciliation (any failure after the connection exists counts as sent), so
nothing is lost, but it logs "execute call failed" with the wrong code, and an
HTTP caller reaching the same path would be told a transient failure is its
own error, not to be retried (`retry: false`). Every connection the executor
closed unanswered was logged so (52 in the main run, 37 in the other), and
nothing else was.
Needs a read that ends early to be `Unavailable`. Pinned.

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
| no lifetime or unexpected refusal | `StateNotDurable`, `Overloaded`, `BeyondReceiptHistory` from any store call or executor answer, a failed retention rule, any refusal a call site does not list as legitimate, and any error core logs or any failure it logs with a code other than its dispatcher's legitimate ones |
| provoked refusals observed | an approval after expiry and a session used after expiry, each refused as such |
| audit counter and retained rows match what retention promises | at every checkpoint and before every restart: `meta.audit_bytes` is exactly `EVENT_BYTES` per event row; once the run has written as many events as the events rule keeps, at least that many remain; standalone and stack receipts never fall below what retention keeps once reached; the prune watermark never decreases and never passes the youngest expiry retention may prune |
| every retention batch within its rule's limit | every batch the writer thread reports, one transaction, removes at most its rule's limit |
| database files under their bounds | core file 80 MiB and WAL 8 MiB; receipt file half its page cap and WAL 2 MiB, every checkpoint |
| every table within its budget | every table in both stores, found by walking `sqlite_master`; the events budget from the production rule, the receipt budgets from `limits` |
| sizes and bounded tables flat across the last third | the bounded tables' bytes and rows, and the file size beyond what per-job tables added, over days 60 to 90 |
| slow growth leaves at least five years | the per-job tables' growth, projected to the core file bound (expected failure, pinned) |
| startup under its bounds | every open's wall time (5 s core, 2 s receipts: the catastrophic only); on every copy, the cold page reads of core's startup statements but the proof check at most 3.2 per page of file, the receipt store's open at most 2.0 |
| core startup reads nothing its checks do not explain | on every copy: at most 256 page reads outside every statement; no startup statement scans the events table itself (only through an index); the open reads at most what its statements read cold, plus 3% and 256 pages |
| the storage-mount proof check reads little without storage mounts | the statement that reads `storage_mount_results`: its plan does not look events up by kind, and it reads at most an eighth of the file cold (expected failure, pinned) |
| core startup work grows only with the per-job tables | over the last third, the statements' cold reads (but the proof check's) grow by at most 10 per page the per-job tables gain |
| core / receipt-store startup work leaves at least five years | the nightly page reads, projected to the bound of a store at its file bound (core: expected failure, pinned); the receipt store's no further than the reads of a store with every table at its budget |
| an idle hour writes nothing | once retention has nothing left, an hour of dispatcher ticks and the writer's own passes changes no file of either store (database or WAL); every scheduled idle hour is measured or skipped for a recorded reason, and at least three in four are measured |
| locks, ledger and retained history consistent after every restart | locks match job states; every job the workload queued exists and no other does; unsettled and contested jobs keep their history; the executor holds no lock for a job core has released |
| every job settled; none lost, duplicated or stranded | after a drain, every job is terminal and no resolution request is open |
| every Engine effect ran at most once and matches its job | per action, counted by the executor |
| authority events all kept | retention never removed one, and their count never went down between checkpoints |
| recovery closes every absent action it asks about | every receipt lookup core sends asks to close an absent action, and every absence the executor answers is closed |
| a lost executor answer reads as unavailable | core never logs a failure as `InvalidInput` (expected failure, pinned) |

## Expected failures, pinned

`EXPECTED_FAILURES` in `checks.rs` lists each known failure with the measures
it must show; a failure with any measure outside its range fails the run, and
so does a listed check that passes.

| Check | Pinned to | Measured (main / restarts) |
|---|---|---|
| slow growth leaves at least five years | per-job growth 25 to 100 KiB a day; 2.5 to 10 KiB per job; 1.25 to 5 years to the file bound | 48.0 / 48.7 KiB a day; 4.7 / 4.8 KiB; 2.77 / 2.75 years (last round 50.5 / 49.7; 4.7 / 4.8; 2.50 / 2.56) |
| core startup work leaves at least five years | the startup statements but the proof check grow 32 to 128 page reads a night; 1 to 4.5 years to the bound | 66 / 68 a night; 2.06 / 2.00 years (last round 64 / 61; 2.02 / 2.12) |
| the storage-mount proof check reads little without storage mounts | the statement reading `storage_mount_results` looks events up by kind on every copy; its largest cold read 0 to 15 files' worth | by kind on every copy; 6.1 / 6.6 files' worth (last round 7.3 / 6.0) |
| a lost executor answer reads as unavailable | failures core logs as `InvalidInput` per connection the executor closed unanswered: 0.01 to 1 | 1.0 / 1.0 |

The two older pins' figures are the same defects measured on production's own
statements: the proof check is now judged on its own statement in the open
instead of a copy of its text, and the startup series is the cold sum of every
statement the open runs (50, not the 11 copied before), but the proof check.

## Mutation checks

Each past lifetime defect, and a regression in each real path the soak now
runs, reintroduced in the production source (never committed; restored after
each run, `git status` clean), and the check that fails:

| Defect or regression | Mutation | Failing test and check |
|---|---|---|
| Audit counter that only grows | `prune_events` no longer calls `account(tx, -removed)` | `soak_ninety_days`, accounting: "day 21 04:00:00: daily checkpoint: meta.audit_bytes is 37288704, but 13 events take 3328 (256 each): the audit counter drifted by 145646 events" and "13 events kept of 145659 written, below the 131072 the events rule keeps: retention over-pruned by 131059" |
| Receipt rows never pruned | the receipt store's `prune` returns at once | `soak_receipt_store_at_its_caps`: "row cap, prunable: job 0 ended needs_intervention with 8192 rows" (and "row cap, aged rows: the execute was still refused: Overloaded", the file cap refusing every execute) |
| Occurrences capped with no pruner | `prune_occurrences` removes nothing | `soak_occurrences_stay_at_their_budget`: "day 0.36: a pass removed 0 of 1025 occurrences (budget 1024, batch 512)" |
| Missing index: one extra linear scan at startup | the audit reconciliation's count reads `events NOT INDEXED` | `soak_smoke` (and every run), core startup reads nothing its checks do not explain: "a startup statement scans the events table itself (… plan [\"SEARCH meta USING INTEGER PRIMARY KEY (rowid=?)\", \"SCALAR SUBQUERY 1\", \"SCAN events\"]): SELECT audit_bytes,(SELECT count(*) FROM events NOT INDEXED) FROM meta WHERE id=1", on every copy, from day 0 |
| Prune batch with no limit | `run` passes the whole excess instead of `excess.min(rule.batch)` | `soak_core_retention_works_off_a_backlog`: "pass 1 removed 12290 events with 12290 over budget: each transaction removes one batch of at most 1024" |
| Store refuses to open at its cap | `Store::open` refuses when the reconciled `audit_bytes >= AUDIT_LIMIT` | `soak_core_store_at_its_audit_cap`: "AtCap, prunable history, no running job: the store refused to open: Error(StateNotDurable)", in every case that must open |
| The writer never prunes at startup | the writer thread's `maintain` before its loop removed (the loop's kept) | `writer_thread_runs_retention_unprompted`: "the writer thread ran no retention pass on its own before its first call: 134146 events before it started, 134146 when it served that call; one batch of 1024 was due" |
| The writer never prunes again | the `maintain` in the writer thread's loop removed (the startup one kept) | `writer_thread_works_off_a_backlog_on_its_own`: "after 1 simulated minutes the writer thread removed 0 events with 11266 over budget: it runs one pass of at most 1024 each minute on its own"; and `soak_ninety_days`, every table within its budget: "day 21 04:00:00: core.events: 145659 rows over its budget of 132096", and again from day 43, once core stops restarting (each restart's startup pass removes one batch) |
| Recovery releases a job without asking the executor (core's dispatcher) | `dispatch_one`'s recovery skips the `RestartReceipt` lookup and acts on a closure it makes up (a `precondition_changed` receipt, `closed`) | `soak_smoke`: locks, ledger and retained history: "executor crash with an execute unread: the executor holds a lock for job eb9f9e21d15b that core has as Some(\"precondition_changed\")"; every Engine effect: "job eb9f9e21d15b (Container Restart, fault Some(AnswerLost)) is precondition_changed after 1 Engine effects for 1 targets" |
| The executor skips the closure on an absent receipt (its request handling) | `container_requests::answer` passes `false` for `close_if_absent` to both receipt lookups | `soak_smoke`, recovery closes every absent action it asks about: "the lookup for job 753c9591dbb3 answered Some(NeverRecorded) without closing the action" (393 lookups) |
| Core's `propose_stack` drifts from the soak's shim | `propose_stack` asks for one more Engine listing | `stack_proposals_match_core`: "core's propose_stack (home) asked, wrote and kept ({\"inspect stack\": 2}, [(\"plan_created\", 1)], 1); the soak's shim ({\"inspect stack\": 1}, [(\"plan_created\", 1)], 1)" |

## Reproducibility

The workload is fixed by a seeded SplitMix64 generator and nothing sleeps for
correctness: the writer thread's passes happen when the clock is advanced,
which returns once the writer has run its gate, and core's work is polled
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
measured, and the pins span a factor of two around today's values.

## Running it

```sh
# Smoke, the fast cap and writer tests: part of every cargo test
cargo test -p limeos-core --bin limeos-core soak::

# Everything the CI soak step runs
cargo test -p limeos-core --bin limeos-core -- --ignored soak_ --nocapture

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
faults, restarts, the executor's requests by kind, legitimate refusals (core's
logged ones as `core: …`), retention removed and the largest batch per rule,
row counts and sizes every ten days, startup page reads on copies, the largest
startup statements on the last copy with their reads in the open and cold, the
measures the pins read, and one line per check: `PASS`, `FAIL` (with its first
20 failures) or `XFAIL` (a known defect, within its pins).

A `has no budget` failure means a table was added: give it a budget in
`core_budget` or `receipt_budget`. A change to core's dispatcher, recovery,
resolution or handlers, or to the executor's container requests, is exercised
as it is; only a change to `propose_stack` needs the shim in `workload.rs`
changed with it, and `stack_proposals_match_core` says so.

## What it does not cover

- Real hardware, fsync and SD-card latency (the stores are on tmpfs).
- The real Engine and its HTTP client (covered by `docker/tests.rs`).
- The executor daemon's accept loop, peer-credential check and connection
  permits, and its role's own reads (observations, logs, storage
  inventories); the soak serves the same request handling from its own
  accept loop.
- Core's HTTP layer (`limeos_api`: routing, request framing, deadlines,
  limits): the soak calls core's handlers directly.
- The password workers: the password check behind logins and failed logins.
- The storage reader's inventory cohort behind `propose_stack` (the shim,
  checked against the real function above).
- A process killed inside a SQLite transaction: a simulated death drops a
  process between two steps (core's futures, the executor's handler at an
  await), and its connections close cleanly.
- Core's other loops: the observation runtime, reader health and the RPC
  server's own framing.
- Automation producers (not in the shipped scope); see the measured limit above.
- Session revocation under a full queue, disk-full, schema upgrades: unit tests.

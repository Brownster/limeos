# Long-uptime soak (PR-41)

The acceptance test for the first readiness condition of the
[production readiness plan](plans/2026-11-19-production-readiness-plan.md): no
counter, table or cap turns normal long-running use into a refusal that needs
hand repair. It lives in `bins/core/tests/soak/` and runs months of a household
NAS in minutes, deterministically, against the real stores.

## What it drives

| Component | How | Time |
|---|---|---|
| Core store (`limeos_persistence::Store`) | Real: opened with `Store::open` at every restart, every write through its public methods | `now` passed by the harness, as core passes it |
| Core retention | Real: `Store::prune`, once per simulated minute | The writer thread's once-a-minute gate (`maintain`) reads the wall clock, so the harness calls the pass directly |
| Receipt store (`limeos_executor_container::ReceiptStore`) | Real: opened with `open_at`; every request answered by a handler that mirrors the container arms of `bins/executor/src/main.rs` (protocol ceiling, one store call, epoch on every answer) | The store's own `clock`/`now` parameters |
| Docker Engine | Simulated container state behind the receipt store's `Engine` trait (`engine.rs`): the reference host's 18 containers in four Compose stacks; a restart takes its start time from the simulated clock | Simulated clock |
| Core's dispatcher, recovery and resolution loops | Mirrored step for step in `dispatch.rs` (`dispatch_one`, `dispatch_stack`, `complete_pending_resolutions`, `finish_resolution`, `resolve_as`), with core's `Rotation` on the simulated clock | `limeos-core` is a binary whose loops read `SystemTime` and `Instant`, so the harness makes the same store and executor calls in the same order instead of calling them |

One simulated clock drives all of it. Nothing sleeps; a seeded SplitMix64
generator decides the workload, so a run is the same every time (the stores draw
their own random IDs and epochs, which changes no outcome).

The only production change is `Store::prune` made public (it was test-only):
one retention pass without the wall-clock gate.

## Workload

Rates are in `workload.rs`, with their reasons. Normal days:

| What | Rate |
|---|---|
| Logins (2 administrators) | 8 a day; 40% log out, the rest expire after 8 h |
| Failed logins | 3 wrong passwords and 1 unknown name a day |
| Dashboard and job-progress polls | every 30 s for 15 min after a login; every 2 s for 30 s after a job is queued |
| Container proposals | 10 a day (restart 85%, stop 15%, start a stopped one); 8% expire unapproved, 3% canceled, 2% approved too late, 2% never queued |
| Stack actions | 2 a day over four stacks |
| Faults per dispatched job | about 6%: execute answer lost; executor dead with the execute unread; executor stopped between `ReceiptEpoch` and the execute; executor killed inside the Engine call (after the `prepared` insert, which also prunes once the store is full); Engine error before or after acting; core restart after the claim, after the execute answer, after the executor's verify; recovery lookup lost |
| Operator reviews | four a day; every `needs_intervention` job resolved on fresh evidence (10% of decisions interrupted by a core restart) |
| Containers restarted outside LimeOS | 2 a day |
| Restarts | both daemons every night at 04:00, measured; plus every fault above |

Nights are quiet: the daily checkpoint runs at 04:00 and the weekly idle hour at
02:00. Activity outside the store (observation polls of the Engine, password
workers, the login limiter) writes nothing and is not modelled.

At household rates no budget fills within 90 days, so two bursts, each within
what the code admits, saturate the tables early and the last two months run
against full tables:

- Day 20, 06:00 to 16:00: a runaway client logs in and out twice a second (the
  two password workers cap logins near four a second on a Pi): 144,000 events.
- Day 25, 08:00 to 20:00: a scripted client proposes a container action every
  40 s and a stack action every 45 s.

## Results of the 90-day run

Measured on 20 November 2026 (debug build, x86-64, tmpfs), 140 s of wall time;
the smoke run takes 2 s. The run is deterministic: two runs give the same
counts.

| Over 90 days | |
|---|---|
| Jobs | 2,322: 2,151 succeeded, 112 refused or stale (`precondition_changed`), 2 partly done (`failed`), 57 resolved by the operator |
| Logins | 73,064 (72,000 in the burst), 427 failed attempts |
| Events written | about 159,500; 28,458 pruned; peak 131,313 rows against the 131,072 budget |
| Executor | 2,172 executes; 1,714 action and 256 stack rows kept; prune watermark from day 25.6 |
| Faults | 141 injected (and 185 container restarts outside LimeOS): 40 core restarts at awkward moments, 48 executor deaths or stops (6 right after an insert that pruned), 31 lost execute answers, 22 Engine errors |
| Restarts | 180 nightly (90 of each daemon) plus 88 at awkward moments |
| Refusals | 0 lifetime refusals; 12,112 lock conflicts, 22 late approvals, 427 bad logins and 51 expired sessions, all legitimate |

| Day | core.sqlite (WAL) | events | jobs | receipts.sqlite | core open: reads, debug time | receipts open |
|---:|---:|---:|---:|---:|---:|---:|
| 9 | 1.1 MiB (1.0) | 729 | 96 | 0.19 MiB | 1.1 MiB, 18 ms | 204 KiB |
| 27 | 31.3 MiB (1.0) | 131,072 | 1,664 | 2.08 MiB | 74.9 MiB, 175 ms | 2,164 KiB |
| 54 | 33.0 MiB (1.1) | 131,072 | 1,950 | 2.08 MiB | 82.6 MiB, 207 ms | 2,316 KiB |
| 89 | 35.1 MiB (1.1) | 131,072 | 2,311 | 2.16 MiB | 93.1 MiB, 251 ms | 2,508 KiB |

Every idle hour (twelve) made no write call. Every bounded table stayed flat
over the last third.

### The defect it found: core.sqlite has no size bound

The events table is flat at its budget, but the per-job tables (`jobs`,
`job_resources`, `container_plans`, `container_results`, `stack_plans`,
`stack_results`, `job_resolutions`) are never pruned: about 5 KiB per job, about
50 KiB a day at household rates. core.sqlite passes its 80 MiB bound in about
2.5 years. Startup grows faster than the file: `Store::open` reads about 2.5
times the file (quick_check 55 MiB, the foreign-key check 20 MiB and the audit
count 4 MiB of a 35 MiB file), and its reads grew by 0.3 MiB a day, reaching the
read bound of a store at its file bound in about 2.4 years. No refusal follows
(no cap counts these rows), so it is not yet a cliff that needs hand repair, but
neither size nor startup is bounded. Both checks are marked as expected failures
in `checks.rs` until job-row retention lands.

### The automation limit, measured

`automation_limit_at_minimum_interval` enables one observe-health schedule at
the 30 s minimum and observes every occurrence through the store's real paths,
with retention once a simulated minute. The first refused write comes after
**22.76 simulated days** (65,535 observations): the reservation is refused
with `StateNotDurable` once the events table reaches the 64 MiB audit cap
(262,143 events, almost all of them pinned automation proofs), with `automation_observation_results`
one row short of its 65,536 cap and core.sqlite at 202.6 MiB. That matches the
estimate of about 23 days in `retention.rs`. The run takes about 14 minutes (its
retention passes get slower as unprunable history grows) and up to 210 MiB of
disk, so it is not part of CI. It fails once the limit is gone, as a reminder to
drive the producer in the main soak instead.

## Assertions

| Check | What it catches |
|---|---|
| no lifetime or unexpected refusal | `StateNotDurable`, `Overloaded`, `BeyondReceiptHistory` from any store call or executor answer, anywhere; and any refusal a call site does not list as legitimate |
| provoked refusals observed | the run itself provokes an approval after expiry, a session used after expiry, wrong passwords and unknown names: each must be refused as such |
| database files under their bounds | core file 80 MiB and WAL 8 MiB; receipt file 8 MiB (half its 16 MiB cap) and WAL 2 MiB, at every daily checkpoint |
| every table within its budget | every table in both stores, found by walking `sqlite_master`: a table added later fails until it has a budget in `checks.rs`; per-job tables may hold a bounded number of rows per job |
| sizes and bounded tables flat across the last third | the fitted growth of the bounded tables' bytes, each bounded table's rows by name, and the file size beyond what per-job tables added, over days 60 to 90 |
| slow growth leaves at least five years | the per-job tables' growth rate over the last third, projected to the core file bound |
| startup under its bounds | every measured `Store::open` and `ReceiptStore::open_at` (schema, `quick_check`, startup invariant checks, recovery), about 130 of each in a run: at most 5 s (core) and 2 s (receipts) of debug-build wall time, and at most 4 bytes read per byte of file and WAL (measured: 2.5 for core, 1.0 for receipts) |
| core / receipt-store startup work leaves at least five years | the bytes the daily opens read (per-thread `/proc/thread-self/io`, deterministic where CI wall time is not), fitted over the last third and projected to the read bound of an open of a store at its file bound |
| an idle hour writes nothing | once retention has nothing left, an hour of dispatcher and retention passes (recovery polls of held jobs included) makes no write call |
| locks, ledger and retained history consistent after every restart | after every restart and checkpoint: locks match job states; every job the workload queued exists and no other does; unsettled and contested jobs keep their history; the executor holds no lock for a job core has released |
| every job settled; none lost, duplicated or stranded | after a drain, every job is terminal and no resolution request is open |
| every Engine effect ran at most once and matches its job | per action: a succeeded job ran its effect exactly once per target, a refused or canceled one never, a `not_done` resolution never |
| authority events all kept | every authority event the run wrote is still there |

## Running it

```sh
# Smoke: three simulated days, a few seconds, part of every cargo test
cargo test -p limeos-core --test soak

# The 90-day acceptance run (CI runs it in both native-architecture jobs)
LIMEOS_SOAK_DIR=/dev/shm cargo test -p limeos-core --test soak -- --ignored --exact soak_ninety_days --nocapture

# The automation limit, measured (not a gate)
LIMEOS_SOAK_DIR=/dev/shm cargo test -p limeos-core --test soak -- --ignored --exact automation::automation_limit_at_minimum_interval --nocapture
```

`LIMEOS_SOAK_DIR` picks where the temporary stores go (default: the system temp
directory); a tmpfs keeps fsync cost out of the run time. The stores are deleted
when the test ends. Peak disk use is under 60 MiB for the soak runs and about
210 MiB for the automation measurement.

## Reading a failure

A violation is printed when it happens, as `[profile] check: day D HH:MM:SS: what`,
and the run continues. At the end the report shows the job outcomes, faults,
restarts, legitimate refusals, row counts and sizes every ten days, startup work
and time, and then one line per check: `PASS`, `FAIL` (with its first 20
failures) or `XFAIL` (a known defect listed in `EXPECTED_FAILURES` in
`checks.rs`, with its evidence). A listed check that passes fails the run, so a
fixed defect cannot stay marked.

The run is deterministic: rerun the same profile to reproduce. A `has no budget`
failure means a table was added: give it a budget in `core_budget` or
`receipt_budget`, or an allowlisted reason if it is meant to grow. A change to
core's dispatcher, recovery or resolution paths must be mirrored in
`dispatch.rs`, or the soak no longer covers it.

## What it does not cover

- Real hardware, fsync and SD-card latency: the stores run on a tmpfs.
- The real Engine and its HTTP client (covered by `docker/tests.rs`), and the
  executor daemon's socket, peer checks and store mutex (`try_lock` contention).
- Real time: core's own loops (`operations.rs`, `stack_operations.rs`,
  `job_resolution.rs`) are mirrored, not executed, and empty dispatcher ticks
  are skipped (a tick with nothing queued cannot change anything until a job is
  queued or a recovery pause ends). The wall-clock paths left: the writer
  thread's retention gate and the timestamp of startup recovery events.
- A process killed inside a SQLite transaction: a simulated crash drops the
  process between two steps and its connections close cleanly.
- Automation producers (not in the shipped scope); see the measured limit above.
- Session revocation under a full queue, disk-full, schema upgrades: unit tests.

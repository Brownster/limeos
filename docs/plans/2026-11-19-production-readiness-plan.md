# Production readiness plan

Status: proposed, 19 November 2026. Source: the whole-repository review of `6043cba`
(seven subsystem reads plus local gates: fmt, clippy `-D warnings`, 1933 tests green).
Scope: the agreed basic cutover surface (host metrics, container status/logs/actions,
stack status/actions) on wybie. Storage mounts, backup/restore, schedules and the
assistant stay deferred; this plan only stops them from costing anything meanwhile.

Findings marked ✔ were re-checked in the code by the integrator. The rest come from
reading only and need a failing test before the fix is accepted.

## What "production ready" means here

The build may replace the frozen Python app on wybie when all of these hold:

1. **No lifetime cliff.** No counter, table or cap turns normal long-running use into
   a refusal that needs hand repair. Proven by a compressed-time soak (PR-41).
2. **Degrade per item, not per feature.** One odd container, disk or mount hides
   that item with a reason; it never blanks the inventory.
3. **One failed subsystem does not stop the control plane.**
4. **Cutover is interruptible.** Any signal, SSH drop or prompt during activation
   ends in the pre-cutover state.
5. **The trust model is written down and matches the code.**
6. **Footprint budgets are CI gates,** not one-off measurements.

## Phase 0 — Cutover blockers

Small, independent fixes. Each lands with a regression test that fails first.

| Id | Change | Done when |
|---|---|---|
| PR-01 ✔ | `activate.sh`: trap INT, TERM, HUP and EXIT once armed and run the existing rollback. Run activation under `systemd-run` or `nohup` so an SSH drop cannot kill it mid-step. | Stub test kills the script at each of the 14 mutating steps with each signal; snapshot state is restored every time. |
| PR-02 ✔ | `activate.sh:184`: install with `DEBIAN_FRONTEND=noninteractive` and `--force-confold`; install only the named, version-checked `.deb`, not every file in the directory. | VM rehearsal with a pre-existing differing `/etc/limeos/core.json` completes without a prompt and keeps the operator's file. |
| PR-03 | `lib.sh:109`: preflight fails when every default state path is `ABSENT`; state paths come from the `migrate scan` report, not a hardcoded home directory. | Test: empty root fails preflight with a named reason. |
| PR-04 | Health check tolerates restarts of containers outside the managed set and names the container that changed. `return.sh` restores Rust unit states from the snapshot instead of disabling unconditionally. | Stub tests for a watchtower restart during the window and for pre-enabled Rust units. |
| PR-05 | Jellyfin playback gate takes an API key from a root-only file and refuses on active sessions; without a key it requires an explicit `--no-playback-check`. | Tests for active, idle and no-key cases. |
| PR-06 | CI: shellcheck `scripts/cutover/*.sh`. | Workflow step present and green. |
| PR-07 ✔ | `docker.rs:147`: map Engine 404 to `NotFound` and 409 to `Conflict`; treat 304 on start/stop as success. | HTTP-level tests against a fake Engine socket for 204, 304, 404, 409, 500. A job whose container was removed can be resolved. |
| PR-08 ✔ | `docker.rs:40`: keep the newest bytes of the log tail. Fix the PEM filter that drops everything after a single-line BEGIN/END. | Test: 200 lines over the limit returns the last lines, `truncated=true`. |
| PR-09 | Login limiter: key on username plus a small global bucket, charge after body validation, and do not charge successful logins. | Test: ten junk requests do not block a valid login. |

## Phase 1 — Reliability over a long uptime

### Retention (one pattern, used everywhere)

Add a single retention module in `persistence`: each bounded table declares a row
or byte budget and a prune rule that never removes rows referenced by an open job,
lock or unresolved intervention. Pruning runs in the writer thread, in small
batches, at most once a minute, and writes nothing when there is nothing to prune.

| Id | Change | Done when |
|---|---|---|
| PR-10 ✔ | `events`: prune settled history beyond the budget and decrement `audit_bytes` by what was removed. The 64 MiB cap stays as a last-resort stop, with an alert at 75%. Stop emitting events for revocations that matched no row. | Soak test writes ten times the cap; store keeps opening and writing. |
| PR-11 | Container executor receipts (`lib.rs:269`, `stack.rs:16`): prune terminal receipts beyond a budget; keep prepared and unknown ones. | Test: 20,000 completed actions, the next one still succeeds. |
| PR-12 | Automation occurrences: add the pruner before any producer is wired. | Test at the minimum 30 s interval, simulated for 30 days. |
| PR-13 | Index `events(job)`; measure the startup integrity check at 250k events and 5k jobs on the Pi. | Core ready in under 1 s at that size, or the check moves out of the startup transaction. |
| PR-14 | Session and task revocation bypass the free-space and audit gates and the bounded queue. | Test: revoke succeeds with the audit cap hit and the queue full. |

### Per-item degradation

| Id | Change | Done when |
|---|---|---|
| PR-15 | Container inventory over 64: return the first 64 by a stable order with `truncated`, not an error. Count running containers first. | The scale suite's 65-container case returns data plus a flag. |
| PR-16 | Disk observation: skip loop/rom before the bound; a duplicate UUID or serial marks those devices `ambiguous` and keeps the rest. `[SWAP]` is not a mountpoint. | Fixtures for btrfs members, cloned serials and a swap partition. |
| PR-17 | Engine-wide double inspection: compare per container; a container that changed between passes is reported `changing`, the others are returned. Unknown mount types and labels mark one container unsupported. | Fixture with one crash-looping container returns the other stacks. |
| PR-18 | Derive the mountinfo and task caps from the measured wybie census plus headroom, and state the supported size in the operator docs. | Census recorded; caps and docs agree. |

### Fault containment

| Id | Change | Done when |
|---|---|---|
| PR-19 | Core: the observation runtime restarts with backoff on error and reports itself unhealthy; it never ends the process. | Fault injection: store error during a pass, HTTP keeps serving. |
| PR-20 | Storage reader: move blocking sysfs and block-device reads to `spawn_blocking` with a deadline; add `WatchdogSec` and `sd_notify` to resident units. | A hung read is abandoned and the unit recovers without operator action. |
| PR-21 | Separate the permits shared by stack refresh, health and storage inventory, or queue with a deadline instead of `try_lock`. | Overlapping requests no longer mark the stack cache stale. |
| PR-22 | Per-request timeouts in place of the 30 s whole-connection timeout; SSE readers capped per session. | Keep-alive test across the 30 s mark; one session cannot take all eight streams. |
| PR-23 | Migrations: one ordered table used by both `open` and the schema check; snapshot the database file before migrating; check the `PRAGMA journal_mode` result; verify the `jobs` table SQL after v16. | A migration added to the table alone is applied and checked. |
| PR-24 | `postrm`/`prerm`: tolerate already-removed or never-enabled units; restart optional units that were running before an upgrade. | VM: remove twice, upgrade with the reader running. |

## Phase 2 — Security

| Id | Change | Done when |
|---|---|---|
| PR-30 | ADR: state plainly that the executor trusts core within the root-owned ceiling and does not verify approvals. Add what the executor can enforce alone: a per-resource action rate limit and an executor-side append-only action log core cannot rewrite. | ADR merged; tests for the rate limit; the threat model says what a compromised core can do. |
| PR-31 | Every ceiling loads through the no-follow walk behind `read_protected_json` (each path component opened without following symlinks, root-owned, not group- or world-writable) and is reread per request: at accept, as a peer gate before any frame, and after the frame, which alone authorizes the request; a request already authorized runs to its end. Every ceiling rejects `core_uid == 0`; a ceiling that cannot be read or admitted answers `Unavailable`, never `Forbidden`, so a half-written file cannot withdraw an operator's resolve request. `limeosctl check-ceiling FILE CORE_UID ROLE` runs the executor's walk and role check. Before: the shared host, container and reader ceiling was read once at start with an lstat of the final component only and accepted `core_uid` 0; the target ceiling was walked but read once at start; the mount ceiling already met this. Container ceilings already key on full container IDs, which change on every recreate; whether to key on names instead is an operator decision (below). | Revoking a container takes effect without a restart. Code and tests on branch `pr-ceilings`, 23 November; the names-or-IDs decision is still open, and so is whether lookup, verification and resolution of an existing receipt should keep checking the managed list (a revocation strands open actions; ADR 0013). |
| PR-32 | systemd: `limeos-storaged` and the storage reader run as a dedicated user with only the capabilities they use; add `ProtectHome` and `PrivateTmp` where mount identity allows. For the dormant mounts unit, hold `CAP_SYS_ADMIN` only in the helper. | `systemd-analyze security` score recorded per unit and gated in CI. |
| PR-33 | Log redaction: add cookies, bearer and JWT shapes and cloud key prefixes; match `token`/`secret` as keys, not substrings. | Table-driven tests for both leaks and over-redaction. |
| PR-34 | `limeosctl job resolve` requires `--evidence-digest` or an interactive confirmation; `--help`/`--version` are parsed only as the first argument. | CLI tests. |
| PR-35 | Uniform error envelope for query-extractor rejections; `Backend` methods without a default so an unwired route fails to compile; one session-check helper used by every handler. | No route returns axum's plain-text rejection; a route-table test asserts every mutating route checks session, CSRF and scope. |

**Not done in PR-32** (the row above is unchanged; these two parts remain open):

- *The storage reader still runs as root.* Core accepts storage evidence only from a UID 0 peer (`fresh_storage` in `bins/core/src/storage.rs`), and the reader and its sibling roles refuse a non-root effective UID. A dedicated user needs core to trust that user instead (as it trusts the container executor: the socket's owner, neither root nor core, must be the peer), and the reader to keep the device and protected-path reads it does as root, through file ownership, group access or a narrow capability.
- *`CAP_SYS_ADMIN` is not held only in the mounts helper.* For a root service the bounding set gives `CAP_SYS_ADMIN`, `CAP_SYS_PTRACE` and `CAP_DAC_READ_SEARCH` to every child of the dormant mounts unit (its unit file records the gap). Confining it needs either a separate helper unit that alone holds the capability, reached over its own socket, or the role dropping it from its bounding and permitted sets in code before it starts anything but the helper.

## Phase 3 — Patterns and scope

| Id | Change | Done when |
|---|---|---|
| PR-40 | Remove from the default build everything with no production caller: `backup-archive`, the SnapRAID and catalog planners, `container_updates`, and `bins/executor/src/updates/` ✔ (never compiled today). Move them behind a `deferred` workspace member or feature that CI still tests. | The shipped binaries contain only the basic-cutover surface; binary size and test time recorded before and after. |
| PR-41 | Soak harness: a fake clock and fake Engine drive 90 simulated days of jobs, logins, polls and restarts in CI, asserting no refusal, bounded database size and bounded startup time. | Runs on every push in under ten minutes. |
| PR-42 | Domain newtypes for absolute path, resource id, UUID and plan lifetime, with private fields and `TryFrom` on deserialise. Replace the eight path validators and six id validators one module at a time. | One validator per concept; public structs cannot hold an unvalidated value. |
| PR-43 | One receipt-journal implementation shared by container actions, storage targets and mounts; one private-object-tree implementation in `backup-archive`. | Duplicates deleted; existing crash tests pass unchanged. |
| PR-44 | Evidence policy: keep summaries and hashes in git; move raw logs and archives (80 MB today) to release assets. Replace the roadmap's status header with a link to the latest status file. | Tracked `docs/` under 10 MB going forward. |

## Phase 4 — Footprint gates

Budgets are the architecture document's targets; the gate is `measure.sh` run in
the installed VM in CI, with the Pi night as confirmation.

| Measure | Budget | Last recorded | Action |
|---|---|---|---|
| Core plus base executors, idle PSS | 30 MiB | 10.44 MiB (Pi guest, 7 Oct); 13.2–13.5 MiB fresh; settled on a month-old used store 15.78 MiB before the cache change, 13.94–14.09 MiB after (amd64 guest, 23 Nov) | **Gated at 15 MiB on the settled state** (below). |
| Idle CPU | 0.1% of a core | 0.051% earlier; 0.203–0.205% (laptop amd64 guest with the stack reader, 23 Nov), 0.148% (GitHub runner's guest) — **budget not met** | **Gated at a 0.3% regression ceiling**, not the budget (below). Re-measure with the 1 Hz automation pass enabled. |
| Resident disk writes | 20 MB/day | 4.72 MB/day was a fresh store's; 7.07 MB/day settled on a month-old store (amd64 guest, 23 Nov) | **Gated at 13 MB/day on the settled state** (below). |
| Cold readiness | 1 s | 457 ms; 111–167 ms (amd64 guest, 23 Nov) | **Gated at 1 s** as readiness after `systemctl restart` (warm cache, the stop included; below); a true cold start is not measured. Repeat at the PR-13 database size. |
| Cached overview p95 | 20 ms | 0.83 ms (AMD64) | Not gated in CI: gate on the Pi figure. |
| Storage inventory p95 | 20 ms | 61.4 ms — fails | Not gated. Profile; cache topology between udev events, or set a separate reasoned budget. |
| Mount admission | 4 s window | 3.0–4.6 s — fails | Not gated; deferred with the feature; do not widen the window without a profile. |

Add: per-request round trips to the store reduced from 2–4 to 1 (session check
and authorisation in one call), and a matched Python measurement on wybie so the
improvement claim has a baseline.

### What CI gates (23 November)

`tests/privileged_vm/ci_footprint_guest.py` runs last in the gating `debian-vm`
job. The guest is left as the basic cutover leaves a host: Docker with a Compose
project (four running members, one created) and two standalone containers, the
candidate installed from the signed test repository with one enrolled account,
the management catalog from `limeosctl migrate propose-catalog --install`, and
the stack reader started, not enabled. After a settle (90 s in the first round, now 360 s; see below), the unchanged
`scripts/cutover/measure.sh` runs three consecutive 200 s idle windows, then three
`--measure-core-restart` runs. The gated figures are measure.sh's own row values:
PSS per window, and CPU and disk writes as each window's row weighted by its
elapsed time:

First round, on a fresh install (superseded by the settled state below):

| Gate | Threshold then | Statistic | Local guest runs 1 / 2 / 3 / 4 |
|---|---|---|---|
| Core plus base executors, idle PSS | ≤ 15 MiB | every window | 13.48 / 13.24 / 13.48 / 13.32 MiB |
| Resident disk writes | < 10 MB/day | all 600 s together | 4.716 / 4.716 / 4.715 / 4.716 MB/day |
| Idle CPU | < 0.3% of a core (was 0.4%) | all 600 s together | 0.2032 / 0.2032 / 0.2049 / 0.2049 % |
| Core ready after a restart | < 1000 ms | median of three (includes the stop) | 122 / 124 / 133 / 129 ms |

Figures over the whole 600 s, not per window: core's metrics store flushes every
five minutes, so a 200 s window holds zero or one flush (the windows read 0.0,
7.07 and 7.07 MB/day each run) while the three windows normally hold two flushes
(4.72 MB/day). About 2% of runs, by estimate, the flush timing puts a third flush
inside them (7.07 MB/day): today that is still well under the gate, but a future
increase in flush size would first show as an intermittent failure, not a steady
one.

A row value alone could pass vacuously, because measure.sh sums only the
processes it found. So every window must also show each base unit (lib.sh's
`RUST_UNITS`, the list measure.sh uses) and the stack reader with its systemd
main process measured at both ends (PSS above zero, readable CPU and write
deltas), and the per-process sums must equal the rows. `systemctl show MainPID
NRestarts` is taken before and after each window, and a changed main process or
restart count fails the run. A process that comes or goes during a window fails
it unless it is a helper of the stack reader, not its main process, which the
reader starts for its 30 s stack refresh. A helper's CPU is not lost: the reader
waits for it, so its whole CPU time joins the reader's cutime and cstime, which
measure.sh counts, in the window where it ends. Only a helper still running at
the last sample is left out, and the result records its ticks. Over the whole
run, core and the reader must have used CPU and core must have written something.
`tests/reference/test_footprint_guest.py` checks all of this offline against
synthetic measure.sh reports, and it runs under Debian 12 too.

Every guest call has a bound (a window 380 s, a restart run 120 s, the apt steps
120 to 300 s), and the suite has a 20-minute budget of its own. A hang therefore
ends the suite with its result written, not the job by cancellation. The suite
prints a line per window and per restart, and its result, pass or fail, carries
per-window and per-unit CPU, writes and PSS. The local runs were on a 2-vCPU,
1 GiB KVM guest on an i5-10310U laptop. The suite adds about 13 minutes to the
job (fixture 30 s, settle 90 s, windows 600 s, restarts 50 s, plus boot), so the
job's limit went from 30 to 45 minutes.

- **The idle CPU budget is not met.** 0.203 to 0.205% of a core against 0.1%, in every run
  and every window (0.18 to 0.225%). No reader helper was caught at a window
  boundary in the twelve windows. The stack reader takes 0.12% of it, which
  confirms the attribution, core 0.07%, storaged 0.01%, containerd 0.003%.
  The gate is a regression ceiling, not a widened budget. The first CI run gave
  the runner's figure: 0.148% (windows 0.145 to 0.155%; reader 0.058, core
  0.075, storaged 0.015). The ceiling is now 0.3%: 1.46 times the laptop's
  0.205% and twice the runner's, so a doubling of idle CPU on the runner fails. The suite's result says `budget_met: false`.
- **PSS has 1.5 MiB of headroom here,** not the 4.5 MiB the Pi figure suggests:
  core alone is 10.2 MiB in this guest and each executor 1.6 MiB, steady to
  within 0.05 MiB across windows. Growth past 15 MiB fails the job, as intended.
- **An unchanged poll writes zero bytes** is not expressible in measure.sh, which
  measures windows, not polls. The check job gates it with
  `stable_real_runtime_passes_leave_process_write_bytes_unchanged`. In the guest,
  only core wrote anything (32 KiB per 600 s); the executors and the reader wrote
  nothing.
- **Not gated in CI:** idle swap (recorded; the guest has no swap), all-resident
  PSS and the installed footprint (recorded in the window reports, not Phase 4
  rows), the two HTTPS p95 rows (no proxy in the guest; cached overview p95 stays
  a Pi gate), storage inventory p95 and mount admission (failing or deferred, and
  not measured by measure.sh).

### Core's memory and the settled state (23 November, second round)

The first CI run failed the PSS gate: core stepped from 10.37 to 12.36 MiB in
its third window, idle, while the executors stayed put. Traced in the VM:

- **What core's PSS is.** On a fresh install, idle for 90 minutes (180 samples),
  core settles at 10.36 MiB: 7.2 MiB is its own code (the binary's text,
  file-backed and clean, which the kernel can drop under pressure), 2.2 MiB heap,
  0.55 MiB other anonymous memory, 0.24 MiB libc and libm. It grew 130 KiB in
  40 minutes and then stayed flat. glibc's `malloc_trim` gave back only 0.45 MiB,
  so the allocator does not hold much freed memory.
- **The step was a transparent huge page.** The Debian 12 guest runs THP
  `[always]`. When an aligned 2 MiB block falls inside core's heap, a fault or
  khugepaged backs all of it, used or not: in eight restarts with khugepaged
  scanning fast, every one showed 2 to 4 MiB of `AnonHugePages`. Whether the heap
  holds an aligned block depends on address randomisation, which is why one run
  in CI stepped and none of the laptop's did. **The production Pi's kernel has no
  THP** (6.12.25+rpt-rpi-2712, 16 KiB pages), so this cannot happen there. The
  suite now sets THP to `never` in the guest before LimeOS starts, records
  `AnonHugePages`, `Pss_Anon` and `Pss_File` per unit per window, and fails if a
  unit still has huge pages. On the Pi, PSS moves in 16 KiB pages, not 4 KiB.
- **What grows with time: SQLite page caches.** No connection set `cache_size`,
  so each kept SQLite's 2000 KiB default. Each open-time `quick_check` reads
  every page, so the cache fills from the first minute on a grown store and
  stays filled. With a month of metrics (`metrics.sqlite` 2.2 MB) core started
  at 13.65 MiB under THP, and at 12.4 MiB with THP off (`Pss_Anon` 4.65 MiB
  against 2.8 MiB fresh). Core plus base executors came to 15.77 to 15.78 MiB,
  over the 15 MiB gate. After a 60 s burst of 8 clients, core sat 0.7 MiB higher
  for good (arenas and code pages).
- **The change** (`crates/persistence/src/cache.rs`, `telemetry.rs`, the
  receipt store): each connection gets a cache sized to its work. The
  `core.sqlite` writer has 1024 KiB. A full retention batch (1024 events)
  writes 72 pages, and the same WAL bytes as with the default, so nothing
  spills. A migration keeps the default until it commits. The reader has 512
  KiB, and `metrics.sqlite` and the container receipt store have 256 KiB each
  (a full prune batch writes 26 pages, the same WAL bytes as with the
  default). The open-time checks now read through the small caches. The soak
  passed all five long tests. Its only moved figures are startup page reads:
  52 to 54 statements (the two cache pragmas), the core open's page reads per
  page of file at median 1.88 to 2.12 and at most 2.32 to 2.73 (bound 3.2), and
  "in the open beyond the statements cold" from at most -5 to at most 283 and
  367 pages (bound: 3% plus 256 pages). Each statement reads the same cold.
  Metrics writes (113 KiB an hour) and idle-hour writes (none) are unchanged.
- **Estimate for a `core.sqlite` near its cap** (events at their 32 MiB budget).
  This is an estimate, not measured. Before the change the writer and the
  reader could each hold about 2 MiB, so core would sit near 16 MiB and core plus
  base near 19 to 20 MiB. With it they hold at most 1 MiB and 0.5 MiB, about
  2.5 MiB less.

**The gate now measures a settled state.** Before the windows, the suite fills
`metrics.sqlite` with 31 days of samples: it stops core, inserts the samples
and starts core again, so the measured process opened a month-old store.
Postinst starts core on install, so this is core's second start, not its
first. The suite then runs a use pass (login, overview, resources, 30-day and
24-hour history) before the settle. With the settle lengthened to 360 s, past
core's first metrics flush after that restart (below), the suite takes about
1050 s, about 18 minutes with the guest's boot; the job's 45-minute limit
holds it with the other suites' 15.

**Before and after, on the same month-old, used store.** Both runs used the
same fixture: THP off, 31 days of metrics, the use pass. "Before" is CI's
package of production-readiness `c5b654a8`, the same code without the cache
change but with the ceilings merge (PR-31). "After" is CI's package of this
branch at `3298cff5`. Each was traced for 600 s of memory, then 1000 s under
strace with core's `/proc/PID/io`:

| | Before | After |
|---|---|---|
| Core PSS | 12.19 MiB | 11.06 MiB |
| Core heap, other anonymous | 3.64, 0.71 MiB | 2.09, 0.84 MiB |
| Core `Pss_Anon`, `Pss_File` | 4.65, 7.55 MiB | 3.23, 7.83 MiB |
| Core plus base executors | 15.78 MiB | 14.22 MiB |
| Metrics flush | 2 WAL frames, 24 KiB of `write_bytes` | the same |
| First flush after a start | 48 KiB | the same |

The heap is 1.55 MiB smaller. The file-backed and executor differences come from
the two builds being different binaries, not from the cache; the receipt store
here is small. The smaller cache changed no write: the same frames, the same
pages and the same bytes per flush.

**Where the disk writes go.** Core samples host metrics once a minute
(`crates/observations/src/collectors.rs:192`). The telemetry thread flushes
when it holds five samples (`telemetry.rs:70-87`), in one transaction:
`INSERT OR REPLACE` of the five rows (`:137`), then `DELETE FROM samples WHERE
at<now-31 days` (`:148`). `samples` has no index besides its rowid (`at`).
`synchronous=NORMAL` means no fsync per commit, and `wal_autocheckpoint=128`
(`:40`) checkpoints once the WAL holds 128 frames.

On a month-old store each flush wrote exactly two frames in the WAL, read back
from its frame headers: one for the leaf holding the newest rows (the insert)
and one for the leaf holding the oldest (the delete). That is 4 `pwrite64`
calls, 8,240 bytes, no fsync. Index maintenance is nil, and there was no
checkpoint in 1,600 s; one comes every 64 flushes (about 5.3 hours), copies the
few distinct pages back and adds about 12 KiB. `/proc/PID/io` `write_bytes`
counted 24,576 bytes per flush, every time:

- the two frames, at unaligned WAL offsets, dirty three file pages (12 KiB);
- the WAL index (the mmapped `-shm`, no system call to see) is inferred to
  account for the rest, about 12 KiB.

A fresh store's flush has no delete frame: 16 KiB, which is the long-standing
4.72 MB/day. A month-old store's is 24 KiB, 7.07 MB/day. The first flush after
core starts writes 48 KiB, because the WAL and its index are created again.

The earlier 10.6 MB/day (windows 0 / 21.2 / 10.6) was that first flush falling
inside the windows: the suite had restarted core around the seeding 90 s
before them. It was not aliasing alone. The settle is now 360 s from the use
pass, past that flush. The 600 s still normally holds two flushes and rarely
three (10.6 MB/day at the steady rate), which the threshold allows for.

**Reductions, proposed and not applied.** Each changes the soak's metrics
assertions (rows kept, "a flush is five samples"), so none is a small change the
soak already covers:

- Flush every 15 samples instead of 5. One transaction then writes the same two
  frames for three times the rows, about 2.4 MB/day instead of 7.1. The cost is
  up to 15 minutes of history lost on a crash instead of 5; pending samples are
  already served from memory.
- Delete the oldest rows once an hour instead of at every flush. This drops the
  delete frame from 11 flushes in 12, about 1.5 to 2.5 MB/day.
- `locking_mode=EXCLUSIVE` on the metrics store would keep the WAL index in heap
  memory instead of the mmapped `-shm` file, saving its share (inferred about
  12 KiB a flush). It is rejected: it locks out every other connection,
  including the soak's checker.
- A larger `wal_autocheckpoint` saves almost nothing, because checkpoints are
  already rare.

**The settled gates**, from three runs of the final suite on the branch's
package (THP off, a month of metrics, the use pass, 360 s settle; 1040 to 1070 s
each):

| Gate | Threshold | Runs 1 / 2 / 3 |
|---|---|---|
| Core plus base executors, idle PSS (every window) | ≤ 15 MiB | 14.05 / 13.96 / 14.09 MiB (core 10.74–10.81 MiB) |
| Resident disk writes (600 s) | < 13 MB/day | 7.074 / 7.074 / 7.073 MB/day |
| Idle CPU (600 s) | < 0.3% | 0.2032 / 0.2032 / 0.1999 % |
| Core ready after `systemctl restart` (median) | < 1000 ms | 171 / 139 / 141 ms |

- **PSS, 15 MiB.** The highest settled figure seen was 14.22 MiB, in the trace
  run. The threshold is 0.78 MiB above that, about three times the largest
  run-to-run swing (0.28 MiB, mostly the executors' shared code pages), so a
  regression of a MiB fails. The number equals the old gate, but it now rests
  on a measured plateau.
- **Disk, 13 MB/day.** The steady figure is 7.07. Three flushes in one 600 s
  give 10.6, and adding a checkpoint gives about 12.4; 13 MB/day passes those
  alignments, fails a flush a third larger, and stays under the 20 MB/day
  budget.
- **`AnonHugePages`** was 0 for every unit in every window.

**Startup reads with the smaller writer cache.** The soak comparison above
showed where the reads moved. On the soak's largest store (28 to 31 MiB, days
27 to 54), core's open read 2.62 to 2.72 pages per page of file against 2.32 to
2.44 before, 85% of the 3.2 bound. Over days 63 to 89 the figure was unchanged
(1.75 to 1.93).

"In the open beyond the statements cold" reached 367 pages against a bound of
about 711 (3% of 15,185 plus 256), which is 52%. The open took the same time as
before (74 to 114 ms against 74 to 111 ms). The extra reads are re-reads of
pages the open itself just read, so they come from the kernel's page cache, not
the SD card, unless memory is short. A store near the 64 MiB cap is not built by
the soak. If its ratio grew the way the job-heavy copies' did, it would come
close to the 3.2 bound.

Running the check with the default cache and shrinking afterwards would bring
the reads back. In a test with Debian's Python and SQLite on the 2.2 MB metrics
store, the default-cache check added 2.0 MiB of anonymous memory and
`shrink_memory` gave back only 0.6 MiB of it: glibc keeps the freed pages. That
would forfeit most of the saving, so the check stays on the small cache. The
margin is recorded here, to revisit if the bound is approached.

The architecture budget for core plus base executors stays 30 MiB.

### Shell and Python suites on Debian 12

The `debian12-suites` job runs `scripts/debian12_suites.sh` in a `debian:12`
container pinned by digest, and publishing needs it. It runs shellcheck over
`scripts/cutover/*.sh` and the maintainer scripts, then, as an unprivileged user,
`tests/cutover` (169 tests), `tests/repository/test_maintainer_scripts.py` (63)
and `tests/reference` (162, with the footprint suite's offline tests) under Debian's bash 5.2, mawk, coreutils, procps and
Python 3.11, with en_GB.UTF-8 generated so the collation case runs. All passed,
none skipped, on the second laptop's Docker (about nine and a half minutes).
None needs root, systemd or git history; git is installed only for a test that
runs `git init` in a temporary directory.

Left out, and why:

- `tests/repository/test_unit_exposure.py` and `scripts/check_unit_exposure.py`:
  `systemd-analyze security` scores differ between systemd versions (the ceilings
  are recorded under the runner's 255, Debian 12 has 252), and the gate compares
  with the merge base from git history.
- `tests/repository/test_check_repository.py`, `test_leaf_runner.py`,
  `tests/evidence`, `tests/qualification`: tooling that runs only on the CI runner
  or a developer machine, never on a Debian 12 host.
- `tests/frontend`: needs Playwright and a browser, and rewrites tracked evidence.
- `tests/privileged_vm` and `tests/arm64`: need KVM or real arm64 binaries; the VM
  suites already run on Debian 12 inside the guest.

## Order and exit

1. Phase 0 and PR-40 first; PR-40 shrinks everything that follows.
2. PR-10, PR-11, PR-19 and PR-41 next: they are what make an unattended Pi safe.
3. Remaining Phase 1 and Phase 2 items, then a Pi night on the resulting candidate.
4. Cutover. Phases 3 and 4 beyond PR-40/PR-41 can follow it, except the gates in
   Phase 4 that already have passing figures, which cost little to add now.

Exit for cutover: every Phase 0 row closed, PR-10/11/14/15/16/17/19/20 closed,
PR-30 written, PR-41 green, and one Pi night on that exact candidate.

## Operator decisions needed

- PR-30: accept "executor trusts core within the ceiling" as the documented model,
  or require an independent approval path before any storage effect ships.
- PR-31: ceilings keyed on container names or IDs. They key on IDs today; an edit
  to the list now takes effect on the next request, so a recreate costs an edit,
  not a restart.
- PR-40: whether deferred code is feature-gated in place or moved out of the
  workspace.
- PR-44: whether raw evidence may leave git.

## Operator decisions recorded

- 23 November: the approved-storage mount suite (`p04_approved_guest.py`) left the
  gating `debian-vm` job. It covers the dormant mount path and had not passed on a
  runner since 16 October. It runs in `storage-mount-vm`, which uploads its evidence
  and does not gate the workflow. Before storage mounts ship, fix the suite's
  failure (last seen at `await_terminal_receipt`, "mount job is uncertain/refused")
  and move it back into the gating job.
- 23 November: ceilings stay keyed as they are — full container IDs for containers,
  Compose project names for stacks. Consequence to plan for: a container that is
  recreated (an image update by watchtower, a `docker compose up` that replaces it)
  gets a new ID and leaves the managed set until `limeosctl migrate propose-ceiling`
  is run again and the ceiling reinstalled. Follow-up, after the first migration:
  evaluate keying on names, or names with the ID as a recorded incarnation.
- 23 November: lookup, verification and resolution of an existing receipt stay
  subject to the managed-list check. Delisting a container freezes its open jobs;
  the recovery procedure is in `docs/p03-operations.md`.
- 23 November: `limeos-core.service` gets a start limit (ten starts in five
  minutes) so a store that cannot be opened or upgraded stops being retried every
  two seconds, which could otherwise write the SD card continuously. A reboot or
  `systemctl reset-failed limeos-core && systemctl start limeos-core` tries again.
- 23 November: `watchtower` stays in the cutover's default managed list (the
  conservative choice: a watchtower restart or an update it applies during
  activation rolls back). Choose a cutover window outside watchtower's schedule.
- 23 November: idle CPU is accepted for the first migration at its measured figure
  (0.15–0.20 % of a core against the 0.1 % target, most of it the stack reader's
  30-second refresh). The gate is a regression ceiling; the target stays recorded
  as not met, and the reader's refresh cost is a follow-up.

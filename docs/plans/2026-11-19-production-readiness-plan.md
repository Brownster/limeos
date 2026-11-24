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
| PR-23 ✔ | Migrations: one ordered table used by both `open` and the schema check; snapshot the database file before migrating; check the `PRAGMA journal_mode` result; verify the `jobs` table SQL after v16. | A migration added to the table alone is applied and checked. Done: see the note below the table. |
| PR-24 | `postrm`/`prerm`: tolerate already-removed or never-enabled units; restart optional units that were running before an upgrade. | VM: remove twice, upgrade with the reader running. |

**PR-23 as built.** `crates/persistence/src/migrations.rs` holds `MIGRATIONS`, one entry per
version (its SQL, a description, the tables and indexes the startup check compares, and the
tables it rewrites as text, compared right after it). `Store::open` applies the entries above
the store's version and confirms each set its version; the startup check builds its reference
from the same entries and compares every trigger plus the objects they name, and a mismatch is
logged by name. `SCHEMA_VERSION` is the last entry. The version-17 space check and the
version-19 estimate stay keyed by version in `lib.rs`.
`a_migration_added_to_the_table_alone_is_applied_and_checked` adds a test-only entry and
nothing else. An upgrade runs on its own connection with `locking_mode=EXCLUSIVE`, taken by an empty
write transaction as its first access (every other connection locked out, in this process or
another; a test reaches it from a second process; SQLite still keeps a `-shm` file), from the store's
hash through the snapshot to the migration's commit, so all three see one state. It keeps
`core.sqlite.v<from>.snapshot` (`VACUUM INTO`, synced, checked, mode 0600) with a record of the
store's and the snapshot's digests; a repeated attempt on an unchanged store keeps it (cheap
checks, then hashes) and writes no copy. Leftovers go before SQLite opens the store; other
snapshots count by the blocks removing them frees, and go before the new one is written only
when it would not fit otherwise. The estimate covers the snapshot, the v17 build, the v19
rewrite bounded from the pages of `jobs`, and recovery bounded from b-tree depth over the 14
trees SQLite's plan opens (a test reads the count from the plan); residual
risks (stated in `lib.rs`): retention's `make_room`, and the v19 bound's reliance on records
growing by less than their size and SQLite rewriting a transaction's WAL frames in place.
Running out of space anywhere in the upgrade is reported with figures, and a program holding
`core.sqlite` open, which blocks the upgrade, is named as the reason in core's log.
`limeos-core.service` stops retrying after ten starts in six hours (long enough to trip even for
starts that fail late in a 31-minute start); postinst resets the count before it restarts core. Open fails unless
`journal_mode` answers `wal`. The v16 hazard: its `replace` on the `jobs` text does nothing,
silently, when the text is not exactly as expected, and the startup check never compared
`jobs`; the store then reached v19 without the `resolved` state. `jobs` is now compared right
after v16 and on every start. Not verified: an upgrade of a real pre-upgrade store (no such
fixture exists; tests build older stores from the same SQL files), and the snapshot's time and
reads at full size on the Pi. Gap: no supported tool lists or cancels every queued job across
users (the restore procedure relies on plan expiry instead). Operator steps:
`docs/p03-operations.md`.

## Phase 2 — Security

| Id | Change | Done when |
|---|---|---|
| PR-30 | ADR: state plainly that the executor trusts core within the root-owned ceiling and does not verify approvals. Add what the executor can enforce alone: a per-resource action rate limit and an executor-side append-only action log core cannot rewrite. Done for the container executor (branch `pr-executor-limits`, 23 November, revised after review): each managed container is charged at most 10 effects in ten minutes and 60 a day, a stack charging each target, never refunded (a charge never sent still counts, so every action-log row is bounded by the budget), counted from an action-log table in the receipt store (schema 9) that core cannot open, with charges dated within one window of now either way; core settles a job refused with the new `rate_limited` code as refused and records when the budget frees on the job, which the web interface shows; `limeosctl action-log` reads the log as the executor's account. The dormant storage effect roles have neither (ADR 0013). | ADR merged; tests for the rate limit; the threat model says what a compromised core can do. ADR 0013 updated with both and with what a compromised core can still do per unit of time; rate-limit and action-log tests in `crates/executor-container/src/limit/tests.rs`. |
| PR-31 | Every ceiling loads through the no-follow walk behind `read_protected_json` (each path component opened without following symlinks, root-owned, not group- or world-writable) and is reread per request: at accept, as a peer gate before any frame, and after the frame, which alone authorizes the request; a request already authorized runs to its end. Every ceiling rejects `core_uid == 0`; a ceiling that cannot be read or admitted answers `Unavailable`, never `Forbidden`, so a half-written file cannot withdraw an operator's resolve request. `limeosctl check-ceiling FILE CORE_UID ROLE` runs the executor's walk and role check. Before: the shared host, container and reader ceiling was read once at start with an lstat of the final component only and accepted `core_uid` 0; the target ceiling was walked but read once at start; the mount ceiling already met this. Container ceilings already key on full container IDs, which change on every recreate; whether to key on names instead is an operator decision (below). | Revoking a container takes effect without a restart. Code and tests on branch `pr-ceilings`, 23 November; the names-or-IDs decision is still open, and so is whether lookup, verification and resolution of an existing receipt should keep checking the managed list (a revocation strands open actions; ADR 0013). |
| PR-32 | systemd: `limeos-storaged` and the storage reader run as a dedicated user with only the capabilities they use; add `ProtectHome` and `PrivateTmp` where mount identity allows. For the dormant mounts unit, hold `CAP_SYS_ADMIN` only in the helper. `limeos-storaged`: done (unprivileged, no capability, 2.4 → 0.4). Storage reader: done on branch `pr-reader-user` (24 November): it runs as `limeos-storage-reader` with `CAP_DAC_READ_SEARCH` (block-device reads) and the `docker` group (Engine GET reads), and core trusts its socket by the container executor's rule (`limeos_contracts::ExecutorOwner`, one implementation for both); exposure 3.5 → 3.4 under systemd 258. `ProtectHome` and `PrivateTmp` are not possible for it (mount identity). Mounts helper: open (below). | `systemd-analyze security` score recorded per unit and gated in CI. |
| PR-33 | Log redaction: add cookies, bearer and JWT shapes and cloud key prefixes; match `token`/`secret` as keys, not substrings. | Table-driven tests for both leaks and over-redaction. |
| PR-34 | `limeosctl job resolve` requires `--evidence-digest` or an interactive confirmation; `--help`/`--version` are parsed only as the first argument. | CLI tests. |
| PR-35 | Uniform error envelope for query-extractor rejections; `Backend` methods without a default so an unwired route fails to compile; one session-check helper used by every handler. | No route returns axum's plain-text rejection; a route-table test asserts every mutating route checks session, CSRF and scope. |

**PR-32, the storage reader (done on branch `pr-reader-user`, 24 November).** What it read as root, and how each read works now. Sysfs (topology, serials), `/proc/self/mountinfo`, `/proc/1/mountinfo` (world-readable; PID 1's table needs no ptrace access), `/proc/swaps`, `/etc/fstab` and the management catalog (0644): unchanged, any account reads them. Its ceiling `storage.json`: configure now sets it `root:limeos-host-access 0640`, as for `host.json`, unless it refused it. Block-device nodes (`root:disk 0660`), opened read-only and probed by `blkid` through `/proc/PID/fd/N`: `CAP_DAC_READ_SEARCH`, ambient so that `blkid` inherits it. `/run/docker.sock` (`root:docker 0660`) for the stack and container-health reads, GET only, from the read workers: the `docker` group from the unit's `SupplementaryGroups=` (never `/etc/group`, where configure refuses it). The storage inventory's "refuse a non-root euid" checks became "root or `CAP_DAC_READ_SEARCH` in the effective set" (`limeos_executor_storage::full_read_access`), so its evidence is still never a partial view. The Compose and Engine readers the stack and health workers call lost their root checks instead: they protected nothing a non-root failure would not (the catalog walk demands root-owned components, the Engine socket is authenticated as root's and needs the `docker` group to reach). The guardians clear their ambient set before starting a worker, so the workers, which parse the catalog and the Engine's answers, hold no capability and refuse to collect if they do; the guardians keep it permitted until they exit, because `capset` is in `@privileged`, which the unit's filter refuses. The Compose host-view check now expects the process's own `/proc/self/mountinfo` to be owned by its own account (PID 1's by root). The read guardian and workers need nothing more: their exec of sealed images, `setsid`, subreaper, `setrlimit` and pidfd signals are unprivileged, and `/proc/PID/fd` access between the reader, the guardian and `blkid` is a same-account ptrace check with equal capability sets. Core's four reader clients (`fresh_storage`, the 1 Hz health read, the reader-health watcher and the stack proxy) and the container executor's storage-inventory client share `limeos_contracts::ExecutorOwner`, checked through descriptors: the socket's directory's parent root-owned and writable only by root (reached without a symlink), the directory owned by the socket's owner and not group- or world-writable, the socket owned by neither root nor core, the connection made to that inode, the peer equal to the owner. Not yet under it (open): the container operation RPC (`bins/core/src/operations.rs`) and the observation RPC to the host and container executors (`crates/observations/src/collectors.rs`), which do not authenticate the server peer; they need a test seam for the soak's simulated executor first. A reader core refuses is listed in core's health (`storage_reader`, code `forbidden`) and logged once; `limeosctl reader-status` has core ask it now; the cutover's activation requires the reader's socket to be the account's and `reader-status` to pass, and preflight and activate name any drop-in on the LimeOS units. Hardening that became possible: `RemoveIPC=yes`. Not possible: every mount-namespace setting (`ProtectHome`, `PrivateTmp`, `ProtectSystem`, ...), since the reader compares its root mount ID with PID 1's. Considered and left: the `disk` group instead of the capability (raw write to every disk); `DevicePolicy=closed` with `DeviceAllow=block-* r` (kernel-enforced read-only devices, and would allow `ProtectClock`; needs Pi qualification: systemd resolves `block-*` from `/proc/devices` at start, so a later-loaded driver's disks would be refused); udev ACLs giving the account read-only access to block nodes (narrower than the capability; unqualified). What remains: Docker access is root-equivalent, so code execution in the reader still reaches root through dockerd; the account removes root's owner write on root-owned files and disks. Exposure 3.5 → 3.4 (systemd 258): the non-root user and `RemoveIPC` gain more than the capability, ambient set and supplementary group cost. Upgrade: configure creates the account, hands `storage.json` to the group, restarts core, then starts the reader if prerm recorded it running; the old root reader kept no state, and its runtime directory goes when prerm stops it (systemd would chown a leftover one). Only a VM or the Pi can confirm the device reads, `blkid` and the Docker reads as the new account. In CI, the foundation guest checks the account, the unit's credentials and the reader accepting core's UID (and refusing another account); the footprint guest checks core accepting the reader: its stack status turns fresh, its storage inventory names the guest's ext4 root identified by `blkid`, a container-health read answers for the guest's containers, and the reader is dumpable with its exact capability sets and groups. The cutover rehearsal guest (case 23) reproduces a root reader with a drop-in, checks core refuses it and activation rolls back, and upgrades with the root reader running to show it comes back as the account; it cannot run the earlier package's own prerm and binaries, which only the production upgrade will.

**Not done in PR-32** (still open):

- *`CAP_SYS_ADMIN` is not held only in the mounts helper.* For a root service the bounding set gives `CAP_SYS_ADMIN`, `CAP_SYS_PTRACE` and `CAP_DAC_READ_SEARCH` to every child of the dormant mounts unit (its unit file records the gap). Dropping it in code does not work in the current process tree: the helper (`mount(8)`) is started by the guardian, which the socket-facing role starts, and under `NoNewPrivileges` a capability that is not in a process's permitted set can never reappear in a descendant (file capabilities are ignored, and ambient capabilities must be permitted). So the role and the guardian must keep `CAP_SYS_ADMIN` permitted to pass it on; they could only drop it from their effective sets, which either can raise again, and that confines nothing. A broker forked early in code, the only process to keep `CAP_SYS_ADMIN`, asked for each mount by the role over a private socket, would work, but it is the same redesign of the helper's lifecycle and IPC as a separate unit. Dropping it from the bounding set needs `CAP_SETPCAP`, which the unit does not have, and would also take it from the helper. What would confine it: a separate helper unit, the only one with `CAP_SYS_ADMIN`, reached over its own socket (socket-activated per request, `Accept=yes`), receiving the source, target and `mount(8)` image descriptors over `SCM_RIGHTS` instead of reopening them from `/proc/PARENT/fd` (`copy_parent_fd`), with the parent-death gate (`parent_gone` on its stdin pipe) moved to the socket's EOF and its GO frame and acknowledgement carried over that socket; the role then keeps `CAP_SYS_PTRACE` and `CAP_DAC_READ_SEARCH` for admission and the guardian runs in the helper unit. That is a rewrite of the guardian protocol and needs the storage-mount VM suite (not gating since 16 October) to pass; it was not attempted for a dormant unit.

## Phase 3 — Patterns and scope

| Id | Change | Done when |
|---|---|---|
| PR-40 | Remove from the default build everything with no production caller: `backup-archive`, the SnapRAID and catalog planners, `container_updates`, and `bins/executor/src/updates/` ✔ (never compiled today). Move them behind a `deferred` workspace member or feature that CI still tests. | The shipped binaries contain only the basic-cutover surface; binary size and test time recorded before and after. |
| PR-41 | Soak harness: a fake clock and fake Engine drive 90 simulated days of jobs, logins, polls and restarts in CI, asserting no refusal, bounded database size and bounded startup time. | Runs on every push in under ten minutes. |
| PR-42 | Domain newtypes for absolute path, resource id, UUID and plan lifetime, with private fields and `TryFrom` on deserialise. Replace the eight path validators and six id validators one module at a time. | One validator per concept; public structs cannot hold an unvalidated value. |
| PR-43 | One receipt-journal implementation shared by container actions, storage targets and mounts; one private-object-tree implementation in `backup-archive`. | Duplicates deleted; existing crash tests pass unchanged. |
| PR-44 | Evidence policy: keep summaries and hashes in git; move raw logs and archives (80 MB today) to release assets. Replace the roadmap's status header with a link to the latest status file. | Tracked `docs/` under 10 MB going forward. |

**Open from PR-30 (core startup at high job counts):** past about 2,700
retained jobs core's startup statements read more than the soak's 3.2 pages
per page of file (3.84 at 5,113 jobs, 210 ms in a debug build); job-row
retention keeps 30 days of settled jobs, so 90 to 170 jobs a day reaches it.
The job walks by `json_extract(intent,'$.operation')` (eight of them) and
the lock and resource invariants scale with the jobs. Measured and
reproducible with `startup_at_high_retained_job_count` (`docs/soak.md`,
"Core startup at high job counts"); not fixed.

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
120 to 300 s), and the suite has a budget of its own (now 1,500 s, 25 minutes). A hang therefore
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
- **The change, first round** (`crates/persistence/src/cache.rs`,
  `telemetry.rs`, the receipt store; sizes revised in the third round below):
  each long-lived connection got a cache sized to its work, the writer 1024
  KiB, the reader 512 KiB, `metrics.sqlite` and the container receipt store
  256 KiB each, and the open-time checks read through the small caches. The
  first-round evidence that nothing spilled (equal WAL lengths) was not proof;
  see the third round. The soak
  passed all five long tests. Its only moved figures (withdrawn with the rest of
  that comparison; see the third round) are startup page reads:
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
| Core plus base executors, idle PSS (every window) | ≤ 15 MiB then | 14.05 / 13.96 / 14.09 MiB (core 10.74–10.81 MiB) |
| Resident disk writes (600 s) | < 13 MB/day | 7.074 / 7.074 / 7.073 MB/day |
| Idle CPU (600 s) | < 0.3% | 0.2032 / 0.2032 / 0.1999 % |
| Core ready after `systemctl restart` (median) | < 1000 ms | 171 / 139 / 141 ms |

- **PSS, 15 MiB.** The highest settled figure seen was 14.22 MiB, in the trace
  run. The threshold is 0.78 MiB above that, about three times the largest
  run-to-run swing (0.28 MiB, mostly the executors' shared code pages), so a
  regression of a MiB fails. (Raised to 15.5 MiB in the third round, with the
  runner's figure.)
- **Disk, 13 MB/day.** The steady figure is 7.07. Three flushes in one 600 s
  give 10.6, and adding a checkpoint gives about 12.4; 13 MB/day passes those
  alignments and stays under the 20 MB/day budget. It fails when a steady
  flush passes 45 KiB (1.9 times today's) with two flushes in the run, or 30
  KiB with three, or when another idle writer adds about 6 MB/day.
- **`AnonHugePages`** was 0 for every unit in every window.

**Startup reads with the smaller writer cache** (figures withdrawn: measured in
a shared target directory; see the third round's clean comparison). The soak
comparison above showed where the reads moved. On the soak's largest store (28 to 31 MiB, days
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

### Third round (24 November): spill proven, worst cases, a realistic seed, the runner

**Spill, measured directly.** SQLite writes a transaction's pages to the WAL at
commit, unless its cache cannot hold them. Then it spills dirty pages early, and
a page changed again after its spill is written again. Equal final WAL lengths
(the first round's test) cannot show this: SQLite overwrites a frame the same
transaction already wrote.

The tests now start each transaction from an empty WAL with automatic
checkpoints off. Any WAL bytes on disk before the commit are a spill; the frames
after it are the pages the transaction dirtied (`measured` in
`crates/persistence/src/retention/tests.rs`, and its receipt-store twin). The
fixtures are the worst shapes the code allows, with the bundled SQLite and 4 KiB
pages:

| Transaction | Pages dirtied | Spilled at 512 / 1024 / 1536 / 2000 KiB |
|---|---|---|
| Event batch (1,024 events), densely packed | 67 | 0 / 0 / 0 / 0 |
| Event batch, 1 prunable in 2 among pinned history | 132 | 111 KB / 0 / 0 / 0 |
| Event batch, 1 prunable in 4 | 272 | 684 KB / 223 KB / 0 / 0 |
| Event batch, 1 prunable in 8 | 556 | 1.85 MB / 1.38 MB / 927 KB / 511 KB |
| `make_room` at the cap (1 in 2), inside startup's transaction | 132 | 107 KB / 0 / 0 / 0 |
| A day's job-row batch (10 jobs, interleaved) | 42 to 46 | not spilled at 1536 |
| Backlog job-row batch (52 jobs, about 1,770 scattered events, the `ROWS` bound) | 442 | 1.38 MB / 894 KB / 437 KB / 21 KB |
| Receipt store: full prune (tombstones) then a 16-member stack insert, random ids, real sizes | 109 | 4 KB at 512 KiB, 0 at 768 KiB |

**Sizes, revised.**

- **Writer: 1536 KiB.** This holds every normal shape in the table up to one
  prunable event in four (272 of 384 pages, 41% headroom).
  - One prunable event in eight spills even at SQLite's 2000 KiB default; such
    layouts mean pinned history dominating, which is not normal operation.
  - A backlog job-row batch spills 437 KB at this size, against 21 KB at the
    default. It runs only while at least 64 settled jobs are past their horizon
    and floor, as a month after a day of more than 1,000 jobs. A day's batch is
    42 to 46 pages.
  - Halving the job-row `ROWS` bound to 1,024 would roughly halve that batch.
    That is proposed, not applied, since it changes retention's pace.
- **Known spill cases, accepted.** The two shapes that spill even at the
  default are not changed in this branch (`ROWS` stays):
  - **Backlog job-row batch:** 442 pages; spills 437 KB at 1536 KiB, 21 KB at
    the default. It needs at least 64 settled jobs of one family past the
    30-day horizon beyond the floor of 128, all at once. On a household host
    (about 10 jobs a day, retention running every minute) jobs cross the
    horizon a few at a time, so a backlog forms only after core has not pruned
    for about a week (64 jobs at 10 a day), on the first prune after the
    upgrade that brought job-row retention (every old settled job at once,
    once), or 30 days after a burst of 64 or more jobs within a minute. Each
    batch of such a backlog spills once: a 1,000-job backlog is about 16
    batches, about 7 MB of extra writes, once.
  - **Event batch with one prunable event in eight:** 556 pages; spills 927 KB
    at 1536 KiB and 511 KB at the default. It needs seven pinned events for
    every prunable one across the oldest 9,000 or so events. Pinned history is
    the events of jobs still open or held for an operator, pinned automation
    proofs and authority events. A household host's oldest events are almost
    all prunable session events and settled-job histories (the soak's store at
    its budget is), so this takes jobs left held for an operator for weeks with
    long histories, not normal operation.
- **Reader: 256 KiB.** The reader answers session and revocation checks only;
  listings and job histories run on the writer (`crates/persistence/src/lib.rs`
  around 330).
- **Metrics store: 256 KiB.** Unchanged.
- **Container receipt store: 768 KiB.** Its largest transaction dirties 109
  pages and does not spill at 768 KiB; at 512 KiB it spilled.
- **When each size is set:**
  - Writer: an upgrade runs on its own exclusive connection with SQLite's
    default, and that connection closes when the upgrade ends. The normal
    connection is sized, and shrunk, before the startup check when no upgrade
    runs, and again after startup or after the upgrade in every case.
  - Metrics store: size set before its check, with nothing yet to shrink.
  - Receipt store: size and `shrink_memory` after any migration, before its
    check.
- **Which connections have one.** Every long-lived connection has a size. There
  are two deliberate exceptions, each listed in the test that fails on a new
  open: `upgrade_space`'s temporary read-only connection, closed before it
  returns, and `resources::check`'s in-memory reference schema.

In the VM these sizes change nothing measurable: `core.sqlite` is about 0.5 MB
and the receipt store a few KiB, so no cache reaches its size. For a
`core.sqlite` near its cap the estimate becomes at most 1.5 MiB (writer) plus
0.25 MiB (reader) against 2 MiB plus 2 MiB at the default: about 2.2 MiB less.
This is an estimate, not a measurement.

**Startup reads at the final sizes.** The soak was run on the same tree twice,
with and without the cache settings, each in a private target directory. The
second round's comparison, and a first run this round, used the target
directory other worktrees share; a crate built from another branch can stand
in for this one's there, so those figures are withdrawn. The clean runs (both
scenarios):

| Figure | Without the cache settings | With them | Bound |
|---|---|---|---|
| Startup statements | 52 | 54 (the two cache pragmas) | — |
| Core open, page reads per page of file, max | 2.306 / 2.448 | 2.443 / 2.712 | 3.2 |
| Core open, beyond the statements cold, max | -8 / -49 pages | 4 / 248 pages | about 700 (3% plus 256) |
| Receipt-store open, page reads per page, max | 1.249 / 1.281 | 1.870 / 1.868 | 2.0 |

- **The 1536 KiB writer does not restore the core open's reads.** They still
  rise, to 85% of the per-page bound and 35% of the other.
- **The receipt store's open reads 1.87 pages per page, 93% of its bound.**
  That was so at both 256 and 768 KiB: a measurement at each size on the same
  7 MB store read 3,386 calls either way. The cause is checking through a cache
  smaller than the store, against the default's 2000 KiB, which nearly holds the
  soak's 2.4 MB store.
- **The extra reads are re-reads of pages the open just read.** They come from
  the kernel's page cache, not the SD card, unless memory is short. Open times
  are unchanged: 5 to 7 ms for the receipt store, about 75 to 115 ms for core.
- **Decided (24 November): keep the small-cache check.** Checking through the
  default cache and shrinking afterwards would restore the reads but keep most
  of the memory: glibc held 1.4 of 2.0 MiB after `shrink_memory` in the Python
  test, up to 1.4 MiB more PSS in the container executor for good. The soak's
  receipt bound is now derived instead of a bare 2.0
  (`RECEIPT_PAGE_FACTOR` in `bins/core/src/soak/checks.rs`, and
  `docs/soak.md`). Through a cache smaller than the store, `quick_check` walks
  every B-tree page once for the structure and every table's pages again for
  their NOT NULL and CHECK constraints. That is at most 2.0 reads per page,
  measured 1.87; the open's other statements add a few pages. The bound is
  2.2, which leaves room for new tables and fails any further full pass
  (1.87 plus 1.0). Core's 3.2 stands, and its comment now records the 2.71
  measured with the writer's cache.

**On the merged tree** (with the migrations rework, 24 November) the soak
passed again. Core's open read at most 2.463 and 2.711 pages per page (bound
3.2), and at most 7 and 351 pages beyond the statements cold (bound about 700).
The receipt store's open read at most 1.872 (bound now 2.2).

**Core starts in the suite.** Core's unit now allows ten starts in six hours.
The suite starts core twice between resets: postinst resets the count and
starts it on install, and the suite starts it again after seeding. measure.sh
resets the count before each of its three restarts, so no `reset-failed` is
needed in the suite.

**A realistic metrics seed.** Core appends samples in ascending time, five at
a time, and prunes in the same transaction. The first seeding wrote 31 days at
once, newest first: 546 pages and 2.2 MB, against the real layout's 315 pages
and 1.14 to 1.29 MB. The suite now writes 35 days as core does: ascending, one
transaction per five samples, each deleting what is 31 days older than its
newest. The last four days of pruning and page reuse are therefore behind the
store.

On that store, the strace accounting gives the same per-flush writes as
before: two frames (leaf 3, holding the oldest rows; leaf 315, the newest), 24,576
bytes of `write_bytes`, and 49,152 for the first flush after a start. So
**7.07 MB/day stands**.

**Settled gates, final.** Three runs of the final suite on the ascending seed,
with CI's package of `4b6c2b32` (cache sizes of the first round; the final
sizes do not change these stores, see above). The runner column is CI run
45232884985: earlier suite, newest-first seed, the same package.

| Gate | Threshold | Laptop runs 1 / 2 / 3 | Runner |
|---|---|---|---|
| Core plus base executors, idle PSS (every window) | ≤ 15.5 MiB | 14.02 / 14.31 / 14.10 MiB | 14.26 MiB |
| Resident disk writes (600 s) | < 13 MB/day | 7.074 / 7.074 / 7.074 MB/day | 7.073 MB/day |
| Idle CPU (600 s) | < 0.3% | 0.2099 / 0.2066 / 0.2049 % | 0.1882 % (core 0.103, reader 0.070) |
| Core ready after `systemctl restart` (median) | < 1000 ms | 161 / 189 / 157 ms | 143 ms |

- **PSS, 15.5 MiB.**
  - Nine settled figures span 13.94 to 14.31 MiB: six laptop runs, the runner
    and two traces. Core alone is 10.67 to 11.06 MiB. The spread, 0.37 MiB, is
    mostly the executors' shared code pages (they read 1.56 to 1.97 MiB each).
  - 15 MiB would leave 0.69 MiB above the highest, under twice that spread.
    15.5 MiB leaves 1.19 MiB, three times it, so noise does not fail it and a
    regression of 1.5 MiB does.
  - It is half the 30 MiB budget.
- **CPU, 0.3%.** The runner now reads 0.148% and 0.188% in two runs; 0.3% is
  1.43 times the highest figure from either host.
- **Time.** The suite took 1,035 s on the runner, and the `debian-vm` job 31
  minutes of its 45.

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
- 23 November: `limeos-core.service` gets a start limit (ten starts in six hours;
  a five-minute window would never trip for a start that fails late, after
  hashing or migrating) so a store that cannot be opened or upgraded stops being
  retried every two seconds, which could otherwise write the SD card
  continuously. Successful starts count too, so postinst and the cutover scripts
  clear the count before their own starts. A reboot or
  `systemctl reset-failed limeos-core && systemctl start limeos-core` tries again.
  Follow-up: an hourly retry after the limit trips, for unattended recovery.
- 23 November: `watchtower` stays in the cutover's default managed list (the
  conservative choice: a watchtower restart or an update it applies during
  activation rolls back). Choose a cutover window outside watchtower's schedule.
- 23 November: idle CPU is accepted for the first migration at its measured figure
  (0.15–0.20 % of a core against the 0.1 % target, most of it the stack reader's
  30-second refresh). The gate is a regression ceiling; the target stays recorded
  as not met, and the reader's refresh cost is a follow-up.

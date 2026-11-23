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
| PR-31 | All ceilings load through `read_protected_json` (no-follow walk, reread per request) and reject `core_uid == 0`. Decide whether ceilings key on container names or IDs, since IDs change on every recreate. | Revoking a container takes effect without a restart. |
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
| Core plus base executors, idle PSS | 30 MiB | 10.44 MiB (Pi guest, 7 Oct); 13.2–13.5 MiB (amd64 guest, 23 Nov) | **Gated at 15 MiB** (below). |
| Idle CPU | 0.1% of a core | 0.051% earlier; 0.203–0.205% (amd64 guest with the stack reader, 23 Nov) — **budget not met** | **Gated at a 0.4% regression ceiling**, not the budget (below). Re-measure with the 1 Hz automation pass enabled. |
| Resident disk writes | 20 MB/day | 4.72 MB/day; 4.72 (amd64 guest, 23 Nov) | **Gated at 10 MB/day** (below). |
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
the stack reader started, not enabled. After 90 s to settle, the unchanged
`scripts/cutover/measure.sh` runs three consecutive 200 s idle windows, then three
`--measure-core-restart` runs. The gated figures are measure.sh's own row values:
PSS per window, and CPU and disk writes as each window's row weighted by its
elapsed time:

| Gate | Threshold | Statistic | Local guest runs 1 / 2 / 3 / 4 (4 on the final suite) |
|---|---|---|---|
| Core plus base executors, idle PSS | ≤ 15 MiB | every window | 13.48 / 13.24 / 13.48 / 13.32 MiB |
| Resident disk writes | < 10 MB/day | all 600 s together | 4.716 / 4.716 / 4.715 / 4.716 MB/day |
| Idle CPU | < 0.4% of a core | all 600 s together | 0.2032 / 0.2032 / 0.2049 / 0.2049 % |
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
  The gate is a regression ceiling at twice today's figure, which leaves room for
  a slower or busier runner and still fails if idle CPU doubles; it is not a
  widened budget. No runner figure is behind it yet: the first CI runs record
  one (per window and per unit), and the ceiling should then move towards 1.5
  times the runner's figure. The suite's result says `budget_met: false`.
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
- PR-31: ceilings keyed on container names or IDs.
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

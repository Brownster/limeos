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
| Core plus base executors, idle PSS | 30 MiB | 10.44 MiB (Pi guest, 7 Oct) | Gate at 15 MiB so growth is noticed early. |
| Idle CPU | 0.1% of a core | 0.051% | Gate; re-measure with the 1 Hz automation pass enabled. |
| Resident disk writes | 20 MB/day | 4.72 MB/day | Gate at 10 MB/day; assert an unchanged poll writes zero bytes. |
| Cold readiness | 1 s | 457 ms | Gate, and repeat at the PR-13 database size. |
| Cached overview p95 | 20 ms | 0.83 ms (AMD64) | Gate on the Pi figure. |
| Storage inventory p95 | 20 ms | 61.4 ms — fails | Profile; cache topology between udev events, or set a separate reasoned budget. |
| Mount admission | 4 s window | 3.0–4.6 s — fails | Deferred with the feature; do not widen the window without a profile. |

Add: per-request round trips to the store reduced from 2–4 to 1 (session check
and authorisation in one call), and a matched Python measurement on wybie so the
improvement claim has a baseline.

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

# Operator resolution of `needs_intervention` jobs

Status: design for review, 6 November. Nothing is built. Covers container and
stack jobs (both use the same states and locks). Storage and backup jobs also
reach `needs_intervention` but are out of scope here.

## Problem

A job that ends `needs_intervention` (or is left `running`/`verifying` by a core
restart, `crates/persistence/src/lib.rs:238`) keeps its `resource_locks` rows.
There is no code path that releases them, so one ambiguous Engine call blocks
every later action on that container or stack until someone edits SQLite by
hand. On a household Pi that means the UI offers no restart for `jellyfin`
after a single timeout. The stack lifecycle design lists this as a known limit.

The locks are deliberate: the core cannot know whether an Engine call took
effect, and issuing a second one blind is the failure the architecture exists to
prevent. So the fix is not "release after N minutes". It is a way for an
administrator to state, on evidence, that the question is settled.

## Two lock layers

1. **Core**: `jobs.state` in (`running`,`verifying`,`outcome_unknown`,
   `needs_intervention`) plus `job_resources`/`resource_locks` rows
   (`crates/persistence/src/resources.rs:93`). `resource_locks` must be empty for
   any job outside those states (checked at startup).
2. **Executor**: `actions` and `stack_actions` rows in state `prepared`,
   `effect_accepted` or `outcome_unknown`, protected by unique indexes on
   `resource` (`crates/executor-container/src/lib.rs`). These survive
   independently of the core.

Releasing only the core layer is wrong: the executor would still refuse the next
action with a unique-constraint conflict, and the core and executor would
disagree about whether the resource is free. Resolution must settle both, in an
order that is safe if it is interrupted between them.

## Design

### Resolution is a new job transition, not a delete

`needs_intervention -> resolved` (new terminal state, lock-releasing), recorded
with: resolving administrator, time, a mandatory reason, the evidence taken, and
the operator's declared outcome. The original job row, plan, receipts and member
results are never edited or deleted; `resolved` is an additional audit record.
`outcome_unknown` stays what the system measured; `resolved` is what a human
concluded.

### Evidence comes first, from the executor

The core never trusts the operator's claim alone. Resolution starts with a fresh
read-only `Verify`/`Inspect` (already exists: `VerifyContainer`, `VerifyStack`,
`InspectStack`) for exactly the job's targets, returning for each target:
current running state, container ID, start time, image, and whether it still
matches the plan's `expected`. Shown to the administrator with the receipt's
per-member outcomes.

### Three declared outcomes

| Declared | Meaning | Allowed when | Effect |
| --- | --- | --- | --- |
| `verified_done` | The intended end state is present on every target | Fresh verify proves it for every target | Job -> `resolved(done)`; locks released |
| `not_done` | Nothing in the plan took effect, current state equals the plan's `expected` | Fresh inspection shows every target (ID, start time, image, running) matches `expected` | Job -> `resolved(not_done)`; locks released; operator may plan again |
| `accepted_divergence` | State differs from both before and intended, operator accepts the present state | Always for an administrator, with reason of at least 20 characters | Job -> `resolved(divergent)`; locks released; the next plan snapshots the present state |

`verified_done` and `not_done` are checked by the system and refused if the
evidence disagrees. `accepted_divergence` is the explicit escape hatch: it
cannot be done by a non-administrator, cannot be done by a token (needs an
interactive session), and is the only outcome whose truth rests on the operator.
No outcome sends an Engine call; resolution is bookkeeping plus read-only
verification. It never starts, stops or restarts anything.

### Order of operations (crash-safe)

1. Core takes the job's resource locks (already held) and writes
   `resolution_requested` (job id, admin, declared outcome, reason, evidence
   digest) in one transaction. The job stays locked.
2. Core asks the executor `ResolveAction { action, evidence_digest }`. The executor
   in one transaction re-verifies, then moves the matching `actions` /
   `stack_actions` row (and member rows) from `prepared`/`effect_accepted`/
   `outcome_unknown` to a new state `resolved` (not in the unique index's
   `WHERE` list, so the index frees), keeping the receipt. A different or
   missing row is an error, never silently ignored. Idempotent by action id.
3. Only after the executor confirms, core in one transaction sets the job state
   `resolved`, deletes the `resource_locks` rows and writes the audit event.

If interrupted after step 2 and before step 3 the executor is free and the core
is held: the next startup sees `resolution_requested` and an executor row in
`resolved` and finishes step 3. If interrupted after step 1 the job remains
`needs_intervention` and nothing is freed. There is no state in which the core
believes the resource is free while the executor still holds it.

### Authority and surface

- New permission `JobResolve`, granted only to the `Administrator` role,
  never to tokens or API keys; session plus fresh re-authentication (same as
  other irreversible actions) and CSRF as for plan approval.
- API: `GET /api/v1/jobs/{id}/resolution` (evidence, read-only) and
  `POST /api/v1/jobs/{id}/resolution` with `{declared, reason, evidence_digest}`.
  The digest binds the decision to the evidence shown; stale evidence returns
  conflict and the page re-fetches.
- CLI parity: `limeosctl job resolve <id> --outcome ... --reason ...` over the
  root peer socket, for use when the UI cannot be reached. Same checks.
- UI: a "Needs attention" banner on the container/stack page and a resolution
  dialog showing plan vs measured state per target and the three outcomes, with
  `accepted_divergence` visually distinct and requiring the reason.
- Contracts regenerated; one schema migration (job state, `job_resolutions`
  table, executor `resolved` state in `actions` and `stack_actions`).

## What this deliberately does not do

- No automatic expiry or watchdog release.
- No retry, replay or "force" of the original action.
- No bulk resolve; one job at a time.
- Does not change `outcome_unknown` receipts or the recovery lookup.
- Storage mount and backup `needs_intervention` jobs are unchanged and still
  need their own design.

## Failure matrix

| Event | Result |
| --- | --- |
| Non-admin, token, or stale session | forbidden, nothing written |
| Evidence digest stale (state changed since shown) | conflict; page re-fetches |
| `verified_done` but a target is not in the end state | refused with the per-target diff |
| `not_done` but a target differs from `expected` | refused; use `accepted_divergence` |
| Executor unreachable at step 2 | request recorded, job stays held, retry allowed |
| Crash between step 2 and step 3 | startup completes step 3 |
| Two admins resolve at once | one wins on the request transaction; the other gets conflict |
| New action planned on the resource while held | still `conflict` until resolved |
| Resolved job's resource immediately re-planned | allowed; the plan snapshots current state |

## Test plan

- Persistence: transition table (only `needs_intervention`, and
  `running`/`verifying` after restart, can become `resolved`), lock release,
  startup invariant still holds, request-then-crash completion, concurrent
  resolve.
- Executor: `resolved` frees the unique index, wrong action id or digest
  refused, idempotent repeat, no Engine call made (fake Engine records zero
  requests).
- Core: each declared outcome against fake verify results, including refusal
  paths and permission cases.
- Frontend: dialog states, evidence refresh on conflict, divergence reason
  required.
- VM: induce `outcome_unknown` with a stalled Engine socket, resolve via UI
  path and CLI, confirm a new restart then succeeds and the executor and core
  locks agree.

## Questions for the operator

1. Is "administrator plus re-authentication" the right bar, or should
   `accepted_divergence` need a second administrator? (Recommended: single
   administrator; the household has one or two.)
2. Should `not_done` be allowed to proceed straight to a re-plan, or only
   release the lock? (Recommended: release only; the operator re-plans.)
3. Include a CLI path in the first cut? (Recommended: yes; the UI may be the
   thing that is down.)

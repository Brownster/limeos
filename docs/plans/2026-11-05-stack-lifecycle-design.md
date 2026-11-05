# Whole-stack start, stop and restart

Scope: approved whole-stack lifecycle on a registered, fresh Compose project,
copied from the single-container path. It is not `up`, `down` or recreate. It
writes no Compose file and never runs `docker compose`; it only issues Engine
start/stop/restart for exact full container IDs. Part of the "basic cutover"
minimum (metrics, containers, stacks with whole-stack lifecycle).

## Decisions

- **Plan.** `StackPlan` (domain) mirrors `ContainerPlan`: id, version,
  `ContainerAction`, principal, grant revision, 300 s TTL, and `expected`, a
  `StackSnapshot` of resource `stack:<project>` plus the member list (full
  container ID, image, start time, running) taken from the protected stack
  inventory cohort. Digest, expiry, grant revision, human approval, one-use
  approval nonce and idempotency key checks are those of `RestartPlan`.
- **Eligible stacks.** The project must be registered in the catalog, both
  sources fresh and matching, no `StackIssue` (identity conflict, overlapping
  project, undeclared services), and 1..=16 members. Otherwise `conflict`.
- **One-off containers** (`docker compose run`) are never members: not planned,
  not acted on, not locked, ignored by drift. **Stopped members** are members.
  Each action selects targets by state, recorded in the plan: start acts on
  stopped members, stop and restart on running members; the others are
  untouched. No target means `conflict` (nothing to do). **Declared services
  with no container** cannot be created here and are not members.
  **Undeclared services** make the stack ineligible (above). No dependency
  ordering: targets run in container-ID order.
- **Authority.** Needs `ContainerManage` on `stack:<project>` and on every
  member `container:<id>`; so a stack grant never reaches a container the
  operator could not manage individually. Executor ceiling adds `allow_stack`
  and `managed_stacks` (project names), and still requires every target in
  `managed_containers` plus the per-action `allow_*` flag. Defaults are false,
  `can_write()` includes `allow_stack`, so the read-only shadow executor still
  refuses it at startup.
- **Jobs and locks.** Same `jobs` table, intent `stack_{start,stop,restart}`,
  primary resource `stack:<project>`, plus `job_resources` rows for every member.
  The schema-7 triggers acquire all at dispatch, so a single-container job on any
  member (primary resource `container:<id>`) and a stack job exclude each other
  in both directions; a conflict leaves the job queued. Plans and results are in
  `stack_plans` and `stack_results` (schema 15).
- **Executor.** `ExecuteStack`, `StackReceipt`, `VerifyStack`, `InspectStack`.
  `execute_stack` authorizes (uid, ceiling), checks digest and expiry, lists the
  project fresh from the Engine and compares the full member list with the plan
  (any drift: a terminal `precondition_changed` receipt, no effect, no lock),
  then in ONE transaction writes the
  stack `Prepared` row and a `Prepared` row per target in the existing `actions`
  table (so executor-side single-container jobs on the same ID are excluded by
  the existing unique index). Only then are Engine calls issued, one target at a
  time, each member row updated after its call. Fail-stop: the first member that
  is not accepted ends the run; later targets are recorded `precondition_changed`
  (never sent). The stack row is the aggregate of the member rows.
- **Recovery.** After a crash or lost response core only calls `StackReceipt`
  (lookup) and `VerifyStack` (fresh inspection of each target; a member moves to
  `verified` only if the post-state proves it). A lookup finding `Prepared`
  member rows on a stack no live call owns converts them to `outcome_unknown`;
  nothing is ever resent. Receipts are keyed by action and plan digest.
- **Partial failure.** Per-member outcomes live in the receipt and result row.
  Verification is per member and tolerates partial success: verified members stay
  verified. The job is `succeeded` only if every target verified; any other
  mix is `needs_intervention` and keeps the locks (no automatic unlock, as for
  containers). Drift before any effect is `precondition_changed`.
- **API/UI.** `/api/v1/stack/{plans,plans/{id}/approval,plans/{id}/cancel,jobs,
  jobs/{id},jobs/{id}/cancel}`. `StackManagement.tsx` gets Start/Stop/Restart
  per stack using the `ContainerActions` preview, approval and queue flow.

## Failure matrix

| Event | Result |
| --- | --- |
| Grant revoked, plan expired, plan edited | refused or `precondition_changed`, no executor call |
| Member added, removed, recreated, changed state before claim | core pre-check or executor re-list differs: `precondition_changed`, no effect (executor writes a terminal receipt) |
| Another job holds a member or the stack | stays `queued`; executor unique index also refuses |
| Executor ceiling disallows stack, action, project or any target | `forbidden`, nothing written |
| Crash before Prepared transaction | no receipt; core marks `needs_intervention` (no replay) |
| Crash after Prepared, before or between Engine calls | lookup: unsent targets become `outcome_unknown`; verify may prove sent ones; never replayed |
| Engine error or timeout on member k | k `outcome_unknown`, later targets `precondition_changed`, earlier verified independently, job `needs_intervention` |
| Engine accepted but state not reached | not `verified`, job `needs_intervention` |
| Queue response lost | replay by idempotency key + approval returns the same job without any Engine call |
| Executor busy, unreachable or erroring after the claim | `needs_intervention` (as for containers); the dispatcher then only looks up the receipt |
| Plan expires between the executor's listing and its Prepared write | terminal `precondition_changed` receipt, nothing sent |
| Executor ceiling or digest check fails | error before any write; core marks `needs_intervention` after a claim |

Human sessions only: task tokens cannot plan or submit stack actions in this
slice. Plans expire after 300 s but a long run is not cut off at expiry; the
plan is checked once more immediately before the first Engine call.

Not covered: Compose `depends_on` ordering, health waits, creating or removing
containers, one-off handling beyond exclusion, unlocking `needs_intervention`.

## Review follow-ups

- **Unsent release.** If `ExecuteStack` errors (or a recovery lookup returns no
  row), core does one `StackReceipt` lookup. A lookup that succeeds and finds no
  row proves nothing was sent (a durable Prepared row precedes every Engine
  call), so the job becomes `precondition_changed` and the locks are released.
  A failed lookup, or any row, keeps `needs_intervention`.
- **Partly done.** When every sent member is verified and every other member is
  provably unsent, the stack aggregate is `partly_done`: terminal and
  lock-releasing. The job state is `failed` (never `succeeded`) and the UI says
  "Partly done, N of M members" with per-member outcomes. Any unknown or
  unverified sent member stays `needs_intervention`.
- **Catalog stack lock key.** The catalog `stack` name and the Compose `project`
  can differ (compose and deploy jobs lock `stack:<stack>`). The plan records
  `catalog_stack`; the job locks and authorizes both `stack:<project>` and
  `stack:<stack>`.
- **Per-target recheck.** Before each target's Engine call the executor
  inspects that target and fail-stops, recording the rest not sent, if it no
  longer matches the plan.
- **A transient store error** while checking authorization at dispatch leaves
  the job queued; only a definite refusal is treated as revocation.
- **UI retry.** The "response was interrupted" notice and same-key retry apply
  only to network or 5xx failures; a definite 4xx asks the operator to start
  again.

### Known limits

- An operator unlock path for `needs_intervention` is still required and is not
  built. Until then such jobs hold the stack and member locks.
- If core loses the connection while the executor has not yet reached its
  Prepared write, the lookup is serialized behind the executor's store lock, so
  a still-running call makes the lookup fail and the stack stays held rather
  than being released wrongly.
- Two further review items (numbered 5 and 8 in the review request) were
  deferred by instruction and are not implemented.

# P03 execution tracker

Status: implementation in progress. Started 2026-10-05 after P02 local qualification. P02's reference-host comparison and hardware gates remain pending; the overnight continuation request authorizes local progress while wybie stays untouched.

Estimate: twelve implementer days (96 hours) for RW-030 through RW-033. Review the cutover scope at 144 hours. Approximate allocation: 24 hours for typed plans/approvals and durable queueing; 32 hours for the restart executor, receipts and crash reconciliation; 24 hours for start/stop, logs and the Compose plan foundation; 16 hours for disposable-host setup and qualification. Record actual effort separately from elapsed time and approval waits.

The first slice implements typed restart preconditions, plan-bound single-use approval, current-grant authorization, idempotent queueing and cancellation in the core authority store. A separate executor library implements an independent allowlist, fresh preflight, bounded inspection/restart deadlines and private FULL-synchronous receipts. It is exercised through synthetic Engine fixtures and is not wired into the installed daemon or UI. No mutation is tested on wybie or on the workstation's real Docker socket.

The phase cannot be signed off until its complete interruption matrix passes on a separate test host, the browser/assistant authority paths agree, and the narrow executor's independent ceiling and protected receipts have been exercised.

## First slice validation

The [workspace test log](rust-tests.txt) records 59 passing Rust tests; [validation metadata](validation.json) identifies the checked source. Local commits are `53bc472` (observation deadline), `9b81fae` (restart authority) and `48642e8` (executor receipts). New authority regressions cover changed plans, wrong approvals, expiry, fresh container incarnation/state, revoked/scoped grants, forged roles, ownership of plans/events, racing duplicate requests, approval consumption rollback, queued cancellation and uncertain dispatch locks across restart. Transactional v1-to-v2 migration preserves existing identity; an injected migration failure rolls back the new table and schema version.

Executor fixtures prove that wrong peer UIDs, disabled/unmanaged ceilings and changed resource state prevent effects. A prepared receipt is committed before dispatch. The test process exits immediately after writing a separately durable simulated effect, before recording its result. Reopening the store returns the prepared receipt and performs no second effect. Timeout, acknowledgement failure and failure to persist a result retain uncertainty and the per-resource barrier. Accepted effects remain distinct from independently verified success.

Formatting, Clippy with warnings denied, generated contracts, repository boundaries, frontend tests, strict TypeScript and the production build pass. The dependency policy checks pass and the cached advisory audit finds no vulnerabilities in 214 dependencies; the new executor library adds no third-party dependency. All thirteen [release-gate rejection probes](gate-probes.json) pass.

The full socket fixture suite needs execution outside the sandbox, which prohibits its temporary Unix listener. The protected-receipt tests initially rejected the test directories because Rust tempfile uses the caller's default directory permissions. Fixtures now explicitly create private `0700` directories; the production permission checks remain strict.

Review also reproduced a P02 collector defect: 64 stats requests timing out in groups of four took 32 seconds and exhausted the enclosing 15-second inventory deadline. Optional statistics now have an independent eight-second budget. The regression preserves all 64 inventory rows and leaves unavailable metrics unknown. This fix is source-tested; it is not present in the inherited P02 Debian package used for the earlier VM measurement.

The first slice took approximately 40 minutes of elapsed agent work on October 5, excluding the earlier approval wait. This is not a human implementer-day measurement and does not revise the 96-hour phase estimate.

## Next work

1. Add the real GET inspection/POST restart Engine adapter, generated executor operation framing and a protected receipt directory in packaging. Keep shadow ceilings read-only.
2. Wire current-session planning/approval/job APIs and a bounded core dispatcher. Persist receipts and fresh independent verification; expose durable scoped progress and recovery.
3. Exercise core/executor death before dispatch, during the real Engine request and before result persistence on a disposable Debian host. Prove session and scoped-task callers share policy while task proposals cannot self-approve.
4. Migrate the existing container controls, then add start/stop, bounded logs and the approved Compose plan/diff foundation. Qualify the full phase and reference-hardware budgets separately.

No P03 defect-register row is closed by the prototypes. The full phase, operation UI, real Docker effects, package rebuild and test-host interruption matrix remain open.

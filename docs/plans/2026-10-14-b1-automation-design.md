# B1 observation schedules and notifications

Engineer B owns the leaves listed in the [stream brief](2026-10-14-engineer-p04-deployment-scheduling-runtime.md). This design is an integration proposal. The integrator owns the database, transaction boundaries, authority, runtime/route registration and shared UI mounting. B1 finishes only after those registrations and installed qualification.

## Decisions

Use bounded UTC interval schedules with an explicit anchor, revision and maintenance window. Calculate due times arithmetically; never iterate across missed intervals. Coalesce to the latest eligible occurrence within a bounded grace period, or skip missed occurrences. A persisted monotonic occurrence watermark prevents replay after clock rollback. Maintenance boundaries are half-open and may span midnight. UTC avoids an implicit timezone/DST interpretation; migration of legacy local-time cron settings must explicitly resolve that difference before parity is claimed.

A due key is the tuple `(schedule ID, revision, due time)`. The shared Store must reserve this tuple, create the observation job, and advance the watermark in one transaction. Concurrent ticks submit the same candidate and receive the existing job. Revision/pause/owner changes are checked again inside that transaction. Queue exhaustion leaves the watermark unchanged. Unsupported backup/protection/deployment/package adapters produce a visible blocked occurrence rather than starting an effect.

Use an explicit deterministic health-observation operation. The Core runner has no executor, model, filesystem or network delivery capability. Its proposed Store interface is a handoff boundary, not a second Store. An effect cannot enter this runner even if a caller invents an adapter registration. Recurring effect authority remains a separate central contract.

Alert folding preserves the frozen two-failure default and opening/recovery transitions. Missing or unavailable observations do not recover incidents or advance failure streaks. Deduplicate by observation sequence as well as resource key. Repeated failures after opening write nothing. Persist changed state and its notification atomically; failed outbox insertion must roll back the alert transition. Silencing suppresses delivery, not observation or incident tracking.

In-app delivery has one stable event key, bounded payload, visible queued/delivered/failed/ambiguous/acknowledged states and explicit retry limits. Only acknowledged or verified delivered rows may be retained away; unresolved delivery remains visible and blocks admission at the queue cap. External retries require receiver idempotency; ambiguous delivery suspends automatic retry otherwise. B1 provides no external sink or credentials.

## Alternatives considered

Process-local timers lose state across restarts and cannot reserve a job atomically. A separate scheduler database would split recovery and ownership. Both are rejected in favor of the existing Store and job system. Reusing human plan approval for recurring work is also rejected: one-time approval does not authorize later effects.

## Shared Store handoff

The integrator supplies methods matching the Core leaf's proposed `ObservationStore` interface or an adapter to it. This interface remains proposed until accepted. No SQL or persistence implementation is supplied against an invented schema.

- Read a bounded page of schedule rows plus their revision watermark; reads never mutate state.
- Reserve the exact proposed occurrence under the current row revision, pause status, definition, owner/grant revision and current authorization. Recalculate due time with the transaction's trusted clock. Atomically enforce the unique occurrence tuple, queue cap, job creation and watermark. Return reserved/existing/blocked/stale. Roll back every part on error.
- Finish observation jobs through existing job transitions. Persist a unique observation sequence and atomic alert-state/event/outbox changes. Interrupted observation may be retried under the same job ID; duplicate completion produces no second alert or delivery.
- Reconcile queued/running observation jobs at startup using the existing recovery mechanism; do not create replacement occurrences.
- Authenticated CRUD/read/pause and notification acknowledgment run through the existing identity and CSRF mechanisms. Schedules are owned by the current principal, never an input owner. Mutation uses revision compare-and-swap. Each pause/resume increments the revision and resets the anchor/watermark explicitly; resuming must not replay paused history.

Proposed caps: 128 schedules; 32 schedules per tick/page; 256 observed resources; 128-byte schedule/job IDs and 256-byte alert keys; 512-byte alert summaries; 1,024 notifications; 4 delivery attempts; retry delays 30/120/600 seconds; seven-day delivered retention. Final cap admission and storage retention are central responsibilities. None permits growth of credential-bearing payloads or arbitrary strings as operation names.

## Verification and integration

Focused leaf tests cover due arithmetic, grace/coalescing, cross-midnight windows, rollback/forward jumps, revision identity, partial observations, duplicate sequences, threshold and recovery transitions, retry/ambiguity and blocked effects. API cases cover authenticated reads, CSRF/origin checks before body parsing, POST-only mutations, bounded input and stale revisions. Frontend cases cover partial/blocked state, delivery failure and acknowledgment without optimistic success.

Leaf compilation uses small isolated harnesses that include the unchanged crate roots plus the proposed module declarations. This verifies actual leaf code without modifying integrator-owned roots. It is not production registration, SQL durability, installed acceptance or broad qualification. The test owner receives exact source/fixture hashes and the installed restart/concurrent tick scenarios after central integration.

Frozen reference: `/home/marc/Documents/github/pi-health` at `80593b2`, especially `alert_evaluator.py`, `alert_policy.py`, `alert_signals.py`, `backup_scheduler.py`, `update_scheduler.py` and `stack_notifications_service.py`. Mattermost webhook settings exist in the reference; their migration/retirement requires an explicit inventory decision. B1's in-app transport is a proposed ordinary transport, not full legacy notification-channel parity.

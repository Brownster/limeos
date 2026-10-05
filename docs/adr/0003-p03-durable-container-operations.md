# ADR 0003: durable container operation authority

Status: accepted and implemented for the restart path, 2026-10-05. Implements architecture A04 and roadmap RW-030.

Core creates a typed `container.restart@1` plan from a selected, fresh Docker inspection. The plan binds the complete container ID, image ID, running state and last start timestamp, its owner and grant revision, a normalized digest and a five-minute expiry. The registry owns the operation's risk, scope, timeout and recovery classification. Caller text never sets them.

A separate human-session approval binds that digest. Core stores only the digest of a random approval token; the token is single-use, expires with the plan, and cannot approve a modified or newly resolved resource. Queuing consumes approval and records intent plus audit events in one FULL-synchronous SQLite transaction. Browser and future assistant proposals use the same plan and policy definitions; possession of an assistant task token will not constitute human approval.

Idempotency keys belong to the principal. Repeating a queued request with the same plan returns the original job; changing the plan conflicts. A revoked grant invalidates pending approval and dispatch. The executor must independently inspect the resource again immediately before an effect and enforce its root-owned allowlist.

Cancellation of waiting or queued work records a terminal state without a host effect. Once dispatch is uncertain, core retains the per-resource lock and requires reconciliation. Cancellation must never label an in-flight restart as if it had been prevented.

Authority schema v2 adds plans without rewriting the v1 tables. A transactional migration preserves existing identity, jobs and audit state. The v1 binary refuses the newer schema; automatic downgrade across this migration requires recovery rather than being advertised as compatible.

A protected executor receipt is committed before dispatch and deduplicates action IDs across interruption. Core records the authenticated receipt with independent selected inspection evidence in schema v3. Both resource locks remain held until a fresh new running incarnation is verified; executor verification performs another inspection before releasing its lock. Recovery reads receipts and never sends an effect request for an interrupted job. Missing or prepared receipts retain `needs_intervention`.

Session mutations recheck authentication inside the final authority transaction after inspection. Scoped task callers resolve their principal from a token bound to their kernel UID, task, scope, grant revision and core generation. Their protocol offers proposal and approved queueing, with no approval command. Duplicate queue requests recover the original job from durable state without contacting Engine. Schema v3 migrates v1/v2 transactionally and records receipt plus verification with the terminal job event.

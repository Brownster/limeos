# ADR 0003: durable container operation authority

Status: accepted for the first P03 slice, 2026-10-05. Implements architecture A04 and roadmap RW-030; later slices add executor effects.

Core creates a typed `container.restart@1` plan from a selected, fresh Docker inspection. The plan binds the complete container ID, image ID, running state and last start timestamp, its owner and grant revision, a normalized digest and a five-minute expiry. The registry owns the operation's risk, scope, timeout and recovery classification. Caller text never sets them.

A separate human-session approval binds that digest. Core stores only the digest of a random approval token; the token is single-use, expires with the plan, and cannot approve a modified or newly resolved resource. Queuing consumes approval and records intent plus audit events in one FULL-synchronous SQLite transaction. Browser and future assistant proposals use the same plan and policy definitions; possession of an assistant task token will not constitute human approval.

Idempotency keys belong to the principal. Repeating a queued request with the same plan returns the original job; changing the plan conflicts. A revoked grant invalidates pending approval and dispatch. The executor must independently inspect the resource again immediately before an effect and enforce its root-owned allowlist.

Cancellation of waiting or queued work records a terminal state without a host effect. Once dispatch is uncertain, core retains the per-resource lock and requires reconciliation. Cancellation must never label an in-flight restart as if it had been prevented.

Authority schema v2 adds plans without rewriting the v1 tables. A transactional migration preserves existing identity, jobs and audit state. The v1 binary refuses the newer schema; automatic downgrade across this migration requires recovery rather than being advertised as compatible.

The first slice remains effect-free. A future protected executor receipt must be committed before dispatch, deduplicate action IDs, preserve uncertainty across interruption and allow independent verification. A completed receipt alone is not proof that the desired state was reached.

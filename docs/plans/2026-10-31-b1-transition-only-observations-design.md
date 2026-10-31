# Commit observation transitions without writing unchanged polls

Baseline `e11842b` binds configured policy but the queued observation path writes
an occurrence, job and result on every poll. This is useful durable recovery
machinery, yet it conflicts with architecture section 10: an unchanged periodic
poll writes nothing. Retention alone cannot fix those writes. Add the production
polling boundary before connecting a genuine collector or supervising a timer.

Keep the existing queued path for existing work and recovery. Do not change
effect jobs, locks, approvals, storage claims, publication or delivery authority.
The new path uses the sole Store/Database worker and private bounded leases.

## Read-only admission and unchanged completion

Admit an observation-only schedule under current owner/grant revision, schedule
revision/pause, configured policy and concrete HealthRead scopes. No browser
session or one-time effect approval authorizes recurring reads. Derive due state
under trusted post-lock time. One active health collection per owner, at most
64 globally, shares the existing lease exclusion. Original monotonic lifetime
is at most ten seconds and is never renewed. A queued legacy observation for
that owner blocks a new poll until its normal completion/discard path settles.

Return a private lease carrying the original occurrence, admission time,
generation, policy, resources and nonce. A bounded volatile schedule watermark
remembers completed unchanged polls for the life of this Store. It has at most
128 schedule entries and is discarded on restart. Pauses/revisions invalidate
it. Maintenance, skipped/coalesced runs and clock rollback retain their existing
due rules; actual skipped occurrences remain durably recorded.

Completion rechecks original lifetime, generation, nonce, current schedule,
owner/grants, policy revision, scopes and time after acquiring the transaction.
Validate the whole normalized batch and preview the real alert fold before
writing. A first observation, changed alert material, completeness or current
owner/policy binding is a material transition. Otherwise commit no rows/events,
advance only the volatile watermark, consume the lease and return an explicit
unpersisted observation outcome. Repeating the same lease cannot consume a
failure twice. Abandonment releases only its own private lease, without writes.

Unchanged read-only sampling may repeat after a process restart. It has no
durable occurrence/job/effect to duplicate. Exactly-once probe execution across
a crash cannot be promised without writing every poll; the requirement retained
here is one durable occurrence/job/notification for each committed occurrence.
Freshness after restart requires a new genuine read, not an old cache timestamp.

## Atomic transition recording

When material changed, reserve the original occurrence and create/complete its
existing undispatchable observation job in one transaction. Reuse current
sequence, fold, outbox, result, audit and resource invariants. Queue/audit/space
refusal rolls back every write. No queued intermediate job becomes visible.
Preserve admission-time due/grace evaluation, so a valid bounded collection may
finish after its admission grace without renewing its lifetime. Actual queue,
result and audit times use trusted completion time.

Schema 14 adds an immutable poll-origin row with admission time, previous
volatile due and a digest of the private nonce. A deferred foreign key requires
the matching observation result in the same commit. A typed poll event binds
its canonical digest. Startup requires an observed successful result, matching
owner/event/time, valid original due calculation and at most ten wall seconds
from admission; generic dispatch remains forbidden. Legacy canonical intent
bytes are unchanged. Missing/corrupt origin proofs refuse before recovery.

Completed replay checks the original private origin and batch digest and returns
the original result without another write. The unique occurrence and existing
schedule watermark prevent duplicate durable work after restart or concurrent
requests. Policy, grant and resource replacement cannot reinterpret an in-flight
poll. Unresolved storage/effect work is never pruned or released by this change.

## Integration and qualification

Add the real async Core bridge methods with the existing worker and original
deadlines. A future production supervisor must use these poll methods for normal
observation sampling, settle old queued jobs first and retain fair cursors.
Do not activate a timer with a stub reader or substitute overview cache values.

Use real file-backed SQLite and isolated process I/O tests. Compare the old
stable queued path with 2,880 new unchanged half-minute polls after one baseline;
check unchanged database/WAL bytes and row/audit counters, and observe isolated
process write bytes. Simulated wall time is not a real 24-hour installed soak.
Cover thresholds, muted/recovery transitions, completeness, policy/grant changes,
original deadlines, late grace, restart, duplicate/replayed leases, full quotas,
rollback and corrupt origins. Compiled schema-13 and older upgrade/recovery
fixtures must pass. Qualify affected Rust/Clippy/contracts and frontend contract
compilation under repository-disk scratch, one Cargo job/test thread and the
unchanged 3-GiB floor.

Genuine protected collection, concrete replacement, production supervision,
safe history/audit compaction, external notifications, installed idle-write and
ARM64/Pi/package/return-path evidence remain open. This is a necessary runtime
write correction, not completion of B1, A/B/P04 or P05/P06 cutover. No production
Pi access, deployment or push is authorized by this design.

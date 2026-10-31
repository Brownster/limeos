# B1 transition-only health polling

Source `05049bb` implements the
[design](plans/2026-10-31-b1-transition-only-observations-design.md), removing
per-sample writes from the normal observation path. The existing queued path
remains available for older jobs and recovery. Both paths use the sole Store
worker and share owner-exclusive collection leases. No collector or timer is
registered by this change.

`ObservationStoreBridge::poll_begin` admits a current observation-only schedule
and concrete HealthRead scopes without writing an occurrence or job. Its private
lease binds owner/grant revision, generation, schedule, configured policy,
original due/admission time, resources, nonce and a maximum ten-second monotonic
deadline. Completion checks current authority after acquiring the transaction,
validates the whole normalized batch and previews the actual alert fold.
Unavailable signals cannot claim a complete sample.

An unchanged sample returns an explicitly unpersisted outcome. It changes no
database row, audit event, alert, notification or durable sequence. A volatile
schedule watermark prevents repeated sampling within the current interval while
the worker lives. At most 128 schedules can exist. Restart discards that
watermark and requires a fresh read; an unchanged read may repeat, without
creating duplicate durable work. Exactly-once probe execution across a crash is
not promised. Durable history timestamps remain transition timestamps.

A first sample, changed alert material, completeness or owner/policy binding
creates and finishes the existing undispatchable observation job atomically.
Threshold failures, muted incidents, re-enablement and recovery retain the real
fold and outbox rules. Normal skipped occurrences remain durable. Queue, space,
audit or SQL failures roll back every change. Unchanged reads still work when
the audit budget is full. A refusal consumes its matching poll lease; it cannot
release another poll, a queued observation or an effect claim.

Schema 14 adds immutable poll origins, a typed canonical proof event and a
deferred foreign key requiring the successful observation result in the same
commit. Startup checks original due/grace evaluation, admission-to-result time,
owner/job/event proofs and the compiled schema before recovery. Historical
occurrence detail may still be pruned under its existing rules: the immutable
job intent retains the proof identity. Completed replay needs the original
private nonce, body and current authority. Legacy canonical intent bytes remain
unchanged. Admission may finish after the schedule's grace only within its
original read lifetime.

The next producer must resolve configured paths and legacy keys to concrete
protected resources, gather fresh restart-policy/healthcheck/SMART/mount and
protection evidence, then use these poll methods. It must first settle old
queued work and retain bounded fair cursors. Overview-cache values do not prove
health. Concrete replacement, safe history/audit compaction, ordinary external
notification destinations/transports and actual-state migration remain open.

Local qualification and exact source identities are recorded in the
[evidence packet](rewrite-evidence/p04/2026-10-31-b1-transition-polling/README.md).
It passes 810 runtime tests, nine compile-fail doctests, strict Clippy, actual
contract regeneration, 242 frontend tests and the checked build. Five final
guards agree on 722 committed inputs. The isolated real-Store measurement has
zero SQLite changes, identical database/WAL bytes and zero process write bytes
over 2,880 unchanged samples after one baseline. The older queued-path baseline
records 480 SQLite changes and 8,200,192 process write bytes over 32 stable
samples. Failed and preliminary runs remain separately attributed.
The simulated-day fixture is separate from a real installed 24-hour idle-write
soak. Schema 14 needs fresh package/upgrade/return-path and ARM64/Pi evidence;
older packages do not qualify it. B1, A1 reliability, broader A/B/P04 parity,
P05's assistant and P06's migration/rehearsal remain open. No Pi access,
activation, external send or push was performed.

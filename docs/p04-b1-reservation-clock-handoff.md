# B1 reservation clock admission

Source `7f22d13` implements the
[writer-boundary design](plans/2026-10-21-b1-reservation-clock-design.md).
`Store::automation_reserve_current` takes Core's trusted clock callback and the
original monotonic deadline. It samples time after `BEGIN IMMEDIATE`, validates
the advisory occurrence and rechecks current stored authority, revision, pause,
definition and due/window decision in that transaction. Checks before the write,
after acquiring the lock, after the callback and before commit preserve the
original deadline. A database wait never renews it.

The supplied-time `automation_reserve` remains a deterministic fixture seam.
Both methods share the reservation transaction body. Production integration must
use the current-clock entry point; a tick/page timestamp supplies only a proposed
occurrence. SQL rollback preserves atomic jobs, occurrences, watermarks and audit
rows. The final deadline check precedes commit; it does not promise that SQLite's
commit/fsync finishes before that deadline.

[Source-bound local evidence](rewrite-evidence/p04/2026-10-21-b1-reservation-clock/README.md)
records 133 persistence unit tests and two compile-fail doctests, strict
all-target persistence Clippy and workspace formatting. The focused 22-case
reservation run overlaps those unit tests. Each passing record matches all 677
tracked inputs in its snapshot to the committed source.

Four new test functions cover actual SQLite lock waits with a later due slot,
a closed UTC window, deadline expiry and a successful reservation whose job
deadline uses fresh time; same-tuple replay after restart and changed owner grant
revision; expiry inside the clock callback before an injected insert trap; and
initial expiry/invalid clock refusal. Existing queue, audit, SQL rollback and
startup-corruption regressions also pass. The initial regression attempt was
refused before compilation; a later candidate build hit the unchanged 3-GiB host
memory floor. Neither is credited as a passing test or a reproduced assertion.

This adds no schema, timer, producer, collector or delivery registration. The
next runtime milestone still needs owner-scoped ordered observation completion,
current resource-specific read authority, atomic alert/outbox folding, durable
publication/acknowledgment and installed API/UI/restart qualification. Observation
jobs remain subject to the existing queued/canceled guards. No Pi operation,
package qualification, ARM64 execution or cutover acceptance ran for this slice.

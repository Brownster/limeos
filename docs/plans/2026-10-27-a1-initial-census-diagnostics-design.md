# Identify the exact initial host-consumer census conflict

The [current-source disposable run](../rewrite-evidence/p04/2026-10-27-a1-current-initial-census/README.md)
installed schema 12 and reached A1 without a host-resource stop. It refused the
first genuine plan at `scan.initial_collection`. The existing exact task-vector
diagnostic runs only after a complete collection, so it emitted no record here.
Later proc snapshots cannot establish the failed scan's identities.

Four comparison groups inside collection currently return an unlabelled
`Conflict`: kernel-task identity, per-task start/mount namespace, per-process
leader start/thread membership, and the final process list. The called helpers
already diagnose their own read, capacity and consumer failures. Add static
phases and bounded numeric before/after details at these remaining comparisons.

Preserve the exact read order and short-circuit behavior of the existing checks.
Record only values already obtained by the comparison: PID/TID, start ticks,
known namespace identities, list counts and at most four added/removed IDs.
Unknown namespace data stays explicitly absent. Do not read command lines,
environments, paths or a later proc snapshot to reconstruct a failed sample.
Keep each event below 2 KiB at maximum bounded inputs. Diagnostics are
failure-only and cannot change the returned error on serialization/write failure.

Keep every identity/membership difference as `Conflict`. Add no retry, task or
FD filtering, capacity increase, lease renewal, predicate exception or effect
dispatch. Keep the original four-second lifetime and current FD/task/namespace
budgets. Deterministic cases must verify unchanged refusal for same-count
replacements and ordering changes, exact retained data, bounded output and
unknown-data handling. Run focused/full storage checks and strict Clippy before
building a distinct source-bound candidate for one diagnostic guest run.

The purpose is to identify the actual condition, not classify an initial conflict
as harmless. Root-cause correction and genuine approved API/UI mount acceptance
remain subsequent work. Preserve the failed candidate and its evidence; never
retry its uncertain effect or overwrite its result.

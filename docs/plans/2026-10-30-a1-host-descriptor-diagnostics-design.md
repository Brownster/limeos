# Identify the exact changing host FD table

The [fresh journal-budget candidate](../rewrite-evidence/p04/2026-10-30-a1-host-fd-membership-refusal/README.md)
passed negative pre-effect cases and refused positive plan creation at
`scan.fds Conflict`. The remaining unlabelled return compares sorted FD-number
lists around per-link checks. Neither the affected PID/TID nor the two compared
lists were recorded. No journal-held execution was reached in this reproduction.

Pass the already-read process/task identity into the private `check_fds` call:
PID, TID, start ticks and the known mount-namespace identity. Label this
`owner_before`; an early FD refusal does not prove its later task revalidation.
At the existing equality check, retain full added/removed counts and at most four
numeric FD samples per side, reusing the bounded membership diagnostic helper.
Use the existing collection-change event with a static `fds.ids_changed` phase.

Obtain no later proc snapshot, path, command line, environment or FD-link text
for logging. These are FD-number membership facts, not complete open-file
identity or a harmlessness classification. Preserve exact equality, including
same-count replacement and ordering differences; preserve read order, original
four-second lifetime, poisoning, per-process FD/task/namespace limits and every
consumer predicate. Serialization or stderr failure must leave `Conflict` intact.
Keep the event below 2 KiB at maximum bounded inputs.

Test unchanged success on equal lists, exact owner-before and numeric samples,
same-count replacement and ordering refusal, full difference counts and maximum
output size. Run focused and full storage checks using the established small
fixture-only runner where filesystem semantics require it, plus strict Clippy,
formatting and executor compilation. Freeze a distinct candidate for the next
diagnostic run. The purpose is to identify the actual owner and comparison,
before any runtime correction; no filter, retry, limit increase or effect
exception is part of this slice.

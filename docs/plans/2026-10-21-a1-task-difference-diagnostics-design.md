# A1 exact task-difference diagnostics

The installed namespace-budget candidate reaches `scan.recheck.user_tasks`:
116 fresh tasks differ from 118 retained tasks. The external fixture's before
and after snapshots cannot identify the exact observations compared inside
the failed request. The disappearing identities and their cause remain unknown.

Add a failure-only structured journal record at that comparison. Report exact
removed/added `(pid, tid, start_ticks, namespace_device, namespace_inode)` values
from the two already-held bounded vectors. Include total differences, at most
four samples per side, the existing diagnostic class and explicit sample limit.
Use borrowed fixed arrays; output stays below 2 KiB even for maximum-width
integers. No new proc reads, task, polling thread, retained descriptor or caller
input is introduced. Logging failure must preserve the original refusal.

The exact vector comparison, deadline, poisoned-owner behavior, namespace and
mount checks, limits, locking and effect admission remain unchanged. Numeric
diagnostics are observations, never process ownership or effect authority.
Reordered-but-identical membership still fails the original equality check and
reports zero set additions/removals with the existing task-order class.

External sampling alone leaves the failed comparison ambiguous. Capturing whole
process metadata on every scan would add reads, races and resource costs before
the cause is known. Failure-only bounded differences provide the missing direct
evidence with the smallest runtime change.

Validate exact identity replacement, same-count changes, ordering-only changes,
truncation/counts and the encoded output bound. Rebuild current Debian binaries
from frozen inputs, then run the existing disposable A1 fixture with synchronous
process witnesses and capture the real journal. Keep prior failed attempts.
No policy exception or acceptance follows from diagnostics; A1, ARM64, resource,
recovery and Pi cutover requirements remain open until their actual tests pass.

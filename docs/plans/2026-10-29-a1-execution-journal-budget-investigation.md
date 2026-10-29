# Account for the retained mount execution journal

The [continuously guarded corrected guest](../rewrite-evidence/p04/2026-10-29-a1-execute-capacity-refusal/README.md)
reached a genuine human-approved queued job, then Root refused at
`inspect.capacity.admit`. No task-difference event occurred. The protected receipt
lookup returned `NotFound` and Core conservatively kept `needs_intervention`.
The actual resource subcondition remains unrecorded; later samples are not the
failed admission's values.

Execution opens `MountJournal` before fresh inspection, keeping its cross-process
lock, protected directory and SQLite WAL connection live. Ordinary inspection
has no journal. `Capacity::admit` currently permits only a 16-descriptor baseline;
the retained read-owner model reserves 213 total within 226. Investigate this
execution-only ownership before changing any policy or resource ceiling.

Measure actual descriptor deltas for the real journal constructor in an isolated
native child, using its existing private owned-directory fixture rather than a
mock connection or a public constructor override. Include first creation,
reopening and drop; record file descriptors while the genuine owner is live.
Keep fixtures and output on repository disk. Confirm the fixed upper bound
against the SQLite configuration and retained fields. If this supports the
hypothesis, reproduce the inspection/execution budget difference deterministically.
Otherwise instrument the exact sampled capacity subcondition before another VM.

Any correction must give execution's genuine retained journal an explicit
reservation within the unchanged aggregate 226-FD model and 256 service limit.
Preserve the ordinary inspection budget and its refusal of unrelated excess
descriptors. A caller-supplied count, report, receipt or resource claim cannot
select a larger allowance. Prefer a private typed composition seam consuming or
borrowing the real journal owner. Retain the original four-second owner window,
single flight, poisoning and all task/namespace/FD comparisons.

Do not move replay behind fresh effect checks, drop the process lock to hide
descriptors, blindly replay uncertain work, increase service ceilings or remove
resource sampling. Prove valid exact bounds and refusal immediately beyond each
operation's bound, then run focused/full executor and journal checks with strict
Clippy and formatting. Freeze a distinct candidate and use fresh empty test disks
for continuously guarded genuine mount, verification and restart/replay evidence.

This investigation does not complete A1. Installed UI, full A/B runtime, ARM64,
P05 assistant and P06 migration/return acceptance remain required for Pi cutover.

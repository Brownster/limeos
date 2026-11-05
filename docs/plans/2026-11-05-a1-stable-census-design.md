# Take the A1 host-consumer census only from a stable pair of scans

The [4 Nov run](../rewrite-evidence/p04/) refused before any effect because Docker
created one thread (PID 7055, TID 8386) between the first census scan and its
immediate recheck. Normal thread churn is not a mount consumer, but the exact
task-vector comparison cannot tell the two apart.

The operator approved one relaxation of the earlier no-retry rule, for the
read-only census scan only. `HostConsumers::capture` repeats the complete scan
inside its existing four-second window until two consecutive scans are
identical (tasks with start ticks and namespaces, namespace digests, inventory
digest). That second scan becomes the baseline and replaces the former
`scan.initial_recheck`. A scan that fails with a churn symptom
(`Conflict`, `Unavailable`, `IdentityMismatch`) breaks the pair and is not
compared; time-out, cancellation and any other failure end the capture.

Why this is safe: a mount decision is taken only against a census that was
identical twice in a row, and `check` still revalidates it exactly before and
after the other checks, so any later task, FD, mapping, namespace or mount-table
difference still poisons the owner and refuses. Scans take no effect; refusal
returns before a `MountTarget` exists, so nothing can be dispatched.

Unchanged: no task, FD or process filtering, no ignored missing entries, no
deadline extension (the scan loop checks the same `expires` before every
scan and each scan polls it), no effect retry, same task/namespace/FD/read
budgets. Still refused: any genuine consumer, no identical consecutive pair
within six scans or the window, and every difference found by a later recheck.

Bounds: at most six scans, no sleeps, at most two collections retained (two
sets of at most eight namespace descriptors) while comparing, which matches the
former baseline-plus-recheck peak.

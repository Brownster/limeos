# B1 reservation clock at the writer boundary

The accepted schedule metadata paths sample trusted time after acquiring their
SQLite snapshot/write lock. The reservation foundation instead accepts a fixed
`now` before `Store::write`. A producer that passes its tick timestamp would use
old due/window/grace decisions after a lock wait. No producer currently runs,
so this is a prerequisite to registration rather than an observed production
timer failure.

Add `Store::automation_reserve_current(expected, settlement, clock, deadline)`.
The expected page and proposed settlement remain advisory. Preserve the original
monotonic deadline before the write, after acquiring the lock, after sampling
the trusted wall clock and immediately before returning to commit. Validate the
time/key and recompute the due decision using that post-lock sample. Recheck the
current owner, grants, revision, pause and definition in the same transaction.
Stale candidates reserve nothing; expiry or an audit/SQL error rolls back every
job, occurrence, watermark and audit change.

Keep one shared reservation implementation. The supplied-time method remains
the deterministic foundation seam used by existing fixtures; it is not the
runtime admission path. Future Core producer integration must call the current
clock entry with Core's clock function and the original operation deadline.
No deadline is derived again after a wait. The callback is internal trusted
code, never a request timestamp or a caller-owned principal.

Exercise actual SQLite write-lock waits: a moved next due, a closed UTC window
and an expired original deadline. Verify the clock runs only after the wait,
and never after deadline expiry. Test same-tuple replay at a later time, changed
grants, and expiry inside the clock callback before an injected insert trap.
These are admission/rollback checks, not scheduler or restart acceptance.

No schema migration, job-state guard, collector, timer or delivery is enabled by
this change. The next runtime contract must still provide ordered owner-scoped
observation completion, current per-resource read grants, atomic alert/outbox
folding, bounded retention and restart-safe publication in the sole Store.
Those requirements are not replaced by this prerequisite. Keep this branch
unmerged until focused Rust tests, strict Clippy and existing regressions pass.
The host is currently below the unchanged 3-GiB test/build floor.

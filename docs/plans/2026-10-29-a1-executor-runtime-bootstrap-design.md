# Remove artificial mount-executor thread churn

The [fresh installed diagnostic](../rewrite-evidence/p04/2026-10-29-a1-executor-idle-thread/README.md)
passed the original initial census, then refused human approval because the
executor's own TID 7190 disappeared. Its service starts ten seconds before the
refusal. The bootstrap explicitly starts one no-op blocking worker; pinned
Tokio 1.52.3 retires idle blocking workers after ten seconds. The global exact
task comparison correctly refuses this change.

First reproduce the bootstrap behavior in an isolated native test process.
Extract the existing runtime bootstrap into a private function used by `main`
and the probe, retaining its behavior for the failing test. Compare the native
process's thread identities before bootstrap and inside its supplied future.
Exercise real Tokio Unix-stream I/O and an owned child process with piped output
to cover the asynchronous mechanisms used by the protected storage read path.
The test must fail with the current artificial startup worker.

Remove the no-op startup spawn after observing that failure. Keep the
current-thread runtime and one-thread blocking-pool ceiling for storage roles;
do not create a resident spare thread, extend idle lifetimes, ignore self tasks
or change host-task equality. The examined mount path uses synchronous protected
filesystem reads plus Tokio Unix I/O and subprocess pipes, with no blocking-pool
filesystem operations. A future unexpected worker remains subject to the same
host-census refusal. The guardian must still bootstrap before any runtime.

Verify unchanged process thread identities through the real runtime and child
I/O probe. Run executor tests, strict all-target Clippy and formatting. Preserve
the red and green records with source hashes. Build a distinct candidate for a
fresh, continuously guarded disposable run; the missing terminal guard record
from the diagnostic run cannot qualify this correction.

No four-second owner renewal, service ceiling increase, scan retry, task/FD
filter or effect exception is part of this change. Installed approved API/UI
mount acceptance and all wider A/B, assistant and migration gates stay open.

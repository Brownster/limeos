# RW-040 retained combined collection

Base: local main `56afdd6`. The closed profiles and resource session now exist.
Compose their genuine public owners in the unregistered executor library. Keep
the standalone probe's combined case blocked: collection alone cannot bound a
synchronous kernel read or reap a probe child.

## Boundary and sequence

Accept the existing four trusted `ConfigurationPaths`, requiring every file.
Use only `/run/docker.sock` and `EnginePeerPolicy::root()`. There is no endpoint,
UID, proc/sys root, clock, provider or numeric profile override. Public root,
namespace, pathname and kernel checks remain in their existing adapters.

Admit the current worker through `BudgetSession`, then collect configuration,
Engine, processes, physical dependencies, destinations and the configured graph
in that order. Enter each lexical budget phase before its adapter; commit only
after its complete successful result and a collection-window check. Derive
process bindings directly from the retained Engine owner's complete running set.
Fallibly reserve their bounded vector before copying those checked records.
Drop that temporary vector after process acquisition.

Keep all owners as locals through final serialization. Destination evidence
borrows the physical and process owners; it cannot outlive them. The configured
graph borrows checked configuration and original Engine declarations. It describes
declared pool/protection/share propagation, rather than complete physical users.

Final validation checks configuration, then Engine, then destinations (whose
existing implementation checks both physical/process parents before and after
its paths), then Engine and configuration again. No owner is reacquired and no
original time, lease, Engine byte ledger or profile is reset. This sequential
bracket proves completed comparisons, not an atomic host snapshot.

## Whole-result refusal

One four-second monotonic window begins before resource admission, stricter than
the original configuration lease. Observe wall time at every boundary, rejecting
negative time, backward observations or more than four elapsed wall seconds.
Check cancellation before and after each operation. Existing cancellation-aware
adapters also receive a callback including window expiry. A cancellation or
expired window after an operation discards even a successful result. Typed
refusals retain the failing stage and existing adapter reason where available;
no failure becomes empty success and no intermediate report is exposed.

Serialize one borrowed report through the session's fixed 256 KiB writer. Include
configuration metadata, original Engine declarations/PIDs, process, physical and
destination reports, the configured graph and resource model. Check the original
window once more before returning opaque bytes. Metadata and reports remain
past read-only observations: serialization never transfers retained authority.

This window is cooperative. An async timeout cannot terminate blocked synchronous
work; this slice deliberately makes no independent deadline, cancellation latency,
parent-death, process-reaping, peak RSS or installed-footprint claim. A future
supervisor must retain single-flight admission until all owned children are reaped.
It must establish the worker's own soft descriptor limit before this entry and
enforce termination independently. No command, RPC, route, schema, launch, service,
package, claim, lock, effect or recovery path is registered here.

## Alternatives and validation

Compose genuine owners now, leaving launch disabled, to make the reusable boundary
reviewable. Adding fake public owners would weaken authentication; enabling the
probe now would imply missing lifecycle guarantees. A self-referential aggregate
is unnecessary: local ownership permits direct borrowed serialization.

Local tests cover cancellation before/after work, discard on post-operation
failure, monotonic/wall expiry and rollback, error-stage preservation and the
real public nonroot refusal under an owned child's lowered descriptor limit.
Compile-fail checks prohibit importing report bytes as evidence. They do not
replace root-positive collection or independent supervision qualification.
Run full local Rust, strict Clippy, formatting, contracts/repository checks and
the local ARM64 harness; preserve exact source hashes and lossless logs.

Use disk worktrees, build/cache/general scratch; only precompiled existing storage
kernel fixtures use small owned and cleaned tmpfs scratch. No Pi, SSH, workstation
Docker, raw block probing, guest launch or reserved backup/external qualification
paths. Installed capability/namespace positives, original-age races, performance,
resource headroom and complete physical users remain open. P04, defect and cutover
gates do not close.

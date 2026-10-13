# Retained running destination and physical-source composition

Base `649dce8`; branch `engineer/container-destination-evidence`, with source and
build scratch under the repository on disk. The operator authorized the next
rewrite slice with local tests. Pi cutover remains deferred, and the separate
backup reconstruction worktree stays outside this change.

Estimate: 16 human engineering hours, review at 24. P04 retains 320/480 hours;
actual human effort is unmeasured. Agent/build time is not human effort.

## Boundary and choice

Compose genuine borrowed `RunningContainerProcessEvidence` and
`ContainerDependencyEvidence` in an additive, unregistered read-only library.
Retain one O_PATH destination descriptor for every running declared bind/volume.
The physical owner already retains source descriptors and raw UUID/backing facts;
the process owner already retains pidfds, proc, namespace and root descriptors.
The new owner cannot outlive either. Reports bind both original snapshots and
the complete original declarations, without importing reports as authority.

A pure mount-table join would miss a different inode or a hidden overmount. A
complete worker now would also need resource admission and independently killable
supervision. The bounded descriptor composition is the useful next seam: it adds
same-object proof while preserving existing owners and service limits. Root
success, cross-namespace behavior and installed capability policy remain separate
qualification requirements.

Require equal declaration, host-root and complete raw host mount-table identities
across the owners. Inspect every running declared destination through its retained
process root with openat2 IN_ROOT/NO_SYMLINKS/NO_MAGICLINKS/CACHED and O_PATH.
Before inode attributes, bind the descriptor's fdinfo mount ID to exactly one
retained process mount row and an explicit supported local filesystem. The row
must have the literal declared destination, selected writable state and the same
filesystem/device as the retained host source. Require equal device, inode and
regular-file/directory kind. Reconstruct the source's filesystem-relative root
from its resolved host path and host mount root; compare that complete literal
root with the process mount root. Host and process mount IDs are never equated.

Initially refuse symlink destinations, Btrfs/unknown/remote/FUSE filesystems,
ambiguous/stacked selected mounts, extra process mounts beneath a selected
destination and host sources with nested mount candidates. Refuse complete
collection rather than claiming a partial dependency graph. Stopped consumers
remain explicitly declared and uninspected in the new report, and remain in the
borrowed physical dependency report. Mountless running consumers remain covered
by the original process owner. The original full container/source identity and
meaningful literal paths are preserved.

Collection/revalidation brackets destination name/held-descriptor checks with
both original owners' full revalidation. Reopen each destination from the same
retained process root and compare current and held identities, including mode and
numeric UID/GID, without reading file data, entering namespaces or sending signals.
Each owner's original lease remains authoritative; a further four-second local
lease never renews. Any failed or reentrant revalidation poisons the new owner.
Cooperative work checks and asynchronous timeouts do not interrupt blocked
synchronous kernel calls; the future worker must enforce independent termination.

Explicit limits allow at most 32 destinations, 32 additional retained descriptors
and a 32 KiB report. Count all running mounts and serialized inputs before opening
or cloning. Existing parent owners, their complete mount tables and transient
descriptors add separate costs; this does not qualify a combined worker under
LimitNOFILE 256 or MemoryMax 64M. No cap, syscall or capability relaxation occurs.

## Implementation and validation

Own a new `executor_storage::destinations` module and its private tests, an
additive export, and dedicated docs/evidence. Add only crate-private borrowed
access to original declarations/process roots and the physical owner's sources,
plus reuse of the existing bounded digest and fdinfo mount-ID helpers. Existing
public collection and revalidation semantics remain unchanged. No new dependency,
RPC/API/schema, executor entry point, service, claim, UI or storage write belongs
to this slice.

Use synthetic mount/UUID/process facts only through private test seams, with real
scratch root and destination descriptors for resolution, type checks, retained
replacement, unlink and permission changes. Public owners remain unforgeable.
Cover directory/file/named-volume relationships, independent namespace mount IDs,
different inode on the same filesystem, wrong roots/devices/types/write state,
case/component boundaries, nested/stacked/unknown mounts, stopped/mountless
membership, declaration/table mismatches, exact count/report bounds, cancellation,
expiry, poisoned retries and borrowed-owner compile-fail checks. Retain source-
bound focused/full Rust, strict Clippy, fmt, contracts and repository evidence.
The original kernel fixtures alone use small auto-cleaned tmpfs scratch after
disk-backed compilation; general scratch and caches stay on disk.

Fresh authenticated Engine reacquisition, retained configuration integration,
actual pool/protection/share state, nested propagation, whole-worker admission,
physical consumers of unselected process mounts and root filesystem backing,
independent supervision, storage locks, claims, approvals, effects and recovery
remain open. No successful protected root/foreign namespace collection, installed
ARM64 or cutover claim is made. No Pi, Docker, raw workstation storage or frozen
Python operation is needed. P04 and every defect/performance/cutover gate stay
open; historical inventory p95 61.401 ms against 20 ms remains failed.

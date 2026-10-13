# Retained container destination evidence

`executor_storage::destinations::inspect_container_destinations` composes genuine
borrowed `ContainerDependencyEvidence` and `RunningContainerProcessEvidence`.
It adds same-object checks for each running declared bind/volume destination and
retains an O_PATH descriptor per destination. The owner borrows both parents;
serialized snapshots cannot recreate its handles or extend their lifetimes.
This is an unregistered library with no command, RPC, route or storage effect.

Admission requires equal complete declaration digests, original declaration issue
time, host-root identity and complete raw host mount-table digests. Source rows
must cover every declaration, including stopped consumers. Running declarations
must match full retained process snapshots. A destination has exactly one selected
process mount row with the literal destination, expected writable state, matching
device/filesystem and complete filesystem-relative source root. Descriptor fdinfo
mount ID is checked before inode attributes. Device, inode and file/directory kind
then match the retained host source. Host and process mount IDs are independent.
UID/GID observations preserve the collector's view; cross-view ownership and
idmapped-mount semantics remain unqualified.

The existing process owner supplies the retained process root. Linux documents
the process filesystem view through `/proc/PID/root`, including its mount namespace
and ptrace access checks. [proc_pid_root(5)](https://man7.org/linux/man-pages/man5/proc_pid_root.5.html).
Destination resolution uses openat2 IN_ROOT/NO_SYMLINKS/NO_MAGICLINKS/CACHED with
O_PATH, without namespace entry or file data reads. IN_ROOT anchors absolute
paths to the supplied root; CACHED refuses resolution that needs I/O or
revalidation. [openat2(2)](https://man7.org/linux/man-pages/man2/openat2.2.html).
Foreign namespace behavior under installed permissions still needs qualification.

The initial supported subset is flat local source/destination mounts. Nested host
mount candidates, extra process mounts beneath a selected destination, ambiguous
stacked selected mounts, destination symlinks, Btrfs, remote/FUSE and unknown
filesystems refuse the whole result. Explicit tmpfs has no block UUID claim.
Bind aliases and resolved host source paths use their actual host filesystem
roots, with literal component and case checks. Named volume name/driver/path,
full container identity and writable state remain visible.

`snapshot()` returns the checked destination rows, explicit uninspected stopped
declarations and digests of both parent snapshots. `physical_snapshot()` borrows
the original physical dependency report, including stopped consumers and the
distinction between same-filesystem and conservative shared-backing relationships.
Join these observations by full container resource and literal destination;
the digest binds the complete physical report. Physical consumers of unselected
process mounts and root filesystem backing remain unqualified. The retained
complete process mount table remains available through its original owner.

Collection/revalidation brackets destination descriptor/name checks with both
parents' full revalidation. Held and fresh destinations must retain device,
inode, mount, kind, mode and numeric UID/GID. Mutable file contents and directory
entries are outside object identity; this library reads neither file data nor
commands/environment. Each parent keeps its original lease. A further four-second
local lease and work deadline never renew. Cancellation, timeout, failed checks,
reentrancy or a dropped in-flight validation leave the new owner invalid for retry.

`DestinationLimits` is explicit: at most 32 destinations, 32 additional retained
descriptors and a 32 KiB report. Whole-count admission and bounded input hashing
precede descriptor opens; borrowed report serialization counts bytes before
cloning report rows. Destination revalidation needs one temporary destination
descriptor plus its temporary fdinfo reader. Parent owners/revalidation add their
own descriptors, tables and allocations. Whole-worker memory/descriptor admission
and independent termination of synchronous kernel work remain required under
the existing service limits.

Local tests use real scratch descriptors for O_PATH resolution, type/inode/mode,
replacement, unlink, symlink and data-read refusal checks. Device/mount/UUID/process
facts are explicitly synthetic private inputs; no public evidence owner is forged.
They cover directories/files/volumes/tmpfs, independent and coincident mount IDs,
wrong inode/root/device/kind/write state, nested/stacked/unknown mounts, stopped
and mountless membership, parent digest changes, exact bounds, cancellation,
expiry and poisoned state. Three compile-fail checks cover snapshot/owner import
and parent borrowing. Successful public root composition, process changes during
this composed operation and installed foreign namespaces remain unqualified.

Fresh authenticated Engine acquisition/reacquisition, retained planning-file
integration, actual pool/protection/share state, nested propagation, complete
affected-resource claims, storage locks, approvals, effects and recovery remain
open. The existing kernel, capability and service guards remain in place. No
P04, defect, performance or cutover gate closes; no Pi operation is included.

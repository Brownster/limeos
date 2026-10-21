# Stack reader host-view correction

Installed source `54022bb` launches the sealed stack worker successfully, but the
catalog's host-root constructor gets `EACCES` opening `/proc/1/ns/mnt` under the
reader's empty capability set. The independent Engine refusal is `openat2`
returning `ENOSYS`; it remains a separate confinement correction. Neither
unavailable source can become a successful empty inventory.

This slice retains a genuine current namespace descriptor and authenticates its
root mount against bounded kernel mount tables. It follows the existing storage
reader's mount-ID proof, additionally tying the comparison to the held root
descriptor's required `statx` mount ID. It does not add ptrace capability or change
service limits, ceilings, syscall settings, API authority or serialized contracts.

## Proof and alternatives

Linux 6.1 allocates a new mount ID for each cloned mount when copying a mount
namespace. A held root descriptor pins its mount while the proof is made. Require
its nonzero mount ID to equal the unique `/` entry in both `/proc/self/mountinfo`
and `/proc/1/mountinfo`. Compare filesystem device and mount-root fields too, so
a changed root cannot pass by matching a path or device alone. Retain the actual
current NSFS descriptor and compare a fresh current namespace identity before
returning the root. Existing owner revalidation reacquires this complete proof.

The kernel mechanism is in [Linux 6.1 namespace.c](https://github.com/torvalds/linux/blob/v6.1/fs/namespace.c),
particularly `copy_mnt_ns`, `copy_tree`, `clone_mnt`, `alloc_vfsmnt` and
`mnt_alloc_id`. Namespace cloning does not preserve the original mount objects.
The existing storage reader uses this distinction rather than requiring ptrace.

Granting `CAP_SYS_PTRACE` would enlarge the read role's authority. Keeping catalog
data permanently unavailable would leave the required runtime unfinished. The
mount-ID proof keeps the empty capability set while retaining a concrete kernel
identity check; it is not a fixture, cached report or imported authority object.

## Bounded acquisition and revalidation

Open only the two fixed proc paths. Require their descriptors to refer to procfs
regular files owned by UID 0. Read one table at a time with a 64 KiB bound, a
4 KiB row bound and at most 512 rows. Reject truncation, malformed rows, absent or
duplicate roots, zero/overflowing IDs, disagreement with the held root or a
different PID 1 root. Compare the root mount's device and mount-root bytes across
the tables, without requiring unrelated filesystem paths to be UTF-8.

Keep the original protected directory traversal, root/ancestor ownership and
identity, mount IDs, descriptor retention, lease, cancellation and final owner
validation. No JSON field or caller supplies an accepted host proof. The
supervised worker's existing four-second deadline covers acquisition.

The added peak is one proc descriptor, a 64 KiB table allocation and bounded
stack scratch, released before the next table. This fits the stack worker's
existing transient descriptor and payload reservations; its 20 MiB sampled RSS
guard and whole-service 64 MiB/256-FD/16-task limits stay unchanged. Measurements
remain required for successful collection and maximum layouts.

## Verification and remaining scope

Regress same-device private namespace IDs, root descriptor disagreement, changed
mount roots/devices, absent/duplicate roots, malformed/overflowing IDs and each
input bound. Exercise actual current namespace/procfs primitives locally. A
disposable Debian probe compares the unchanged service profile, exact syscall
controls and a real private mount namespace; it is diagnostic evidence only.

Build a distinct immutable Debian candidate after the code correction and verify
genuine catalog collection with empty capabilities. Full installed B2 acceptance
still requires the Engine confinement fix, both fresh sources, maximum layouts,
refusal/recovery, ownership, worker faults, resources and native ARM64. A1, B1,
whole-stack effects and P05/P06 remain part of the original cutover objective.
No production/Pi window or activation is implied.

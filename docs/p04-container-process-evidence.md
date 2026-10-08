# RW-040 retained running-process evidence

`limeos_executor_storage::processes` binds supplied running-container declarations
and PIDs to retained kernel observations. It provides no Docker acquisition,
operation registration, approval, claim, receipt or effect authority. Authenticate
Engine identity and each container/PID incarnation before and after collection;
then compose this evidence with fresh host source, destination and filesystem
UUID/backing evidence. A namespace mount ID does not establish a host filesystem
or physical device identity.

```rust,ignore
pub struct RunningContainerProcessBinding {
    pub container: limeos_domain::ContainerSnapshot,
    pub pid: u32,
}
pub fn inspect_running_container_processes(
    declarations: &limeos_domain::ContainerStorageInventory,
    bindings: &[RunningContainerProcessBinding],
) -> Result<RunningContainerProcessEvidence, limeos_executor_storage::Failure>;
impl RunningContainerProcessEvidence {
    pub fn snapshot(&self) -> &RunningContainerProcessSnapshot;
    pub fn mounts(&self, resource: &str) -> Option<&[ContainerProcessMount]>;
    pub fn revalidate(&self) -> Result<(), limeos_executor_storage::Failure>;
}
```

Every running declaration requires exactly one binding, including consumers with
no declared storage mounts. Resource, image, running state and started-at text
must equal the complete validated snapshot. Duplicate resources/PIDs, missing,
extra and stopped bindings refuse collection. Supported PIDs are 2 through
4,194,303. Observations are sorted by full resource; borrowed mount rows are
sorted by namespace mount ID. The declaration digest covers the entire original
inventory, including Engine identity, stopped consumers, sources and destinations.
The binding digest covers sorted complete bindings. Stopped consumers have no
entry in this running-process evidence; use existing source evidence for their
declared storage. Their absence does not freshly authenticate their Engine state.

Each process retains four private descriptors: pidfd, numeric proc directory,
mount namespace and process root. The owner also retains five protected host
context descriptors. A report contains the declaration/binding digests, host boot
UUID, host mount-table digest/root observation, original declaration timestamp,
full container snapshot/PID, start ticks, four numeric UID/GID values, namespace
device/inode, root device/inode/mount ID/mode and complete raw mount-table
digest/byte/row counts. Borrowed rows contain mount/parent IDs, device, filesystem
root, mountpoint, filesystem and writability. Raw options, propagation and source
tokens affect the digest but are not retained as additional parsed fields.
Comm, command line, environment and unrelated proc records never enter reports
or error messages.

Snapshots and nested observations implement Serialize without Deserialize. Their
public data fields can be copied or constructed as observations; they cannot
create an owner. Only collection constructs the descriptor owner. Compile-fail
tests cover report deserialization and external owner construction. Borrowed
reports/rows remain observations after expiry; callers must successfully
revalidate and complete the remaining Engine/physical composition checks before
using them in a later authority decision.

Collection polls the pidfd and reads its bounded fdinfo `Pid` through the retained
collector proc directory. That value uses the PID namespace of the procfs
instance, tying the live handle to its numeric proc directory. Retained proc
handles do not transfer to a later process that reuses the number; liveness
checks reject the old handle after exit. These kernel semantics are described in
[pidfd documentation](https://man7.org/linux/man-pages/man2/pidfd_open.2.html),
[the kernel's pidfd fdinfo implementation](https://github.com/torvalds/linux/blob/master/fs/pidfs.c)
and [proc documentation](https://docs.kernel.org/filesystems/proc.html).

Stat/status/stat reads bracket numeric credentials and start ticks. Stat comm is
opaque bytes: spaces, parentheses, newlines and invalid UTF-8 cannot shift numeric
fields into the report. Retained and freshly opened proc/namespace/root identities
are compared; complete raw tables are compared before and after these reads.
Namespace descriptors preserve identity, not an immutable mount table. Process
and mount changes can still occur immediately after the final observation;
collection is not a transaction, freeze or effect-time guarantee.

Root identity requires mandatory statx fields, directory type and a nonzero link
count. The count is a current validity check, not identity: `/proc` link counts
change as processes come and go. A root mount must appear in that process's
table. Its stat device and table device are preserved separately: Btrfs reports
an inode's subvolume device from `getattr`, which can differ from the table's
device. No device-equality fallback or physical identity inference is made.
See [Btrfs kernel getattr](https://raw.githubusercontent.com/torvalds/linux/v6.17/fs/btrfs/inode.c)
and the retained local Btrfs regression logs.

Production requires effective UID 0 and protected root/proc directories, real
procfs, matching collector/PID-1 mount-namespace device/inode identities and the
existing host root-view check. It compares held and fresh root/proc paths, both
host raw tables, namespace identity and boot UUID. Numeric PID paths are fixed
and validated; records use held directory-relative `openat2` with beneath,
no-symlink and no-magic-link resolution. Only fixed `ns/mnt` and `root` kernel
links are deliberately followed. Namespace handles must be nsfs regular files.
Missing pidfd, openat2, required statx fields, denied proc/ptrace access or an
unsupported table refuses the whole result. There are no caller-selected proc
roots, providers, imports or capability changes.

UID 0 alone is insufficient for every inspection. Namespace/root links are
subject to ptrace access checks; a service with empty capabilities may be unable
to inspect another UID or a nondumpable task. The integrator must qualify actual
credentials, proc restrictions, capability bounding set and service confinement.
This slice adds no CAP_SYS_PTRACE or other service policy and substitutes no
weaker metadata. See [process-root access rules](https://man7.org/linux/man-pages/man5/proc_pid_root.5.html).
Production observation sends no signals, enters no namespace and changes no
mount or data. Tests kill/wait only their own children and remove only private
scratch directories.

| Limit | Enforcement |
|---|---|
| 64 declarations/bindings | Existing inventory validation plus complete running membership |
| Five-second age | Wall time before/after collection and revalidation; original remaining monotonic lease; observed rollback refuses |
| Two seconds per collection/revalidation | Cooperative checks around reads, opens, parsing and final validation; collection retains its initial deadline |
| 16 KiB per record | Stat/status/fdinfo/boot reads; one overflow sentinel, no oversized append |
| 1 MiB/table; 8 MiB aggregate | Each complete process-table pass counts all consumers, including duplicate namespaces; next read is limited to remaining quota |
| 4,096 rows/table | Count before vector reservation; no unbounded split-token vector; at most 32 optional tokens per row |
| 64 KiB declaration/binding serialization | Bounded writer before declaration cloning; amplification refuses |
| 48 KiB report | Bounded writer, no omitted or clipped consumer |
| Descriptors | Five host handles plus four per binding; at 64, 261 retained and at most 265 during fresh comparisons/record reads |

The 8 MiB aggregate is the sum of process tables in each complete observation
pass. Collection performs initial, before and after passes (up to 24 MiB of
process-table read traffic); revalidation performs two (up to 16 MiB). Raw table
copies are temporary, with at most two 1 MiB process tables in a comparison.
Host checks separately read two bounded 1 MiB tables and parse one at a time.
Parsed rows persist; raw process tables do not. Revalidation starts a new work
budget but never a new declaration lifetime. Any observed revalidation failure
permanently invalidates that owner; later recovery cannot revive it.

`InvalidPlan` covers malformed declarations/bindings; `Conflict` covers stale,
dead, replaced or changed facts. `WrongNamespace` refuses the protected host
boundary. `Unavailable` covers denied/unsupported kernel access, malformed
records and resource ceilings; `TimedOut` covers cooperative work expiry.
Failure never returns a partial or empty evidence owner implying no consumers.
Synchronous kernel operations need a later independent worker-process deadline
and single-flight admission; the cooperative checks cannot interrupt a blocked
syscall. These bounds do not qualify production inventory latency or footprint.

The [local evidence handoff](rewrite-evidence/p04/rw040-process-evidence/README.md)
separates real child lifetime/descriptor proof from private injected PID,
credential, namespace, mount-table and clock interleavings. UID 1000 cannot open
PID 1's namespace here. Real child tests privately substitute only host
authentication, then delegate child operations to the production Linux adapter;
they do not qualify the production host boundary. Installed cross-UID,
nondumpable, namespace-switch, service-capability and Engine scenarios remain
untested. No Docker, root guest, namespace mutation, SSH or Pi operation ran.

No gate closes. DSK-001, MNT-001/MNT-002, RT-001 and applicable P04 rows remain
open. Engine acquisition, physical composition, pool/protection/share propagation,
approvals/shared claims, root ceilings, bounded admission, installed collection,
effect-time checks, mount/fstab/unmount execution and runtime-loss shutdown remain
integration work. The existing 61.401 ms result against the 20 ms inventory budget
is unchanged.

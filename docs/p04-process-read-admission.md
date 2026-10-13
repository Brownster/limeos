# Combined process read admission

The trusted Rust seam is
`processes::inspect_running_container_processes_with_profile(declarations,
bindings, ProcessReadProfile::CombinedV1)`. It returns the existing opaque
`RunningContainerProcessEvidence`. The original collection function continues
to use Standard. Profiles have no numeric override or serde/wire representation.
This library neither authenticates supplied Engine facts nor launches a worker.

| Bound | Standard | CombinedV1 |
|---|---:|---:|
| Complete declarations, including stopped | 64 | 16 |
| Running processes, including mountless | 64 | 8 |
| Declared mounts, including stopped consumers | 256 | 32 |
| Each host/process raw mount table | 1 MiB | 64 KiB |
| Complete rows per table | 4096 | 512 |
| Running-process raw tables per complete pass | 8 MiB | 256 KiB |
| Retained parsed mount vector/string capacities | No separate allowance | 512 KiB |
| Decoded filesystem root or mountpoint | Within raw table limit | 512 bytes |
| Filesystem name | Within raw table limit | 64 bytes |
| Retained descriptors at maximum running count | 261 | 37 |
| Linux temporary descriptors during validation | 4 | 4 |

The CombinedV1 declaration/running/mount counts match the existing
[Engine profile](p04-rw040-engine-read-admission.md). Complete membership is
checked before kernel opens, copying declarations or binding serialization.
There is no clipping or namespace deduplication. Stopped mounts consume the
same budget, while running mountless containers still retain process evidence.
Original declaration validation and exact full-snapshot/PID membership checks
remain required. Invalid membership preserves its existing error; excess profile
admission returns Unavailable and drops every partial handle.

Raw table byte limits reach the bounded proc reader before acquisition. Bytes
and row counts are checked before reserving the parsed vector. Optional tokens
use an iterator with a 32-token cap. CombinedV1 validates/counts each escaped
path before fallible allocation of its decoded output. Filesystem strings are
also bounded and fallibly allocated. Requested vector/string space must fit
the shared allowance before allocation; actual capacity is charged afterwards,
and allocator rounding beyond the allowance refuses. A partial parse never
becomes a partial successful owner.

All initial running tables share one retained payload allowance, even for equal
namespace identities. Each host table uses its own bounded scratch allowance.
Initial, held and freshly named running-table passes each have a separate
256 KiB raw counter; the remaining allowance limits the next read. Revalidation
uses the profile retained by the original owner, with the original dual-clock
lease and unchanged two-second cooperative work bound. Failed revalidation
poisons that owner. A new pass renews neither the lifetime nor retained capacity.
Declaration/binding serialization remains bounded at 64 KiB and reports at
48 KiB. Snapshots preserve their existing observation schema.

Five context descriptors plus four per running process yield 37 retained
descriptors at eight processes. Revalidation adds a fresh proc, namespace and
root plus a temporary proc reader, for at most 41 in this adapter. The local
owned-child test measured 37 retained and 41 peak above its baseline, then
confirmed descriptor cleanup. It uses real child pidfds, proc, root and namespace
handles with a private host-authentication substitution. This does not qualify
public root collection, PID 1 authentication, foreign namespaces or installed
permissions.

The private maximum-row fixture retains 4096 rows across eight processes:
425984 vector bytes and 24576 string-capacity bytes on the local 64-bit build,
with 122320 raw input bytes. Its frozen clock and synthetic kernel facts measure
admission, not elapsed-time performance. BTreeSet nodes, allocator metadata,
raw buffers, declaration/report copies, stacks, transport and runtime memory
remain outside the mount-payload allowance. This is no total-heap or RSS proof.

Local regression tests cover exact/excess counts, stopped mounts, decoded fields,
table bytes/rows, optional tokens, aggregate passes, shared parsed capacity,
refusal before opens, held/fresh validation bounds, cleanup, expiry and standard
compatibility. Existing tests retain real child exit, PID reuse and namespace,
credential, root and mount changes. No authentication fallback, kernel primitive,
capability, syscall, service or resource ceiling changed.

The reader's installed 256-descriptor, 64 MiB, zero-swap and 16-task ceilings
remain independent final guards. Storage/topology/source admission and all other
owners still need a concrete combined budget. Independent worker termination,
single-flight admission, cancellation, parent death and descendant reaping must
follow before collection is enabled. Engine/configuration integration, complete
physical consumers and claims, locks, effects, recovery, native/installed ARM64
qualification and cutover remain open. This slice performs no Pi operation.

# Retained SnapRAID source evidence

`executor_storage::protection_sources` supplies an unregistered read-only
prerequisite for the existing pool/protection planners. It executes no SnapRAID
command and grants no sync, scrub, mount or unmount authority. SRA-001, SRA-002,
DSK-001 and P04 effect gates remain open.

## API and trusted inputs

```rust,ignore
use limeos_executor_storage::protection_sources::{
    SourceLimits, inspect_protection_sources,
};

let sources = inspect_protection_sources(
    &genuine_inventory, &contract, &pools, &snapraid, limits, &cancelled,
).await?;
sources.revalidate(&contract, &pools, &snapraid, limits, &cancelled).await?;
let report = sources.snapshot();
let additional_handles = sources.retained_descriptor_count();
```

The public operation borrows genuine `StorageInventoryEvidence`, constructed
only by the existing protected raw collector. It does not accept a deserialized
inventory as authority or provide a public filesystem/clock/provider seam.
The owner cannot outlive that inventory and cannot be deserialized. Its report
uses the existing `ProtectionSourceEvidence` shape inside a versioned snapshot
with a complete configuration identity. Reports cannot reconstruct owners or
authorize effects.

Trusted typed storage, pool and SnapRAID configuration generate the requirements
through the existing planner. The caller supplies current protected configuration,
authorization, writer exclusion, physical claims and explicit limits. The full
configuration identity covers the domain/version string, storage contract,
pools, SnapRAID configuration and limits in tuple order using canonical serde
JSON and the existing identity digest. No revision number or rendered text
substitutes for complete current facts. Revalidation rejects changed identity
or limits before filesystem work.

This does not implement the complete `PoolProtectionReader` port: pool display
observations and fixed status/diff process execution remain separate adapters.
There is no route, RPC, job, schema migration, receipt, worker or service change.

## Filesystem and lifecycle checks

Every generated ordered Source/Data/Content/Parity requirement must have exactly
one current expected UUID, supported ext2/ext3/ext4/xfs filesystem, configured
serial and writable full-filesystem mount. Boot/swap backing, duplicate identity,
wrong filesystem/root/mount, missing mounts and any raw nested mount beneath a
selected root refuse the whole read. A bind or unsupported multi-device/FUSE
view is not accepted as a verified source.

A retained host-root descriptor must match the genuine inventory's root mount
ID. Parent traversal uses openat2 IN_ROOT/NO_SYMLINKS/NO_MAGICLINKS with no
fallback. O_PATH leaf opens do not wait on FIFO/device candidates. Sources/data
must be directories; content/parity may be regular single-link files or absent
leaves. An absent leaf needs its retained existing parent on the verified target
filesystem; a bare directory without genuine mount evidence is insufficient.
No file is created and no file content is read or hashed here.

Repeated Source/Data rows share one pinned path without dropping any report
row. Fresh checks reopen current names through the retained root and compare
parent/object inode, mount/device, type/mode, ownership, link count, size,
attributes and modification/change timestamps. Missing/appeared/replaced names,
symlinks, unsafe types and observed metadata changes refuse. Access times are
excluded. Source ownership/mode are observed facts, not a configuration-file
protection or writer lock. The caller remains responsible for ACL policy and
excluding same-UID/privileged writers.

The path owner has a four-second monotonic lifetime from collection start.
The borrowed inventory retains its earlier independent expiry and is revalidated
before and after whole-path checks. No blocking path operation follows that
final inventory check. The path report's wall observation time is set once at
collection start; revalidation renews neither clock. That timestamp alone does
not represent the borrowed inventory's earlier remaining lifetime. Retain and
revalidate the genuine owners at every later authority boundary.

Cancellation and local deadlines are cooperative. Synchronous kernel work can
block despite async callers; installed independent process deadlines, reaping
and resource admission remain required. Raw inventory signature/topology/swap,
fstab/name and complete mount-table revalidation stay with the existing owner.

## Bounds and resource accounting

`SourceLimits` has no defaults: 1–128 ordered paths, 1–128 new retained descriptors
and a positive report limit up to 64 KiB. Configuration and raw inventory
counting serialization is capped at 64 KiB before planner/clone work. Invalid
labels and whole-request path/descriptor admission are checked before opening
new source paths. The admission bound is conservative: one root plus two
handles for each distinct path, even when an absent leaf later needs one handle.
Existing inventories and transient traversal handles are additional.

Serialization counts before reserving the output buffer. New descriptor/report
vectors reserve fallibly; bounded domain planning/clones and digest bookkeeping
still use ordinary allocation. This is not an allocator-abort or universal heap
bound. Oversized reports return no owner; partial evidence is never a success.

The local protected-pool fixture has 33 rows, 19 distinct paths and 35 retained
handles; the ordered multi-parity fixture has 12 rows, 10 paths and 15 handles.
Those are scratch observations with synthetic mount/device/UUID facts. They do
not prove actual raw-device binding, installed ceilings, performance or ARM64
behavior. Drop closes handles without filesystem writes or cleanup/adoption.

## Remaining integration

Actual successful root/confinement collection, protected configuration sourcing,
full pool/share dependencies, independently supervised worker admission,
physical shared claims, freshness across diff execution, fixed package-owned
SnapRAID process reads, sync/scrub effects and recovery remain unfinished.
Local tests do not close SRA/DSK/P04/cutover gates. The historical storage
inventory 61.401-ms p95 against 20 ms remains failed. No workstation raw storage,
Docker, Pi/wybie/SSH, guest or frozen Python testing was performed for this slice.

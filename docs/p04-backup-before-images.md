# Private managed-target before-images

`limeos_backup_archive::before_images` copies explicitly selected existing
managed objects into private verified storage. It returns an ephemeral owner.
**BKP-001 remains open.** No target replacement or installed recovery runs.

## Caller boundary

```rust,ignore
use limeos_backup_archive::before_images::{
    BeforeImageRoot, CaptureLimits, Source, capture,
};

let sources = [Source {
    resource: "configuration",
    relative_path: "core.json",
    parent: &authorized_parent,
    name: "core.json",
    object: &authorized_readonly_file,
}];
let owner = capture(&sources, &private_root, limits, &cancelled)?;
owner.revalidate(&cancelled)?;
for entry in owner.entries() {
    let original_metadata = &entry.metadata().source;
    if let Some(mut reader) = entry.reader()? {
        // Independently captured current bytes, separate from replacement bytes.
    }
}
owner.discard()?;
```

The library never opens a managed pathname. The caller supplies read-only
object and parent descriptors, a single-component current name and trusted
resource/relative-path labels. A nonempty relative path must end in that name.
Only descriptor-relative no-follow stat uses the name; labels do not select
filesystem operations. Positional reads preserve the caller's file offset.

The caller must authorize every descriptor and label, prove protected ancestors
and mount/backing identity, exclude same-UID/privileged writers, separate private
storage from managed targets and coordinate services. These checks do not
freeze writers or establish a simultaneous multi-file snapshot. Reads may
update source access times. Cancellation is cooperative; independent worker
deadlines and descendant closure belong to the future coordinator.

## Supported metadata

Regular files have one link and an ordinary nonnegative size. Directories
capture only their own metadata; children require separate explicit selection.
Directory before-images are empty private objects, not directory-tree backups.
Special types, writable/O_PATH descriptors, hard-linked files, special mode
bits, duplicate physical objects, invalid/duplicate labels and file/descendant
collisions refuse the entire capture. Empty paths are admitted only for a
resource directory. Labels retain case and reject traversal/control characters.

The record captures numeric UID/GID, ordinary permission bits, modification and
change timestamps, device/inode and parent identity, and exact optional
`security.selinux` label bytes, bounded to 4096 bytes. The SELinux label is data;
future application needs independent current policy. ACLs, file capabilities,
user/other attributes and oversized attribute lists refuse. Required attribute
query errors refuse rather than assuming there are no attributes.

Access/birth times, inode flags, sparse extent layout, absent objects, recursive
membership, online database consistency and service state are outside this
subset. Change timestamps are evidence and cannot be restored by an executor.
No chown, metadata application, SELinux relabeling or source write occurs.

## Private lifecycle

Open an existing normalized absolute private root using openat2
NO_SYMLINKS/NO_MAGICLINKS, with no fallback. It must be effective-UID-owned,
exactly 0700, with only the optional bounded SELinux attribute. The module also
rejects direct root/source or root/parent inode overlap; this is not complete
physical alias or namespace separation proof. Root path renaming cannot
redirect the retained descriptor.

An exclusive random `before-images-<128-bit hex>` directory holds flat generated
`object-<index>` names. Private files seal to 0400; metadata-only directories to
0500. Private ownership remains the collector's, distinct from captured source
ownership. Writable handles close and reopen read-only/nonblocking/no-follow.
Before return, capture verifies:

1. Complete bounded selection and current source/parent/name facts.
2. Full exact-length source reads and independent private-copy SHA-256.
3. Object sealing and file/directory fsync.
4. Bounded canonical record, attempt fsync and root fsync.
5. Every source again, including full hashes, then the entire private set.

Borrowed readers recheck private names/stat/attributes around reads, verify the
complete EOF checksum and latch failures. `revalidate` checks all objects and
hashes. Copies remain readable after original handles close or sources change.
Records stay in memory. Neither JSON nor an old attempt can construct an owner;
there is no publication marker, promotion, scan or adoption API.

The canonical record is `{format_version:1,limits,objects}`, in declaration and
caller selection order. Object records contain original facts, hashes and
private UID/GID/mode, excluding random names and private inode/timestamp facts.
Identity is SHA-256 over `limeos.backup-before-images.record.v1`, NUL and the
complete canonical bytes. It is evidence, not a durable receipt or approval.

Cleanup preserves primary failure plus a separate cleanup report. It removes
only verified owned names/inodes, retains foreign/replaced/unverified objects,
and attempts once. Drop does not retry explicit failed cleanup. Root-fsync
failure remains observable even when names were removed. Ordinary Drop is best
effort. SIGKILL leaves an inert orphan; fresh capture leaves it untouched.

## Limits and remaining integration

`CaptureLimits` supplies 1–256 objects, positive per-file/aggregate logical byte
ceilings no greater than `i64::MAX`, and a positive record ceiling up to 16 MiB.
There are no defaults. Arithmetic is checked and dynamic vectors/strings/buffers
reserve fallibly before growth. Fixed digest/error bookkeeping uses ordinary
allocation; no process-wide allocator-abort guarantee is made.

For N selected objects, the owner retains N+2 descriptors, plus one transient
descriptor for reopen/listing. Caller source/parent/private-root descriptors,
custody, staging and replacement owners add separate costs. The local 256-file
fixture held 258 additional descriptors and 2048 logical bytes. Its record size
is an observation, not a universal heap/RSS/disk bound. This does not fit a
256-descriptor service simply by concatenating owners; whole-request admission
remains necessary. No global quota, block reservation or free-space guarantee
exists. ENOSPC refuses capture.

Production target inventory/import rules, writer exclusion, service/online-DB
snapshots, complete metadata policy, physical shared claims, durable before-image
publication/receipts, installed coordinator ceilings, fsynced live replacement
and interruption reconciliation remain unfinished. Local scratch SIGKILL/fsync
faults do not establish installed recovery or power-loss behavior. No P04 defect
or cutover gate closes; the historical storage inventory latency failure remains.

# Private restore preparation

RW-043 prepares independently verified replacement payloads and bound target
facts from a genuine `staging::VerifiedCatalog`. Its opaque owner is ephemeral.
Managed destinations are never opened or changed. **BKP-001 stays open.**

## API and trusted binding

```rust,ignore
use limeos_backup_archive::preparation::{PreparationRoot, plan, prepare};

let bound = plan(&catalog, expected_manifest, selection, limits)?;
let root = PreparationRoot::open(private_root_path)?;
let prepared = prepare(
    &catalog, &bound, &current_selection, &root, &current_limits, &cancelled,
)?;
prepared.revalidate(&cancelled)?;
for entry in prepared.entries() {
    let facts = entry.metadata();
    if let Some(mut reader) = entry.reader()? {
        // Borrow replacement bytes. No writable descriptor escapes.
    }
}
prepared.discard()?;
```

The caller owns the expected manifest, selection and finite limits.
`Selection`, `ResourceSelection`, `ManagedTarget`, `InstalledMetadata` and
`PreparationLimits` are closed data types. Selection binds a nonempty set of
resource IDs, trusted destination-root strings and explicit file/directory
targets, each with a normalized relative path and numeric UID/GID/mode.
The current complete selection and limits must equal the immutable plan; vector
order is part of equality. The expected full manifest must equal the genuine
catalog's manifest. Changed roots, metadata, limits or manifest facts refuse.

Unknown, absent, duplicate or overlapping resources, duplicate targets, missing
files, kind mismatches and file/parent collisions refuse the whole request.
Every ancestor of a nested target must be an explicit required directory.
Trusted directories may be absent from the archive. Every archive entry in a
selected resource needs an exact target policy. Unselected resources are not
copied. Empty relative paths are allowed only for resource directories. Modes
contain only ordinary permission bits; reserved UID/GID `u32::MAX` is refused.

Archive strings and permissions never select private filenames, extra
resources, roots or installed metadata. No chown runs. Metadata exposes
installed plan facts separately from actual private UID/GID/mode. Files are
privately sealed to 0400 and empty directory objects to 0500; neither is the
planned installed mode. JSON cannot construct `PreparedRestore` or
`PreparationPlan`, and borrowed readers cannot outlive the owner. Serialized
records remain evidence, not operation authority.

## Retained root and lifecycle

The caller supplies an existing normalized absolute private root owned by the
effective UID with mode exactly 0700. `openat2` rejects symlink/magic-link
components; unsupported protection refuses. Component comparisons reject root
labels overlapping selected destination labels. This is not physical alias or
mount-namespace proof. The caller owns protected ancestors, ACLs, mount/backing
identity, exclusion of same-UID/privileged writers and independent separation
from all managed destinations. Renaming the root path does not redirect its
held descriptor.

All later access is relative to retained descriptors. A random 128-bit name
creates an exclusive `preparing-<hex>` attempt, mode 0700. Objects have exclusive
flat `object-<index>` names. No archive hierarchy is extracted and no live target
or pathname-based recovery capture is opened.

Before return, preparation:

1. Validates the complete plan, current policy, limits and genuine catalog.
2. Reserves bounded bookkeeping before creating private objects.
3. Re-reads each borrowed payload with exact size and SHA-256 checks. It handles
   short/interrupted reads and writes; zero writes refuse.
4. Seals and fsyncs objects, closes writable handles and reopens read-only.
   It independently reads/hashes each private copy and checks retained/name
   identities.
5. Verifies the exact object set and encodes the bounded canonical record.
6. Fsyncs the attempt and root, then revalidates the entire catalog and all
   private objects again. Cancellation is polled before returning the owner.

`revalidate()` checks private names, types, owners/modes, size, link count,
sealed modification/change timestamps and complete hashes. Reader creation and
every read check retained object/attempt evidence; complete reads verify the
checksum. A reader latches failure. Checks detect change; read-only handles
cannot prevent same-UID or privileged writers. Caller writer exclusion remains
necessary. These are cooperative library checks, not independent deadlines.

## Catalog seam

The pinned reader stops at manifest length. It alone cannot report appended
bytes or a replaced name while its held inode remains readable. After that
concrete API proposal the operator said `continue`; this branch isolates the
read-only `VerifiedCatalog::revalidate()` seam in a separate commit for
integrator review.

Staging retains each object's sealed stat. Fresh validation checks private
root/attempt modes and name identity, exact object set, regular type, ownership,
0400 mode, single link, exact size, sealed timestamps and complete SHA-256 before
and after positional reads. Preparation calls it before copying and after root
fsync. It exposes no writer, raw descriptor, provider seam, decoder or restore
operation. Other staging behavior and all custody corrections are unchanged.

## Canonical identities

Plan identity is lowercase SHA-256 of `limeos.backup-preparation.plan.v1`, then
NUL, then canonical `serde_json` for `{version:1, manifest, selection, limits}`
in that field order. Structs use declaration order; vectors retain input order.

The prepared record is canonical JSON for
`{format_version:1, selection, limits, manifest, objects}`. Objects are ordered
by resource ID and relative path and contain the target, kind, complete
size/hash, installed facts and actual private metadata. Random names, inode
numbers and timestamps are excluded, so equivalent preparations have identical
records. Record identity uses `limeos.backup-preparation.record.v1`, NUL, then
every record byte.

Records remain in memory. No record/success marker is published, no rename runs
and no restart/adoption API exists. SIGKILL leaves inert private orphans. Restart
must recover custody using its exact trusted binding, stage a fresh genuine
catalog and obtain a fresh trusted plan. Old staging or record JSON cannot
create an owner.

## Bounds and accounting

Limits are explicit; there are no production defaults.

| Limit | Supported ceiling |
|---|---:|
| Selected resources | 1–64 |
| Objects, including required directories | 1–256 |
| Each file and aggregate logical payload | Positive, at most `i64::MAX` bytes |
| Each canonical record | 1 byte–16 MiB |
| Relative/root path | 4096 bytes, 32 components, 255 bytes per component |
| Resource ID | 128 ASCII identifier bytes |

Arithmetic is checked. Vectors, buffers, copied strings and canonical record
growth reserve fallibly before growth. Record capacity grows within its ceiling;
two 64 KiB streaming buffers may coexist. Fixed-size digests/control/error
bookkeeping use ordinary Rust allocation; injected refusal and capacity
overflow do not prove survival of every process-wide allocator abort.

For `N` objects the owner retains `N + 2` descriptors, plus caller root/catalog
handles. One extra descriptor is transient during reopen or listing. The
256-object case retains 258 additional descriptors. With `F` staged files,
collection can require `F + N + 6` descriptors excluding other caller state.
Future service ceilings must admit or refuse the whole request; this is not
installed admission evidence.

Preparation adds selected logical file bytes plus filesystem metadata.
Canonical records stay in memory. Custody and the complete staged catalog add
separate compressed/all-file disk use. No global quota, physical-block bound or
disk reservation is provided. ENOSPC refuses; a free-space check is not a
reservation.

## Failure and remaining gates

Every failure/cancellation returns no owner. `PreparationError` preserves the
primary typed failure and any cleanup report. Cleanup attempts once, checks
names against owned inodes and removes only those objects. Unknown children,
replaced names and unverified creations remain for inspection. Drop does not
silently retry a failed cleanup; ordinary Drop is best effort. Failed cleanup
root fsync reports uncertainty even if names are already gone.

Existing-target before-images remain deferred. A future operation must supply
authorized retained managed descriptors and independently copy/verify their
bytes and metadata. A pathname or hard link is not a recovery snapshot.

Still required: production inventory/limits, durable job/approval binding,
shared claims, independent worker deadlines, snapshots/service quiescing,
protected live replacements, installed metadata application, uncertainty
journals, receipts and installed recovery/power-loss rehearsal. Local SIGKILL
and injected fsync failures do not qualify them. No P04 defect or cutover gate
closes; the historical inventory latency failure remains.

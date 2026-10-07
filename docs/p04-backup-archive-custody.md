# P04 private archive custody and restart staging

RW-043 can now keep one admitted compressed archive in private custody across
a restart, re-admit it there from its actual bytes, and generate a **fresh**
`VerifiedCatalog` through the existing staging API. A published custody
record is an *admitted retained archive*: not a staged catalog, not a resumable
attempt and not restore authority. Existing catalogs stay ephemeral. **BKP-001
remains open.** The library, its tests and evidence are in
`crates/backup-archive/src/custody.rs`, `tests/custody.rs` and
[custody evidence](rewrite-evidence/p04/rw043-custody/README.md).

## API

```rust,ignore
use limeos_backup_archive::custody::{CustodyLimits, CustodyRoot, recover, retain};

let root = CustodyRoot::open(private_custody_root)?;       // existing, 0700, own UID
let limits = CustodyLimits { max_archive_bytes, max_record_bytes };
// Consumes the selected descriptor; its pathname is never used.
let retained = retain(selected_file, trusted_policy, &root, &limits, &cancelled)?;
let binding = retained.binding().clone();                  // store in the durable job record
drop(retained);                                            // closes handles, keeps the record

// After a restart, from the trusted job record only (never a directory scan):
let mut retained = recover(&root, &binding, current_policy.clone(), &limits, &cancelled)?;
let catalog = retained.stage(&current_policy, &staging_root, &cancelled)?;
// ...consume catalog readers, then catalog.discard()...
retained.discard()?;                                       // explicit, reports cleanup failures
```

`RetainedArchive` has private fields and only `retain` or `recover` construct
it, each from bytes it has just admitted. It exposes `binding()`, `manifest()`,
`stage()` and `discard()`, and no reader or writable descriptor. Dropping it
closes descriptors and keeps the published record. `discard` removes the record
name first, so an interrupted discard can never leave a recoverable record.
Each `stage` call creates a new catalog with its own ephemeral cleanup;
catalog Drop or discard never touches custody. [The usage example](../crates/backup-archive/examples/custody.rs)
runs both halves in separate processes.

## Trust boundary

The caller owns the custody root's protected ancestors, ACLs, mount identity,
space reservation and the exclusion of other writers, including processes
with the same UID and privileged processes. `CustodyRoot::open` uses
`openat2` with `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS` and requires an
existing directory owned by the effective UID with mode exactly 0700. A kernel
without `openat2` refuses. Every later operation is relative to held directory
descriptors, so renaming the root path does not redirect it. Read-only modes,
held handles and `chmod` do not make files immutable; there is no `chattr`
scheme. Same-UID or privileged writers can still change bytes, which
re-admission then detects, but cannot be prevented.

The trusted caller supplies the policy and, at recovery, the binding from its
own future durable job record. Metadata on disk never authorizes recovery:
the binding must name the record, and every bound fact is recomputed.

## Retention

1. **Validate before any state.** Check cancellation, finite limits and the
   policy, then `fstat` the selected descriptor: it must be a regular file no
   larger than `max_archive_bytes`.
2. **Generate names.** A 128-bit random record ID names `pending-<id>` and
   `retained-<id>`. An existing name of either kind is a `Collision`; nothing
   is replaced. Archive member names never become custody filenames.
3. **Copy once.** `pending-<id>/archive` is created with `O_EXCL | O_NOFOLLOW`,
   mode 0600, and the selected descriptor is copied with positional reads in
   64 KiB chunks. The copy reads at most one sentinel byte past the limit,
   retries short and interrupted reads and writes, polls cancellation, and
   refuses zero-length writes. Source-path replacement cannot redirect a held
   descriptor. The completed copy defines the archive; timestamps say nothing
   about concurrent source writers.
4. **Seal and sync.** The copy is set to 0400, checked for inode identity,
   regular type, one link, ownership, mode and size, then fsynced. The writable
   handle is closed.
5. **Reopen and admit.** The archive is reopened read-only through the pending
   directory with `O_NOFOLLOW` and checked against the created inode. It is
   rehashed against the copy digest, then admitted with
   `staging::admit_for_staging` from positional reads of that sealed copy. Only a
   complete verified-EOF admission yields the manifest and policy snapshot.
   The source is never inspected separately.
6. **Write the record.** `record.json` is written with `O_EXCL`, set to 0400 and
   fsynced, then the pending directory is fsynced.
7. **Publish.** `renameat2(RENAME_NOREPLACE)` moves `pending-<id>` to
   `retained-<id>`, followed by a root fsync. A filesystem without
   no-replace rename refuses rather than falling back.
8. **Report ambiguity.** If the root fsync fails after a successful rename,
   `AmbiguousDurability` carries the binding. The record exists but may not
   survive a crash. It is not rolled back, and a later `recover` still
   validates it fully.

Up to step 7, any failure or cancellation removes only owned names after inode
checks. A replaced name or a foreign entry is left in place and reported as
`Cleanup { failure, cleanup }`, preserving both errors. After the rename,
nothing is rolled back.

## Recovery

`recover` checks the binding's form, then opens only `retained-<record_id>`
with `O_DIRECTORY | O_NOFOLLOW`:
- **Directory:** owned by the effective UID, mode 0700, still bound to its
  name, and containing exactly `archive` and `record.json`. Any other entry,
  including a forged success marker, is `ForeignEntry`.
- **Objects:** each is opened `O_NOFOLLOW` and must be a regular file with one
  link, owned by the effective UID, mode 0400, at the bound size and still
  bound to its name.
- **Record:** at most `max_record_bytes`; read exactly, with EOF and size
  rechecked. It must deserialize as a closed record (unknown and duplicate
  fields refused) and re-serialize to exactly the stored bytes. That rejects
  trailing data, whitespace, reordering and other non-canonical encodings.
- **Binding equality:** the saved binding must equal the trusted binding.
  Both recomputed identities, the archive digest, the size and the revision
  must agree.
- **Policy:** the saved full policy must equal the caller's current policy
  (formats, limits, registry roots, mappings and their order), and the
  current policy's identity must match the binding. Revision equality alone
  is insufficient.
- **Re-admission:** the actual sealed bytes go through `admit_for_staging`.
  The whole resulting manifest, digest, size and manifest identity must match
  the record and binding, and object identities are checked again afterwards.

A same-size reordered archive is refused: its sorted entries match, but its
compressed identity does not. Recovery never deserializes a `PolicySnapshot`
owner and exposes no payload reader.

## Staging

`stage` first rechecks that the full policy equals the retained admission
policy and its identity. It then checks that the held archive inode is
unchanged and that every published name still refers to the held inodes. Next
it calls the existing `staging::replay` with the held archive (positional
reads), the admitted manifest and the snapshot. That parser owns payload
writes, checksums and fsync. The held identities are checked again after
replay, and a changed state discards the new catalog. There is no system tar,
second decoder, `unpack`, writer callback or live destination path.

## Binding, record and identity encoding

`CustodyBinding` is closed JSON (`deny_unknown_fields`):
- `format_version` (currently 1)
- `record_id` (32 lowercase hex)
- `archive_sha256` and `archive_bytes`
- `policy_identity` and `manifest_identity` (64 lowercase hex)

`record.json` is the canonical `serde_json` encoding of `{binding, policy,
manifest}` in that field order, using the domain's closed `AdmissionPolicy` and
v2 `RestoreManifest`.

Identity version 1 is the SHA-256 of a domain-separated encoding:
- **Domain:** the string `limeos.backup-custody.policy-identity.v1` or
  `limeos.backup-custody.manifest-identity.v1`, encoded as a string.
- **Integers:** fixed-width big-endian; Rust `u64` is 8 bytes and `u32` is 4.
- **Strings:** a u64 byte length followed by UTF-8 bytes.
- **Sequences:** a u64 count followed by the items in stored order.
- **Enums:** one byte. `tar_gzip` is 1 and `tar_zstd` is 2; `file` is 1 and
  `directory` is 2.
- **Options:** byte 0, or byte 1 followed by the value.
- **Policy field order:** revision, formats, the ten limits in declaration
  order, resources (id, destination root), then mappings (archive prefix,
  resource, resource prefix).
- **Manifest field order:** version, archive digest, format, policy revision,
  compressed, decompressed, header, repeat, file and directory counts, file
  bytes, then entries (resource, relative path, kind, size, optional digest,
  permissions, archive path).

The Rust encoder destructures every domain struct, so a new field fails to
compile until the encoding version changes. Encodings longer than
`max_record_bytes` are refused rather than hashed. The evidence directory holds
an independent Python implementation written from this text.

## Crash states

| Interrupted | Disk state | Recovery result |
|---|---|---|
| Before the rename (copy, archive sync, admission, record sync) | `pending-<id>` orphan, 0700 | `NotFound` for any binding; never adopted or resumed |
| After the rename, before or after the root fsync, before return | `retained-<id>` that the caller never received | Recoverable only with the exact binding and full fresh validation |
| During re-admission | Unchanged record | Recovery repeats normally |
| During replay | Unchanged record plus a staging `incomplete-*` orphan | A fresh `stage` creates a new attempt; the orphan is untouched |

A crash before the binding reaches a durable job record leaves an orphan, not a
runnable job. Automatic orphan recovery, retention cleanup and queue/receipt
reconciliation belong to the integrator.

SIGKILL and injected fsync faults do not prove power-loss durability.

## Limits and disk accounting

`CustodyLimits` are explicit trusted values:
- `max_archive_bytes` is positive and at most `i64::MAX`.
- `max_record_bytes` is positive and at most `MAX_RECORD_BYTES` (64 MiB).

Admission limits stay in the policy and are unchanged. The fixture values are
synthetic, not defaults.

Peak disk use beyond the caller's source is:
- the compressed archive (`archive_bytes`),
- plus one record (about 1.9 KiB for the measured single-file cases, at most
  `max_record_bytes`) and two directory entries,
- plus one fresh staging attempt holding every admitted file's bytes
  (`file_bytes`), while a catalog exists.

For example, a 64 MiB incompressible payload needs about 128 MiB at peak. A
64 MiB zero file in a 2 KiB zstd archive needs about 64 MiB, all in staging.
Each concurrent catalog or custody record adds its own share.

Global retained quotas and reservations belong to the caller. ENOSPC fails
safely with cleanup, and a free-space check is not a reservation.

## Preserved decisions and remaining gates

Every admission decision is unchanged. The reviewed zero-size hard-link repeat
to its exact path after a fully verified regular file remains inert and
creates no link. All other links, aliases, forward references, traversal and
collisions are refused. Structural compatibility grants no credential or
database import authority. P00 production mappings and limits remain open.

Still with the integrator:
- trusted production target and import policy
- installed job and approval binding
- complete shared claims and locks
- live destination, filesystem and mount qualification
- trusted owners and modes
- snapshots and affected-service coordination
- fsynced destination replacement
- post-effect receipt reconciliation

This library closes none of them, and no BKP-001 or P04 effect or cutover gate
closes. The historical storage-inventory result, 61.401 ms against 20 ms,
remains failed.

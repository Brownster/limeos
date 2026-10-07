# P04 verified archive staging

RW-043 now provides an owned private catalog for a future restore executor.
Manifest version remains 2. This library does not authorize or perform a
restore. **BKP-001 remains open** until the installed operation consumes this
path and proves recovery.

## API and trust boundary

`limeos_backup_archive::staging::admit_for_staging` takes a bounded `Read`, an
owned trusted `AdmissionPolicy` and a cancellation predicate. It performs
initial inspection/admission and returns `AdmittedArchive`. Its private
`PolicySnapshot` contains the complete policy and the inspected compressed
archive digest. Only successful initial admission constructs this identity;
deserializing a v2 manifest cannot construct it. Retain the admitted object
until replay. Production policy still comes from the P00 inventory.

`replay(source, current_policy, expected_manifest, snapshot, root, cancelled)`
first compares the complete policy against that snapshot, including formats,
limits, registry roots and mappings. Vector order is part of that comparison.
It checks the expected digest against initial admission and re-admits its
mapping/counts before creating any attempt. This closes the changed-order case:
sorted manifest entries can remain equal when tar order changes, but a caller
cannot substitute the new archive digest into the original admission identity.

Both passes use the inspector's same internal decoder/tar streaming function.
The replay recomputes the full compressed digest, format, compressed and
decompressed byte counts, header/repeat counts, file/directory counts, file
bytes and every mapped entry/checksum. It compares the entire resulting v2
manifest. GNU/PAX effective-name handling, integrity/EOF checks and all original
admission refusals remain on that path. No second parser or system extraction
command exists.

The caller supplies an existing absolute staging root. `StagingRoot::open`
requires a directory owned by the effective UID with mode exactly 0700 and
rejects symlinks in every path component using Linux `openat2`. The kernel must
support `openat2`; unavailable protection fails closed. The caller controls
ancestors, ACLs, mount and available disk space and must exclude other writers,
including processes with the same UID. Keep the selected archive immutable
through both passes and keep completed staging immutable until apply. Read-only
handles do not protect against another privileged writer changing an inode.

```rust,ignore
let admitted = admit_for_staging(&mut archive, trusted_policy, &cancelled)?;
archive.seek(SeekFrom::Start(0))?;
let root = StagingRoot::open(private_root)?;
let catalog = replay(
    archive,
    admitted.policy_snapshot().policy(),
    admitted.manifest(),
    admitted.policy_snapshot(),
    &root,
    &cancelled,
)?;
for entry in catalog.entries() {
    // Immutable resource/path metadata; directories have no reader.
    if let Some(reader) = entry.reader() {
        // Borrowed, bounded positional reads; no writable descriptor escapes.
        consume_verified_bytes(reader, entry.metadata())?;
    }
}
catalog.discard()?; // Observe cleanup errors; Drop otherwise tries cleanup.
```

The executable [usage example](../crates/backup-archive/examples/stage.rs)
opens one source descriptor, admits it, rewinds it, replays it, independently
hashes catalog readers and discards the attempt. It accepts `POLICY.json ARCHIVE
PRIVATE_ROOT`; root creation/permissions remain the caller's responsibility.
Use `cargo run --release -p limeos-backup-archive --example stage -- ...`.

## Tentative files and completion

Each attempt has a randomly generated `incomplete-*` directory (0700). All
payload files are flat internally generated `file-*` names, exclusively created
relative to held descriptors with `O_NOFOLLOW`, initially 0600. Archive names,
resource paths and link targets only enter catalog metadata. Directories never
create host trees. Archived ownership, special bits, xattrs and ACLs are not
applied. Literal case remains in metadata.

Chunks stay private and tentative until decoder/tar EOF, exact manifest equality
and independent staged size/SHA-256 checks pass. Each file then becomes 0400,
is fsynced and reopened read-only with inode/link-count/size checks. The attempt
directory and root are fsynced, followed by a final cancellation check. Only
then can `VerifiedCatalog` be returned. No public writer/chunk callback or
serialized success marker exposes tentative output. Catalog readers borrow
the owning catalog and maintain independent offsets.

The reviewed legacy exception is unchanged: only a zero-size hard link to its
exact own path after a fully inspected regular file is an inert repeat. It
creates no second file/link; the v2 repeat count and compressed archive binding
remain. All other links, aliases, forward references, sparse/device/FIFO members
and destination collisions are refused.

## Limits and failure handling

The compressed/decompressed/header budgets are the minimum of trusted policy
limits and expected manifest totals. Payload file count and aggregate declared
bytes are capped before file creation, with checked arithmetic. Decoder window,
per-file size, metadata and path budgets remain trusted policy limits. Read
limit checks consume at most one extra sentinel byte to detect overflow, as in
the original inspector. File data uses 64 KiB chunks; payload size does not
determine buffer allocation. Catalog/report metadata grows with bounded entry
and name counts; file descriptors grow with the bounded file count. Descriptor
exhaustion is a typed I/O refusal.

`StagingError` distinguishes admission findings, policy change, manifest
mismatch, unsafe root, quota, allocation, cancellation, I/O operations and a
combined cleanup failure. Input I/O has its own domain finding. Short and
interrupted reads/writes retry with cancellation polling. Fallible reserves
cover staging file slots, stream/verification/extension buffers, report slots
and PAX values/keys. Rust/third-party allocator aborts and decoder process death
cannot become recoverable errors; they follow the incomplete-artifact rule.

Normal refusal/cancellation and `discard` remove only owned names through held
descriptors, checking inode identity before unlinking. They never recurse into
foreign entries. Replacement of an owned name or a foreign entry obstructing
directory removal yields cleanup failure and retains the foreign object.
`Drop` attempts the same cleanup but cannot report an error; use `discard` when
cleanup must be observed. Root path renaming does not redirect operations.

SIGKILL or a machine crash may leave an owner-only `incomplete-*` attempt.
Completed attempts also keep this name: disk contents alone are never evidence
of success. There is no discovery, resume or promotion API. A later receipt
integration must distinguish a durable completed result from abandoned work;
until then orphan cleanup requires trusted ownership validation and must never
resume it automatically. These tests prove SIGKILL behavior, not power-loss or
installed service recovery.

## Remaining executor work

The catalog supplies evidence only. Integration still requires fresh human
approval, resource claims/storage locks, protected destination traversal,
policy-owned ownership/permissions, recovery snapshots and receipts, service
coordination, atomic replacement and independent post-restore verification.
Credential/database policy, online snapshots, scheduling, encryption and HTTP
routes are outside this slice. The Python reference checkout stays read-only.

Local tests, source/fixture hashes, the adversarial matrix and memory samples
are recorded in [staging evidence](rewrite-evidence/p04/rw043-staging/README.md).

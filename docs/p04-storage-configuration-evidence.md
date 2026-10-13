# Retained storage planning configuration

`limeos_executor_storage::configuration::inspect_storage_configuration` reads
four explicitly selected, protected typed JSON files: the storage contract,
pools, SnapRAID and declared Samba shares. All four are required, including
files that explicitly declare an empty configuration. The existing domain
validators and cross-file planners accept the complete set before collection
returns. This library is unregistered; it adds no executor command or API route.

`StorageConfigurationEvidence` owns the root, every traversed ancestor and four
read-only file descriptors. `configuration()` borrows the checked typed values;
they can feed `configured_storage_dependency_graph` or the existing independent
planners. `snapshot()` exposes issue time, limits, root/ancestor/file identities,
per-role raw byte digests and a canonical typed configuration digest. Whitespace
changes alter the raw digest even when the typed digest stays equal. Reports
contain metadata rather than copies of configuration bytes. Reports serialize
for inspection; JSON cannot reconstruct either reports or owners.

Public collection and revalidation require effective UID 0 in PID 1's host mount
view. Every path component must have protected ownership and permissions; files
must be regular, single-link files. Symlinks, FIFO/device/directory leaves and
unprotected ancestors refuse. Leaves are classified through O_PATH before a
nonblocking, no-follow read open; identity must match before any content read.
File device/inode/mount, mode/UID/GID, link count, size and mtime/ctime bracket
bounded positional reads. Revalidation walks names again from a fresh host root,
compares held and fresh ancestors/files, and compares both sets of file bytes
with the originals. Unrelated directory entries may change; directory timestamps
are deliberately outside configuration identity.

Revalidation consumes the original four-second monotonic lease and rejects
backward or expired wall issue time. Each pass also has a two-second cooperative
work deadline and cancellation checks. Every failure permanently poisons the
owner, including cancellation and reentrant checks. Acquire a new owner to retry.
Borrowed values and reports remain past observations after failure or expiry;
they carry no currentness or execution authority.

The caller supplies `ConfigurationLimits`: at most 64 KiB per file, 128 KiB
across the four raw file bodies, 65 retained descriptors and a 16 KiB metadata
report. Canonical paths have at most 512 UTF-8 bytes and 16 components. Admission
charges repeated ancestor descriptors before opening. Revalidation uses at most
four additional descriptors. Raw buffers reserve only admitted file sizes plus
one growth sentinel per file; parsing and serialization stay within fixed byte
bounds. Whole-worker admission still must account for this owner, other retained
owners, transient descriptors and parser allocations together.

Local tests use genuine scratch descriptors and the production traversal/read
code. A private test seam selects the scratch root and actual test UID; the
public API exposes neither override. The four frozen pool/protection layouts feed
the existing configured graph goldens. Tests cover closed JSON, explicit empty
configuration, mismatched branches, disabled records, order, missing roles,
ownership/mode and file types, replacements and edits, ancestor changes,
cancellation, expiry, poisoned retries and exact resource boundaries. Two
compile-fail checks cover owner/report non-deserialization. Successful protected
root collection and installed confinement require separate qualification.

These files describe declared planning configuration. Their identities do not
prove actual Samba global/account/include state, live pool mounts, protection
writers, authenticated Engine completeness or running namespace destinations.
Separate files are not a transaction or a writer lock. Storage locks and fresh
effect-time checks must protect later operations. The independently killable
worker, physical composition, complete claims, approvals and recovery remain
open; existing service limits stay in place. No P04, defect, performance or
cutover gate closes. The Pi remains outside this local work.

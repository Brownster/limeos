# Retained storage planning configuration

Base `0ab4870d6b689cffd314241922feadd041d056cc`; branch
`engineer/storage-configuration-evidence`. Source, build outputs, caches and
general scratch stay under the repository on disk. The operator authorized
continuing the rewrite without further check-ins. Pi cutover remains deferred;
this slice uses local fixtures only. Backup reconstruction stays with its owner.

Estimate: 12 human engineering hours; review at 18. P04 retains its 320/480-hour
estimate. Actual human effort is unmeasured. Agent/build time does not establish
the review threshold.

## Choice and boundary

The configured graph currently accepts trusted typed values. The physical and
process owners cannot establish the origin or continued identity of those values.
Add an independent read-only `executor_storage::configuration` library first.
This gives later composition a retained configuration input without expanding
the executor protocol or concatenating existing collectors beyond service caps.
Putting all collection in a new worker would also require admission and killable
supervision now; using the existing one-shot JSON reader would lose descriptors
and cannot detect replacement. Neither alternative fits this bounded slice.

The caller supplies exactly four trusted absolute paths: storage contract, pools,
SnapRAID and declared Samba shares. None is optional. Missing or unreadable files
refuse; absence never means an empty configuration. The format is the existing
closed typed JSON, not frozen Python configuration or live Samba includes. Run
the existing contract validator and pool, protection and share planners across
the complete set before returning an owner. Preserve order, case, disabled
records and numeric ownership. Reuse the integrated graph through these borrowed
typed values; this does not authenticate caller-supplied Docker declarations.

Only effective UID 0 in PID 1's host mount view may collect or revalidate publicly.
Retain the host root, every traversed ancestor and each read-only file descriptor.
Every component must be a root-owned directory or regular file without group or
other write permission, symlinks, magic links or hard-linked files. Preflight
canonical path lengths and depth before opening. Classify leaves with O_PATH;
nonblocking, no-follow read opens must match that classification before reading.

Bracket bounded positional reads with complete file identity checks. Retain the
original bytes and compare them on both held and freshly reopened descriptors.
Rewalk every name from a freshly opened host root, compare retained/fresh ancestor
device, inode, mount and ownership/mode, and reject replacement, disappearance,
permission changes, changed content or metadata. Directory timestamps are not
configuration identity: unrelated directory entries may change. File identity
includes size, links, mtime and ctime, so a same-byte rewrite still refuses.

The owner has a four-second original monotonic lease, an original wall issue time
and a two-second cooperative work deadline per pass. Revalidation consumes that
lease, rejects backward wall time and permanently poisons the owner after failure.
Cancellation also poisons it. Synchronous kernel calls still require independently
killable worker supervision later. The report is Serialize-only and cannot
reconstruct descriptors; the owner is neither cloneable nor deserializable.

Admission is explicit: up to 64 KiB per file, 128 KiB across four raw files, 65
retained descriptors and a 16 KiB metadata report. Paths are at most 512 UTF-8
bytes and 16 components. Charge root, repeated ancestors and file handles before
opening; revalidation adds at most four transient descriptors. Read and
report allocation is fallible and capped before serialization or parsing. These
are this owner's bounds, not proof that a whole combined worker fits LimitNOFILE
256 or MemoryMax 64M. Service caps remain unchanged.

## Validation and remaining work

Private tests use genuine scratch descriptors with the actual test UID and a
scratch root. Only that private authentication boundary is substituted; no public
owner can accept a UID override, proc root, fixture provider or imported report.
Cover four frozen layouts, typed graph equivalence, missing/invalid/unknown JSON,
cross-file mismatch, duplicate paths/inodes, ownership/mode, symlinks, hardlinks,
FIFO/directory leaves, same-byte edits, replacements and ancestor changes, bounded
bytes/descriptors/report, cancellation, expiry, backward time and poisoned retry.
Compile-fail tests protect the owner and report boundaries. Run focused and full
local Rust tests, fmt, strict Clippy, contracts and repository checks with exact
source hashes and lossless raw transcripts. No workstation Docker/storage probe,
Pi, SSH, VM, configuration write, service change or new dependency is needed.

This is declared planning configuration. It does not prove actual Samba global
settings/accounts/includes, pool state, protection writers, namespace destinations
or complete Engine membership. Physical composition, whole-worker admission,
storage locks, affected-resource claims, approvals, effects and recovery remain
open. P04, DSK-001, MNT-001/MNT-002, RT-001 and SRA-001/SRA-002 stay open; the
historical inventory 61.401 ms p95 against 20 ms remains failed. Current native
CI and installed ARM64 qualification require separate evidence and authorization.

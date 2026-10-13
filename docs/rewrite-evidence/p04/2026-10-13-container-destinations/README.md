# Running container destinations: local acceptance

Qualified source `93ec390fc8c5ebe3c896fb88a913f98e4d4e4a7d`, based on local main `649dce8`.
The new `executor_storage::destinations` library borrows genuine process and
physical-dependency owners, verifies flat running destinations against retained
host sources, and retains O_PATH descriptors without reading file data.
Independent namespace mount IDs, device/inode/kind, full filesystem roots and
writable state must agree. The complete parent reports remain digest-bound;
their original leases remain unchanged. See [the design](../../../plans/2026-10-13-container-destination-evidence-design.md)
and [the interface guide](../../../p04-container-destination-evidence.md).

**589 Rust cases pass:** 576 unit/integration cases and 13 compile-fail
doctests. The 27 new focused cases are already included. **61 local ARM64
harness tests pass.** All-target strict Clippy, fmt, generated contracts and
repository checks pass. [checks.json](checks.json) binds every command,
environment, runtime, source hash and original/compressed transcript hash.

Local proof uses real scratch descriptors for confined cached O_PATH resolution,
inode/type/mode/UID/GID/link checks, replacement/unlink and symlink refusal,
and refusal to read file data. Device/UUID, process membership and mount-table
facts are explicitly synthetic private inputs. Snapshot/owner import and
parent-lifetime compile-fail tests preserve the public ownership boundary.
Successful public root composition, foreign namespace resolution and process
changes during the composed operation remain unqualified. Linux interface
references in the guide explain the chosen mechanism; they are not execution
evidence for the current installed target.

Source, compilation, caches, logs and general scratch stayed on disk. Existing
kernel storage fixtures were compiled on disk before using an owned, auto-cleaned
tmpfs directory; its sampled peak was 86336 bytes in 8 files.
Existing kernel/service/capability guards were preserved. No Pi, Docker or raw
workstation storage probe was used, and the separate backup recovery worktree
stayed untouched. The earlier signed ARM64 candidate contains neither this owner
nor the subsequently integrated configuration/graph libraries.

[source-manifest.json](source-manifest.json) binds 300 source,
fixture, contract and script files to the qualified commit. Lossless gzip retains
every raw stdout/stderr byte, with a fixed timestamp and both raw/compressed
SHA-256 values. [artifact-sha256.json](artifact-sha256.json) binds the evidence
packet. The implementing agent reviewed this slice; independent review is pending.

Initial success covers flat declared source/destination relationships. Nested
propagation, destination symlinks and unsupported filesystems refuse complete
collection. Physical consumers of unselected mounts/root filesystem backing,
fresh Engine reacquisition, retained configuration integration, actual pool/share
state, whole-worker admission, independent supervision, claims, storage locks,
approvals, effects and recovery remain open. P04 and every defect, performance
and cutover gate stay open; see [scope.json](scope.json).

Reproduction uses the retained check helper with fresh disk-backed log paths.
Compile storage fixtures with `storage-compile-on-disk` before `storage-fixtures`.
Other checks use disk scratch. Root/VM/installed qualification requires separate
authorization and exact-source evidence.

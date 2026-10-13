# Retained storage configuration: local acceptance

Qualified source `a303e72fe3484643f34bc4b394e13a507cc32a1f`, based on local main `0ab4870`. The additive
`executor_storage::configuration` library retains four protected typed JSON files
and their root/ancestor/file descriptors. It validates the frozen planning rules,
compares held and freshly opened identities/bytes, preserves the original lease
and permanently refuses failed or reentrant validation. The design and API
boundary are in [the design](../../../plans/2026-10-13-storage-configuration-evidence-design.md)
and [the library guide](../../../p04-storage-configuration-evidence.md).

**559 Rust cases pass:** 549 unit/integration cases plus 10 compile-fail
doctests. The 24 focused configuration tests are already included in that total.
The four existing layouts feed the existing graph goldens. All-target strict
Clippy, fmt, generated contracts and repository checks pass. **61 ARM64 harness
tests pass locally.** [checks.json](checks.json) retains every command, runtime,
environment, source hash and original/compressed transcript hash.

Compilation, Cargo/UV caches, logs and general scratch stayed on disk. Existing
kernel storage fixtures were compiled on disk first, then used an owned tmpfs
directory that was removed after execution. Its sampled peak was
12020 bytes in 8 files. This exception preserves existing
filesystem guards; it does not put build outputs or caches on tmpfs.

[source-manifest.json](source-manifest.json) binds 298 source,
fixture, contract and script files to the qualified commit. Logs use lossless
gzip with a fixed timestamp; decompress them to recover the exact combined
stdout/stderr bytes. The [precommit diagnostic](diagnostics/README.md) records the
reentrant regression before its fix and is excluded from qualification totals.
Checks were reviewed by the implementing agent; no independent review is claimed.

Qualification is local. Private scratch-root/UID tests exercise genuine descriptors
and production traversal/read logic. Public successful root collection, installed
confinement, native CI and current installed ARM64 remain unqualified. No Pi,
workstation Docker or raw workstation storage probe was used. The separate backup
recovery worktree stayed untouched. The signed ARM64 candidate remains bound to
its earlier source and contains neither this owner nor the configured graph.

The returned configuration describes declared planning state. Actual Samba
global/account/include state, physical and running-namespace composition,
whole-worker admission/supervision, complete claims, storage locks, approvals,
effects and recovery remain open. Separate files do not establish an atomic
configuration transaction. P04 and every defect/cutover/performance gate remain
open; the historical inventory p95 failure is unchanged. See [scope.json](scope.json).

To reproduce, use a fresh disk-backed output directory in the retained check
helper. Compile storage fixtures with `storage-compile-on-disk` before invoking
`storage-fixtures`; all other checks use disk-backed scratch. Current host
qualification requires separate authorization and exact-source evidence.

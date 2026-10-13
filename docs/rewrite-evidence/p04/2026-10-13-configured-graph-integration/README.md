# Configured graph integration acceptance

Qualified merged source `bd87ee2b2a462cdf896c3ea4b68907cdc59d2210`, based on local
main `7f5dca3`. The merge preserves the engineer's design/code/evidence history at
`f5d5f6adb5219ec4286e8c8a4adba0a1e8d81c57` and combines it with the current ARM64
CI-input harness. The unfinished backup-bound recovery worktree is excluded.
The operator deferred replacing Holly's frozen application and requested that
rewrite work continue until a later readiness decision. No host was accessed.

The graph composes configured container, pool, protection and Samba consumers.
It joins full expected device metadata, retains each complete existing storage
row and stores desired share settings once. Original Docker time, numeric media
ownership, branch order, disabled/stopped/read-only consumers and unmapped
shares survive. One 512-match ceiling covers expanded container and share
routes; exceeding it refuses the whole graph. Existing preview formats and
functions stay unchanged. The [public interface](../../../p04-storage-dependency-graph.md)
remains a pure configured view with no clock, I/O, descriptor, claim or effect
authority.

Combined local acceptance passes **533 Rust cases**: 525 unit/integration cases
and eight compile-fail doctests. The workspace excluding kernel-storage passes
432; kernel-storage passes 101. The focused 12 graph cases are already included
in that total. They cover four frozen layouts, complete metadata, inherited
rows, ordering, disabled/stopped/read-only consumers, original time, literal
paths, invalid input, immutability and mixed 511/512/513 route boundaries.
The current ARM64 harness also passes **61 tests**. Strict workspace/all-target
Clippy, formatting, generated-contract drift and repository checks pass. No
runtime fix or conflict resolution was needed for this integration.

[checks.json](checks.json) records commands, exits, timings, environment and raw/
compressed log hashes. [source-manifest.json](source-manifest.json) binds the
source, fixtures, contracts and scripts to the exact qualified Git snapshot.
The retained capture/packaging scripts make the local check procedure reviewable.
Raw logs use lossless gzip with `mtime=0`; [evidence-sha256.json](evidence-sha256.json)
binds every packet file except itself. The engineer's historical transcripts
retain exact terminal blank lines through their separately recorded
[compression bindings](../rw040-configured-graph/raw-transcript-compression.json).

All worktrees, builds, compiler/UV caches, logs and general scratch stay on disk
under the repository. Storage test binaries were compiled there before runtime
fixtures used their small owned tmpfs directory. Its sampled peak was 1849 bytes
in one file at 50 ms sampling intervals; this is an observation, not a guaranteed
maximum. That directory was removed after the 101-case run. No physical disk,
production Docker/Samba operation or frozen-Python command ran.

The [integration status](integration-status.json) keeps live gates explicit.
Protected configuration revisions, physical/namespace composition, actual
share completeness, complete affected-resource claims, approvals, effect-time
revalidation, storage locks, mount/fstab/unmount effects, mount-loss shutdown
and durable recovery remain work. Empty or unmapped graph rows never authorize
an unmount or prove absence of consumers. No P04, defect-register or cutover
gate closes.

The signed ARM64 `0.4.8+arm64.3` candidate remains frozen at source `af94f19` and
does not contain this graph. Native CI, installed ARM64, actual Pi performance,
minimum container/stack UI acceptance, migration and rehearsed return are separate
pending evidence. The proposed October 13 overnight window was not authorized
and is no longer an active request. Continue locally; revisit replacement after
the supported minimum UI and installed checks justify it.

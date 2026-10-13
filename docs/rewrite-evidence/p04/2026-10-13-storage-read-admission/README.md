# Storage read admission: local acceptance

Qualified source `6c4a5e4923f20df46d8dc8ff359d1f3898585832`, based on local main `4b5744a`.
The closed CombinedV1 storage profile propagates through inventory, source and
physical dependency collection. It bounds host parsing, topology nodes/edges,
probe candidates, swap input, source references and pending symlink expansion.
Retained parents preserve their profile and original expiry. Standard observations
and existing guards remain. See [the design](../../../plans/2026-10-13-storage-read-admission-design.md)
and [interface guide](../../../p04-storage-read-admission.md).

**623 Rust cases pass:** 609 unit/integration cases and 14 compile-fail doctests.
The 21 new regression cases are already included; the source measurement rerun
adds no cases to that total. **61 local ARM64 harness tests pass**, with strict
all-target workspace Clippy, fmt, contract and repository checks.
[checks.json](checks.json) binds successful final-source commands, environments,
runtime hashes and raw/compressed transcript hashes. A redundant allocation in
one pre-acceptance test fixture was removed after Clippy reported it.

Local source fixtures use real scratch O_PATH descriptors, actual host/PID 1
mount tables and retained identity/read-refusal checks through a private nonroot
collection seam. The exact 32-source fixture measured 34 retained descriptors
above baseline and verified cleanup plus whole-count refusal. Topology/probe
counts and device/UUID facts in pure tests are synthetic; no block device was
opened or probed for this evidence. The existing parent deadlines are cooperative for synchronous work; independent
termination remains unqualified. Public root/device collection, foreign namespaces and
installed permissions remain unqualified.

The combined profile admits at most 41 retained storage descriptors and 34
retained source descriptors, with a 128 KiB parsed-table capacity allowance.
Temporary descriptors, probe child/transport pipes and other owners are outside
those retention counts. Payload exclusions are explicit in [scope.json](scope.json).
No adapter count or private measurement proves whole-worker RSS, total heap,
descriptor headroom or elapsed performance. The implementing agent reviewed this
slice; independent review remains pending.

Sources, compilation, caches, logs and general scratch stayed on disk. Storage
fixtures were compiled on disk before owned, auto-cleaned tmpfs runtime checks.
Full-storage 50 ms samples observed lower-bound peaks of 12020 bytes and
8 files; these are not exact maxima. Source measurement also used
precompiled code and an owned cleaned fixture directory. No Pi, Docker or raw
workstation storage probe was used. The reserved backup recovery worktree and
installed service/kernel/capability guards were preserved.

[source-manifest.json](source-manifest.json) binds 305 source,
fixture, contract and script files to the qualified commit. Fixed-timestamp gzip
preserves every stdout/stderr byte. [artifact-sha256.json](artifact-sha256.json)
binds the packet. Reproduce with fresh disk log paths and the retained helper;
compile storage fixtures before their tmpfs runtime checks.

Next is the concrete budget across all retained owners, then independent worker
supervision. Engine/configuration integration, complete physical consumers and
claims, actual pool/share state, locks, effects, recovery, current native and
installed ARM64 qualification stay open. The frozen signed ARM64 candidate
contains no current storage profile. P04, defect, performance and cutover gates
remain open.

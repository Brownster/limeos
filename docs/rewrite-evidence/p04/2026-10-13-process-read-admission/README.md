# Process read admission: local acceptance

Qualified source `e0ee0f3243a5a225894afbc538191044715b7f7a`, based on local main `6791dd2`.
The additive CombinedV1 process profile admits complete Engine-sized membership,
bounds raw host/process tables and decoded fields before parsing allocations,
and shares a retained vector/string-capacity allowance across running processes.
The opaque owner retains the profile and original lease through validation.
Standard collection keeps its existing limits. See [the design](../../../plans/2026-10-13-process-read-admission-design.md)
and [interface guide](../../../p04-process-read-admission.md).

**602 Rust cases pass:** 589 unit/integration cases and 13 compile-fail doctests.
The 13 new focused cases and separate measurement reruns are already included.
**61 local ARM64 harness tests pass**, with strict all-target workspace Clippy,
fmt, contract and repository checks. [checks.json](checks.json) binds commands,
environments, exact runtime hashes and both transcript hashes. Only successful
checks on the frozen committed source are acceptance evidence. One precommit
boundary fixture had an extra separator; it was corrected without changing the
parser and both exact/excess optional-token cases pass.

The owned-child measurement uses real local kernel handles and mountinfo reads
with an explicit private host-authentication substitution. It measured 37
retained and 41 peak descriptors above baseline, then verified cleanup.
The synthetic maximum-row fixture retains 4096 rows in eight processes:
425984 vector bytes plus 24576 string-capacity bytes, from 122320 raw input bytes.
Its frozen clock proves admission, not timing. These measurements do not qualify
public root/PID 1 authentication, foreign namespaces, installed permissions,
whole-worker descriptors, total heap or RSS. Payload exclusions are explicit in
[scope.json](scope.json). The implementing agent reviewed this slice; independent
review remains pending.

Sources, compilation, caches, logs and general scratch stayed on disk. Storage
kernel fixtures were compiled on disk first, then ran in an owned automatically
cleaned tmpfs directory. Its 50 ms samples observed a lower-bound peak of
61440 bytes in 8 files, not an exact maximum. No Pi,
Docker or raw workstation storage probe was used. The reserved backup recovery
worktree stayed untouched. Existing service/kernel/capability guards remain.

[source-manifest.json](source-manifest.json) binds 302 source,
fixture, contract and script files to the qualified commit. Gzip with a fixed
timestamp preserves every stdout/stderr byte; original and compressed hashes
are retained. [artifact-sha256.json](artifact-sha256.json) binds the packet.
Reproduce with the retained helper and fresh disk log paths; compile storage
fixtures before their tmpfs runtime check.

Next is storage/topology/source admission and a concrete whole-worker budget,
then independent worker supervision. Engine/configuration integration, complete
physical consumers/claims, actual pool/share state, locks, effects, recovery,
current native and installed ARM64 qualification remain open. The frozen signed
ARM64 candidate does not contain this profile. P04, defect, performance and
cutover gates stay open.

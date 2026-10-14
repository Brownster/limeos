# Typed Compose override compiler: local acceptance

Qualified source `85e6bf2f3aca7dedb621036aca87ac4ea9ded1bd`, based on local main `746eaec`.
The pure compiler produces conservative replacement managed JSON from supplied
typed operator/current/desired projects. Actual file admission, retained binding
and fresh executable approval remain unresolved. See the
[design](../../../plans/2026-10-14-compose-override-compiler-design.md) and
[interface guide](../../../p04-compose-override-compiler.md).

**813 Rust cases pass:** 780 unit/integration cases and 33 compile-fail doctests.
The 20 new cases and focused rerun are already in that total. Full workspace
checks enable the existing opt-in local worker fixture feature.
**61 local ARM64 harness tests pass**, with strict feature all-target Clippy,
fmt, contracts and repository checks. [checks.json](checks.json) binds commands,
environments, source hashes and complete merged stdout/stderr. Accepted runtime
stays unchanged through documentation and evidence edits.

18 unit cases cover two byte-exact synthetic artifacts, numeric UID/GID including
zero and u32 maximum, immutable image digests, long-form ports/volumes/binds,
explicit false read-only/privileged values, host mode and explicit device rwm.
An independent test-side JSON merge oracle checks expected supported combinations
without using production representability/delta helpers. It qualifies no pinned
installed Compose binary or arbitrary YAML behavior.

Single managed-service removal preserves unrelated operator and managed rows;
last-managed-service removal retains shutdown requirements and every unresolved
gate. Inherited service/port/mount/device removal, port/device remapping, bind/
volume kind switches and clearing inherited numeric user/host mode refuse.
Managed-only additions can disappear when replacing the whole managed file.
The existing P03 catalog keeps complete impacts and elevated privileges, without
reusing its preview approval. Invalid/unrepresentable current projects refuse too.

Ordering/IP normalization preserve borrowed input. All collection/string ceilings
refuse before cloning. Invalid desired paths/images, wildcard-port conflicts and
mount/device collisions retain P03 validation. Maximum sixteen-service/eight-mount
fixtures cover shared-name deduplication and 128 distinct volume declarations.
Oversized compact artifacts refuse; capped writers accept exact limits and never
extend/count beyond them. Two compile-fail cases prevent plan/report import.

Bounds are sixteen services/eight ports/mounts/devices per service, at most 128
named volume declarations, 64 KiB artifacts and 256 KiB counted borrowed summaries.
These are admission ceilings, not peak allocator/RSS or installed headroom proof.
The operator document is never serialized or written. No YAML body, reset/override
tag, null deletion, environment, include or interpolation adapter is added.

The initial 136-case domain suite passed. Final review added the 128-distinct-volume
boundary and an invalid-desired regression; the reviewed 137-case precommit suite
passed. Both logs remain lossless diagnostics, distinct from exact committed-source
acceptance. No failed check was discarded. The implementing agent reviewed the
code; independent review remains pending.

Actual supported source admission, retained file/Engine mapping, complete physical
consumers, principal/fresh approval, locks, atomic writes/recovery and verified
shutdown/effect-time checks remain unresolved. Shutdown names are requirements,
not receipts or deletion authority. Supplied current facts are no retained recovery
before-image. No CLI/RPC/route/schema/Core/Store/service/package/worker activation
or dependency change is included. P04/defect/installed/performance/cutover gates
remain open; the frozen signed ARM64 candidate remains unchanged.

Build/cache/general scratch stayed on disk. Existing storage fixtures compiled
there before small owned auto-cleaned tmpfs runtime scratch. 50 ms samples observed
lower-bound peaks of 36864 bytes/4 files, not exact maxima.
No Pi, SSH, Docker invocation, raw block probe, guest, reserved backup or external
qualification path was used. Service guards remain 256 FDs/64 MiB/zero swap/16 tasks.
Historical inventory p95 61.401 ms still exceeds the 20 ms target.

[source-manifest.json](source-manifest.json) binds 340 source, fixture,
contract and script files. Fixed-timestamp gzip preserves every log byte with raw/
compressed hashes; [artifact-sha256.json](artifact-sha256.json) binds this packet.
Reproduce against the qualified source using the retained helpers and fresh work/
log/output paths. Compile storage fixtures before their small tmpfs runtime run.

Next: supported source admission and independent Compose conformance, followed by
a fresh approved lifecycle operation through the existing locks/recovery boundary.
